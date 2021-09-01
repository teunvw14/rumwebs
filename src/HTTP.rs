use std::collections::HashMap;
use std::fmt;

pub mod HTTPError;
pub mod RequestMethods;
pub mod StatusCodes;

#[derive(Eq, PartialEq)]
pub enum RequestMethod {
    GET,
    HEAD,
    POST,
    PUT,
    DELETE,
    CONNECT,
    OPTIONS,
    TRACE,
    PATCH,
}

impl fmt::Display for RequestMethod {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let text = match self {
            RequestMethod::GET => "GET",
            RequestMethod::HEAD => "HEAD",
            RequestMethod::POST => "POST",
            RequestMethod::PUT => "PUT",
            RequestMethod::DELETE => "DELETE",
            RequestMethod::CONNECT => "CONNECT",
            RequestMethod::OPTIONS => "OPTIONS",
            RequestMethod::TRACE => "TRACE",
            RequestMethod::PATCH => "PATCH",
        };
        write!(f, "{}", text)
    }
}

pub struct Request {
    pub method: RequestMethod,
    pub uri: String,
    pub http_version: String,
    pub headers: Option<HashMap<String, String>>,
    pub body: Option<Vec<u8>>,
}

impl Request {
    pub fn new() -> Request {
        Request {
            method: RequestMethod::GET,
            uri: "/".to_string(),
            http_version: "1.1".to_string(),
            headers: None,
            body: None,
        }
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Request, HTTPError::RequestParseError> {
        // Check if the bytes even form an HTTP request.
        if !String::from_utf8_lossy(bytes).contains("HTTP/") {
            return Err(HTTPError::RequestParseError::new(
                "Bytes don't form a valid HTTP request.",
            ));
        }

        // Separate head and body for further parsing.
        // Especially relevant: while everything before
        // the body (here called "the head") of the request
        // is required to be valid utf-8, while the body
        // is not - and often isn't.
        let head_body_separator = b"\r\n\r\n";
        let head: Vec<u8>;
        let body: Option<Vec<u8>>;
        if let Some(pos) = bytes
            .windows(head_body_separator.len())
            .position(|window| window == head_body_separator)
        {
            head = bytes[..pos].to_vec();
            body = Some(bytes[pos..].to_vec());
        } else {
            head = bytes.to_vec();
            body = None;
        };
        let head_str = match String::from_utf8(head) {
            Ok(h) => h,
            Err(_) => {
                return Err(HTTPError::RequestParseError::new(
                    "Failed to parse HTTP request into valid UTF-8.",
                ));
            }
        };
        let crlf = "\r\n";
        let mut head_split_crlf = head_str.split(crlf);
        let mut first_line_items = head_split_crlf.next().unwrap().split(' ');
        let method_str = first_line_items.next().unwrap();
        let uri = first_line_items.next().unwrap().to_string();
        let http_version = first_line_items.next().unwrap().to_string();
        let method = match method_str {
            "GET" => RequestMethod::GET,
            "HEAD" => RequestMethod::HEAD,
            "POST" => RequestMethod::POST,
            "PUT" => RequestMethod::PUT,
            "DELETE" => RequestMethod::DELETE,
            "CONNECT" => RequestMethod::CONNECT,
            "OPTIONS" => RequestMethod::OPTIONS,
            "TRACE" => RequestMethod::TRACE,
            "PATCH" => RequestMethod::PATCH,
            // TODO: return error when this happens
            _ => {
                return Err(HTTPError::RequestParseError::new(
                    "Failed to find a valid HTTP request method.",
                ))
            }
        };
        // everything after the host should be
        let mut headers = HashMap::new();
        for header in head_split_crlf {
            let k = header.split(": ").nth(0).unwrap().to_string();
            let v = header.split(": ").nth(1).unwrap().to_string();
            headers.insert(k, v);
        }
        Ok(Request {
            method,
            uri,
            http_version,
            headers: Some(headers),
            body,
        })
    }
}

impl fmt::Display for Request {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} request to {}", self.method, self.uri)
    }
}

pub struct Response {
    http_version: String,
    status: StatusCodes::StatusCode,
    headers: Option<HashMap<String, String>>,
    body: Option<Vec<u8>>,
}

impl Response {
    pub fn new() -> Response {
        Response {
            http_version: "1.1".to_string(),
            status: StatusCodes::OK,
            headers: Some(HashMap::new()),
            body: None,
        }
    }

    pub fn with_http_version(mut self, http_version: &str) -> Response {
        self.http_version = String::from(http_version);
        self
    }

    pub fn with_status(mut self, status: StatusCodes::StatusCode) -> Response {
        self.status = status;
        self
    }

    pub fn with_header(mut self, (k, v): (&str, &str)) -> Self {
        if let Some(headers) = &mut self.headers {
            headers.insert(k.to_string(), v.to_string());
        }
        self
    }

    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Response {
        self.headers = Some(headers);
        self
    }

    pub fn with_body(mut self, body: Vec<u8>) -> Response {
        self.body = Some(body);
        self
    }

    fn set_content_length_header(self) -> Response {
        let length: usize = match &self.body {
            Some(body) => body.len(),
            None => 0,
        };
        self.with_header(("Content-Length", &length.to_string()))
    }

    pub fn prepare_response(self) -> Response {
        self.set_content_length_header()
    }

    pub fn message_bytes(&self) -> Vec<u8> {
        let mut result: Vec<u8> = Vec::with_capacity(8); // HTTP version is at least 8 bytes.
        let status_line = format!("HTTP/{} {}\r\n", self.http_version, self.status);
        result.extend(status_line.into_bytes());
        let mut headers_str = String::new();
        if let Some(headers) = &self.headers {
            for (header, value) in headers {
                headers_str += &format!("{}: {}\r\n", header, value);
            }
        }
        result.extend(headers_str.into_bytes());
        if let Some(body) = &self.body {
            result.extend(b"\r\n");
            result.extend(body);
        }
        result
    }
}

impl fmt::Display for Response {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let bytes = self.message_bytes();
        let text = String::from_utf8_lossy(&bytes).to_string();
        write!(f, "{}", text)
    }
}
