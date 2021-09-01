use std::fs;
use std::io::prelude::*;
use std::net::TcpListener;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

use rumwebs::ThreadPool;
use rumwebs::HTTP;

fn main() {
    
    let thread_count = 4;

    println!("Starting server with {} threads...", thread_count);
    let listener = TcpListener::bind("127.0.0.1:7878").unwrap();
    let pool = ThreadPool::new(thread_count);
    println!("Server running.");

    for stream in listener.incoming() {
        let stream = stream.unwrap();
        pool.execute(|| {
            handle_connection(stream);
        });
    }
}

fn handle_connection(mut stream: TcpStream) {
    let mut buffer = [0; 1024];

    stream.read(&mut buffer).unwrap();
    let mut req: HTTP::Request = HTTP::Request::new();
    match HTTP::Request::from_bytes(&buffer) {
        Ok(request) => {
            req = request;
            println!("[New Request] {}", req);
        }
        Err(e) => println!("[Request Error] {}", e),
    }

    if req.uri == "/img" {
        // let response = HTTP::Response::new("1.1", StatusCodes::OK, headers, Some(image));
        let response = HTTP::Response::new()
            .with_header(("Content-Type", "image/png"))
            .with_body(fs::read("res/smile.png").unwrap())
            .prepare_response();

        if let Err(e) = stream.write_all(&response.message_bytes()) {
            println!("Something went wrong: {}", e);
        }

        if let Err(e) = stream.flush() {
            println!("Something went wrong flushing TcpStream: {}", e);
        }

        // Read from stream as to not freak out the connected entity.
        if let Err(e) = stream.read(&mut buffer) {
            println!("Something went wrong reading from TcpStream: {}", e);
        }
        return;
    }

    let (status, filename) = if req.uri == "/" {
        (HTTP::StatusCodes::OK, "res/index.html")
    } else if req.uri == "/sleep" {
        thread::sleep(Duration::from_secs(5));
        (HTTP::StatusCodes::OK, "res/index.html")
    } else if req.uri == "/favicon.png" {
        (HTTP::StatusCodes::OK, "res/favicon.png")
    } else {
        (HTTP::StatusCodes::NOT_FOUND, "res/404.html")
    };
    let html_doc = fs::read(filename).unwrap();

    let response = HTTP::Response::new()
        .with_status(status)
        .with_body(html_doc)
        .prepare_response();

    stream.write_all(&response.message_bytes()).unwrap();
    stream.flush().unwrap();
}
