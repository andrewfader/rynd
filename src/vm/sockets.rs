//! Native TCP sockets for Rynd. Sockets are non-blocking underneath: a read,
//! write, or accept that would block parks its fiber until `poll` reports
//! the socket ready, so a fiber waiting on the network lets the others run.
use super::{
    concurrency::{self, blocking},
    fiber,
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::{RyndError, RyndResult};
use std::collections::{HashMap, HashSet};
use std::convert::TryFrom;
use std::io::{self as stdio, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Default timeout for outbound `tcp_connect` calls. Long enough to survive a
/// kernel SYN backlog hiccup, short enough that a broken loopback stack fails
/// fast instead of hanging the script for the default 60+ second retry chain.
pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Largest single read a script can request.
const MAX_READ: usize = 16 * 1024 * 1024;

pub static BUILTINS: &[(&str, usize)] = &[
    ("tcp_listen", 1),
    ("tcp_accept", 1),
    ("tcp_connect", 1),
    ("tcp_local_addr", 1),
    ("socket_read", 2),
    ("socket_read_bytes", 2),
    ("socket_write", 2),
    ("socket_close", 1),
    ("socket_set_nonblocking", 2),
    ("socket_write_http_chunk", 2),
    ("socket_finish_http_chunks", 1),
    ("socket_read_chunk", 2),
];

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);
static LISTENERS: LazyLock<Mutex<HashMap<i64, Arc<TcpListener>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static STREAMS: LazyLock<Mutex<HashMap<i64, Arc<TcpStream>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
/// Handles a script put in non-blocking mode: their reads and accepts
/// return nil instead of waiting.
static SCRIPT_NONBLOCKING: LazyLock<Mutex<HashSet<i64>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub fn io(err: impl std::fmt::Display) -> RyndError {
    RyndError::IoError(err.to_string())
}

fn text(value: &Value) -> RyndResult<&str> {
    <&str>::try_from(value)
}

fn extract_handle(value: &Value, expected_tag: &str) -> RyndResult<i64> {
    match value {
        Value::Variant(v) if &*v.name == expected_tag && v.values.len() == 1 => match v.values[0] {
            Value::Int(id) => Ok(id),
            _ => Err(error(format!("Invalid {expected_tag} handle"))),
        },
        Value::Int(id) => Ok(*id),
        _ => Err(error(format!(
            "Expected {expected_tag}, got {}",
            value.type_name()
        ))),
    }
}

/// The stream behind a `TcpStream(id)` handle. I/O happens on the returned
/// `Arc` outside the registry lock, so one slow peer never blocks the others.
pub fn stream(value: &Value) -> RyndResult<Arc<TcpStream>> {
    let id = extract_handle(value, "TcpStream")?;
    STREAMS
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| error("TcpStream handle is closed or invalid"))
}

pub fn listener(value: &Value) -> RyndResult<Arc<TcpListener>> {
    let id = extract_handle(value, "TcpListener")?;
    LISTENERS
        .lock()
        .unwrap()
        .get(&id)
        .cloned()
        .ok_or_else(|| error("TcpListener handle is closed or invalid"))
}

pub fn register_stream(stream: TcpStream) -> Value {
    if fiber::SUPPORTED {
        let _ = stream.set_nonblocking(true);
    }
    let id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    STREAMS.lock().unwrap().insert(id, Arc::new(stream));
    Value::variant("TcpStream", vec![Value::Int(id)])
}

pub fn register_listener(listener: TcpListener) -> Value {
    if fiber::SUPPORTED {
        let _ = listener.set_nonblocking(true);
    }
    let id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    LISTENERS.lock().unwrap().insert(id, Arc::new(listener));
    Value::variant("TcpListener", vec![Value::Int(id)])
}

fn script_nonblocking(value: &Value) -> bool {
    handle_id(value).is_some_and(|id| SCRIPT_NONBLOCKING.lock().unwrap().contains(&id))
}

fn handle_id(value: &Value) -> Option<i64> {
    extract_handle(value, "TcpStream")
        .or_else(|_| extract_handle(value, "TcpListener"))
        .ok()
}

#[cfg(unix)]
fn raw_fd(socket: &impl std::os::fd::AsRawFd) -> i32 {
    socket.as_raw_fd()
}
#[cfg(not(unix))]
fn raw_fd<T>(_socket: &T) -> i32 {
    -1
}

/// Retry `op` until it stops reporting `WouldBlock`, parking the fiber
/// until the socket is ready in between.
fn retry<T>(
    fd: i32,
    write: bool,
    deadline: Option<Instant>,
    mut op: impl FnMut() -> stdio::Result<T>,
) -> stdio::Result<T> {
    loop {
        match op() {
            Err(e) if e.kind() == stdio::ErrorKind::WouldBlock && fiber::SUPPORTED => {
                concurrency::wait_io(fd, write, deadline)?;
            }
            Err(e) if e.kind() == stdio::ErrorKind::Interrupted => {}
            other => return other,
        }
    }
}

