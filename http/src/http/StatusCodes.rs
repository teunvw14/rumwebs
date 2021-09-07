use std::fmt;

pub struct StatusCode {
    code: usize,
    name: &'static str,
}

impl fmt::Display for StatusCode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{} {}", self.code, self.name)
    }
}

pub const OK: StatusCode = StatusCode {
    code: 200,
    name: "OK",
};

pub const BAD_REQUEST: StatusCode = StatusCode {
    code: 400,
    name: "Bad Request",
};

pub const UNAUTHORIZED: StatusCode = StatusCode {
    code: 401,
    name: "Unauthorized",
};

pub const FORBIDDEN: StatusCode = StatusCode {
    code: 403,
    name: "Forbidden",
};

pub const NOT_FOUND: StatusCode = StatusCode {
    code: 404,
    name: "Not Found",
};

pub const INTERNAL_SERVER_ERROR: StatusCode = StatusCode {
    code: 500,
    name: "Internal Server Error",
};
