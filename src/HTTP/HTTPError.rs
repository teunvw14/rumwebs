use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct RequestParseError {
    details: String,
}

impl RequestParseError {
    pub fn new(msg: &str) -> RequestParseError {
        RequestParseError {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for RequestParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for RequestParseError {
    fn description(&self) -> &str {
        &self.details
    }
}
