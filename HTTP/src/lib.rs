extern crate rumwebs_tcp_thread_pool;
pub mod HTTP {
    pub mod HTTPError;
    pub mod RequestMethods;
    pub mod StatusCodes;

    use std::collections::HashMap;
    use std::net::SocketAddr;
    use std::path::{Path, PathBuf};
    use std::io::BufReader;
    use std::time::Duration;
    use std::str::FromStr;
    use std::sync::Arc;
    use std::thread;
    use std::fmt;
    use std::env;
    use std::fs;
    use std::io;
    
    use chrono::prelude::*;
    use lazy_static::lazy_static;
    use log::{error, warn, info, debug, trace};
    use regex::Regex;
    use rustls;
    use rustls::internal::pemfile::*;
    use mio;
    use mio::{Poll, Events, Token, Interest};
    use mio::net::{TcpListener, TcpStream};

    use rumwebs_tcp_thread_pool::TcpThreadPool;
    use rumwebs_tcp_thread_pool::{Job, WorkerMessage};

    const SERVER_JOB_ID: usize = 1;

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

    impl RequestMethod {
        pub fn from_str(s: &str) -> Result<RequestMethod, HTTPError::InvalidRequestMethod> {
            match s {
                "GET" => Ok(RequestMethod::GET),
                "HEAD" => Ok(RequestMethod::HEAD),
                "POST" => Ok(RequestMethod::POST),
                "PUT" => Ok(RequestMethod::PUT),
                "DELETE" => Ok(RequestMethod::DELETE),
                "CONNECT" => Ok(RequestMethod::CONNECT),
                "OPTIONS" => Ok(RequestMethod::OPTIONS),
                "TRACE" => Ok(RequestMethod::TRACE),
                "PATCH" => Ok(RequestMethod::PATCH),
                // This should never happen, since the 
                _ => {
                    return Err(HTTPError::InvalidRequestMethod::new(
                        "String doesn't make a valid HTTP request method.",
                    ))
                }
            }
        }
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
        pub request_line: String,
        pub method: RequestMethod,
        pub uri: String,
        pub http_version: String,
        pub headers: Option<HashMap<String, String>>,
        pub body: Option<Vec<u8>>,
    }

    impl Request {
        pub fn new() -> Request {
            Request {
                request_line: String::new(),
                method: RequestMethod::GET,
                uri: "/".to_string(),
                http_version: "1.1".to_string(),
                headers: None,
                body: None,
            }
        }

