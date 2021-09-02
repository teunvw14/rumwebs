// use rumwebs::HTTP;
// use rumwebs::HTTP::file_response_generator;
// use rumwebs::HTTP::new_response_generator;
extern crate rumwebs_http;
use rumwebs_http::HTTP;

use std::fs;
use std::thread::sleep;
use std::time::Duration;

fn say_hello(req: HTTP::Request) -> HTTP::Response {
    let name = match req.body {
        Some(body) => String::from_utf8_lossy(&body).to_string(),
        None => String::from("Anonymous User"),
    };
    return HTTP::Response::new()
        .with_body(
            format!(
                r#"<!DOCTYPE html><html>
                <head><link rel="icon" type="image/png" href="/favicon.png"/></head>
                <body><h1>Hello {}!</h1><img src="/img"></body></html>"#,
                name
            )
            .bytes()
            .collect(),
        )
        .prepare_response();
}

fn slp(_: HTTP::Request) -> HTTP::Response {
    sleep(Duration::from_secs(5));
    HTTP::Response::new()
        .with_body(fs::read("res/index.html").unwrap())
        .prepare_response()
}

fn main() {
    let thread_count = 4;

    println!("Starting server with {} threads...", thread_count);
    // Bind server to localhost:
    HTTP::Server::bind("127.0.0.1:7878")
        .with_thread_count(thread_count)
        .with_route("/", HTTP::file_response_generator("res/index.html"))
        .with_route("/favicon", HTTP::file_response_generator("res/favicon.png"))
        .with_route("/img", HTTP::file_response_generator("res/smile.png"))
        .with_route("/name", HTTP::new_response_generator(Box::new(say_hello)))
        .with_route("/sleep", HTTP::new_response_generator(Box::new(slp)))
        .start();
}
