#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("cannot bind loopback listener: {0}")]
    Bind(#[source] std::io::Error),
    #[error("cannot generate access token: {0}")]
    Token(String),
}
