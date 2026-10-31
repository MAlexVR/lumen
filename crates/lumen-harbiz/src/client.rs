//! The Harbiz API over a DDP connection. The connection is a trait so the paging
//! and login logic are tested against a scripted fake; the websocket
//! implementation lives in `ddp.rs`.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::credentials::{Login, Session};
use crate::error::HarbizError;
use crate::parse::{parse_events, Parsed};

/// A failed DDP method call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdpError {
    /// The server answered with a Meteor error (`error` is a code or a slug).
    Method {
        error: String,
        reason: String,
    },
    Transport(String),
}

#[async_trait]
pub trait DdpConnection: Send {
    async fn call(&mut self, method: &str, params: Vec<Value>) -> Result<Value, DdpError>;
}

#[async_trait]
pub trait Connector: Send + Sync {
    async fn connect(&self) -> Result<Box<dyn DdpConnection>, HarbizError>;
}

#[async_trait]
pub trait HarbizApi: Send + Sync {
    /// Every workout on the agenda, past and scheduled, oldest first.
    async fn workouts(&self, now_ms: i64) -> Result<Parsed, HarbizError>;
}

/// Method that lists a client's agenda: `-1` pages into the past using the
/// oldest date seen as the cursor; `1` answers with the upcoming events.
const AGENDA_METHOD: &str = "getAllCalEventsClient";
const MAX_PAGES: usize = 200;
const DAY_MS: i64 = 86_400_000;

fn invalid(what: impl Into<String>) -> HarbizError {
    HarbizError::InvalidResponse(what.into())
}

impl From<DdpError> for HarbizError {
    fn from(error: DdpError) -> Self {
        match error {
            DdpError::Transport(message) => HarbizError::Network(message),
            DdpError::Method { error, reason } if error == "too-many-requests" => {
                HarbizError::Network(reason)
            }
            DdpError::Method { error, reason } => invalid(format!("{error}: {reason}")),
        }
    }
}

fn is_forbidden(error: &DdpError) -> bool {
    matches!(error, DdpError::Method { error, .. } if error == "403")
}

pub async fn login(connector: &dyn Connector, login: &Login) -> Result<Session, HarbizError> {
    let mut conn = connector.connect().await?;
    let params = json!({
        "user": {"email": login.email()},
        "password": {"digest": login.password_digest(), "algorithm": "sha-256"},
    });
    let reply = conn.call("login", vec![params]).await.map_err(|error| {
        if is_forbidden(&error) {
            HarbizError::InvalidCredentials
        } else {
            error.into()
        }
    })?;
    let token = reply
        .get("token")
        .and_then(Value::as_str)
        .ok_or_else(|| invalid("the login reply has no session token"))?;
    Session::new(token)
}

fn event_ms(event: &Value) -> Result<i64, HarbizError> {
    event
        .pointer("/date/$date")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("an agenda event has no date"))
}

/// Events per agenda page, as the server serves them.
const PAGE_SIZE: usize = 50;

async fn agenda_page(
    conn: &mut dyn DdpConnection,
    direction: i64,
    cursor_ms: i64,
) -> Result<Vec<Value>, HarbizError> {
    let params = vec![json!(direction), json!({"$date": cursor_ms})];
    match conn.call(AGENDA_METHOD, params).await? {
        Value::Array(events) => Ok(events),
        _ => Err(invalid("the agenda page is not a list")),
    }
}

/// The past pages back: each page starts before the oldest event of the last.
async fn collect_past(
    conn: &mut dyn DdpConnection,
    start_ms: i64,
    out: &mut Vec<Value>,
) -> Result<(), HarbizError> {
    let mut cursor = start_ms;
    for _ in 0..MAX_PAGES {
        let events = agenda_page(conn, -1, cursor).await?;
        if events.is_empty() {
            return Ok(());
        }
        let dates = events.iter().map(event_ms).collect::<Result<Vec<_>, _>>()?;
        cursor = dates
            .into_iter()
            .min()
            .filter(|&oldest| oldest < cursor)
            .ok_or_else(|| invalid("the agenda pagination did not advance"))?;
        out.extend(events);
    }
    Err(invalid("the agenda has too many pages"))
}

/// The server ignores the cursor going forward (it always answers with what is
/// next), so the future cannot be paged: a full page may hide more events.
async fn collect_upcoming(
    conn: &mut dyn DdpConnection,
    start_ms: i64,
    out: &mut Vec<Value>,
) -> Result<(), HarbizError> {
    let events = agenda_page(conn, 1, start_ms).await?;
    if events.len() >= PAGE_SIZE {
        return Err(invalid("the upcoming agenda may be truncated"));
    }
    out.extend(events);
    Ok(())
}

