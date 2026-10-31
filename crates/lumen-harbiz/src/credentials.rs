//! Login input and the persisted session. The password only lives in a
//! [`Login`] for the duration of the login call; what is stored afterwards is
//! the Meteor resume token.

use sha2::{Digest, Sha256};

use crate::error::HarbizError;

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
    pub fn new(email: &str, password: &str) -> Result<Self, HarbizError> {
        let email = email.trim();
        let has_shape = email
            .split_once('@')
            .is_some_and(|(user, host)| !user.is_empty() && host.contains('.'));
        if !has_shape || email.chars().any(char::is_whitespace) || password.is_empty() {
            return Err(HarbizError::InvalidCredentials);
        }
        Ok(Self {
            email: email.to_string(),
            password: password.to_string(),
        })
    }

    pub fn email(&self) -> &str {
        &self.email
    }

    /// Meteor never receives the password itself, only this SHA-256 digest.
    pub fn password_digest(&self) -> String {
        hex::encode(Sha256::digest(self.password.as_bytes()))
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Session {
    token: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session").finish_non_exhaustive()
    }
}

impl Session {
    pub fn new(token: &str) -> Result<Self, HarbizError> {
        let token = token.trim();
        if token.is_empty() || token.chars().any(char::is_whitespace) {
            return Err(HarbizError::InvalidResponse(
                "the stored session token is malformed".to_string(),
            ));
        }
        Ok(Self {
            token: token.to_string(),
        })
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unusable_logins() {
        for (email, password) in [("", "x"), ("a@b", "x"), ("a b@c.d", "x"), ("a@b.c", "")] {
            assert_eq!(
                Login::new(email, password).unwrap_err(),
                HarbizError::InvalidCredentials
            );
        }
        assert!(Login::new(" a@b.co ", "x").is_ok());
    }

    #[test]
    fn the_digest_is_the_sha256_hex_of_the_password() {
        let login = Login::new("a@b.co", "password").unwrap();
        assert_eq!(
            login.password_digest(),
            "5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8"
        );
    }

    #[test]
    fn debug_never_prints_secrets() {
        let login = Login::new("a@b.co", "hunter2").unwrap();
        let session = Session::new("tok-123").unwrap();
        let shown = format!("{login:?} {session:?}");
        assert!(!shown.contains("hunter2") && !shown.contains("tok-123"));
    }

    #[test]
    fn sessions_reject_empty_or_spaced_tokens() {
        assert!(Session::new("").is_err());
        assert!(Session::new("a b").is_err());
        assert_eq!(Session::new(" tok\n").unwrap().token(), "tok");
    }
}
