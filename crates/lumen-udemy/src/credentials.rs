//! Udemy session credentials: the browser cookie header of a logged-in
//! www.udemy.com session. HYPOTHESIS (unverified until a real diagnostic run):
//! the `access_token` cookie is the bearer token the web app sends.

use crate::error::UdemyError;

/// Cookie that carries the bearer token.
pub const ACCESS_TOKEN_COOKIE: &str = "access_token";

/// Values shorter than this are not treated as secrets to scrub (they would
/// match innocent text everywhere).
const MIN_SECRET_LEN: usize = 6;

#[derive(Clone)]
pub struct Credentials {
    cookie_header: String,
    access_token: String,
    pairs: Vec<(String, String)>,
}

impl std::fmt::Debug for Credentials {
    // Deliberately omits every value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("cookies", &self.pairs.len())
            .finish_non_exhaustive()
    }
}

fn token_of(pairs: &[(String, String)]) -> Option<&str> {
    pairs
        .iter()
        .find(|(n, v)| n == ACCESS_TOKEN_COOKIE && !v.is_empty())
        .map(|(_, v)| v.as_str())
}

impl Credentials {
    /// Accepts a raw `Cookie` header, a DevTools `cookie: a=b; c=d` line or a
    /// multi-line paste. Requires a non-empty `access_token`.
    pub fn from_cookie_text(raw: &str) -> Result<Self, UdemyError> {
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
                    return Err(UdemyError::InvalidCookies(
                        "the cookies contain invalid characters".into(),
                    ));
                }
                match pairs.iter_mut().find(|(n, _)| n == name) {
                    Some(existing) => existing.1 = value.to_string(),
                    None => pairs.push((name.to_string(), value.to_string())),
                }
            }
        }
        let access_token = token_of(&pairs)
            .ok_or_else(|| {
                UdemyError::InvalidCookies(format!(
                    "no {ACCESS_TOKEN_COOKIE} cookie found: log in at udemy.com and copy the full cookie header"
                ))
            })?
            .to_string();
        let cookie_header = join(pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())));
        Ok(Self {
            cookie_header,
            access_token,
            pairs,
        })
    }

    pub fn cookie_header(&self) -> &str {
        &self.cookie_header
    }

    pub fn access_token(&self) -> &str {
        &self.access_token
    }

    /// Value of one cookie, if present.
    pub fn cookie(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Every credential value that must never reach a file or a log.
    pub fn secret_values(&self) -> Vec<String> {
        self.pairs
            .iter()
            .map(|(_, v)| v.clone())
            .filter(|v| v.len() >= MIN_SECRET_LEN)
            .collect()
    }
}

fn join<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    pairs
        .into_iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// Header built from the webview cookie jar, once a session with a token exists.
pub fn header_from_pairs<'a>(
    pairs: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Option<String> {
    let pairs: Vec<(&str, &str)> = pairs.into_iter().collect();
    let has_token = pairs
        .iter()
        .any(|(n, v)| *n == ACCESS_TOKEN_COOKIE && !v.trim().is_empty());
    has_token.then(|| join(pairs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_header_is_kept_and_the_bearer_token_extracted() {
        let c =
            Credentials::from_cookie_text("client_id=c1; access_token=TOK123; other=x").unwrap();
        assert_eq!(
            c.cookie_header(),
            "client_id=c1; access_token=TOK123; other=x"
        );
        assert_eq!(c.access_token(), "TOK123");
    }

    #[test]
    fn devtools_prefixes_multiline_pastes_and_duplicates_are_cleaned() {
        let raw = "  Cookie: a=1; access_token=old\ncookie: access_token=NEW ; b=2 ;\n\n";
        let c = Credentials::from_cookie_text(raw).unwrap();
        assert_eq!(c.cookie_header(), "a=1; access_token=NEW; b=2");
        assert_eq!(c.access_token(), "NEW");
    }

    #[test]
    fn missing_or_empty_tokens_and_bad_characters_are_rejected_without_leaking() {
        for bad in ["", "a=1; b=2", "access_token=", "access_token=  ;"] {
            assert!(
                matches!(
                    Credentials::from_cookie_text(bad),
                    Err(UdemyError::InvalidCookies(_))
                ),
                "{bad:?}"
            );
        }
        let err = Credentials::from_cookie_text("SECRETNAME=SECRETVALUE").unwrap_err();
        assert!(!err.to_string().contains("SECRETVALUE"));
        let err = Credentials::from_cookie_text("access_token=a\u{1}b").unwrap_err();
        assert!(matches!(err, UdemyError::InvalidCookies(_)));
        assert!(!err.to_string().contains('\u{1}'));
    }

    #[test]
    fn debug_never_shows_the_secrets() {
        let c = Credentials::from_cookie_text("access_token=TOKENVALUE; sid=COOKIEVALUE").unwrap();
        let shown = format!("{c:?}");
        assert!(
            !shown.contains("TOKENVALUE") && !shown.contains("COOKIEVALUE"),
            "{shown}"
        );
    }

    #[test]
    fn the_webview_jar_yields_a_header_only_with_a_token() {
        assert_eq!(
            header_from_pairs([("a", "1"), ("access_token", "T")]).as_deref(),
            Some("a=1; access_token=T")
        );
        assert_eq!(header_from_pairs([("a", "1")]), None);
        assert_eq!(header_from_pairs([("access_token", " ")]), None);
    }

    #[test]
    fn secret_values_lists_everything_that_must_never_be_written() {
        let c =
            Credentials::from_cookie_text("access_token=TOKEN1234; sid=SESSION01; x=ab").unwrap();
        let v = c.secret_values();
        assert!(v.contains(&"TOKEN1234".to_string()) && v.contains(&"SESSION01".to_string()));
        assert!(
            !v.contains(&"ab".to_string()),
            "very short values would match everywhere"
        );
    }
}
