use std::collections::HashMap;
use std::default;
use std::error;
use std::fmt::Debug;
use std::hash::Hash;
use std::io;
use std::io::{Read, Write};
use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use std::net::Shutdown;

use log::{error, warn, info, debug, trace};
use mio;
use mio::Interest;
use mio::{Events,Token};

pub enum WorkerMessage {
    NewConnection(mio::net::TcpStream),
    Shutdown,
}

struct TcpConnection {
    pub stream: mio::net::TcpStream,
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
        receiver: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
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
            // Failing to create a poll is a fatal error, thus unwrap.
            let mut poll = mio::Poll::new()
            .expect(&format!("Something went wrong creating the poll in thread {}.", id));
            // TODO: see if we can remove this magic number
            let mut events = Events::with_capacity(1024);
            loop {
                // Shut down the worker if this returns false.
                if !TcpWorker::register_new_message(
                    &receiver,
                    &mut connections,
                    max_connections,
                    connection_read_buffer_size,
                    connection_write_buffer_size,
                    &poll,
                    id) {
                    break;
                };
                // info!("Registered new connection in thread {}.", id);
                thread::sleep(Duration::from_millis(1));
                // Wait at most 1 millisecond to check for new incoming connections
                poll.poll(&mut events, None).unwrap();
                for event in &events {
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
                        debug!("[Worker {}] Ending connection {:?}.", id, conn_token);
                        connection.stream.shutdown(Shutdown::Write).unwrap();
                        poll.registry().deregister(&mut connection.stream).unwrap();
                        connections.remove(&conn_token);
                    }
                }
            }
        });
        worker.thread = Some(thread);
        worker
    }

    fn get_unique_connection_token(open_connections : &HashMap<mio::Token, TcpConnection>) -> mio::Token {
        let mut i: usize = 0;
        while open_connections.contains_key(&Token(i)) {
            i += 1;
        }
        return Token(i);
    }


    /// Register a new message for processing. Returns false if the worker
    /// should shut down.
    fn register_new_message(
        receiver: &Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
        connections: &mut HashMap<Token, TcpConnection>,
        max_connections: usize,
        connection_read_buffer_size: usize,
        connection_write_buffer_size: usize,
        poll: &mio::Poll,
        id: usize,
        // TODO: use result instead of bool here
    ) -> bool {
        // Discard messages that cannot be handled.
        // TODO: make sure this doesn't break stuff.
        if connections.len() >= max_connections {
            info!("Not adding connection, already at max_connections.");
            return true;
        }
        else {
            // TODO: remove the unwrap
            let message = receiver.lock().unwrap().recv().unwrap();
            match message {
                WorkerMessage::NewConnection(mut stream) => {
                    let token = TcpWorker::get_unique_connection_token(&connections);
                    // TODO: remove unwrap here
                    poll.registry().register(
                        &mut stream,
                        token,
                        Interest::READABLE | Interest::WRITABLE
                    ).unwrap();
                    let tcp_connection = TcpConnection::new(stream, connection_read_buffer_size, connection_write_buffer_size);
                    connections.insert(token, tcp_connection);
                    // DON'T REMOVE THE BELOW LINE: THE PROGRAM DOESN'T WORK WITHOUT IT!
                    // info!("Registered new connection in thread {}.", id);
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
    
}

pub type Job = Arc<dyn Fn(&[u8], &mut Vec<u8>) + Send + Sync + 'static>;


/// JobManager is a job with senders and receivers for sharing across
/// threads.
struct JobManager {
    job: Job,
    sender: mpsc::Sender<WorkerMessage>,
    receiver: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
    assigned_workers: usize,
}

#[derive(Default)]
pub struct TcpThreadPool {
    thread_count: usize,
    workers: Vec<TcpWorker>,
    job_list: HashMap<usize, JobManager>,
    max_connections_per_thread: usize,
    connection_read_buf_size: usize,
    connection_write_buf_size: usize,
}

impl TcpThreadPool {
    /// Creates a ThreadPool with `thread_count` threads.
    ///
    /// # Panics
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
            job_list: HashMap::new(),
            max_connections_per_thread,
            connection_read_buf_size,
            connection_write_buf_size,
        }
    }

    pub fn add_job(&mut self, id: usize, job: Job){
        let (sender, receiver) = mpsc::channel();
        // Wrap the receiver with an Arc<Mutex()> for sharing across threads.
        let receiver = Arc::new(Mutex::new(receiver));
        let job_distributor = JobManager {
            job,
            sender,
            receiver,
            assigned_workers: 0,
        };
        self.job_list.insert(id, job_distributor);
    }

    /// Assign a new job to the thread pool for a certain amount of workers.
    pub fn assign_new_job(&mut self, id: usize, job: Job, worker_count: usize) {
        // Add the job and get the newly created receiver to clone for the
        // workers:
        self.add_job(id, job);
        let job_distributor = self.job_list.get_mut(&id).unwrap();
        for _ in 0..worker_count {
            if self.workers.len() >= self.thread_count {
                error!("Unable to assign {} workers to job because all {} workers already busy.", worker_count, self.thread_count);
                break;
            }
            let receiver = Arc::clone(&job_distributor.receiver);
            let job = Arc::clone(&job_distributor.job);
            self.workers.push(TcpWorker::new(
                self.workers.len() + 1,
                receiver,
                job,
                self.max_connections_per_thread,
                self.connection_read_buf_size,
                self.connection_write_buf_size,
            ));
            job_distributor.assigned_workers += 1;
        }
    }

    pub fn new_stream_for_job(&self, job_id: usize, stream: mio::net::TcpStream) {
        if let Some(job_manager) = self.job_list.get(&job_id) {
            let stream_message = WorkerMessage::NewConnection(stream);
            // TODO: proper error handling
            job_manager.sender.send(stream_message).unwrap();
        } else {
            error!("Couldn't send job to TcpThreadPool workers because job with id {} doesn't exist.", job_id);
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
                    job_manager.sender.send(WorkerMessage::Shutdown).unwrap();
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