/// Read into `buf`, letting other fibers run until data arrives (or the
/// peer closes, or `timeout` passes).
pub fn read(stream: &TcpStream, buf: &mut [u8], timeout: Option<Duration>) -> stdio::Result<usize> {
    if !fiber::SUPPORTED {
        stream.set_read_timeout(timeout)?;
    }
    let deadline = timeout.map(|t| Instant::now() + t);
    retry(raw_fd(stream), false, deadline, || (&*stream).read(buf))
}

/// Write all of `bytes`, letting other fibers run while the peer is slow.
pub fn write_all(
    stream: &TcpStream,
    mut bytes: &[u8],
    timeout: Option<Duration>,
) -> stdio::Result<()> {
    if !fiber::SUPPORTED {
        stream.set_write_timeout(timeout)?;
    }
    let deadline = timeout.map(|t| Instant::now() + t);
    while !bytes.is_empty() {
        match retry(raw_fd(stream), true, deadline, || (&*stream).write(bytes))? {
            0 => return Err(stdio::ErrorKind::WriteZero.into()),
            n => bytes = &bytes[n..],
        }
    }
    Ok(())
}

/// Accept a connection, letting other fibers run until one arrives.
pub fn accept(listener: &TcpListener) -> stdio::Result<(TcpStream, SocketAddr)> {
    retry(raw_fd(listener), false, None, || listener.accept())
}

fn read_exact(stream: &TcpStream, mut buf: &mut [u8]) -> RyndResult<()> {
    while !buf.is_empty() {
        match read(stream, buf, None).map_err(io)? {
            0 => return Err(error("Connection closed inside chunk framing")),
            n => buf = &mut buf[n..],
        }
    }
    Ok(())
}

pub fn close(value: &Value) -> RyndResult<()> {
    if let Ok(id) = extract_handle(value, "TcpStream") {
        SCRIPT_NONBLOCKING.lock().unwrap().remove(&id);
        if let Some(stream) = STREAMS.lock().unwrap().remove(&id) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        return Ok(());
    }
    if let Ok(id) = extract_handle(value, "TcpListener") {
        SCRIPT_NONBLOCKING.lock().unwrap().remove(&id);
        LISTENERS.lock().unwrap().remove(&id);
        return Ok(());
    }
    Err(error("Expected TcpStream or TcpListener to close"))
}

/// Connect with a timeout; host names resolve through the system resolver
/// (on a helper thread when other fibers could run meanwhile).
pub fn connect(addr: &str, timeout: Duration) -> RyndResult<TcpStream> {
    // Errors cross from the helper thread as text (script errors are not Send).
    let stream = blocking(|| {
        let mut last = None;
        let addrs: Vec<SocketAddr> = match addr.parse::<SocketAddr>() {
            Ok(a) => vec![a],
            Err(_) => addr.to_socket_addrs().map_err(|e| e.to_string())?.collect(),
        };
        for socket_addr in addrs {
            match TcpStream::connect_timeout(&socket_addr, timeout) {
                Ok(stream) => return Ok(stream),
                Err(e) => last = Some(e),
            }
        }
        Err(match last {
            Some(e) => e.to_string(),
            None => format!("tcp_connect(): no address for {addr}"),
        })
    })
    .map_err(RyndError::IoError)?;
    if fiber::SUPPORTED {
        stream.set_nonblocking(true).map_err(io)?;
    }
    Ok(stream)
}

fn byte_payload(value: &Value) -> RyndResult<Vec<u8>> {
    match value {
        Value::String(s) => Ok(s.as_bytes().to_vec()),
        Value::List(items) => items
            .iter()
            .map(|v| match v {
                Value::Int(b) if (0..=255).contains(b) => Ok(*b as u8),
                _ => Err(error("Byte values must be in 0..255")),
            })
            .collect(),
        _ => Err(error("Expected string or byte list")),
    }
}

fn send(stream: &TcpStream, bytes: &[u8]) -> RyndResult<()> {
    write_all(stream, bytes, None).map_err(io)
}

/// Read one byte at a time up to and including CRLF (for chunk framing).
fn read_line(stream: &TcpStream) -> RyndResult<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        let n = read(stream, &mut byte, None).map_err(io)?;
        if n == 0 {
            return Err(error("Connection closed inside chunk framing"));
        }
        line.push(byte[0]);
        if line.ends_with(b"\r\n") {
            line.truncate(line.len() - 2);
            return String::from_utf8(line).map_err(|_| error("Invalid chunk header"));
        }
        if line.len() > 4096 {
            return Err(error("Chunk header too long"));
        }
    }
}

