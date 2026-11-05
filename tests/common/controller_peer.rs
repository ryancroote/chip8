//! Bounded controlled-peer setup and cleanup for public controller tests.
//!
//! Each scenario owns its listener and chooses when to accept connections,
//! including successive connections during explicit recovery.
use std::net::{TcpListener, TcpStream};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tungstenite::WebSocket;

pub struct Peer {
    pub endpoint: String,
    thread: Option<JoinHandle<()>>,
}

impl Peer {
    pub fn start(run: impl FnOnce(TcpListener) + Send + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        Self {
            endpoint: format!("ws://{}", listener.local_addr().unwrap()),
            thread: Some(std::thread::spawn(move || run(listener))),
        }
    }
}

impl Drop for Peer {
    fn drop(&mut self) {
        let result = self.thread.take().unwrap().join();
        if !std::thread::panicking() {
            result.unwrap();
        }
    }
}

pub fn accept(listener: &TcpListener) -> WebSocket<TcpStream> {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                stream
                    .set_write_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                return tungstenite::accept(stream).unwrap();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < deadline, "peer accept deadline");
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => panic!("{e}"),
        }
    }
}
