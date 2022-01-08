use std::collections::HashMap;
use std::default;
use std::error;
use std::error::Error;
use std::fmt::Debug;
use std::hash::Hash;
use std::io;
use std::io::{Read, Write};
use std::net::SocketAddr;
use std::str::FromStr;
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::RecvTimeoutError;
use std::thread;
use std::time::Duration;
use std::net::Shutdown;

use log::{error, warn, info, debug, trace};
use mio;
use mio::Interest;
use mio::Poll;
use mio::event::Event;
use mio::{Events,Token};
use mio::net::{TcpListener, TcpStream};

pub enum WorkerMessage {
    NewConnection(Token, mio::net::TcpStream),
    Shutdown,
}

pub enum InfoMessage {
    ReceivedConnection(usize, Token),
    DeleteConnection(Token, TcpStream),
}

struct TcpConnection {
    stream: mio::net::TcpStream,
    ready_to_write: bool,
    ready_to_read: bool,
    read_buffer: Vec<u8>,
    write_buffer: Vec<u8>,
}

impl TcpConnection {
    fn new(tcp_stream: mio::net::TcpStream, read_buffer_size: usize, write_buffer_size: usize) -> TcpConnection {
        TcpConnection { 
            stream: tcp_stream, 
            ready_to_write: false, 
            ready_to_read: true, 
            read_buffer: vec![0; read_buffer_size],
            write_buffer: vec![0; write_buffer_size],
        }
    }
}

struct TcpWorker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl TcpWorker {
    pub fn new(
        id: usize,
        message_receiver: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
        event_receiver: mpsc::Receiver<Event>,
        info_sender: mpsc::Sender<InfoMessage>,
        job: Job,
        max_connections: usize,
        connection_read_buffer_size: usize,
        connection_write_buffer_size: usize,
    ) -> TcpWorker {
        let mut worker = TcpWorker {
            id,
            thread: None,
        };
        let thread = thread::spawn(move || {
            let mut connections = HashMap::new();
            info!("Started worker with id {}.", id);
            loop {
                // Shut down the worker if this returns false.
                if !TcpWorker::register_new_message(
                    &message_receiver,
                    &info_sender,
                    &mut connections,
                    max_connections,
                    connection_read_buffer_size,
                    connection_write_buffer_size,
                    id) {
                    break;
                };
                TcpWorker::process_events(&mut connections, &event_receiver, &info_sender, &job);
                // info!("Registered new connection in thread {}.", id);
                thread::sleep(Duration::from_millis(1));
            }
        });
        worker.thread = Some(thread);
        worker
    }

    /// Register a new message for processing. Returns false if the worker
    /// should shut down.
    fn register_new_message(
        message_receiver: &Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
        info_sender: &mpsc::Sender<InfoMessage>,
        connections: &mut HashMap<Token, TcpConnection>,
        max_connections: usize,
        connection_read_buffer_size: usize,
        connection_write_buffer_size: usize,
        id: usize,
        // TODO: use result instead of bool here
    ) -> bool {
        // Discard messages that cannot be handled.
        // TODO: make sure this doesn't break stuff. (it probably does)
        if connections.len() >= max_connections {
            warn!("Not adding connection, already at max_connections.");
            return true;
        }
        else {
            // TODO: make this recv_timeout instead of regular recv 
            let message = message_receiver.lock().unwrap().recv().unwrap();
            match message {
                WorkerMessage::NewConnection(token, stream) => {
                    let tcp_connection = TcpConnection::new(stream, connection_read_buffer_size, connection_write_buffer_size);
                    connections.insert(token, tcp_connection);
                    // TODO: remove the unwrap
                    info_sender.send(InfoMessage::ReceivedConnection(id, token)).unwrap();
                    // DON'T REMOVE THE BELOW LINE: THE PROGRAM DOESN'T WORK WITHOUT IT!
                    info!("Registered new connection in thread {}.", id);
                    return true;
                }
                WorkerMessage::Shutdown => {
                    debug!(
                        "Worker {} received shutdown message, terminating thread.",
                        id
                    );
                    return false;
                }
            }
        }
    }
    
