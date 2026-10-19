use std::path::PathBuf;

/// Errors raised while scanning a library folder.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("invalid group name")]
    InvalidGroup,
    #[error("group not found")]
    GroupNotFound,
    #[error("cannot read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}