fn unique_by_id(events: Vec<Value>) -> Vec<Value> {
    let mut seen = std::collections::HashSet::new();
    events
        .into_iter()
        .filter(|e| match e.get("_id").and_then(Value::as_str) {
            Some(id) => seen.insert(id.to_string()),
            None => true,
        })
        .collect()
}

pub struct HarbizClient {
    connector: Arc<dyn Connector>,
    session: Session,
}

impl HarbizClient {
    pub fn new(connector: Arc<dyn Connector>, session: Session) -> Self {
        Self { connector, session }
    }
}

#[async_trait]
impl HarbizApi for HarbizClient {
    async fn workouts(&self, now_ms: i64) -> Result<Parsed, HarbizError> {
        let mut conn = self.connector.connect().await?;
        conn.call("login", vec![json!({"resume": self.session.token()})])
            .await
            .map_err(|error| {
                if is_forbidden(&error) {
                    HarbizError::Unauthorized
                } else {
                    error.into()
                }
            })?;
        let today = now_ms.div_euclid(DAY_MS) * DAY_MS;
        let mut events = Vec::new();
        collect_past(conn.as_mut(), today, &mut events).await?;
        collect_upcoming(conn.as_mut(), today, &mut events).await?;
        parse_events(&Value::Array(unique_by_id(events)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    type Calls = Arc<Mutex<Vec<(String, Vec<Value>)>>>;

    struct Fake {
        calls: Calls,
        replies: VecDeque<Result<Value, DdpError>>,
    }

    #[async_trait]
    impl DdpConnection for Fake {
        async fn call(&mut self, method: &str, params: Vec<Value>) -> Result<Value, DdpError> {
            self.calls
                .lock()
                .unwrap()
                .push((method.to_string(), params));
            self.replies
                .pop_front()
                .unwrap_or_else(|| Err(DdpError::Transport("script exhausted".to_string())))
        }
    }

    struct FakeConnector {
        conn: Mutex<Option<Fake>>,
    }

    #[async_trait]
    impl Connector for FakeConnector {
        async fn connect(&self) -> Result<Box<dyn DdpConnection>, HarbizError> {
            match self.conn.lock().unwrap().take() {
                Some(fake) => Ok(Box::new(fake)),
                None => Err(HarbizError::Network("no more connections".to_string())),
            }
        }
    }

    fn scripted(replies: Vec<Result<Value, DdpError>>) -> (Arc<FakeConnector>, Calls) {
        let calls: Calls = Arc::default();
        let conn = Fake {
            calls: calls.clone(),
            replies: replies.into(),
        };
        let connector = Arc::new(FakeConnector {
            conn: Mutex::new(Some(conn)),
        });
        (connector, calls)
    }

    const DAY: i64 = 86_400_000;
    const NOW: i64 = 1_793_350_000_000;
    const CURSOR: i64 = NOW / DAY * DAY;

    fn event(id: &str, ms: i64) -> Value {
        json!({"_id": id, "type": "workout", "title": id, "completed": true,
               "date": {"$date": ms}, "workout": {"exercises": []}})
    }

    fn session() -> Session {
        Session::new("resume-token").unwrap()
    }

    fn client(replies: Vec<Result<Value, DdpError>>) -> (HarbizClient, Calls) {
        let (connector, calls) = scripted(replies);
        (HarbizClient::new(connector, session()), calls)
    }

    fn ok(v: Value) -> Result<Value, DdpError> {
        Ok(v)
    }

    fn method_error(error: &str, reason: &str) -> DdpError {
        DdpError::Method {
            error: error.to_string(),
            reason: reason.to_string(),
        }
    }

    #[tokio::test]
    async fn pages_the_past_back_until_an_empty_page_then_reads_the_future() {
        let (client, calls) = client(vec![
            ok(json!({"id": "u", "token": "t"})),
            ok(json!([
                event("oct", CURSOR - 2 * DAY),
                event("sep", CURSOR - 9 * DAY)
            ])),
            ok(json!([event("jun", CURSOR - 100 * DAY)])),
            ok(json!([])),
            ok(json!([event("next", CURSOR + 2 * DAY)])),
        ]);
        let parsed = client.workouts(NOW).await.unwrap();
        let ids: Vec<_> = parsed.workouts.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["jun", "sep", "oct", "next"]);
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls[0],
            ("login".to_string(), vec![json!({"resume": "resume-token"})])
        );
        let agenda: Vec<_> = calls[1..]
            .iter()
            .map(|(m, p)| (m.as_str(), p.clone()))
            .collect();
        assert_eq!(
            agenda,
            [
                (
                    "getAllCalEventsClient",
                    vec![json!(-1), json!({"$date": CURSOR})]
                ),
                (
                    "getAllCalEventsClient",
                    vec![json!(-1), json!({"$date": CURSOR - 9 * DAY})]
                ),
                (
                    "getAllCalEventsClient",
                    vec![json!(-1), json!({"$date": CURSOR - 100 * DAY})]
                ),
                (
                    "getAllCalEventsClient",
                    vec![json!(1), json!({"$date": CURSOR})]
                ),
            ]
        );
    }

    #[tokio::test]
    async fn events_seen_on_both_sides_of_the_cursor_appear_once() {
        let (client, _) = client(vec![
            ok(json!({"token": "t"})),
            ok(json!([event("dup", CURSOR - DAY)])),
            ok(json!([])),
            ok(json!([
                event("dup", CURSOR - DAY),
                event("later", CURSOR + DAY)
            ])),
            ok(json!([])),
        ]);
        let parsed = client.workouts(NOW).await.unwrap();
        let ids: Vec<_> = parsed.workouts.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["dup", "later"]);
    }

