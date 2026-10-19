use crate::error::PluralsightError;

#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    cookie_header: String,
    xsrf_token: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("has_cookies", &!self.cookie_header.is_empty())
            .field("has_xsrf_token", &self.xsrf_token.is_some())
            .finish_non_exhaustive()
    }
}

impl Credentials {
    pub fn new(raw: &str) -> Result<Self, PluralsightError> {
        let mut cookies = Vec::new();
        let mut found_content = false;
        for line in raw.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let is_cookie_header = line
                .get(..7)
                .filter(|prefix| prefix.eq_ignore_ascii_case("cookie:"))
                .is_some();
            if found_content && !is_cookie_header {
                return Err(invalid_cookies());
            }
            found_content = true;
            let line = line
                .get(..7)
                .filter(|prefix| prefix.eq_ignore_ascii_case("cookie:"))
                .map_or(line, |header| &line[header.len()..])
                .trim();
            if line.is_empty() {
                continue;
            }
            for pair in line.split(';') {
                let pair = pair.trim();
                let Some((name, value)) = pair.split_once('=') else {
                    return Err(invalid_cookies());
                };
                let (name, value) = (name.trim(), value.trim());
                if !valid_cookie_name(name) || !valid_cookie_value(value) {
                    return Err(invalid_cookies());
                }
                cookies.push((name.to_string(), value.to_string()));
            }
        }
        if cookies.is_empty() {
            return Err(invalid_cookies());
        }
        let xsrf_token = cookies
            .iter()
            .find(|(name, _)| name == "XSRF-TOKEN")
            .map(|(_, value)| value.clone());
        let cookie_header = cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        Ok(Self {
            cookie_header,
            xsrf_token,
        })
    }

    pub fn cookie_header(&self) -> &str {
        &self.cookie_header
    }

    pub fn xsrf_token(&self) -> Option<&str> {
        self.xsrf_token.as_deref()
    }
}

fn invalid_cookies() -> PluralsightError {
    PluralsightError::InvalidCredentials(
        "paste a valid Cookie header containing one or more name=value pairs".into(),
    )
}

fn valid_cookie_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        ..=b'\'' | b'*' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
                )
        })
}

fn valid_cookie_value(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| matches!(byte, 0x21 | 0x23..=0x2b | 0x2d..=0x3a | 0x3c..=0x5b | 0x5d..=0x7e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_raw_and_multiline_cookie_headers_and_extracts_xsrf() {
        let raw = Credentials::new("a=first; XSRF-TOKEN=csrf-token; b=last").unwrap();
        assert_eq!(
            raw.cookie_header(),
            "a=first; XSRF-TOKEN=csrf-token; b=last"
        );
        assert_eq!(raw.xsrf_token(), Some("csrf-token"));

        let multiline =
            Credentials::new("Cookie: a=first\nCookie: XSRF-TOKEN=csrf-token\n").unwrap();
        assert_eq!(multiline.cookie_header(), "a=first; XSRF-TOKEN=csrf-token");
        assert_eq!(multiline.xsrf_token(), Some("csrf-token"));
    }

    #[test]
    fn rejects_empty_malformed_and_header_injection_input() {
        assert!(Credentials::new(" \n ").is_err());
        assert!(Credentials::new("not-a-cookie").is_err());
        assert!(Credentials::new("a=one\nX-Evil: injected").is_err());
        assert!(Credentials::new("a=one\r\nb=two").is_err());
    }

    #[test]
    fn debug_redacts_cookie_values_and_xsrf_token() {
        let credentials =
            Credentials::new("session=secret-cookie; XSRF-TOKEN=secret-xsrf").unwrap();
        let debug = format!("{credentials:?}");
        assert!(!debug.contains("secret-cookie"));
        assert!(!debug.contains("secret-xsrf"));
    }
}
