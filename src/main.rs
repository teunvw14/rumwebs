use std::collections::HashMap;
use std::error::Error;
#[macro_use]
extern crate log;
extern crate simplelog;

use config;
use simplelog::*;

extern crate rumwebs_http;
use rumwebs_http::HTTP;

fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging:
    SimpleLogger::init(LevelFilter::Debug, Config::default()).unwrap();

    let mut settings = config::Config::default();
    settings
        .merge(config::File::with_name("Config.toml"))
        .unwrap();

    let settings_application = settings.get_table("application")?;
    let thread_count = settings_application
        .get("thread_count")
        .unwrap()
        .clone()
        .into_int()? as usize;

    let settings_server = settings.get_table("server")?;
    let ip = settings_server.get("ip").unwrap().clone().into_str()?;
    let http_port = settings_server
        .get("http_port")
        .unwrap()
        .clone()
        .into_int()? as usize;
    let tls_port = settings_server
        .get("tls_port")
        .unwrap()
        .clone()
        .into_int()? as usize;
    let tls_enabled = settings_server
        .get("tls_enabled")
        .unwrap()
        .clone()
        .into_bool()?;
    let redirect_http = settings_server
        .get("redirect_http")
        .unwrap()
        .clone()
        .into_bool()?;
    let tls_cert_fullchain = settings_server
        .get("tls_cert_fullchain")
        .unwrap()
        .clone()
        .into_str()?;
    let tls_cert_privkey = settings_server
        .get("tls_cert_privkey")
        .unwrap()
        .clone()
        .into_str()?;

    // Bind server to localhost:
    let mut server = HTTP::Server::builder()
        .with_ip(&ip)
        .with_http_port(http_port)
        .with_tls_port(tls_port)
        .with_access_policy(HTTP::ServerAccessPolicy::RestrictUp)
        .set_tls(tls_enabled, &tls_cert_fullchain, &tls_cert_privkey)
        .with_http_redirection(redirect_http)
        .with_thread_count(thread_count)
        .add_route_to_file("/", "res/index.html")
        .add_route_to_file("/favicon", "res/favicon.png")
        .bind();

    info!("Starting server with {} threads...", thread_count);
    server.start();
    Ok(())
}
