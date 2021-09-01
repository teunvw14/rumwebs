use rumwebs::HTTP;
use rumwebs::HTTP::file_response_generator;
use rumwebs::HTTP::new_response_generator;

use std::thread::sleep;
use std::time::Duration;
use std::fs;

fn say_hello(req: HTTP::Request) -> HTTP::Response {
    let name = "Harco";
    return HTTP::Response::new()
        .with_body(
            format!(
                r#"<!DOCTYPE html>
<html>

<head>
    <link rel="icon" type="image/png" href="/favicon.png" />
</head>

<body>
    <h1>Hello {}!</h1>
    <img src="/img  ">
</body>

</html>"#,
                name
            )
            .bytes()
            .collect(),
        )
        .prepare_response();
}

fn slp(_: HTTP::Request) -> HTTP::Response {
    sleep(Duration::from_secs(5));
    HTTP::Response::new().with_body(fs::read("res/index.html").unwrap()).prepare_response()
}

fn main() {
    let thread_count = 4;

    println!("Starting server with {} threads...", thread_count);
    HTTP::Server::bind("127.0.0.1:7878")
        .with_thread_count(thread_count)
        .with_route("/", file_response_generator("res/index.html"))
        .with_route("/favicon.png", file_response_generator("res/favicon.png"))
        .with_route("/img", file_response_generator("res/smile.png"))
        .with_route("/name", new_response_generator(Box::new(say_hello)))
        .with_route("/sleep", new_response_generator(Box::new(slp)))
        .start();
}

// fn strip(input: &str) -> &str {
//     let whitespaces = "\r\n\t\x0b\x0c\x00 ";
//     let mut start = 0;
//     for (i, c) in input.chars().enumerate() {
//         if !(whitespaces.contains(c)) {
//             start = i;
//             break;
//         }
//     }
//     let mut end = input.len() - 1;
//     for (i, c) in input.chars().rev().enumerate() {
//         if !(whitespaces.contains(c)) {
//             end -= i;
//             break;
//         }
//     }
//     println!("start: {}, end: {}", start, end);
//     &input[start..end + 1]
// }
