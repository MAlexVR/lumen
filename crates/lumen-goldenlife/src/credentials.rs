//! Login input and the persisted session. The password only lives in a
//! [`Login`] for the duration of the login call; what is stored afterwards is
//! the cookie header (a long-lived "remember me" cookie among them).

use crate::error::GoldenlifeError;

#[derive(Clone, PartialEq, Eq)]
pub struct Login {
    email: String,
    password: String,
}

impl std::fmt::Debug for Login {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Login").finish_non_exhaustive()
    }
}

impl Login {
    /// Rejects obviously unusable input before any request is made.
    pub fn new(email: &str, password: &str) -> Result<Self, GoldenlifeError> {
        let email = email.trim();
        let has_shape = email
            .split_once('@')
            .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'));
        if !has_shape || email.chars().any(char::is_whitespace) || password.is_empty() {
            return Err(GoldenlifeError::InvalidCredentials);
        }
        Ok(Self {
            email: email.to_string(),
            password: password.to_string(),
        })
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    pub fn password(&self) -> &str {
        &self.password
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    cookie_header: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("has_cookies", &!self.cookie_header.is_empty())
            .finish_non_exhaustive()
    }
}

fn invalid() -> GoldenlifeError {
    GoldenlifeError::InvalidSession("the stored cookie header is malformed")
}

fn visible(text: &str) -> bool {
    text.chars().all(|c| c.is_ascii_graphic()) && !text.contains([';', ','])
}

impl Session {
    /// Parses `name=value; name=value` (the stored form).
    pub fn new(cookie_header: &str) -> Result<Self, GoldenlifeError> {
        let mut pairs = Vec::new();
        for pair in cookie_header.split(';') {
            let pair = pair.trim();
            if pair.is_empty() {
                continue;
            }
            let (name, value) = pair.split_once('=').ok_or_else(invalid)?;
            if name.is_empty() || !visible(name) || !visible(value) {
                return Err(invalid());
            }
            pairs.push(format!("{name}={value}"));
        }
        if pairs.is_empty() {
            return Err(invalid());
        }
        Ok(Self {
            cookie_header: pairs.join("; "),
        })
    }

    pub fn cookie_header(&self) -> &str {
        &self.cookie_header
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn login_needs_an_email_shape_and_a_password() {
        assert!(Login::new("ana@example.com", "pw").is_ok());
        assert!(Login::new("  ana@example.com ", "pw").is_ok());
        for (email, password) in [
            ("", "pw"),
            ("ana", "pw"),
            ("ana@", "pw"),
            ("ana@localhost", "pw"),
            ("a na@example.com", "pw"),
            ("ana@example.com", ""),
        ] {
            assert_eq!(
                Login::new(email, password),
                Err(GoldenlifeError::InvalidCredentials),
                "{email:?}"
            );
        }
    }

    #[test]
    fn login_and_session_debug_never_show_secrets() {
        let login = Login::new("ana@example.com", "hunter2-secret").unwrap();
        let shown = format!("{login:?}");
        assert!(
            !shown.contains("hunter2") && !shown.contains("ana@"),
            "{shown}"
        );
        let session = Session::new("remember_x=tok-secret; app_session=abc").unwrap();
        let shown = format!("{session:?}");
        assert!(
            !shown.contains("tok-secret") && !shown.contains("abc"),
            "{shown}"
        );
    }

    #[test]
    fn a_session_is_a_list_of_cookie_pairs() {
        let session = Session::new(" a=1 ;b=2; ").unwrap();
        assert_eq!(session.cookie_header(), "a=1; b=2");
        for bad in ["", "   ", "novalue", "=x", "a=b c", "a=\u{e9}", "a=1,b=2"] {
            assert!(Session::new(bad).is_err(), "{bad:?}");
        }
    }
}
