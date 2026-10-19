//! What the user supplies: the `hmVlcIntegration` cookie value (used as a
//! Bearer token) and, optionally, a full cookie header for extra headers.

use crate::error::HotmartError;

/// Name of the Hotmart cookie that carries the bearer token.
pub const TOKEN_COOKIE: &str = "hmVlcIntegration";

#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    token: String,
    cookies: Option<String>,
}

impl std::fmt::Debug for Credentials {
    // Deliberately omits the secrets.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("has_cookies", &self.cookies.is_some())
            .finish_non_exhaustive()
    }
}

/// Cleans a pasted token: accepts the bare value, `hmVlcIntegration=<value>`,
/// `Bearer <value>` and quoted values. Errors never include the value.
pub fn normalize_token(raw: &str) -> Result<String, HotmartError> {
    let invalid = |m: &str| HotmartError::InvalidToken(m.to_string());
    let mut text = raw.trim();
    let lower = text.to_ascii_lowercase();
    let cookie_key = format!("{}=", TOKEN_COOKIE.to_ascii_lowercase());
    if let Some(at) = lower.find(&cookie_key) {
        // A pasted cookie header: take the value of the token cookie only.
        text = text[at + cookie_key.len()..]
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
    } else if lower.starts_with("bearer ") {
        text = text[7..].trim();
    }
    let text = text.trim_end_matches(';').trim();
    let text = ['"', '\'']
        .iter()
        .find_map(|q| text.strip_prefix(*q).and_then(|t| t.strip_suffix(*q)))
        .unwrap_or(text)
        .trim();
    if text.is_empty() {
        return Err(invalid("paste the hmVlcIntegration cookie value"));
    }
    if text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(invalid("the token contains spaces or invalid characters"));
    }
    Ok(text.to_string())
}

/// Cleans an optional pasted cookie header (`Cookie: a=b; c=d`, multi-line
/// pastes); blank input means "no cookies".
pub fn normalize_cookies(raw: &str) -> Result<Option<String>, HotmartError> {
    let mut pairs: Vec<(String, String)> = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        let line = match line.get(..7) {
            Some(prefix) if prefix.eq_ignore_ascii_case("cookie:") => &line[7..],
            _ => line,
        };
        for part in line.split(';') {
            let Some((name, value)) = part.trim().split_once('=') else {
                continue;
            };
            let (name, value) = (name.trim(), value.trim());
            if name.is_empty() {
                continue;
            }
            if name.chars().chain(value.chars()).any(char::is_control) {
                return Err(HotmartError::InvalidToken(
                    "the cookies contain invalid characters".into(),
                ));
            }
            match pairs.iter_mut().find(|(n, _)| n == name) {
                Some(existing) => existing.1 = value.to_string(),
                None => pairs.push((name.to_string(), value.to_string())),
            }
        }
    }
    if pairs.is_empty() {
        return Ok(None);
    }
    Ok(Some(
        pairs
            .iter()
            .map(|(n, v)| format!("{n}={v}"))
            .collect::<Vec<_>>()
            .join("; "),
    ))
}

impl Credentials {
    pub fn new(token: &str, cookies: Option<&str>) -> Result<Self, HotmartError> {
        let cookies = match cookies {
            Some(raw) => normalize_cookies(raw)?,
            None => None,
        };
        Ok(Self {
            token: normalize_token(token)?,
            cookies,
        })
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn cookies(&self) -> Option<&str> {
        self.cookies.as_deref()
    }

    /// Serialised form for the private session file.
    pub fn to_stored(&self) -> String {
        serde_json::json!({ "token": self.token, "cookies": self.cookies }).to_string()
    }

    /// Reads [`Credentials::to_stored`] output; `None` when unreadable.
    pub fn from_stored(text: &str) -> Option<Self> {
        let v: serde_json::Value = serde_json::from_str(text).ok()?;
        let token = v.get("token")?.as_str()?.trim();
        if token.is_empty() {
            return None;
        }
        let cookies = v
            .get("cookies")
            .and_then(serde_json::Value::as_str)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        Some(Self {
            token: token.to_string(),
            cookies,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_prefixed_bearer_and_quoted_tokens_are_cleaned() {
        for raw in [
            "abc.def-ghi_jkl",
            "  abc.def-ghi_jkl\n",
            "hmVlcIntegration=abc.def-ghi_jkl",
            "hmVlcIntegration=abc.def-ghi_jkl;",
            "Bearer abc.def-ghi_jkl",
            "bearer abc.def-ghi_jkl",
            "\"abc.def-ghi_jkl\"",
            "'abc.def-ghi_jkl'",
        ] {
            assert_eq!(normalize_token(raw).unwrap(), "abc.def-ghi_jkl", "{raw:?}");
        }
    }

    #[test]
    fn empty_or_malformed_tokens_are_rejected_without_leaking_them() {
        assert!(normalize_token("").is_err());
        assert!(normalize_token("   ").is_err());
        assert!(normalize_token("hmVlcIntegration=").is_err());
        let e = normalize_token("SECRET VALUE WITH SPACES").unwrap_err();
        assert!(matches!(e, HotmartError::InvalidToken(_)));
        assert!(!e.to_string().contains("SECRET"));
        assert!(normalize_token("abc\u{7}def").is_err());
        assert!(normalize_token("a\nb").is_err());
    }

    #[test]
    fn cookies_are_optional_and_cleaned() {
        assert_eq!(normalize_cookies("  \n").unwrap(), None);
        assert_eq!(
            normalize_cookies("Cookie: a=1; b=2\ncookie: c=3;")
                .unwrap()
                .as_deref(),
            Some("a=1; b=2; c=3")
        );
        assert!(normalize_cookies("a=\u{7}").is_err());
    }

    #[test]
    fn credentials_round_trip_through_the_stored_form() {
        let c = Credentials::new("Bearer tok", Some("a=1")).unwrap();
        assert_eq!(c.token(), "tok");
        assert_eq!(c.cookies(), Some("a=1"));
        assert_eq!(Credentials::from_stored(&c.to_stored()), Some(c));
        let bare = Credentials::new("tok", None).unwrap();
        assert_eq!(Credentials::from_stored(&bare.to_stored()), Some(bare));
        assert_eq!(Credentials::from_stored("not json"), None);
        assert_eq!(Credentials::from_stored(r#"{"token":""}"#), None);
        assert!(Credentials::new("", None).is_err());
    }

    #[test]
    fn debug_never_prints_secrets() {
        let c = Credentials::new("SECRETTOKEN", Some("k=SECRETCOOKIE")).unwrap();
        let d = format!("{c:?}");
        assert!(!d.contains("SECRET"), "{d}");
    }
}
