use std::collections::HashMap;
use std::error::Error;
use std::path::PathBuf;
use std::str::FromStr;

#[macro_use]
extern crate log;
extern crate simplelog;

use config;
use simplelog::*;

extern crate rumwebs_http;
use rumwebs_http::HTTP;

fn main() -> Result<(), Box<dyn Error>> {
    // Load settings
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
    let log_level = settings_application
        .get("log_level")
        .unwrap()
        .clone()
        .into_str()?;
    let conn_per_thread = settings_application
        .get("conn_per_thread")
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
    let default_host = settings_server
        .get("default_host")
        .unwrap()
        .clone()
        .into_str()?;

    // Initialize logging:
    SimpleLogger::init(
        LevelFilter::from_str(&log_level).unwrap(),
        Config::default(),
    )
    .expect("Unable to set up logger.");

    // Bind server to localhost:
    let mut server = HTTP::Server::builder()
        .with_ip(&ip)
        .with_http_port(http_port)
        .with_tls_port(tls_port)
        .set_tls(tls_enabled, &tls_cert_fullchain, &tls_cert_privkey)
        .set_default_host(&default_host)
        .with_http_redirection(redirect_http)
        .with_thread_count(thread_count)
        .add_route_to_file("/", PathBuf::from("res/index.html"))
        .add_route_to_file("/favicon", PathBuf::from("res/favicon.png"))
        .bind();

    info!("Starting server with {} threads...", thread_count);
    server.start();
    Ok(())
}
