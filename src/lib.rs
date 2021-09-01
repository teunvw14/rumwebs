use std::sync::mpsc;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;

pub mod HTTP;

enum Message {
    NewJob(Job),
    Shutdown,
}

struct Worker {
    id: usize,
    thread: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, receiver: Arc<Mutex<mpsc::Receiver<Message>>>) -> Worker {
        let thread = thread::spawn(move || loop {
            let message = receiver.lock().unwrap().recv().unwrap();
            match message {
                Message::NewJob(job) => {
                    // println!("Worker {} got a job; executing.", id);
                    job();
                }
                Message::Shutdown => {
                    println!(
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
    sender: mpsc::Sender<Message>,
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

        ThreadPool { workers, sender }
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
        let message = Message::NewJob(Box::new(f));
        self.sender.send(message).unwrap();
    }
}

impl Drop for ThreadPool {
    fn drop(&mut self) {
        println!("Shutting down all workers.");

        for _ in &self.workers {
            self.sender.send(Message::Shutdown).unwrap();
        }

        for worker in &mut self.workers {
            if let Some(thread) = worker.thread.take() {
                thread.join().unwrap();
            }
        }
    }
}
