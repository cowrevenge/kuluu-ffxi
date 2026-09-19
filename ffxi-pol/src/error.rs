use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("{service} refused the request: status {status}")]
    Status { service: &'static str, status: i32 },

    #[error("{0}")]
    Protocol(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl Error {
    pub fn protocol(what: impl Into<String>) -> Self {
        Self::Protocol(what.into())
    }
}
