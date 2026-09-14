//! A tiny stand-in for LibreHardwareMonitor's web server.
//!
//! It answers the two endpoints the adapter uses and records every write, so
//! the integration can be tested end to end — HTTP, URL building, JSON parsing,
//! the device mapping and the write path — without LibreHardwareMonitor, without
//! Windows and without a fan controller.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use parking_lot::Mutex;

/// The fixture tree, shared with the mapping tests.
pub const DATA_JSON: &str = include_str!("../tests/fixtures/data.json");

/// A running fake server.
#[derive(Debug)]
pub struct FakeLhm {
    port: u16,
    requests: Arc<AtomicUsize>,
    writes: Arc<Mutex<Vec<String>>>,
    fail_writes: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
}

impl FakeLhm {
    /// Start the server on an ephemeral port.
    pub fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
        let port = listener.local_addr().expect("local addr").port();
        let requests = Arc::new(AtomicUsize::new(0));
        let writes = Arc::new(Mutex::new(Vec::new()));
        let fail_writes = Arc::new(AtomicBool::new(false));
        let shutdown = Arc::new(AtomicBool::new(false));

        {
            let requests = Arc::clone(&requests);
            let writes = Arc::clone(&writes);
            let fail_writes = Arc::clone(&fail_writes);
            let shutdown = Arc::clone(&shutdown);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let requests = Arc::clone(&requests);
                    let writes = Arc::clone(&writes);
                    let fail_writes = Arc::clone(&fail_writes);
                    std::thread::spawn(move || {
                        handle(stream, &requests, &writes, &fail_writes);
                    });
                }
            });
        }

        Self {
            port,
            requests,
            writes,
            fail_writes,
            shutdown,
        }
    }

    /// `http://127.0.0.1:<port>`
    pub fn base_url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Number of requests served.
    pub fn requests(&self) -> usize {
        self.requests.load(Ordering::Relaxed)
    }

    /// Request lines of every `/Sensor` write.
    pub fn writes(&self) -> Vec<String> {
        self.writes.lock().clone()
    }

    /// Make writes answer with HTTP 500, as LHM does when it cannot reach the
    /// chip.
    pub fn fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::Relaxed);
    }
}

impl Drop for FakeLhm {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        // Unblock the accept loop.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
    }
}

fn handle(
    mut stream: TcpStream,
    requests: &AtomicUsize,
    writes: &Mutex<Vec<String>>,
    fail_writes: &AtomicBool,
) {
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(clone) => clone,
        Err(_) => return,
    });
    let mut request_line = String::new();
    if reader.read_line(&mut request_line).is_err() {
        return;
    }
    // Drain headers.
    loop {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) if line.trim().is_empty() => break,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    requests.fetch_add(1, Ordering::Relaxed);

    let path = request_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("/")
        .to_string();

    let response = if path.starts_with("/data.json") {
        reply(200, "application/json", DATA_JSON)
    } else if path.starts_with("/Sensor") {
        writes.lock().push(request_line.trim().to_string());
        if fail_writes.load(Ordering::Relaxed) {
            reply(500, "text/plain", "could not reach the SuperIO\n")
        } else if path.contains("action=Set") {
            reply(200, "text/plain", "55.0 %\n")
        } else {
            reply(200, "text/plain", "45.0 %\n")
        }
    } else {
        reply(404, "text/plain", "not found\n")
    };

    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

fn reply(status: u16, content_type: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status} OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