/// Decode one chunk of a `Transfer-Encoding: chunked` body; `None` at the end.
pub fn read_chunk(stream: &TcpStream, max_len: usize) -> RyndResult<Option<Vec<u8>>> {
    {
        let header = read_line(stream)?;
        let size_text = header.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| error(format!("Invalid chunk size {size_text:?}")))?;
        if size > max_len {
            return Err(error(format!(
                "Chunk of {size} bytes exceeds the {max_len} byte limit"
            )));
        }
        if size == 0 {
            // Skip trailers through the blank line that ends the body.
            while !read_line(stream)?.is_empty() {}
            return Ok(None);
        }
        let mut data = vec![0u8; size];
        read_exact(stream, &mut data)?;
        let mut crlf = [0u8; 2];
        read_exact(stream, &mut crlf)?;
        if &crlf != b"\r\n" {
            return Err(error("Chunk is missing its CRLF terminator"));
        }
        Ok(Some(data))
    }
}

fn read_limit(value: &Value) -> RyndResult<usize> {
    match value {
        Value::Int(n) => Ok(usize::try_from(*n)
            .map_err(|_| error("Read length must be non-negative"))?
            .min(MAX_READ)),
        _ => Err(error("Expected integer max length")),
    }
}

fn bytes_value(data: Vec<u8>) -> Value {
    Value::list(data.into_iter().map(|b| Value::Int(b as i64)).collect())
}

pub fn call(_rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "tcp_listen" => {
            let addr = text(&args[0])?;
            let listener = TcpListener::bind(addr).map_err(io)?;
            Ok(register_listener(listener))
        }

        "tcp_local_addr" => {
            let listener = listener(&args[0])?;
            let addr = listener.local_addr().map_err(io)?;
            Ok(Value::string(addr.to_string()))
        }

        "tcp_accept" => {
            let listener = listener(&args[0])?;
            let accepted = if script_nonblocking(&args[0]) {
                listener.accept()
            } else {
                accept(&listener)
            };
            match accepted {
                Ok((stream, peer)) => Ok(Value::tuple(vec![
                    register_stream(stream),
                    Value::string(peer.to_string()),
                ])),
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(Value::Nil),
                Err(e) => Err(io(e)),
            }
        }

        "tcp_connect" => Ok(register_stream(connect(
            text(&args[0])?,
            TCP_CONNECT_TIMEOUT,
        )?)),

        "socket_read" | "socket_read_bytes" => {
            let stream = stream(&args[0])?;
            let mut buf = vec![0u8; read_limit(&args[1])?];
            let got = if script_nonblocking(&args[0]) {
                (&*stream).read(&mut buf)
            } else {
                read(&stream, &mut buf, None)
            };
            let n = match got {
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(Value::Nil),
                Err(e) => return Err(io(e)),
            };
            buf.truncate(n);
            if name == "socket_read" {
                Ok(Value::string(String::from_utf8_lossy(&buf).into_owned()))
            } else {
                Ok(bytes_value(buf))
            }
        }

        "socket_write" => {
            let bytes = byte_payload(&args[1])?;
            send(&*stream(&args[0])?, &bytes)?;
            Ok(Value::Int(bytes.len() as i64))
        }

        "socket_close" => {
            close(&args[0])?;
            Ok(Value::Nil)
        }

        "socket_set_nonblocking" => {
            let nonblocking = match &args[1] {
                Value::Bool(b) => *b,
                _ => return Err(error("Expected boolean for nonblocking flag")),
            };
            let socket = stream(&args[0])
                .map(|s| s.set_nonblocking(nonblocking || fiber::SUPPORTED))
                .or_else(|_| {
                    listener(&args[0]).map(|l| l.set_nonblocking(nonblocking || fiber::SUPPORTED))
                });
            if let Ok(applied) = socket {
                applied.map_err(io)?;
                let id = handle_id(&args[0]).expect("socket handle");
                let mut flagged = SCRIPT_NONBLOCKING.lock().unwrap();
                if nonblocking {
                    flagged.insert(id);
                } else {
                    flagged.remove(&id);
                }
                return Ok(Value::Nil);
            }
            Err(error(
                "Expected an open TcpStream or TcpListener for socket_set_nonblocking",
            ))
        }

        "socket_write_http_chunk" => {
            let chunk = byte_payload(&args[1])?;
            let mut framed = format!("{:X}\r\n", chunk.len()).into_bytes();
            framed.extend_from_slice(&chunk);
            framed.extend_from_slice(b"\r\n");
            send(&*stream(&args[0])?, &framed)?;
            Ok(Value::Int(framed.len() as i64))
        }

        "socket_finish_http_chunks" => {
            send(&*stream(&args[0])?, b"0\r\n\r\n")?;
            Ok(Value::Nil)
        }

        "socket_read_chunk" => {
            let stream = stream(&args[0])?;
            Ok(match read_chunk(&stream, read_limit(&args[1])?)? {
                Some(data) => Value::string(String::from_utf8_lossy(&data).into_owned()),
                None => Value::Nil,
            })
        }

        _ => Err(error(format!("Unknown socket builtin '{name}'"))),
    }
}
