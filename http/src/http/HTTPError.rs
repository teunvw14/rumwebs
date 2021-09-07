use std::error::Error;
use std::fmt;

#[derive(Debug)]
pub struct InvalidRequest {
    details: String,
}

impl InvalidRequest {
    pub fn new(msg: &str) -> InvalidRequest {
        InvalidRequest {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for InvalidRequest {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for InvalidRequest {
    fn description(&self) -> &str {
        &self.details
    }
}

impl From<InvalidRequestMethod> for InvalidRequest {
    fn from(e: InvalidRequestMethod) -> InvalidRequest {
        InvalidRequest {
            details: e.to_string(),
        }
    }
}


#[derive(Debug)]
pub struct InvalidRequestMethod {
    details: String,
}

impl InvalidRequestMethod {
    pub fn new(msg: &str) -> InvalidRequestMethod {
        InvalidRequestMethod {
            details: msg.to_string(),
        }
    }
}

impl fmt::Display for InvalidRequestMethod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.details)
    }
}

impl Error for InvalidRequestMethod {
    fn description(&self) -> &str {
        &self.details
    }
}