use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum NotifierError {
    #[error("unknown destination: {0}")]
    UnknownDestination(String),
    #[error("notifier unavailable: {0}")]
    Unavailable(String),
}

pub trait Notifier {
    fn notify(&self, to: &str, message: &str) -> Result<(), NotifierError>;
}
