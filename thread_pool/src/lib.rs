use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

use log::{info, debug};

enum WorkerMessage {
    NewJob(Job),
    Shutdown,
}

struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>) -> Worker {
        let thread = thread::spawn(move || loop {
            let message = receiver.lock().unwrap().recv().unwrap();
            match message {
                WorkerMessage::NewJob(job) => {
                    job();
                }
                WorkerMessage::Shutdown => {
                    debug!(
                        "Worker {} received shutdown message, terminating thread.",
                        id
                    );
                    break;
                }
            }
        });
        Worker {
            id,
            thread: Some(thread),
        }
    }
}

type Job = Box<dyn FnOnce() + Send + 'static>;

pub struct ThreadPool {
    workers: Vec<Worker>,
    sender: mpsc::Sender<WorkerMessage>,
    receiver: Arc<Mutex<mpsc::Receiver<WorkerMessage>>>,
}

impl ThreadPool {
    /// Creates a ThreadPool with `thread_count` threads.
    ///
    /// # Panics
    ///
    /// This will panic if the thread count is zero.
    pub fn new(thread_count: usize) -> ThreadPool {
        assert_ne!(thread_count, 0);

        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));

        let mut workers = Vec::with_capacity(thread_count);

        // Create the threads:
        for i in 0..thread_count {
            workers.push(Worker::new(i + 1, Arc::clone(&receiver)));
        }

        ThreadPool {
            workers,
            sender,
            receiver,
        }
    }

    pub fn set_thread_count(&mut self, new_thread_count: usize) {
        let current_thread_count = self.workers.len();
        if new_thread_count > current_thread_count {
            for i in 0..(new_thread_count - current_thread_count) {
                self.workers.push(Worker::new(
                    i + 1 + current_thread_count,
                    Arc::clone(&self.receiver),
                ));
            }
        } else if new_thread_count < current_thread_count {
            for _ in 0..(current_thread_count - new_thread_count) {
                self.workers.pop();
            }
        }
    }

    /// Execute something with one of the threads in the thread pool.
    ///
    /// # Panics
    ///
    /// This might panic if the sending of the job to the threads fails, but
    /// that should never happen.
    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + 'static,
    {
        let message = WorkerMessage::NewJob(Box::new(f));
        self.sender.send(message).unwrap();
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        info!("Shutting down all ThreadPool workers.");

        for _ in &self.workers {
            self.sender.send(WorkerMessage::Shutdown).unwrap();
        }

        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread.join().unwrap();
            }
        }
    }
}