    fn process_events(
        connections: &mut HashMap<Token, TcpConnection>,
        event_receiver: &mpsc::Receiver<Event>,
        info_sender: &mpsc::Sender<InfoMessage>,
        job: &Job) {
        loop {
            // TODO: remove "magic number"
            match event_receiver.recv_timeout(Duration::from_micros(10)) {
                Ok(event) => {
                    let conn_token = event.token();
                    let connection = connections.get_mut(&conn_token).unwrap();
                    let mut delete_conn = false;
                    if event.is_readable() && connection.ready_to_read {
                        loop {
                            match connection.stream.read(&mut connection.read_buffer) {
                                Ok(0) => {
                                    debug!("Got zero bytes.");
                                    job(&connection.read_buffer, &mut connection.write_buffer);
                                    connection.ready_to_write = true;
                                    connection.ready_to_read = false;
                                    delete_conn = true;
                                    break;
                                }
                                Ok(n) => {
                                    // for now just presume we're done, maybe refactor later
                                    debug!("Read {} bytes", n);
                                    job(&connection.read_buffer, &mut connection.write_buffer);
                                    connection.ready_to_write = true;
                                    connection.ready_to_read = false;
                                    delete_conn = true; 
                                    break;
                                },
                                Err(e) => {
                                    if e.kind() == io::ErrorKind::WouldBlock {
                                        // Spurious wake up; treat as 0 bytes read, so do nothing.
                                        break;
                                    } else {
                                        // "Actual error" writing to stream.
                                        error!("There was an issue reading from a stream: {}", e);
                                        // TODO: properly handle these unwraps
                                        connection.ready_to_write = true;
                                        connection.ready_to_read = false;
                                        delete_conn = true; 
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    if event.is_writable() && connection.ready_to_write && !connection.write_buffer.is_empty() {
                        match connection.stream.write(&connection.write_buffer) {
                            Ok(bytes_written) => {
                                connection.write_buffer.drain(0..bytes_written);
                                debug!("Wrote {} bytes.", bytes_written);
                                if connection.write_buffer.is_empty() {
                                    delete_conn = true;
                                }
                            },
                            Err(e) => {
                                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted {
                                    // Treat as 0 bytes written, so do nothing.
                                } else {
                                    // "Actual error" writing to stream.
                                    error!("There was an issue writing to a stream: {}", e);
                                    delete_conn = true;
                                }
                            }
                        }
                    }
                    if delete_conn {
                        // TODO: properly handle these unwraps
                        debug!("Ending connection {:?}.", conn_token);
                        // connection.stream.shutdown(Shutdown::Write).unwrap();
                        if let Some(conn) = connections.remove(&conn_token) {
                            info_sender.send(InfoMessage::DeleteConnection(conn_token, conn.stream)).unwrap();
                        }
                    }
                }
                Err(e) => {
                    break;
                }
            }
        }
    }
}

pub type Job = Arc<dyn Fn(&[u8], &mut Vec<u8>) + Send + Sync + 'static>;


/// JobManager is a job with senders and receivers for sharing across
/// threads.
struct JobManager {
    job: Job,
    assigned_workers: usize,
    // This maps a connection (identified by Tokens) to the responsible
    // worker (identified by a usize).
    connections_worker_map: HashMap<Token, usize>,
    worker_message_sender: mpsc::Sender<WorkerMessage>,
    worker_message_receiver: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
    // Map from id (of the worker) to the Event sender for that worker
    worker_event_senders: HashMap<usize, mpsc::Sender<Event>>,
    worker_info_sender: mpsc::Sender<InfoMessage>,
    worker_info_receiver: mpsc::Receiver<InfoMessage>,
}

pub struct TcpThreadPool {
    thread_count: usize,
    workers: Vec<TcpWorker>,
    // `TcpListener`s are identified by mio Tokens
    tcp_listeners: HashMap<Token, TcpListener>,
    job_list: HashMap<Token, JobManager>,
    // The open `TcpStream`s managed by the thread pool
    connection_worker_map: HashMap<Token, usize>,
    max_connections_per_thread: usize,
    connection_read_buf_size: usize,
    connection_write_buf_size: usize,
    poll: Poll,
}

impl Default for TcpThreadPool {
    fn default() -> Self {
        return TcpThreadPool {
            thread_count: 1,
            workers: Vec::new(),
            // `TcpListener`s are identified by mio Tokens
            tcp_listeners: HashMap::new(),
            job_list: HashMap::new(),
            // The open `TcpStream`s managed by the thread pool
            connection_worker_map: HashMap::new(),
            max_connections_per_thread: 0,
            connection_read_buf_size: 0,
            connection_write_buf_size: 0,
            poll: Poll::new().unwrap(),
        }
    }
}

impl TcpThreadPool {
    /// Creates a ThreadPool with `thread_count` threads.
    ///
    /// ## Panics
    ///
    /// This will panic if the thread count is zero.
    pub fn new(
            thread_count: usize,
            max_connections_per_thread: usize,
            connection_read_buf_size: usize,
            connection_write_buf_size: usize
        ) -> TcpThreadPool {

        assert_ne!(thread_count, 0);
        
        let workers = Vec::with_capacity(thread_count);
        TcpThreadPool {
            thread_count,
            workers,
            tcp_listeners: HashMap::new(),
            job_list: HashMap::new(),
            connection_worker_map: HashMap::new(),
            max_connections_per_thread,
            connection_read_buf_size,
            connection_write_buf_size,
            poll: Poll::new().unwrap(),
        }
    }

    fn get_unique_poll_token(&self) -> Token {
        let mut i: usize = 1;
        while self.tcp_listeners.contains_key(&Token(i)) || 
            self.connection_worker_map.contains_key(&Token(i)) {
            i += 1;
        }
        return Token(i-1);
    }

    pub fn create_job(&mut self, job: Job, ip: &str, port: usize) -> Result<Token, ()> {
        let bind_address = format!("{}:{}", ip, port);
        match SocketAddr::from_str(&bind_address) {
            Err(e) => {
                error!("Can't parse IP address {}: {}", bind_address, e);
                return Err(());
            }
            Ok(bind_address) => {
                match TcpListener::bind(bind_address) {
                    Err(e) => {
                        error!("There was a problem binding a job to address {}: {}", bind_address, e);
                        return Err(());
                    },
                    Ok(mut tcp_listener) => { 
                        let (worker_message_sender, worker_message_receiver) = mpsc::channel();
                        let (worker_info_sender, worker_info_receiver) = mpsc::channel();
                        // Wrap the receiver with an Arc<Mutex()> for sharing across threads.
                        let worker_message_receiver = Arc::new(Mutex::new(worker_message_receiver));                
                        let job_manager = JobManager {
                            job,
                            assigned_workers: 0,
                            connections_worker_map: HashMap::new(),
                            worker_message_sender,
                            worker_message_receiver,
                            worker_event_senders: HashMap::new(),
                            worker_info_receiver,
                            worker_info_sender,
                        };
                        let token = self.get_unique_poll_token();
                        self.job_list.insert(token, job_manager);
                        //  TODO: remove unwrap
                        self.poll.registry().register(&mut tcp_listener, token, Interest::READABLE).unwrap();
                        self.tcp_listeners.insert(token, tcp_listener);
                        debug!("Created new job for token {:?}", token);
                        return Ok(token);
                    }
                }
            }
        }
    }

    /// Assign a new job to the thread pool for a certain amount of workers.
    pub fn assign_new_job(&mut self, job: Job, ip: &str, port: usize, worker_count: usize) {
        // Add the job and get the newly created receiver to clone for the
        // workers:
        match self.create_job(job, ip, port) {
            Err(_) => {
                error!("Failed to assign new job.");
            }
            Ok(token) => {
                let job_manager = self.job_list.get_mut(&token).unwrap();
                for _ in 0..worker_count {
                    if self.workers.len() >= self.thread_count {
                        error!("Unable to assign {} workers to job because all {} workers already busy.", worker_count, self.thread_count);
                        break;
                    }
                    let connection_receiver = Arc::clone(&job_manager.worker_message_receiver);
                    let (event_sender, event_receiver) = mpsc::channel();
                    let worker_info_sender = job_manager.worker_info_sender.clone();

                    let id = self.workers.len() + 1;
                    job_manager.worker_event_senders.insert(id, event_sender);
                    let job = Arc::clone(&job_manager.job);
                    self.workers.push(TcpWorker::new(
                        id,
                        connection_receiver,
                        event_receiver,
                        worker_info_sender,
                        job,
                        self.max_connections_per_thread,
                        self.connection_read_buf_size,
                        self.connection_write_buf_size,
                    ));
                    job_manager.assigned_workers += 1;
                }
                debug!("Successfully assigned {} workers new job.", worker_count);
            }
        }
    }

    pub fn new_connection_for_job(&self, token: Token, stream: mio::net::TcpStream) {
        if let Some(job_manager) = self.job_list.get(&token) {
            let stream_message = WorkerMessage::NewConnection(token,stream);
            // TODO: proper error handling
            job_manager.worker_message_sender.send(stream_message).unwrap();
        } else {
            error!("Unable to send connection to worker because there's no job for token {:?}.", token);
            error!("Available job tokens are: {:?}", self.job_list.keys());
        }
    }

    fn process_event_listener(&self, event: &Event, listener: &TcpListener) {
        if event.is_readable() {
            loop {
                // If something went wrong dealing with the stream, we don't
                // want to send back data, so we continue the loop to the next
                // incoming connection.
                match listener.accept() {
                    Ok((mut tcp_stream, _remote_addr)) => {
                        // TODO: maybe re-enable timeouts later
                        debug!("Got new valid tcp stream, sending to thread pool.");
                        let token = self.get_unique_poll_token();
                        // TODO: remove unwrap
                        self.poll.registry().register(&mut tcp_stream, token, Interest::READABLE | Interest::WRITABLE).unwrap();
                        self.new_connection_for_job(token, tcp_stream);
                        debug!("Successfully accepted new TCP stream.");
                    }
                    Err(e) => {
                        if e.kind() == io::ErrorKind::WouldBlock {
                            //debug!("---------------");
                            //debug!("Would block to accept new connection, waiting for new readiness event.");
                            // Wait until another readiness event is received.
                            break;
                        }
                        else {
                            error!("Unable to open stream, got error {}", e);
                            continue;
                        }
                    }
                }
            }
        }
    }

    fn process_event(&self, event: &Event) {
        let token = event.token();
        // We receive an event for a particular connection (represented)
        // by a token, so we send it to the responsible worker.
        if let Some(listener) = self.tcp_listeners.get(&token) {
            debug!("Got new connection for TcpLisener.");
            self.process_event_listener(event, listener);
        }
        if let Some(worker_id) = self.connection_worker_map.get(&token) {
            debug!("[Worker {}] Got new event for connection {:?}.", worker_id, token);
            let job_manager = self.job_list.get(&token).unwrap();
            let sender = job_manager.worker_event_senders.get(worker_id).unwrap();
            sender.send(event.to_owned()).unwrap();
        }
    }

    fn process_job_worker_info(&mut self) {
        for (_token, job_manager) in &self.job_list {
            loop {
                // TODO: remove magic number
                match job_manager.worker_info_receiver.recv_timeout(Duration::from_micros(1)) {
                    Ok(InfoMessage::ReceivedConnection(id, token)) => {
                        self.connection_worker_map.insert(token, id);
                    },
                    Ok(InfoMessage::DeleteConnection(token, mut connection)) => {
                        self.connection_worker_map.remove(&token);
                        // TODO: remove unwrap
                        self.poll.registry().deregister(&mut connection).unwrap();
                    }
                    Err(RecvTimeoutError::Timeout) => {
                        break;
                    }
                    Err(RecvTimeoutError::Disconnected) => {
                        error!("The mpsc channel disconnected.");
                    }
                }
            }
        }
    }

    pub fn start(&mut self) -> ! {
        // TODO: remove magic number
        let mut events = Events::with_capacity(8192);
        loop {
            // TODO: remove below magic number
            self.poll.poll(&mut events, Some(Duration::from_micros(1))).unwrap();
            // debug!("Got some events for TCP listener, processing one by one.");
            for event in &events {
                self.process_event(event);
            }
            self.process_job_worker_info();
        }
    }


    // TODO: maybe put this back in later.
    // pub fn set_thread_count(&mut self, new_thread_count: usize) {
    //     let current_thread_count = self.workers.len();
    //     if new_thread_count > current_thread_count {
    //         for i in 0..(new_thread_count - current_thread_count) {
    //             self.workers.push(TcpWorker::new(
    //                 i + 1 + current_thread_count,
    //                 Arc::clone(&self.receiver),
    //                 Arc::clone(&self.job),
    //                 self.max_connections_per_thread,
    //                 self.connection_read_buf_size,
    //                 self.connection_write_buf_size,
    //             ));
    //         }
    //     } else if new_thread_count < current_thread_count {
    //         for _ in 0..(current_thread_count - new_thread_count) {
    //             self.workers.pop();
    //         }
    //     }
    // }
}

impl Drop for TcpThreadPool {
    fn drop(&mut self) {
        // Do nothing if there are no workers.
        if self.workers.len() > 0 {
            info!("Shutting down all ThreadPool workers.");
    
            for (_, job_manager) in &self.job_list {
                for _ in 0..job_manager.assigned_workers {
                    job_manager.worker_message_sender.send(WorkerMessage::Shutdown).unwrap();
                }
            }
    
            for worker in &mut self.workers {
                if let Some(thread) = worker.thread.take() {
                    thread.join().unwrap();
                }
            }
        }
    }
}
