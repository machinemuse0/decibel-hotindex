use std::error::Error;
use std::fmt::{Display, Formatter};

pub type Result<T> = std::result::Result<T, HotIndexError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotIndexError {
    Config(String),
    Address(String),
    Parse(String),
    Storage(String),
}

impl Display for HotIndexError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Config(message) => write!(f, "config error: {message}"),
            Self::Address(message) => write!(f, "address error: {message}"),
            Self::Parse(message) => write!(f, "parse error: {message}"),
            Self::Storage(message) => write!(f, "storage error: {message}"),
        }
    }
}

impl Error for HotIndexError {}

impl From<std::io::Error> for HotIndexError {
    fn from(value: std::io::Error) -> Self {
        Self::Config(value.to_string())
    }
}

impl From<serde_yaml::Error> for HotIndexError {
    fn from(value: serde_yaml::Error) -> Self {
        Self::Config(value.to_string())
    }
}
