use std::collections::HashMap;
use std::fs;

extern crate rumwebs_http;
use rumwebs_http::HTTP;

#[test]
fn format_http_response() {
    let status = HTTP::StatusCodes::NOT_FOUND;
    let body = fs::read("res/404.html").unwrap();
    let resp = HTTP::Response::new()
        .with_status(status)
        .with_header(("Content-Type", "image/png"))
        .with_body(&body);
    assert_eq!(
        format!("{}", resp),
        "HTTP/1.1 404 Not Found\r\nContent-Type: image/png\r\n\r\n<!DOCTYPE html>\r\n<html>\r\n<body>\r\n<h1>404 NOT FOUND</h1>\r\n</body>\r\n</html>"
    );
}

#[test]
fn format_http_response_without_body() {
    let status = HTTP::StatusCodes::OK;
    let resp = HTTP::Response::new().with_status(status);
    assert_eq!(format!("{}", resp), "HTTP/1.1 200 OK\r\n\r\n");
}
