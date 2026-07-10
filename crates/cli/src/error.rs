use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("command execution failed: {0}")]
    CommandFailed(#[from] std::io::Error),
    #[error("configuration error: {0}")]
    ConfigError(String),
    #[error("network error: {0}")]
    NetworkError(String),
}

pub type Result<T> = std::result::Result<T, CliError>;
