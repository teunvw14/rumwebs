use std::collections::HashMap;
use std::default;
use std::error;
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
            read_buffer: Vec::with_capacity(read_buffer_size),
            write_buffer: Vec::with_capacity(write_buffer_size),
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
        println!("Creating worker!");
        let mut connections = HashMap::new();
        let mut worker = TcpWorker {
            id,
            thread: None,
        };
        let thread = thread::spawn(move || {
            // Failing to create a poll is a fatal error, thus unwrap.
            let mut poll = mio::Poll::new().unwrap();
            // TODO: see if we can remove this magic number
            let mut events = Events::with_capacity(1024);
            loop {
                if !TcpWorker::register_new_conn(
                    &receiver,
                    &mut connections,
                    max_connections,
                    connection_read_buffer_size,
                    connection_write_buffer_size,
                    &poll,
                    id,) {
                    break;
                };

                // Wait at most 1 millisecond to check for new incoming connections
                poll.poll(&mut events, Some(Duration::from_millis(1)));
                for event in &events {
                    let conn_token = event.token();
                    let connection = connections.get_mut(&conn_token).unwrap();
                    if event.is_readable() && connection.ready_to_read {
                        println!("Got readability.");
                        match connection.stream.read(&mut connection.read_buffer) {
                            Ok(_) => {
                                // for now just presume we're done, maybe refactor later
                                println!("Starting job");
                                job(&connection.read_buffer, &mut connection.write_buffer);
                                println!("Job done, setting connection to ready to write.");
                                connection.ready_to_write = true;
                            },
                            Err(e) => {
                                if e.kind() == io::ErrorKind::WouldBlock {
                                    // Treat as 0 bytes written, so do nothing.
                                } else {
                                    // "Actual error" writing to stream.
                                    error!("There was an issue reading from a stream: {}", e);
                                    connection.stream.shutdown(Shutdown::Both);
                                    poll.registry().deregister(&mut connection.stream);
                                    connections.remove(&conn_token);
                                    continue;
                                }
                            }
                        }
                    }
                    if event.is_writable() && connection.ready_to_write && !connection.write_buffer.is_empty() {
                        println!("Got writability.");
                        match connection.stream.write(&connection.write_buffer) {
                            Ok(bytes_written) => {
                                connection.write_buffer.drain(0..bytes_written);
                                println!("Wrote {} bytes.", bytes_written);
                                if connection.write_buffer.is_empty() {
                                    connection.stream.shutdown(Shutdown::Both);
                                    poll.registry().deregister(&mut connection.stream);
                                }
                            },
                            Err(e) => {
                                if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::Interrupted {
                                    // Treat as 0 bytes written, so do nothing.
                                } else {
                                    // "Actual error" writing to stream.
                                    error!("There was an issue writing to a stream: {}", e);
                                    connection.stream.shutdown(Shutdown::Both);
                                    poll.registry().deregister(&mut connection.stream);
                                    connections.remove(&conn_token);
                                }
                            }
                        }
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

    fn register_new_conn(
        receiver: &Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
        connections: &mut HashMap<Token, TcpConnection>,
        max_connections: usize,
        connection_read_buffer_size: usize,
        connection_write_buffer_size: usize,
        poll: &mio::Poll,
        id: usize,
    ) -> bool {
        if connections.len() >= max_connections {
            debug!("Not adding connection, already at max_connections.");
            return true;
        }
        else {
            let message = receiver.lock().unwrap().recv().unwrap();
            println!("Got message!");
            match message {
                WorkerMessage::NewConnection(stream) => {
                    let tcp_connection = TcpConnection::new(stream, connection_read_buffer_size, connection_write_buffer_size);
                    let token = TcpWorker::get_unique_connection_token(&connections);
                    connections.insert(token.clone(), tcp_connection);
                    poll.registry().register(
                        &mut connections.get_mut(&token).unwrap().stream,
                        token,
                        Interest::READABLE | Interest::WRITABLE
                    );
                    println!("Registered new connection!");
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


/// JobDistributors are jobs with senders and receivers for sharing across
///  threads.
struct JobDistributor {
    job: Job,
    sender: mpsc::Sender<WorkerMessage>,
    receiver: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
    assigned_workers: usize,
}

pub struct TcpThreadPool {
    thread_count: usize,
    workers: Vec<TcpWorker>,
    job_list: HashMap<usize, JobDistributor>,
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
        let job_distributor = JobDistributor {
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
        info!("Shutting down all ThreadPool workers.");

        for (_, job_distributor) in &self.job_list {
            for _ in 0..job_distributor.assigned_workers {
                job_distributor.sender.send(WorkerMessage::Shutdown).unwrap();
            }
        }

        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread.join().unwrap();
            }
        }
    }
}
