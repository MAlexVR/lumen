//! Normalisation of the pasted browser cookie header.

use crate::error::SkoolError;

/// Name of the cookie that proves a logged-in session.
pub const AUTH_COOKIE: &str = "auth_token";

/// Accepts a raw `Cookie` header, a DevTools `cookie: a=b; c=d` line or a
/// multi-line paste and returns a clean `a=b; c=d` header. Requires
/// `auth_token`. Error messages never include cookie values.
pub fn normalize_cookie_header(raw: &str) -> Result<String, SkoolError> {
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
                return Err(SkoolError::Cookie(
                    "the cookies contain invalid characters".into(),
                ));
            }
            match pairs.iter_mut().find(|(n, _)| n == name) {
                Some(existing) => existing.1 = value.to_string(),
                None => pairs.push((name.to_string(), value.to_string())),
            }
        }
    }
    let header = cookie_header_from_pairs(pairs.iter().map(|(n, v)| (n.as_str(), v.as_str())));
    if !has_auth_token(&header) {
        return Err(SkoolError::Cookie(format!(
            "no {AUTH_COOKIE} cookie found: log in at skool.com and copy the full cookie header"
        )));
    }
    Ok(header)
}

/// Joins `(name, value)` pairs (e.g. from the webview cookie jar).
pub fn cookie_header_from_pairs<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> String {
    pairs
        .into_iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

/// True when the header contains a non-empty `auth_token`.
pub fn has_auth_token(header: &str) -> bool {
    header.split(';').any(|part| {
        part.trim()
            .split_once('=')
            .is_some_and(|(n, v)| n.trim() == AUTH_COOKIE && !v.trim().is_empty())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_header_is_kept() {
        assert_eq!(
            normalize_cookie_header("auth_token=abc; aws-waf-token=xyz").unwrap(),
            "auth_token=abc; aws-waf-token=xyz"
        );
    }

    #[test]
    fn devtools_prefix_and_multiline_paste_are_cleaned() {
        let raw = "  Cookie: a=1; auth_token=t\ncookie: b=2 ;\n\n";
        assert_eq!(
            normalize_cookie_header(raw).unwrap(),
            "a=1; auth_token=t; b=2"
        );
    }

    #[test]
    fn values_with_equals_are_preserved_and_duplicates_keep_the_last() {
        assert_eq!(
            normalize_cookie_header("auth_token=ab==; k=1; k=2").unwrap(),
            "auth_token=ab==; k=2"
        );
    }

    #[test]
    fn missing_auth_token_and_empty_input_are_rejected_without_leaking_values() {
        let e = normalize_cookie_header("a=SECRETVALUE").unwrap_err();
        assert!(matches!(e, SkoolError::Cookie(_)));
        assert!(!e.to_string().contains("SECRETVALUE"));
        assert!(normalize_cookie_header("").is_err());
        assert!(normalize_cookie_header("auth_token=").is_err());
        assert!(normalize_cookie_header("just text").is_err());
    }

    #[test]
    fn control_characters_are_rejected() {
        assert!(normalize_cookie_header("auth_token=a\u{7}b").is_err());
    }

    #[test]
    fn pairs_are_joined_and_checked() {
        let h = cookie_header_from_pairs([("auth_token", "t"), ("x", "1")]);
        assert_eq!(h, "auth_token=t; x=1");
        assert!(has_auth_token(&h));
        assert!(!has_auth_token("x=1"));
        assert!(!has_auth_token("auth_token="));
    }
}
