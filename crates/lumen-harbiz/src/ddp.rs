//! DDP (Meteor's protocol) over a plain websocket: a handshake, then method
//! calls matched to their `result` by id. Server pushes (`added`, `changed`,
//! `ready`, ...) are ignored and `ping`s are answered.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

use crate::client::{Connector, DdpConnection, DdpError};
use crate::error::HarbizError;

const HARBIZ_URL: &str = "wss://app.harbiz.io/websocket";
const HARBIZ_ORIGIN: &str = "https://app.harbiz.io";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
pub struct WsConnector {
    url: String,
    origin: String,
}

impl Default for WsConnector {
    fn default() -> Self {
        Self::with_url(HARBIZ_URL, HARBIZ_ORIGIN)
    }
}

impl WsConnector {
    pub fn with_url(url: &str, origin: &str) -> Self {
        Self {
            url: url.to_string(),
            origin: origin.to_string(),
        }
    }
}

/// An explicit provider: the process-wide default is ambiguous when several
/// crypto providers are linked, and this crate must not depend on that.
fn tls_config() -> Result<Arc<rustls::ClientConfig>, HarbizError> {
    let roots = rustls::RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::aws_lc_rs::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(network)?
    .with_root_certificates(roots)
    .with_no_client_auth();
    Ok(Arc::new(config))
}

fn network(error: impl std::fmt::Display) -> HarbizError {
    HarbizError::Network(error.to_string())
}

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

async fn send(ws: &mut Socket, message: Value) -> Result<(), String> {
    ws.send(Message::text(message.to_string()))
        .await
        .map_err(|e| e.to_string())
}

/// Next JSON text frame; control and non-JSON frames are skipped.
async fn next_message(ws: &mut Socket) -> Result<Value, String> {
    loop {
        match ws.next().await {
            Some(Ok(Message::Text(text))) => {
                if let Ok(value) = serde_json::from_str::<Value>(text.as_str()) {
                    return Ok(value);
                }
            }
            Some(Ok(Message::Close(_))) | None => return Err("connection closed".to_string()),
            Some(Ok(_)) => {}
            Some(Err(e)) => return Err(e.to_string()),
        }
    }
}

async fn handshake(ws: &mut Socket) -> Result<(), String> {
    send(
        ws,
        json!({"msg": "connect", "version": "1", "support": ["1"]}),
    )
    .await?;
    loop {
        let message = next_message(ws).await?;
        match message.get("msg").and_then(Value::as_str) {
            Some("connected") => return Ok(()),
            Some("failed") => return Err("the server refused the DDP version".to_string()),
            _ => {}
        }
    }
}

#[async_trait]
impl Connector for WsConnector {
    async fn connect(&self) -> Result<Box<dyn DdpConnection>, HarbizError> {
        let mut request = self.url.as_str().into_client_request().map_err(network)?;
        let origin = self.origin.parse().map_err(network)?;
        request.headers_mut().insert("Origin", origin);
        let opened = tokio::time::timeout(CONNECT_TIMEOUT, async {
            let tls =
                tokio_tungstenite::Connector::Rustls(tls_config().map_err(|e| e.to_string())?);
            let (mut ws, _) =
                tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(tls))
                    .await
                    .map_err(|e| e.to_string())?;
            handshake(&mut ws).await?;
            Ok::<_, String>(ws)
        })
        .await
        .map_err(|_| network("timed out connecting to Harbiz"))?
        .map_err(network)?;
        Ok(Box::new(WsConnection {
            ws: opened,
            next_id: 0,
        }))
    }
}

struct WsConnection {
    ws: Socket,
    next_id: u64,
}