    #[tokio::test]
    async fn a_page_that_does_not_advance_is_an_error_not_a_loop() {
        let page = json!([event("same", CURSOR - DAY)]);
        let (client, _) = client(vec![
            ok(json!({"token": "t"})),
            ok(page.clone()),
            ok(page.clone()),
        ]);
        let err = client.workouts(NOW).await.unwrap_err();
        assert!(matches!(err, HarbizError::InvalidResponse(m) if m.contains("advance")));
    }

    #[tokio::test]
    async fn a_full_future_page_is_an_error_because_it_cannot_be_paged() {
        let full: Vec<Value> = (0..PAGE_SIZE)
            .map(|i| event(&format!("f{i}"), CURSOR + (i as i64 + 1) * DAY))
            .collect();
        let (client, _) = client(vec![
            ok(json!({"token": "t"})),
            ok(json!([])),
            ok(Value::Array(full)),
        ]);
        let err = client.workouts(NOW).await.unwrap_err();
        assert!(matches!(err, HarbizError::InvalidResponse(m) if m.contains("truncated")));
    }

    #[tokio::test]
    async fn a_rejected_resume_token_means_the_session_expired() {
        let (client, calls) = client(vec![Err(method_error("403", "You've been logged out"))]);
        assert_eq!(
            client.workouts(NOW).await.unwrap_err(),
            HarbizError::Unauthorized
        );
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn transport_failures_are_network_errors() {
        let (client, _) = client(vec![
            ok(json!({"token": "t"})),
            Err(DdpError::Transport("connection reset".to_string())),
        ]);
        assert!(matches!(
            client.workouts(NOW).await.unwrap_err(),
            HarbizError::Network(m) if m.contains("connection reset")
        ));
    }

    #[tokio::test]
    async fn login_sends_the_digest_and_keeps_only_the_token() {
        let (connector, calls) = scripted(vec![ok(json!({"id": "u", "token": "fresh-token"}))]);
        let login_input = Login::new("me@example.com", "password").unwrap();
        let session = login(connector.as_ref(), &login_input).await.unwrap();
        assert_eq!(session.token(), "fresh-token");
        let calls = calls.lock().unwrap();
        assert_eq!(calls[0].0, "login");
        assert_eq!(
            calls[0].1,
            vec![json!({
                "user": {"email": "me@example.com"},
                "password": {"digest": login_input.password_digest(), "algorithm": "sha-256"}
            })]
        );
        assert!(!serde_json::to_string(&calls[0].1)
            .unwrap()
            .contains("\"password\":\"password\""));
    }

    #[tokio::test]
    async fn a_wrong_password_is_invalid_credentials() {
        for reason in ["Incorrect password", "User not found"] {
            let (connector, _) = scripted(vec![Err(method_error("403", reason))]);
            let login_input = Login::new("me@example.com", "nope").unwrap();
            assert_eq!(
                login(connector.as_ref(), &login_input).await.unwrap_err(),
                HarbizError::InvalidCredentials
            );
        }
    }

    #[tokio::test]
    async fn a_login_reply_without_a_token_is_an_invalid_response() {
        let (connector, _) = scripted(vec![ok(json!({"id": "u"}))]);
        let login_input = Login::new("me@example.com", "x").unwrap();
        assert!(matches!(
            login(connector.as_ref(), &login_input).await.unwrap_err(),
            HarbizError::InvalidResponse(_)
        ));
    }
}
