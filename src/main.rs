#[macro_use]
extern crate log;
extern crate simplelog;

use simplelog::*;

extern crate rumwebs_http;
use rumwebs_http::HTTP;

fn main() {
    // Initialize logging:
    SimpleLogger::init(LevelFilter::Debug, Config::default()).unwrap();

    let thread_count = 4;

    // Bind server to localhost:
    let mut server = HTTP::Server::builder()
        // .with_access_policy(HTTP::ServerAccessPolicy::RestrictUp)
        .with_thread_count(thread_count)
        .add_route_to_file("/", "res/index.html")
        .add_route_to_file("/favicon", "res/favicon.png")
        .bind("127.0.0.1:7878");

    info!("Starting server with {} threads...", thread_count);
    server.start();
}