impl WsConnection {
    async fn await_result(&mut self, id: &str) -> Result<Value, DdpError> {
        loop {
            let message = next_message(&mut self.ws)
                .await
                .map_err(DdpError::Transport)?;
            match message.get("msg").and_then(Value::as_str) {
                Some("ping") => {
                    let mut pong = json!({"msg": "pong"});
                    if let Some(ping_id) = message.get("id") {
                        pong["id"] = ping_id.clone();
                    }
                    send(&mut self.ws, pong)
                        .await
                        .map_err(DdpError::Transport)?;
                }
                Some("result") if message.get("id").and_then(Value::as_str) == Some(id) => {
                    return match message.get("error") {
                        Some(error) => Err(method_error(error)),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                }
                _ => {}
            }
        }
    }
}

fn method_error(error: &Value) -> DdpError {
    let code = match error.get("error") {
        Some(Value::String(slug)) => slug.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    };
    let reason = error
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    DdpError::Method {
        error: code,
        reason,
    }
}

#[async_trait]
impl DdpConnection for WsConnection {
    async fn call(&mut self, method: &str, params: Vec<Value>) -> Result<Value, DdpError> {
        self.next_id += 1;
        let id = self.next_id.to_string();
        let message = json!({"msg": "method", "method": method, "params": params, "id": id});
        send(&mut self.ws, message)
            .await
            .map_err(DdpError::Transport)?;
        tokio::time::timeout(CALL_TIMEOUT, self.await_result(&id))
            .await
            .map_err(|_| DdpError::Transport(format!("{method} timed out")))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    type Server = WebSocketStream<TcpStream>;

    async fn next_json(ws: &mut Server) -> Value {
        loop {
            if let Some(Ok(Message::Text(text))) = ws.next().await {
                return serde_json::from_str(text.as_str()).unwrap();
            }
        }
    }

    async fn send_json(ws: &mut Server, value: Value) {
        ws.send(Message::text(value.to_string())).await.unwrap();
    }

    /// Accepts one client, completes the DDP handshake, then hands the socket to `script`.
    async fn serve<F, Fut>(script: F) -> WsConnector
    where
        F: FnOnce(Server) -> Fut + Send + 'static,
        Fut: std::future::Future<Output = ()> + Send,
    {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let hello = next_json(&mut ws).await;
            assert_eq!(hello["msg"], "connect");
            assert_eq!(hello["version"], "1");
            send_json(&mut ws, json!({"msg": "connected", "session": "s"})).await;
            script(ws).await;
        });
        WsConnector::with_url(&format!("ws://{addr}/websocket"), "https://app.harbiz.io")
    }

    #[test]
    fn the_tls_config_does_not_rely_on_a_process_default_provider() {
        assert!(tls_config().is_ok());
    }

    #[tokio::test]
    async fn a_method_call_returns_its_result_and_skips_noise() {
        let connector = serve(|mut ws| async move {
            let call = next_json(&mut ws).await;
            assert_eq!(call["msg"], "method");
            assert_eq!(call["method"], "echo");
            assert_eq!(call["params"], json!([1, "two"]));
            let id = call["id"].clone();
            send_json(
                &mut ws,
                json!({"msg": "added", "collection": "x", "id": "1"}),
            )
            .await;
            send_json(
                &mut ws,
                json!({"msg": "result", "id": "other", "result": "stale"}),
            )
            .await;
            send_json(&mut ws, json!({"msg": "ping", "id": "p"})).await;
            send_json(
                &mut ws,
                json!({"msg": "result", "id": id, "result": {"ok": true}}),
            )
            .await;
            let pong = next_json(&mut ws).await;
            assert_eq!(pong["msg"], "pong");
            assert_eq!(pong["id"], "p");
        })
        .await;
        let mut conn = connector.connect().await.unwrap();
        let result = conn
            .call("echo", vec![json!(1), json!("two")])
            .await
            .unwrap();
        assert_eq!(result, json!({"ok": true}));
    }

    #[tokio::test]
    async fn a_meteor_error_becomes_a_method_error() {
        let connector = serve(|mut ws| async move {
            let id = next_json(&mut ws).await["id"].clone();
            send_json(
                &mut ws,
                json!({"msg": "result", "id": id,
                       "error": {"error": 403, "reason": "Incorrect password"}}),
            )
            .await;
        })
        .await;
        let mut conn = connector.connect().await.unwrap();
        let err = conn.call("login", vec![]).await.unwrap_err();
        assert_eq!(
            err,
            DdpError::Method {
                error: "403".to_string(),
                reason: "Incorrect password".to_string()
            }
        );
    }

    #[tokio::test]
    async fn a_closed_socket_is_a_transport_error() {
        let connector = serve(|mut ws| async move {
            let _ = next_json(&mut ws).await;
            drop(ws);
        })
        .await;
        let mut conn = connector.connect().await.unwrap();
        let err = conn.call("login", vec![]).await.unwrap_err();
        assert!(matches!(err, DdpError::Transport(_)));
    }

    #[tokio::test]
    async fn a_failed_handshake_is_a_network_error() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let _ = next_json(&mut ws).await;
            send_json(&mut ws, json!({"msg": "failed", "version": "pre1"})).await;
        });
        let connector = WsConnector::with_url(&format!("ws://{addr}/websocket"), "https://x.y");
        assert!(matches!(
            connector.connect().await.err(),
            Some(HarbizError::Network(_))
        ));
    }

    #[tokio::test]
    async fn an_unreachable_server_is_a_network_error() {
        let connector = WsConnector::with_url("ws://127.0.0.1:1/websocket", "https://x.y");
        assert!(matches!(
            connector.connect().await.err(),
            Some(HarbizError::Network(_))
        ));
    }
}
