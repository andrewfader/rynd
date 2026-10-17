//! Native TCP sockets for Rynd. Blocking calls release the fiber baton, so a
//! fiber waiting on the network lets the others run.
use super::{
    concurrency::blocking,
    runtime::{Runtime, error},
    value::Value,
};
use crate::error::{RyndError, RyndResult};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

/// Default timeout for outbound `tcp_connect` calls. Long enough to survive a
/// kernel SYN backlog hiccup, short enough that a broken loopback stack fails
/// fast instead of hanging the script for the default 60+ second retry chain.
pub const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Largest single read a script can request.
const MAX_READ: usize = 16 * 1024 * 1024;

pub const BUILTINS: &[(&str, usize)] = &[
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

pub fn io(err: impl std::fmt::Display) -> RyndError {
    RyndError::IoError(err.to_string())
}

fn text(value: &Value) -> RyndResult<&str> {
    <&str>::try_from(value)
}

fn extract_handle(value: &Value, expected_tag: &str) -> RyndResult<i64> {
    match value {
        Value::Variant { name, values } if name == expected_tag && values.len() == 1 => {
            match &values[0] {
                Value::Int(id) => Ok(*id),
                _ => Err(error(format!("Invalid {expected_tag} handle"))),
            }
        }
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
    let id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    STREAMS.lock().unwrap().insert(id, Arc::new(stream));
    Value::variant("TcpStream", vec![Value::Int(id)])
}

pub fn register_listener(listener: TcpListener) -> Value {
    let id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
    LISTENERS.lock().unwrap().insert(id, Arc::new(listener));
    Value::variant("TcpListener", vec![Value::Int(id)])
}

pub fn close(value: &Value) -> RyndResult<()> {
    if let Ok(id) = extract_handle(value, "TcpStream") {
        if let Some(stream) = STREAMS.lock().unwrap().remove(&id) {
            let _ = stream.shutdown(Shutdown::Both);
        }
        return Ok(());
    }
    if let Ok(id) = extract_handle(value, "TcpListener") {
        LISTENERS.lock().unwrap().remove(&id);
        return Ok(());
    }
    Err(error("Expected TcpStream or TcpListener to close"))
}

/// Connect with a timeout; host names resolve through the system resolver.
pub fn connect(addr: &str, timeout: Duration) -> RyndResult<TcpStream> {
    blocking(|| {
        let mut last = None;
        let addrs: Vec<SocketAddr> = match addr.parse::<SocketAddr>() {
            Ok(a) => vec![a],
            Err(_) => addr.to_socket_addrs().map_err(io)?.collect(),
        };
        for socket_addr in addrs {
            match TcpStream::connect_timeout(&socket_addr, timeout) {
                Ok(stream) => return Ok(stream),
                Err(e) => last = Some(e),
            }
        }
        Err(match last {
            Some(e) => io(e),
            None => error(format!("tcp_connect(): no address for {addr}")),
        })
    })
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

fn write_all(stream: &TcpStream, bytes: &[u8]) -> RyndResult<()> {
    blocking(|| {
        let mut writer = stream;
        writer.write_all(bytes)?;
        writer.flush()
    })
    .map_err(io)
}

/// Read one byte at a time up to and including CRLF (for chunk framing).
fn read_line(stream: &TcpStream) -> RyndResult<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    let mut reader = stream;
    loop {
        let n = reader.read(&mut byte).map_err(io)?;
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
    blocking(|| {
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
        let mut reader = stream;
        reader.read_exact(&mut data).map_err(io)?;
        let mut crlf = [0u8; 2];
        reader.read_exact(&mut crlf).map_err(io)?;
        if &crlf != b"\r\n" {
            return Err(error("Chunk is missing its CRLF terminator"));
        }
        Ok(Some(data))
    })
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
            match blocking(|| listener.accept()) {
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
            let n = match blocking(|| (&*stream).read(&mut buf)) {
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
            write_all(&*stream(&args[0])?, &bytes)?;
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
            if let Ok(stream) = stream(&args[0]) {
                stream.set_nonblocking(nonblocking).map_err(io)?;
                return Ok(Value::Nil);
            }
            if let Ok(listener) = listener(&args[0]) {
                listener.set_nonblocking(nonblocking).map_err(io)?;
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
            write_all(&*stream(&args[0])?, &framed)?;
            Ok(Value::Int(framed.len() as i64))
        }

        "socket_finish_http_chunks" => {
            write_all(&*stream(&args[0])?, b"0\r\n\r\n")?;
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
