//! Sanitizer for raw Udemy responses. A diagnostic may only ever persist
//! what passes through here: structure, keys and value TYPES are kept; every
//! secret, personal or opaque value is replaced. Pure and heavily tested.
//!
//! Decisions: course and lecture ids are kept (needed to correlate), user ids
//! (any `_class: "user"` object, including instructors) are redacted; titles,
//! file names, descriptions and article bodies are replaced by length
//! placeholders unless `include_titles` is set, because they are personal
//! library information.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};

/// Values shorter than this are never treated as known secrets.
const MIN_SECRET_LEN: usize = 6;
const MAX_DEPTH: usize = 64;
const OPAQUE_MIN_LEN: usize = 24;
const OPAQUE_MIN_ENTROPY: f64 = 3.5;

/// Key fragments (lowercase) whose values are secrets.
const SECRET_KEYS: [&str; 13] = [
    "token",
    "authorization",
    "license",
    "secret",
    "password",
    "cookie",
    "signature",
    "credential",
    "session",
    "csrf",
    "api_key",
    "apikey",
    "email",
];
/// Exact key names (lowercase) holding a person's name or profile text.
const PERSON_KEYS: [&str; 8] = [
    "name",
    "display_name",
    "first_name",
    "last_name",
    "full_name",
    "job_title",
    "initials",
    "bio",
];
const TITLE_KEYS: [&str; 2] = ["title", "filename"];
const TEXT_KEYS: [&str; 2] = ["description", "body"];

static JWT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)bearer\s+[A-Za-z0-9._~+/=-]+").unwrap_or_else(|_| unreachable!("static regex"))
});
static EMAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")
        .unwrap_or_else(|_| unreachable!("static regex"))
});
static UUID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$")
        .unwrap_or_else(|_| unreachable!("static regex"))
});

#[derive(Debug, Clone, Default)]
pub struct SanitizeOptions {
    /// Keep titles, file names, descriptions and bodies (opt-in).
    pub include_titles: bool,
    /// Exact values (token, cookie values, ids) to remove wherever they occur.
    pub secrets: Vec<String>,
}

fn shannon_entropy(s: &str) -> f64 {
    let mut counts = std::collections::HashMap::new();
    let mut total = 0f64;
    for c in s.chars() {
        *counts.entry(c).or_insert(0f64) += 1.0;
        total += 1.0;
    }
    counts
        .values()
        .map(|n| {
            let p = n / total;
            -p * p.log2()
        })
        .sum()
}

fn is_opaque(s: &str) -> bool {
    let len = s.chars().count();
    (len >= OPAQUE_MIN_LEN
        && !s.chars().any(char::is_whitespace)
        && shannon_entropy(s) >= OPAQUE_MIN_ENTROPY)
        || UUID.is_match(s)
}

/// Removes exact secrets from `text` (final defence on serialized output).
pub fn scrub_text(text: &str, secrets: &[String]) -> String {
    secrets
        .iter()
        .filter(|s| s.len() >= MIN_SECRET_LEN)
        .fold(text.to_string(), |acc, s| {
            acc.replace(s.as_str(), "<secret>")
        })
}

fn scrub_patterns(text: &str) -> String {
    let t = JWT.replace_all(text, "<jwt>");
    let t = BEARER.replace_all(&t, "Bearer <redacted>");
    EMAIL.replace_all(&t, "<email>").into_owned()
}

fn clean_path(path: &str) -> String {
    path.split('/')
        .map(|seg| if is_opaque(seg) { "<opaque>" } else { seg })
        .collect::<Vec<_>>()
        .join("/")
}

/// Keeps scheme, host and path of an absolute URL; drops credentials, query
/// and fragment.
fn clean_url(url: &str) -> Option<String> {
    let (scheme, rest) = match url.split_once("://") {
        Some((s, r)) if matches!(s, "http" | "https") => (format!("{s}://"), r),
        _ => (String::new(), url.strip_prefix("//")?),
    };
    let prefix = if scheme.is_empty() {
        "//".to_string()
    } else {
        scheme
    };
    let cut = rest.find(['?', '#']).map_or(rest, |i| &rest[..i]);
    let (authority, path) = cut.split_at(cut.find('/').unwrap_or(cut.len()));
    let host = authority.rsplit('@').next().unwrap_or(authority);
    Some(format!("{prefix}{host}{}", clean_path(path)))
}

struct Ctx<'a> {
    opts: &'a SanitizeOptions,
    next_key: usize,
}