        pub fn from_bytes(bytes: &[u8]) -> Result<Request, HTTPError::InvalidRequest> {
            // Check if the bytes even form an HTTP request.
            let lossy_http = String::from_utf8_lossy(bytes);
            if !lossy_http.contains("HTTP/") {
                return Err(HTTPError::InvalidRequest::new(
                    &format!("Unable to find request line in bytes {}", lossy_http)
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
                body = Some(bytes[pos+4..].to_vec());
            } else {
                head = bytes.to_vec();
                body = None;
            };
            let head_str = match String::from_utf8(head) {
                Ok(h) => h,
                Err(_) => {
                    return Err(HTTPError::InvalidRequest::new(
                        "Failed to parse HTTP request into valid UTF-8.",
                    ));
                }
            };
            let crlf = "\r\n";
            let mut head_split_crlf = head_str.split(crlf);
            let request_line = head_split_crlf.next()
            .ok_or_else(|| HTTPError::InvalidRequest::new(
                "Unable to get first line from head/body split."
            ))?;
            // Use lazy_static to only compile this Regex once.
            lazy_static! {
                // Unwrap is safe because this fails only if the regex pattern is incorrect.
                static ref RE: Regex = Regex::new(r"^(GET|HEAD|POST|PUT|DELETE|CONNECT|OPTIONS|TRACE|PATCH) (/.*?) (HTTP/\d\.\d)$").unwrap();
            }

            match RE.captures(request_line) {
                Some(caps) => {
                    let method_str = caps.get(1)
                    .ok_or_else(|| HTTPError::InvalidRequest::new(
                        "Failed getting `method_str` from request line."
                    ))?.as_str();
                    let method = RequestMethod::from_str(method_str)?;
                    let uri = String::from(caps.get(2)
                    .ok_or_else(|| HTTPError::InvalidRequest::new(
                        "Failed getting `uri` from request line."
                    ))?.as_str());
                    let http_version = String::from(caps.get(3)
                    .ok_or_else(|| HTTPError::InvalidRequest::new(
                        "Failed getting `http_version` from request line."
                    ))?.as_str());
                    let mut headers = HashMap::new();
                    for header in head_split_crlf {
                        let k = header.split(": ").nth(0)
                        .ok_or_else(|| HTTPError::InvalidRequest::new(
                        &format!("Couldn't get header key from header line {}", &header)
                        ))?.to_string();
                        let v = header.split(": ").nth(1)
                        .ok_or_else(|| HTTPError::InvalidRequest::new(
                        &format!("Couldn't get header value from header line {}", &header)
                        ))?.to_string();
                        headers.insert(k, v);
                    }
                    let owned_request_line = request_line.to_string();
                    Ok(Request {
                        request_line: owned_request_line,
                        method,
                        uri,
                        http_version,
                        headers: Some(headers),
                        body,
                    })
                    }
                None => {
                    return Err(HTTPError::InvalidRequest::new(
                        &format!("Something is not right with this request line. {}", request_line),
                    ))
                }
            }
        }
    }

    impl fmt::Display for Request {
        fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
            write!(f, "{} {} {}", self.method, self.uri, self.http_version)
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

        pub fn get_header_value(&self, header: &str) -> Option<&String> {
            match &self.headers {
                Some(headers) => headers.get(header),
                None => None,
            }
        }

        pub fn with_body(mut self, body: &[u8]) -> Response {
            self.body = Some(body.to_vec());
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
            result.extend(b"\r\n");
            if let Some(body) = &self.body {
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
    
    // create a newtype to improve readability
    #[derive(Clone)]
    pub struct ResponseGenerator(Arc<Box<dyn Send + Sync + Fn(Request) -> Response>>);
    
    impl ResponseGenerator {
        pub fn new(f: Box<dyn Send + Sync + Fn(Request) -> Response>) -> ResponseGenerator {
            ResponseGenerator (
                Arc::new(f)
            )
        }
        
        pub fn from_file(path: PathBuf, panic_on_err: bool) -> ResponseGenerator {
            if panic_on_err {
                // Make sure that the file exists and is readable.
                if let Err(e) = fs::File::open(&path) {
                    panic!("Unable to open file {}, got error: `{}`", &path.display(), e);
                }
            }
            let func = move |_req| {
                let bytes = fs::read(&path);
                match bytes {
                    Ok(bytes) => Response::new().with_body(&bytes).prepare_response(),
                    Err(e) => {
                        // If we get to this point, creation must have succeeded (since we panic
                        // when we can't open the file). This means that something must have changed
                        // about the file.
                        error!("Something went wrong opening route file {}. Problem is: {}", path.display(), e);
                        not_found_response()
                    }
                }
            };
            ResponseGenerator::new(Box::new(func))
        }
    }

    fn moved_permanently_response(to: &str) -> Response {
        Response::new()
        .with_status(StatusCodes::MOVED_PERMANENTLY)
        .with_header(("Location", to))
        .prepare_response()
    }

    fn bad_request_response() -> Response {
        let body = b"<!DOCTYPE html><html><body><h1>400 BAD REQUEST</h1></body></html>";
        Response::new()
            .with_status(StatusCodes::BAD_REQUEST)
            .with_body(body)
            .prepare_response()
    }

    fn forbidden_response() -> Response {
        let body = b"<!DOCTYPE html><html><body><h1>403 FORBIDDEN</h1></body></html>";
        Response::new()
            .with_status(StatusCodes::FORBIDDEN)
            .with_body(body)
            .prepare_response()
    }

    fn not_found_response() -> Response {
        let body = b"<!DOCTYPE html><html><body><h1>404 NOT FOUND</h1></body></html>";
        Response::new()
            .with_status(StatusCodes::NOT_FOUND)
            .with_body(body)
            .prepare_response()
    }

    #[derive(Clone)]
    pub enum ServerAccessPolicy {
        AllowAll, // Serve whatever file is requested.
        RestrictUp, // Serve anything that is in the web dir or deeper.
        Restricted, // Only serve on routed paths.
    }

    pub struct ServerBuilder {
        server: Server,
    }

    impl ServerBuilder {
        pub fn set_tls(mut self, tls_enabled: bool, fullchain_path: &str, privkey_path: &str) -> ServerBuilder {
            self.server.tls_enabled = tls_enabled;
            if tls_enabled {
                let mut cert_file = BufReader::new(fs::File::open(fullchain_path)
                .expect(&format!("Unable to open certificate fullchain path '{}'.", fullchain_path)));
                // `expect` because inability to load certificate is a fatal error.
                let cert_chain = certs(&mut cert_file).unwrap();
                let mut key_file = BufReader::new(fs::File::open(privkey_path)
                .expect(&format!("Unable to open certificate privkey path '{}'.", fullchain_path)));
                // Again: `expect` because inability to load certificate is a fatal error.
                let mut keys = pkcs8_private_keys(&mut key_file).unwrap();
                // Should be only one key so we can pop from the keys vector.
                let key_der = keys.pop().expect("Can't get key from certificate key file.");
                // Build the config with the opened certificates:
                let mut config = rustls::ServerConfig::new(rustls::NoClientAuth::new());
                // `expect` so that this fails if the certificates are invalid.
                config.set_single_cert(cert_chain, key_der).expect("Invalid certificates.");
                self.server.tls_config = Some(Arc::new(config));
            }
            self
        }
        
        pub fn add_route(mut self, route: &str, f: Box<dyn Send + Sync + Fn(Request) -> Response>) -> ServerBuilder {
            self.server.unfinished_routes.insert(route.to_string(), ResponseGenerator::new(f));
            self
        }
        
        pub fn add_route_to_file(mut self, route: &str, path: PathBuf) -> ServerBuilder {
            self.server.unfinished_routes.insert(route.to_string(), ResponseGenerator::from_file(path, true));
            self
        }
        
        pub fn add_routes(mut self, routes: HashMap<String, ResponseGenerator>) -> ServerBuilder {
            // Add (so not replace) server routes.
            for (route, response) in routes {
                self.server.unfinished_routes.insert(route, response);
            };
            self
        }

        pub fn with_ip(mut self, ip: &str) -> ServerBuilder {
            self.server.ip = ip.to_string();
            self
        }

        pub fn with_http_port(mut self, port: usize) -> ServerBuilder {
            self.server.http_port = port;
            self
        }

        pub fn with_tls_port(mut self, port: usize) -> ServerBuilder {
            self.server.tls_port = port;
            self
        }

        // TODO: maybe reinstante this function later
        // pub fn with_thread_count(mut self, count: usize) -> ServerBuilder {
        //     // Drop thread pool to make sure all
        //     // threads finish their tasks.
        //     if let Some(pool) = &mut self.server.thread_pool {
        //         pool.set_thread_count(count);
        //     }
        //     self
        // }

        pub fn set_conn_per_thread(mut self, conn_per_thread: usize) -> ServerBuilder {
            self.server.conn_per_thread = conn_per_thread;
            self
        }

        pub fn with_access_policy(mut self, policy: ServerAccessPolicy) -> ServerBuilder {
            self.server.access_policy = policy;
            self
        }

        pub fn with_http_redirection(mut self, redirect_http: bool) -> ServerBuilder {
            self.server.redirect_http = redirect_http;
            self
        }

        pub fn set_default_host(mut self, default_host: &str) -> ServerBuilder {
            self.server.default_host = String::from(default_host);
            self
        }

        fn set_mandatory_routes(mut self) -> ServerBuilder {
            // Use self = self.add_route(...) to get back ownership after calling add_route.
            if !self.server.routes.contains_key("/400") {
                self = self.add_route("/400", Box::new(|_req| bad_request_response()));
            }
            if !self.server.routes.contains_key("/403") {
                self = self.add_route("/403", Box::new(|_req| forbidden_response()));
            }
            if !self.server.routes.contains_key("/404") {
                self = self.add_route("/404", Box::new(|_req| not_found_response()));
            }
            self
        }

        pub fn bind(mut self) -> Server {
            let port = match self.server.tls_enabled {
                false => self.server.http_port,
                true => self.server.tls_port,
            };
            self.server.bind_addr = format!("{}:{}", self.server.ip, port);
            // expect (i.e. unwrap) because inability to bind TcpListener is a fatal
            let bind_addr = SocketAddr::from_str(&self.server.bind_addr)
            .expect(&format!("Can't parse ip address: {}", &self.server.bind_addr));
            self.server.listener = Some(TcpListener::bind(bind_addr)
            .expect(&format!("Problem binding TcpListener to server bind_addr {}", bind_addr)));
            // Route 400, 403 and 404 by default, as they are necessary for the
            // server to function. They can be overwritten.
            self.set_mandatory_routes()
            .server
        }
    }

    pub struct Server {
        // TODO: see if this Option can be removed cause it's ugly man.
        pub thread_pool: Option<TcpThreadPool>,
        thread_count: usize,
        conn_per_thread: usize,
        pub routes: Arc<HashMap<String, ResponseGenerator>>,
        unfinished_routes: HashMap<String, ResponseGenerator>,
        pub access_policy: ServerAccessPolicy,
        pub bind_addr: String,
        ip: String,
        http_port: usize,
        tls_port: usize,
        pub tls_enabled: bool,
        redirect_http: bool,
        listener: Option<TcpListener>,
        tls_config: Option<Arc<rustls::ServerConfig>>,
        running_path: PathBuf,
        default_host: String,
    }

    impl Server {
        pub fn builder() -> ServerBuilder {
            let empty_job: Job = Arc::new(|_, _| {});
            ServerBuilder {
                server: Server {
                    thread_pool: None,
                    conn_per_thread: 1,
                    thread_count: 1,
                    routes: Arc::new(HashMap::new()),
                    unfinished_routes: HashMap::new(),
                    access_policy: ServerAccessPolicy::Restricted,
                    bind_addr: String::new(),
                    ip: String::new(),
                    http_port: 0,
                    tls_port: 0,
                    tls_enabled: false,
                    redirect_http: false,
                    listener: None,
                    tls_config: None,
                    running_path: PathBuf::new(),
                    default_host: String::new(),
                }
            }
        }

        pub fn server_job(&self) -> Job {
            return Arc::new({                
                let running_path = self.running_path.clone();
                let access_policy = self.access_policy.clone();
                let routes = Arc::clone(&self.routes);
                let tls_enabled = self.tls_enabled.clone();
                // The actual job:
                move |bytes_in: &[u8], write_buff: &mut Vec<u8>| {
                    println!("Doing job");
                    // let mut request_line = String::new();
                    println!("flag 1");
                    let parsed_request = Request::from_bytes(bytes_in);
                    println!("flag 2");
                    let http_response = match parsed_request {
                        Ok(request) => {
                            debug!(r#"New Request "{}""#, request);
                            let unknown_route_handler;
                            let generator = match routes.get(&request.uri) {
                                Some(route_response) => route_response,
                                None => {
                                    unknown_route_handler = Server::get_unknown_route_handler(&request.uri, &access_policy, &running_path);
                                    &unknown_route_handler
                                },
                            };
                            let response_fn = Arc::clone(&generator.0);
                            // request_line = String::from(&request.request_line);
                            response_fn(request)
                        }
                        Err(e) => {
                            error!("Got invalid HTTP request, sending back HTTP 400. Problem was: {}", e);
                            bad_request_response()
                        },
                    };
                    let default_body_size_string = String::from("0");
                    let response_size = http_response.get_header_value("Content-Length")
                    .unwrap_or(&default_body_size_string).to_string();
                    let response_code = http_response.status.clone();
                    *write_buff = http_response.message_bytes();
                }
            })
        }

        pub fn start(&mut self) {
            self.routes = Arc::new(self.unfinished_routes.clone());
            self.panic_on_tls_without_certificates();
            // Set the running path and panic if the current dir cannot
            // be gotten from the system.
            self.running_path = env::current_dir().unwrap();
            if self.tls_enabled && self.redirect_http {
                info!("HTTP redirection currently under maintenance.");
                // self.start_http_redirection();
            }
            info!("Running path set to: {:?}", self.running_path);
            info!("Now serving at {}", self.bind_addr);
            self.thread_pool = Some(TcpThreadPool::new(self.thread_count, 1024, 1024, 2048));
            self.thread_pool.unwrap().assign_new_job(SERVER_JOB_ID, self.server_job(), self.thread_count);
            self.handle_connections();
        }

        fn redirect_non_tls(mut stream: TcpStream, default_host: &str) {
            // if let Err(e) = stream.set_read_timeout(Some(Duration::from_millis(500))) {
            //     error!("Something went wrong setting the stream read timeout: {}", e);
            //     return;
            // };
            let mut response = bad_request_response();
            if let Ok(request) = Server::http_request_from_stream(&mut stream) {
                // Read the host from the request and change HTTP to HTTPS.
                if let Some(headers) = request.headers {
                    if let Some(host) = headers.get("Host") {
                        let host_no_port = match host.contains(':') {
                            false => host,
                            // TODO: come up with a better default for the host 
                            // (now this will return "" if there is nothing 
                            // before the colon) 
                            true => host.split(':').next().unwrap_or(default_host),
                        };
                        let https_host = format!("https://{}", host_no_port);
                        response = moved_permanently_response(&https_host);
                    }
                }
            };
            Server::send_http_response_to_stream(stream, response);
        }

        // fn start_http_redirection(&self) {
        //     // Spawn a thread that redirects all non-TLS traffic to the TLS address.
        //     let http_addr = format!("{}:{}", self.ip, self.http_port);
        //     info!("Starting HTTP redirection at {}", http_addr);
        //     // warn!("HTTP redirection will take up one of the server's thread pool's workers. Performance might be reduced.");
        //     // `expect` here because inability to bind the TcpListener is a fatal error
        //     let forwarder = TcpListener::bind(&http_addr)
        //     .expect(&format!("Problem binding TcpListener to {} http redirection.", &http_addr));
        //     let default_host = self.default_host.clone();
        //     thread::spawn(move || {
        //         for tcp_stream in forwarder.incoming() {
        //             debug!("Got new non-TLS connection, redirecting...");
        //             match tcp_stream {
        //                 Err(e) => {
        //                     error!("Unable to open stream, got error {}", e);
        //                     continue;
        //                 }
        //                 Ok(stream) => {
        //                     Server::redirect_non_tls(stream, &default_host);
        //                 }
        //             }
        //         }
        //     });
        // }
            
        fn panic_on_tls_without_certificates(&self) {
            if self.tls_enabled && self.tls_config.is_none() {
                panic!("TLS was enabled, but no certificates were supplied.");
            }
        }

        fn handle_request<T: io::Write + io::Read>(
            mut stream: T,
            running_path: &Path,
            access_policy: &ServerAccessPolicy,
            routes: Arc<HashMap<String, ResponseGenerator>>
        ) -> (String, StatusCodes::StatusCode, String) {
            let mut request_line = String::new();

            let parsed_request = Server::http_request_from_stream(&mut stream);
            let http_response = match parsed_request {
                Ok(request) => {
                    debug!(r#"New Request "{}""#, request);
                    let unknown_route_handler;
                    let generator = match routes.get(&request.uri) {
                        Some(route_response) => route_response,
                        None => {
                            unknown_route_handler = Server::get_unknown_route_handler(&request.uri, &access_policy, &running_path);
                            &unknown_route_handler
                        },
                    };
                    let response_fn = Arc::clone(&generator.0);
                    request_line = String::from(&request.request_line);
                    response_fn(request)
                }
                Err(e) => {
                    error!("Got invalid HTTP request, sending back HTTP 400. Problem was: {}", e);
                    bad_request_response()
                },
            };
            let default_body_size_string = String::from("0");
            let response_size = http_response.get_header_value("Content-Length")
            .unwrap_or(&default_body_size_string).to_string();
            let response_code = http_response.status.clone();
            Server::send_http_response_to_stream(stream, http_response);
            (request_line, response_code, response_size)
        }

        fn connection_to_thread(&self, mut tcp_stream: TcpStream) {
            let msg = WorkerMessage::NewConnection(tcp_stream);
            println!("sending connection to thread");
            self.thread_pool.as_ref().unwrap().sender.send(msg);
        }

        fn request_to_thread(&self, mut tcp_stream: TcpStream) {
            let mut server_config = None;
            if let Some(config) = &self.tls_config {
                server_config = Some(Arc::clone(config))
            };
            
            let running_path = self.running_path.clone();
            let access_policy = self.access_policy.clone();
            let routes = Arc::clone(&self.routes);
            let tls_enabled = self.tls_enabled.clone();

            // self.thread_pool.execute(move || {
            //     // Do some work that is needed for logging later.
            //     let peer_ip = match tcp_stream.peer_addr() {
            //         Ok(ipaddr) => ipaddr.to_string(),
            //         Err(e) => String::from("*unkown*"),
            //     };
            //     let (request_line, response_code, response_size) =  if tls_enabled {
            //         // Unwrap is safe here because server is always Some() with tls_enabled.
            //         let mut session = rustls::ServerSession::new(&server_config.unwrap());
            //         let stream = rustls::Stream::new(&mut session, &mut tcp_stream);
            //         Server::handle_request(stream, &running_path, &access_policy, routes)
            //     } else {
            //         Server::handle_request(tcp_stream, &running_path, &access_policy, routes)
            //     };
                
            //     // Output log according to "Common Log Format", see https://en.wikipedia.org/wiki/Common_Log_Format
            //     let time: DateTime<Local> = Local::now();
            //         info!(r#"{} - - [{}] "{}" {} {}"#, 
            //     peer_ip, time.format("%d/%b/%Y:%H:%M:%S %z"), request_line, response_code, response_size);
            // })
        }

        fn handle_connections(&mut self) -> ! {            
            let mut poll = Poll::new().unwrap();
            let mut events = Events::with_capacity(2048);

            // Register the socket with `Poll`
            poll.registry().register(self.listener.as_mut().unwrap(), Token(0), Interest::READABLE).unwrap();

            // Unwrap here because the server is expected to have a working
            // TcpListener set up when this function is called.
            loop {
                print!("p");
                poll.poll(&mut events, Some(Duration::from_millis(100))).unwrap();
                for event in &events {
                    println!("Got some event.");
                    if event.is_readable() && event.token() == Token(0) {
                        loop {
                            // If something went wrong dealing with the stream, we don't
                            // want to send back data, so we continue the loop to the next
                            // incoming connection.
                            match self.listener.as_mut().unwrap().accept() {
                                Err(e) => {
                                    if e.kind() == io::ErrorKind::WouldBlock {
                                        println!("would block to accept another conn");
                                        break;
                                    }
                                    else {
                                        error!("Unable to open stream, got error {}", e);
                                        continue;
                                    }
                                }
                                Ok((tcp_stream, remote_addr)) => {
                                    println!("Got new tcp stream");
                                    // TODO: maybe re-enable timeouts later
                                    // if let Err(e) = tcp_stream.set_read_timeout(Some(Duration::from_millis(500))) {
                                        //     error!("Something went wrong setting the stream read timeout: {}", e);
                                        // };
                                        // if let Err(e) = tcp_stream.set_write_timeout(Some(Duration::from_millis(500))) {
                                            //     error!("Something went wrong setting the stream write timeout: {}", e);
                                            // }
                                    self.connection_to_thread(tcp_stream);
                                }
                            }
                        }
                        println!("out of acceptance loop");
                    }
                    println!("done processing event.");
                }
            }
        }

        fn file_within_running_path(running_path: &Path, requested_file: &Path) -> bool {
            let mut result = false;
            let mut buff = PathBuf::from(requested_file);
            buff.pop();
            // Check if the buffer is empty: this means that the file is in the current directory.
            if buff.iter().next() == None {
                result = true;
            }
            // Canonicalize paths and check if the requested file path starts with
            // the running path, i.e. the requested file is within the running path.
            if let Ok(file_canonical_path) = requested_file.canonicalize() {
                if let Ok(running_path_canonical) = running_path.canonicalize() {
                    if file_canonical_path.starts_with(running_path_canonical) {
                        result = true;
                    }
                }
            }
            result
        }

        fn get_unknown_route_handler(uri: &str, access_policy: &ServerAccessPolicy, running_path: &Path) -> ResponseGenerator {
            // Strip the leading "/" from the uri.
            let uri_stripped = &uri.to_string()[1..];
            match access_policy {
                ServerAccessPolicy::AllowAll => ResponseGenerator::from_file(PathBuf::from(uri_stripped), false),
                ServerAccessPolicy::RestrictUp => {
                    if let Ok(requested_file) = PathBuf::from_str(uri_stripped) {
                        if Server::file_within_running_path(running_path, &requested_file) {
                            return ResponseGenerator::from_file(requested_file, false);
                        } else {
                            debug!("Illegal file request, '{:?}' is not in running dir.", &requested_file);
                            return ResponseGenerator::new(Box::new(|_| { forbidden_response() }));
                        }
                    }
                    return ResponseGenerator::new(Box::new(|_| { not_found_response() }));
                },
                // RestrictAll policy sends back a 404 for any unknown paths.
                ServerAccessPolicy::Restricted => ResponseGenerator::new(Box::new(|_| { not_found_response() })),
            }
        }

        fn http_request_from_stream<T: io::Read>(
            stream: &mut T,
        ) -> Result<Request, HTTPError::InvalidRequest> {
            let mut buffer = [0; 1024]; // 1kb buffer
            let mut bytes_vec = Vec::new();
            while let Ok(bytes_read) = stream.read(&mut buffer) {
                for i in 0..bytes_read {
                    bytes_vec.push(buffer[i]);
                }
                if bytes_read < buffer.len() {
                    break;
                }
            };
            match Request::from_bytes(&bytes_vec) {
                Ok(request) => return Ok(request),
                Err(e) => return Err(e),
            }
        }

        fn send_http_response_to_stream<T: io::Write>(mut stream: T, response: Response) {
            if let Err(e) = stream.write_all(&response.message_bytes()) {
                error!("! Something went wrong sending a response: {}", e);
                return;
            };
            if let Err(e) = stream.flush() {
                error!("! Something went wrong flushing the response: {}", e);
                return;
            }
        }
    }
}