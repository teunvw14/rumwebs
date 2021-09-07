extern crate rumwebs_http;
use rumwebs_http::HTTP;

use std::fs;
use std::thread::sleep;
use std::time::Duration;

fn say_hello(req: HTTP::Request) -> HTTP::Response {
    let name = match req.body {
        // TODO: figure out why `body` is always Some instead of None.
        Some(body) => String::from_utf8_lossy(&body).to_string(),
        None => String::from("Anonymous User"),
    };
    let body: Vec<u8> = format!(
        r#"<!DOCTYPE html><html>
            <head><link rel="icon" type="image/png" href="/favicon.png"/></head>
            <body><h1>Hello {}!</h1><img src="/img"></body></html>"#,
        name
    )
    .bytes()
    .collect();
    return HTTP::Response::new().with_body(&body).prepare_response();
}

fn main() {
    let thread_count = 4;

    println!("Starting server with {} threads...", thread_count);
    // Bind server to localhost:
    HTTP::Server::bind("127.0.0.1:7878")
        // .with_access_policy(HTTP::ServerAccessPolicy::RestrictUp)
        .with_thread_count(thread_count)
        .with_route_to_file("/", "res/index.html")
        .with_route_to_file("/favicon", "res/favicon.png")
        .with_route_to_file("/img", "res/smile.png")
        .with_route("/name", Box::new(say_hello))
        .start();
}