impl Ctx<'_> {
    fn string(&self, s: &str, key: &str) -> Value {
        let known = scrub_text(s, &self.opts.secrets);
        // URLs first: credentials before `@` would otherwise look like an email.
        if let Some(url) = clean_url(&known) {
            return Value::String(scrub_patterns(&url));
        }
        let scrubbed = scrub_patterns(&known);
        let lower = key.to_lowercase();
        if matches!(lower.as_str(), "url" | "absolute_url") && scrubbed.starts_with('/') {
            let path = scrubbed.split(['?', '#']).next().unwrap_or_default();
            if self.opts.include_titles {
                return Value::String(path.to_string());
            }
            let mut segs = path.split('/').filter(|p| !p.is_empty());
            return Value::String(match (segs.next(), segs.next()) {
                (Some(first), Some(_)) => format!("/{first}/<slug>/"),
                _ => path.to_string(),
            });
        }
        if is_opaque(&scrubbed) {
            return Value::String(format!("<opaque len={}>", scrubbed.chars().count()));
        }
        Value::String(scrubbed)
    }

    fn redact(v: &Value, label: &str) -> Value {
        match v {
            Value::String(s) => Value::String(format!("<{label} len={}>", s.chars().count())),
            Value::Number(_) => Value::from(0),
            Value::Array(a) => Value::Array(a.iter().map(|x| Self::redact(x, label)).collect()),
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, x)| (k.clone(), Self::redact(x, label)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    fn key(&mut self, k: &str) -> String {
        let scrubbed = scrub_patterns(&scrub_text(k, &self.opts.secrets));
        if is_opaque(&scrubbed) {
            self.next_key += 1;
            return format!("<opaque-key-{}>", self.next_key);
        }
        scrubbed
    }

    fn value(&mut self, v: &Value, key: &str, user: bool, depth: usize) -> Value {
        if depth > MAX_DEPTH {
            return Value::String("<too deep>".into());
        }
        let lower = key.to_lowercase();
        let secret = SECRET_KEYS.iter().any(|f| lower.contains(f)) || lower.contains("avatar");
        if secret {
            return Self::redact(v, "redacted");
        }
        if PERSON_KEYS.contains(&lower.as_str()) || lower.ends_with("_name") {
            return Self::redact(v, "name");
        }
        if user && lower == "id" {
            return Self::redact(v, "redacted");
        }
        if !self.opts.include_titles {
            if TITLE_KEYS.contains(&lower.as_str()) {
                return Self::redact(v, "title");
            }
            if TEXT_KEYS.contains(&lower.as_str()) {
                return Self::redact(v, "text");
            }
        }
        match v {
            Value::String(s) => self.string(s, key),
            Value::Number(n) => {
                let shown = n.to_string();
                if self
                    .opts
                    .secrets
                    .iter()
                    .any(|s| s.len() >= MIN_SECRET_LEN && *s == shown)
                {
                    Value::from(0)
                } else {
                    v.clone()
                }
            }
            Value::Array(a) => Value::Array(
                a.iter()
                    .map(|x| self.value(x, key, false, depth + 1))
                    .collect(),
            ),
            Value::Object(o) => {
                let is_user = o.get("_class").and_then(Value::as_str) == Some("user");
                let mut out = Map::new();
                for (k, x) in o {
                    let nk = self.key(k);
                    let nv = self.value(x, k, is_user, depth + 1);
                    out.insert(nk, nv);
                }
                Value::Object(out)
            }
            other => other.clone(),
        }
    }
}

/// Redacted copy of `v` with the same keys, nesting and value types.
pub fn sanitize(v: &Value, opts: &SanitizeOptions) -> Value {
    Ctx { opts, next_key: 0 }.value(v, "", false, 0)
}

/// Keeps only the first `max` elements of every array (samples stay small).
pub fn truncate_arrays(v: &Value, max: usize) -> Value {
    match v {
        Value::Array(a) => Value::Array(
            a.iter()
                .take(max)
                .map(|x| truncate_arrays(x, max))
                .collect(),
        ),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, x)| (k.clone(), truncate_arrays(x, max)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    fn opts() -> SanitizeOptions {
        SanitizeOptions::default()
    }

    fn clean(v: Value) -> Value {
        sanitize(&v, &opts())
    }

    fn dump(v: &Value) -> String {
        serde_json::to_string(v).unwrap()
    }

    #[test]
    fn structure_keys_and_value_types_survive() {
        let input = json!({
            "_class": "lecture", "id": 11, "is_published": true, "object_index": 3,
            "asset": null, "items": [{"id": 1, "ok": false}, [1, 2, [3]]], "ratio": 1.5
        });
        assert_eq!(clean(input.clone()), input);
    }

    #[test]
    fn secret_keys_are_redacted_by_type_whatever_the_value() {
        let out = clean(json!({
            "access_token": "abcdef", "media_license_token": "L1", "Authorization": "Bearer xyz",
            "cookie": "a=b", "password": "p", "client_secret": "s", "csrf_token": 5,
            "refresh_token": true, "x_session_id": null,
            "license": {"a": "keep?", "b": [1, "two"]}
        }));
        assert_eq!(out["access_token"], "<redacted len=6>");
        assert_eq!(out["media_license_token"], "<redacted len=2>");
        assert_eq!(out["Authorization"], "<redacted len=10>");
        assert_eq!(out["csrf_token"], 0);
        assert_eq!(out["refresh_token"], true, "booleans carry no secret");
        assert_eq!(out["x_session_id"], Value::Null);
        assert_eq!(
            out["license"],
            json!({"a": "<redacted len=5>", "b": [0, "<redacted len=3>"]})
        );
    }

    #[test]
    fn urls_lose_query_fragment_and_credentials_but_keep_scheme_host_path() {
        let out = clean(json!({
            "a": "https://mp4.udemycdn.com/x/720.mp4?token=SECRET&Expires=1#frag",
            "b": "https://user:pw@host.test/p/q?sig=1",
            "c": "//cdn.test/v.m3u8?hdnts=abc",
            "d": "http://127.0.0.1:8080/api"
        }));
        assert_eq!(out["a"], "https://mp4.udemycdn.com/x/720.mp4");
        assert_eq!(out["b"], "https://host.test/p/q");
        assert_eq!(out["c"], "//cdn.test/v.m3u8");
        assert_eq!(out["d"], "http://127.0.0.1:8080/api");
        let all = dump(&out);
        assert!(!all.contains("SECRET") && !all.contains("pw@") && !all.contains("hdnts"));
    }

    #[test]
    fn opaque_path_segments_of_signed_urls_are_replaced() {
        let out = clean(
            json!({"u": "https://cdn.test/Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZg/video.mp4?x=1"}),
        );
        assert_eq!(out["u"], "https://cdn.test/<opaque>/video.mp4");
    }

    #[test]
    fn jwts_bearer_tokens_and_emails_are_scrubbed_inside_text() {
        let jwt = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTYifQ.c2lnbmF0dXJl";
        let out = clean(json!({
            "message": format!("failed with {jwt} for ana.perez+x@example.com, header Bearer abcDEF123"),
            "nested": [{"note": "write to Soporte@Udemy.Test now"}],
            "url_with_jwt": format!("https://a.test/{jwt}/x")
        }));
        let all = dump(&out);
        assert!(!all.contains("eyJ") && !all.contains("example.com") && !all.contains("abcDEF123"));
        assert!(!all.contains("Soporte"));
        assert!(out["message"].as_str().unwrap().contains("<jwt>"));
        assert!(out["message"].as_str().unwrap().contains("<email>"));
        assert!(out["message"]
            .as_str()
            .unwrap()
            .contains("Bearer <redacted>"));
        assert_eq!(out["nested"][0]["note"], "write to <email> now");
    }

    #[test]
    fn long_high_entropy_strings_are_opaque_but_prose_is_kept() {
        let out = clean(json!({
            "blob": "Zm9vYmFyYmF6cXV4MTIzNDU2Nzg5MGFiY2RlZg",
            "hex": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b",
            "uuid": "123e4567-e89b-12d3-a456-426614174000",
            "prose": "this is a long sentence with spaces that is not a secret at all",
            "dull": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "short": "abc123"
        }));
        assert_eq!(out["blob"], "<opaque len=38>");
        assert_eq!(out["hex"], "<opaque len=40>");
        assert_eq!(out["uuid"], "<opaque len=36>");
        assert_eq!(
            out["prose"],
            "this is a long sentence with spaces that is not a secret at all"
        );
        assert_eq!(out["dull"], "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(out["short"], "abc123");
    }

    #[test]
    fn titles_and_texts_become_placeholders_unless_opted_in() {
        let input = json!({
            "title": "Curso privado de Ana", "filename": "mi-clase.mp4",
            "description": "<p>Notas personales</p>", "body": "<p>Texto</p>",
            "nested": {"title": "Sección"}
        });
        let out = clean(input.clone());
        assert_eq!(out["title"], "<title len=20>");
        assert_eq!(out["filename"], "<title len=12>");
        assert_eq!(out["description"], "<text len=23>");
        assert_eq!(out["body"], "<text len=12>");
        assert_eq!(out["nested"]["title"], "<title len=7>");
        let rel = clean(json!({"url": "/course/mi-curso-privado/?x=1", "plain": "/course/x/"}));
        assert_eq!(rel["url"], "/course/<slug>/");
        assert_eq!(rel["plain"], "/course/x/");
        let kept = sanitize(
            &input,
            &SanitizeOptions {
                include_titles: true,
                ..Default::default()
            },
        );
        assert_eq!(kept["title"], "Curso privado de Ana");
        assert_eq!(kept["body"], "<p>Texto</p>");
        let leaky = sanitize(
            &json!({"title": "Contact ana@example.com"}),
            &SanitizeOptions {
                include_titles: true,
                ..Default::default()
            },
        );
        assert_eq!(leaky["title"], "Contact <email>");
    }

    #[test]
    fn people_avatars_and_user_ids_are_redacted() {
        let out = clean(json!({
            "email": "ana@example.com",
            "visible_instructors": [{"_class": "user", "id": 777, "display_name": "Ana Ejemplo",
                "first_name": "Ana", "job_title": "Instructora", "image_100x100": "https://i.test/a.jpg?x=1",
                "avatar": "https://i.test/avatar.jpg"}],
            "owner": {"_class": "user", "id": "u-99", "name": "Yo Mismo"},
            "course": {"_class": "course", "id": 1001}
        }));
        let i = &out["visible_instructors"][0];
        assert_eq!(i["id"], 0);
        assert_eq!(i["display_name"], "<name len=11>");
        assert_eq!(i["first_name"], "<name len=3>");
        assert_eq!(i["job_title"], "<name len=11>");
        assert_eq!(i["avatar"], "<redacted len=25>");
        assert_eq!(i["image_100x100"], "https://i.test/a.jpg");
        assert_eq!(out["email"], "<redacted len=15>");
        assert_eq!(out["owner"]["id"], "<redacted len=4>");
        assert_eq!(out["owner"]["name"], "<name len=8>");
        assert_eq!(out["course"]["id"], 1001, "course ids stay");
    }

    #[test]
    fn known_secret_values_are_scrubbed_everywhere_even_in_keys_and_numbers() {
        let o = SanitizeOptions {
            secrets: vec!["SESSIONVALUE1".into(), "4242424".into()],
            ..Default::default()
        };
        let out = sanitize(
            &json!({
                "note": "sid=SESSIONVALUE1; more", "SESSIONVALUE1": "k", "n": 4242424, "ok": 1001,
                "list": ["xSESSIONVALUE1x"]
            }),
            &o,
        );
        let all = dump(&out);
        assert!(
            !all.contains("SESSIONVALUE1") && !all.contains("4242424"),
            "{all}"
        );
        assert_eq!(out["ok"], 1001);
        assert_eq!(out["n"], 0);
    }

    #[test]
    fn scrub_text_removes_exact_secrets_from_serialized_output() {
        let s = scrub_text(
            "a TOKEN123456 b TOKEN123456",
            &["TOKEN123456".into(), "ab".into()],
        );
        assert_eq!(s, "a <secret> b <secret>");
        assert_eq!(
            scrub_text("keep ab", &["ab".into()]),
            "keep ab",
            "too short to be a secret"
        );
    }

    #[test]
    fn deep_and_wide_documents_are_handled_and_arrays_can_be_truncated() {
        let mut v = json!("leaf@example.com");
        for _ in 0..200 {
            v = json!({"n": [v]});
        }
        let out = clean(v);
        assert!(!dump(&out).contains("example.com"));
        let t = truncate_arrays(
            &json!({"a": [1, 2, 3, 4, 5], "b": [[1, 2, 3, 4], {"c": [9, 9, 9, 9]}]}),
            3,
        );
        assert_eq!(
            t,
            json!({"a": [1, 2, 3], "b": [[1, 2, 3], {"c": [9, 9, 9]}]})
        );
    }

    #[test]
    fn the_assumed_fixtures_come_out_without_personal_or_secret_data() {
        let doc: Value = serde_json::from_str(include_str!(
            "../tests/fixtures/assumed_curriculum_page1.json"
        ))
        .unwrap();
        let out = clean(doc.clone());
        let all = dump(&out);
        for leak in [
            "FAKELICENSETOKEN",
            "token=FAKE",
            "sig=1",
            "sig=2",
            "Bienvenida",
            "Texto de la lectura",
        ] {
            assert!(!all.contains(leak), "{leak} leaked: {all}");
        }
        assert_eq!(out["results"][1]["_class"], "lecture");
        assert_eq!(out["results"][1]["id"], 11);
        assert_eq!(out["results"][1]["asset"]["asset_type"], "Video");
        assert_eq!(out["results"][1]["asset"]["course_is_drmed"], false);
        assert_eq!(
            out["results"][4]["asset"]["media_sources"][0]["src"],
            "https://x.udemycdn.com/m.mpd"
        );
        let shape = |v: &Value| -> String { dump(&strip_values(v)) };
        assert_eq!(shape(&out), shape(&doc), "same keys and nesting");
    }

    /// Replaces every leaf by its JSON type name, to compare structures.
    fn strip_values(v: &Value) -> Value {
        match v {
            Value::Object(o) => Value::Object(
                o.iter()
                    .map(|(k, v)| (k.clone(), strip_values(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(strip_values).collect()),
            Value::String(_) => json!("s"),
            Value::Number(_) => json!(0),
            other => other.clone(),
        }
    }
}
