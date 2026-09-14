//! A tiny stand-in for LibreHardwareMonitor's web server.
//!
//! It answers the two endpoints the adapter uses and records every write, so
//! the integration can be tested end to end — HTTP, URL building, JSON parsing,
//! the device mapping and the write path — without LibreHardwareMonitor, without
//! Windows and without a fan controller.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

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
    /// What a `Get` answers. Updated by successful writes, so a read-back test
    /// sees what the channel "holds".
    channel_value: Arc<Mutex<f64>>,
    /// When set, writes are acknowledged but the channel keeps this value — the
    /// signature of a board that silently ignores control writes.
    stuck_at: Arc<Mutex<Option<f64>>>,
    /// Number of `Get` requests, so a test can prove the adapter read back.
    reads: Arc<AtomicU32>,
    /// How `Get` answers: normally the channel value, or a refusal/N-A that makes
    /// the confirmation impossible.
    read_mode: Arc<Mutex<ReadMode>>,
}

/// How the fake channel answers a read-back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadMode {
    /// Answer with the value the channel holds.
    Value,
    /// Answer `N/A`, which LHM does for a channel that reports nothing.
    NotAvailable,
    /// Answer HTTP 500, as LHM does when it cannot reach the chip.
    Error,
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
        let channel_value = Arc::new(Mutex::new(40.0f64));
        let stuck_at = Arc::new(Mutex::new(None::<f64>));
        let reads = Arc::new(AtomicU32::new(0));
        let read_mode = Arc::new(Mutex::new(ReadMode::Value));

        {
            let requests = Arc::clone(&requests);
            let writes = Arc::clone(&writes);
            let fail_writes = Arc::clone(&fail_writes);
            let shutdown = Arc::clone(&shutdown);
            let channel_value = Arc::clone(&channel_value);
            let stuck_at = Arc::clone(&stuck_at);
            let reads = Arc::clone(&reads);
            let read_mode = Arc::clone(&read_mode);
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if shutdown.load(Ordering::Relaxed) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    let requests = Arc::clone(&requests);
                    let writes = Arc::clone(&writes);
                    let fail_writes = Arc::clone(&fail_writes);
                    let channel_value = Arc::clone(&channel_value);
                    let stuck_at = Arc::clone(&stuck_at);
                    let reads = Arc::clone(&reads);
                    let read_mode = Arc::clone(&read_mode);
                    std::thread::spawn(move || {
                        handle(
                            stream,
                            &requests,
                            &writes,
                            &fail_writes,
                            &channel_value,
                            &stuck_at,
                            &reads,
                            &read_mode,
                        );
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
            channel_value,
            stuck_at,
            reads,
            read_mode,
        }
    }

    /// Make read-backs answer `N/A` (the channel reports nothing).
    pub fn read_not_available(&self) {
        *self.read_mode.lock() = ReadMode::NotAvailable;
    }

    /// Make read-backs fail with HTTP 500.
    pub fn read_errors(&self) {
        *self.read_mode.lock() = ReadMode::Error;
    }

    /// Restore normal read-backs.
    pub fn read_normally(&self) {
        *self.read_mode.lock() = ReadMode::Value;
    }

    /// Make the channel acknowledge writes while holding a fixed value, which is
    /// what a board that ignores control writes looks like from the outside.
    pub fn stick_channel_at(&self, value: f64) {
        *self.stuck_at.lock() = Some(value);
        *self.channel_value.lock() = value;
    }

    /// How many `Get` requests were served (i.e. how often a read-back happened).
    pub fn reads(&self) -> u32 {
        self.reads.load(Ordering::Relaxed)
    }

    /// What the channel currently reports.
    pub fn channel_value(&self) -> f64 {
        *self.channel_value.lock()
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

#[allow(clippy::too_many_arguments)]
fn handle(
    mut stream: TcpStream,
    requests: &AtomicUsize,
    writes: &Mutex<Vec<String>>,
    fail_writes: &AtomicBool,
    channel_value: &Mutex<f64>,
    stuck_at: &Mutex<Option<f64>>,
    reads: &AtomicU32,
    read_mode: &Mutex<ReadMode>,
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
            let requested = path
                .split("value=")
                .nth(1)
                .and_then(|value| value.trim().parse::<f64>().ok());
            // A channel stuck at a fixed value acknowledges the write and ignores it.
            let effective = match *stuck_at.lock() {
                Some(stuck) => stuck,
                None => {
                    if let Some(requested) = requested {
                        *channel_value.lock() = requested;
                    }
                    *channel_value.lock()
                }
            };
            reply(200, "text/plain", &format!("{effective:.1} %\n"))
        } else {
            reads.fetch_add(1, Ordering::Relaxed);
            match *read_mode.lock() {
                ReadMode::Value => reply(
                    200,
                    "text/plain",
                    &format!("{:.1} %\n", *channel_value.lock()),
                ),
                ReadMode::NotAvailable => reply(200, "text/plain", "N/A\n"),
                ReadMode::Error => reply(500, "text/plain", "could not read the chip\n"),
            }
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
