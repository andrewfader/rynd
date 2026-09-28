//! Native TCP sockets and HTTP parsing for Rynd.
use super::{
    runtime::{error, Runtime},
    value::Value,
};
use crate::error::{RyndError, RyndResult};
use std::collections::{BTreeMap, HashMap};
use std::convert::TryFrom;
use std::io::{Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

/// Default timeout for outbound `tcp_connect` calls. Long enough to survive a
/// kernel SYN backlog hiccup, short enough that a broken loopback stack fails
/// fast instead of hanging the script for the default 60+ second retry chain.
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);

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
    ("parse_http_request", 1),
    ("format_http_response", 3),
    ("socket_write_http_chunk", 2),
    ("socket_finish_http_chunks", 1),
    ("socket_read_chunk", 2),
    ("match_route", 2),
    ("conn", 2),
    ("halt", 1),
    ("put_status", 2),
    ("put_resp_header", 3),
    ("put_resp_body", 2),
    ("text_response", 3),
    ("json_response", 3),
    ("html_response", 3),
    ("plug_send", 2),
];

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);
static LISTENERS: LazyLock<Mutex<HashMap<i64, TcpListener>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static STREAMS: LazyLock<Mutex<HashMap<i64, TcpStream>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn io(err: impl std::fmt::Display) -> RyndError {
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

pub fn call(_rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "tcp_listen" => {
            let addr = text(&args[0])?;
            let listener = TcpListener::bind(addr).map_err(io)?;
            let id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
            LISTENERS.lock().unwrap().insert(id, listener);
            Ok(Value::variant("TcpListener", vec![Value::Int(id)]))
        }

        "tcp_local_addr" => {
            let id = extract_handle(&args[0], "TcpListener")?;
            let listeners = LISTENERS.lock().unwrap();
            let listener = listeners
                .get(&id)
                .ok_or_else(|| error("TcpListener handle is closed or invalid"))?;
            let addr = listener.local_addr().map_err(io)?;
            Ok(Value::string(addr.to_string()))
        }

        "tcp_accept" => {
            let id = extract_handle(&args[0], "TcpListener")?;
            let (stream, peer_addr) = {
                let listeners = LISTENERS.lock().unwrap();
                let listener = listeners
                    .get(&id)
                    .ok_or_else(|| error("TcpListener handle is closed or invalid"))?;
                match listener.accept() {
                    Ok(pair) => pair,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        return Ok(Value::Nil);
                    }
                    Err(e) => return Err(io(e)),
                }
            };
            let stream_id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
            STREAMS.lock().unwrap().insert(stream_id, stream);
            Ok(Value::tuple(vec![
                Value::variant("TcpStream", vec![Value::Int(stream_id)]),
                Value::string(peer_addr.to_string()),
            ]))
        }

        "tcp_connect" => {
            let addr = text(&args[0])?;
            // Parse to a `SocketAddr` so we can pass it to `connect_timeout`;
            // parsing accepts both `"host:port"` and `""` (for an unspecified
            // listener) — fall back to a manual `connect()` if the string is
            // not a literal SocketAddr (e.g. an interface name).
            let parsed: Option<SocketAddr> = addr.parse().ok();
            let stream = match parsed {
                Some(socket_addr) => {
                    TcpStream::connect_timeout(&socket_addr, TCP_CONNECT_TIMEOUT).map_err(io)?
                }
                None => TcpStream::connect(addr).map_err(io)?,
            };
            let stream_id = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
            STREAMS.lock().unwrap().insert(stream_id, stream);
            Ok(Value::variant("TcpStream", vec![Value::Int(stream_id)]))
        }

        "socket_read" | "socket_read_bytes" => {
            let id = extract_handle(&args[0], "TcpStream")?;
            let max_len = match &args[1] {
                Value::Int(n) => usize::try_from(*n)
                    .map_err(|_| error("Read length must be non-negative"))?
                    .min(16 * 1024 * 1024),
                _ => return Err(error("Expected integer max length")),
            };
            let mut buf = vec![0u8; max_len];
            let n = {
                let mut streams = STREAMS.lock().unwrap();
                let stream = streams
                    .get_mut(&id)
                    .ok_or_else(|| error("TcpStream handle is closed or invalid"))?;
                match stream.read(&mut buf) {
                    Ok(bytes_read) => bytes_read,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        return Ok(Value::Nil);
                    }
                    Err(e) => return Err(io(e)),
                }
            };
            buf.truncate(n);
            if name == "socket_read" {
                Ok(Value::string(String::from_utf8_lossy(&buf).into_owned()))
            } else {
                Ok(Value::list(
                    buf.into_iter().map(|b| Value::Int(b as i64)).collect(),
                ))
            }
        }

        "socket_write" => {
            let id = extract_handle(&args[0], "TcpStream")?;
            let bytes = match &args[1] {
                Value::String(s) => s.as_bytes().to_vec(),
                Value::List(items) => items
                    .iter()
                    .map(|v| match v {
                        Value::Int(b) if (0..=255).contains(b) => Ok(*b as u8),
                        _ => Err(error("Byte values must be in 0..255")),
                    })
                    .collect::<RyndResult<Vec<u8>>>()?,
                _ => return Err(error("socket_write expects string or byte list")),
            };
            let mut streams = STREAMS.lock().unwrap();
            let stream = streams
                .get_mut(&id)
                .ok_or_else(|| error("TcpStream handle is closed or invalid"))?;
            stream.write_all(&bytes).map_err(io)?;
            stream.flush().map_err(io)?;
            Ok(Value::Int(bytes.len() as i64))
        }

        "socket_close" => {
            if let Ok(id) = extract_handle(&args[0], "TcpStream") {
                if let Some(stream) = STREAMS.lock().unwrap().remove(&id) {
                    let _ = stream.shutdown(Shutdown::Both);
                }
                return Ok(Value::Nil);
            }
            if let Ok(id) = extract_handle(&args[0], "TcpListener") {
                LISTENERS.lock().unwrap().remove(&id);
                return Ok(Value::Nil);
            }
            Err(error("Expected TcpStream or TcpListener to close"))
        }

        "socket_set_nonblocking" => {
            let nonblocking = match &args[1] {
                Value::Bool(b) => *b,
                _ => return Err(error("Expected boolean for nonblocking flag")),
            };
            if let Ok(id) = extract_handle(&args[0], "TcpStream") {
                let streams = STREAMS.lock().unwrap();
                let stream = streams
                    .get(&id)
                    .ok_or_else(|| error("TcpStream handle is closed or invalid"))?;
                stream.set_nonblocking(nonblocking).map_err(io)?;
                return Ok(Value::Nil);
            }
            if let Ok(id) = extract_handle(&args[0], "TcpListener") {
                let listeners = LISTENERS.lock().unwrap();
                let listener = listeners
                    .get(&id)
                    .ok_or_else(|| error("TcpListener handle is closed or invalid"))?;
                listener.set_nonblocking(nonblocking).map_err(io)?;
                return Ok(Value::Nil);
            }
            Err(error(
                "Expected TcpStream or TcpListener for socket_set_nonblocking",
            ))
        }

        "parse_http_request" => {
            let raw = text(&args[0])?;
            let (header_part, body) = match raw.split_once("\r\n\r\n") {
                Some((h, b)) => (h, b),
                None => match raw.split_once("\n\n") {
                    Some((h, b)) => (h, b),
                    None => (raw, ""),
                },
            };
            let mut lines = header_part.lines();
            let first_line = lines.next().unwrap_or("").trim();
            let mut parts = first_line.split_whitespace();
            let method = parts.next().unwrap_or("GET");
            let path = parts.next().unwrap_or("/");
            let version = parts.next().unwrap_or("HTTP/1.1");

            let mut headers = BTreeMap::new();
            for line in lines {
                if let Some((k, v)) = line.split_once(':') {
                    headers.insert(k.trim().to_lowercase(), Value::string(v.trim()));
                }
            }

            let mut req = BTreeMap::new();
            req.insert("method".into(), Value::string(method));
            req.insert("path".into(), Value::string(path));
            req.insert("version".into(), Value::string(version));
            req.insert("headers".into(), Value::map(headers));
            req.insert("body".into(), Value::string(body));
            Ok(Value::map(req))
        }

        "format_http_response" => {
            let status_code = match &args[0] {
                Value::Int(code) => match *code {
                    200 => "200 OK".to_string(),
                    201 => "201 Created".to_string(),
                    204 => "204 No Content".to_string(),
                    400 => "400 Bad Request".to_string(),
                    401 => "401 Unauthorized".to_string(),
                    403 => "403 Forbidden".to_string(),
                    404 => "404 Not Found".to_string(),
                    500 => "500 Internal Server Error".to_string(),
                    other => format!("{other}"),
                },
                Value::String(s) => s.as_str().to_string(),
                _ => return Err(error("Status must be an integer code or string")),
            };

            let Value::Map(headers_map) = &args[1] else {
                return Err(error("Expected map for HTTP headers"));
            };
            let body = text(&args[2])?;

            let mut headers_str = String::new();
            let mut has_content_length = false;
            for (k, v) in headers_map.iter() {
                if k.eq_ignore_ascii_case("content-length") {
                    has_content_length = true;
                }
                headers_str.push_str(&format!("{k}: {v}\r\n"));
            }
            if !has_content_length {
                headers_str.push_str(&format!("Content-Length: {}\r\n", body.len()));
            }

            let response = format!("HTTP/1.1 {status_code}\r\n{headers_str}\r\n{body}");
            Ok(Value::string(response))
        }

        "socket_write_http_chunk" => {
            let id = extract_handle(&args[0], "TcpStream")?;
            let chunk_bytes = match &args[1] {
                Value::String(s) => s.as_bytes().to_vec(),
                Value::List(items) => items
                    .iter()
                    .map(|v| match v {
                        Value::Int(b) if (0..=255).contains(b) => Ok(*b as u8),
                        _ => Err(error("Byte values must be in 0..255")),
                    })
                    .collect::<RyndResult<Vec<u8>>>()?,
                _ => return Err(error("Expected string or byte list for chunk")),
            };
            let mut formatted = format!("{:X}\r\n", chunk_bytes.len()).into_bytes();
            formatted.extend_from_slice(&chunk_bytes);
            formatted.extend_from_slice(b"\r\n");

            let mut streams = STREAMS.lock().unwrap();
            let stream = streams
                .get_mut(&id)
                .ok_or_else(|| error("TcpStream handle is closed or invalid"))?;
            stream.write_all(&formatted).map_err(io)?;
            stream.flush().map_err(io)?;
            Ok(Value::Int(formatted.len() as i64))
        }

        "socket_finish_http_chunks" => {
            let id = extract_handle(&args[0], "TcpStream")?;
            let mut streams = STREAMS.lock().unwrap();
            let stream = streams
                .get_mut(&id)
                .ok_or_else(|| error("TcpStream handle is closed or invalid"))?;
            stream.write_all(b"0\r\n\r\n").map_err(io)?;
            stream.flush().map_err(io)?;
            Ok(Value::Nil)
        }

        "socket_read_chunk" => {
            let id = extract_handle(&args[0], "TcpStream")?;
            let max_len = match &args[1] {
                Value::Int(n) => usize::try_from(*n)
                    .map_err(|_| error("Read length must be non-negative"))?
                    .min(16 * 1024 * 1024),
                _ => return Err(error("Expected integer max length")),
            };
            let mut buf = vec![0u8; max_len];
            let n = {
                let mut streams = STREAMS.lock().unwrap();
                let stream = streams
                    .get_mut(&id)
                    .ok_or_else(|| error("TcpStream handle is closed or invalid"))?;
                match stream.read(&mut buf) {
                    Ok(bytes_read) => bytes_read,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        return Ok(Value::Nil);
                    }
                    Err(e) => return Err(io(e)),
                }
            };
            buf.truncate(n);
            Ok(Value::string(String::from_utf8_lossy(&buf).into_owned()))
        }

        "match_route" => {
            let path = text(&args[0])?;
            let pattern = text(&args[1])?;

            let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
            let pat_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();

            let mut params = BTreeMap::new();
            let mut matched = true;
            let mut p_idx = 0;

            for pat in &pat_segments {
                if let Some(param_name) = pat.strip_prefix('*') {
                    let rest = path_segments[p_idx..].join("/");
                    params.insert(param_name.to_string(), Value::string(rest));
                    p_idx = path_segments.len();
                    break;
                }
                if p_idx >= path_segments.len() {
                    matched = false;
                    break;
                }
                if let Some(param_name) = pat.strip_prefix(':') {
                    params.insert(param_name.to_string(), Value::string(path_segments[p_idx]));
                    p_idx += 1;
                } else if *pat == path_segments[p_idx] {
                    p_idx += 1;
                } else {
                    matched = false;
                    break;
                }
            }

            if matched && p_idx == path_segments.len() {
                Ok(Value::variant("Some", vec![Value::map(params)]))
            } else {
                Ok(Value::variant("None", vec![]))
            }
        }

        "conn" => {
            let method = text(&args[0])?.to_uppercase();
            let path = text(&args[1])?;

            let mut conn_map = BTreeMap::new();
            conn_map.insert("method".into(), Value::string(method));
            conn_map.insert("path".into(), Value::string(path));
            conn_map.insert("params".into(), Value::map(BTreeMap::new()));
            conn_map.insert("query".into(), Value::map(BTreeMap::new()));
            conn_map.insert("headers".into(), Value::map(BTreeMap::new()));
            conn_map.insert("body".into(), Value::string(""));
            conn_map.insert("status".into(), Value::Int(200));

            let mut resp_headers = BTreeMap::new();
            resp_headers.insert(
                "content-type".into(),
                Value::string("text/html; charset=utf-8"),
            );
            conn_map.insert("resp_headers".into(), Value::map(resp_headers));
            conn_map.insert("resp_body".into(), Value::string(""));
            conn_map.insert("halted".into(), Value::Bool(false));
            Ok(Value::map(conn_map))
        }

        "halt" => {
            let Value::Map(m) = &args[0] else {
                return Err(error("halt() expects conn map"));
            };
            let mut out = (**m).clone();
            out.insert("halted".into(), Value::Bool(true));
            Ok(Value::map(out))
        }

        "put_status" => {
            let Value::Map(m) = &args[0] else {
                return Err(error("put_status() expects conn map"));
            };
            let mut out = (**m).clone();
            out.insert("status".into(), args[1].clone());
            Ok(Value::map(out))
        }

        "put_resp_header" => {
            let Value::Map(m) = &args[0] else {
                return Err(error("put_resp_header() expects conn map"));
            };
            let name = text(&args[1])?.to_lowercase();
            let val = text(&args[2])?;
            let mut out = (**m).clone();
            let mut headers = match out.get("resp_headers") {
                Some(Value::Map(h)) => (**h).clone(),
                _ => BTreeMap::new(),
            };
            headers.insert(name, Value::string(val));
            out.insert("resp_headers".into(), Value::map(headers));
            Ok(Value::map(out))
        }

        "put_resp_body" => {
            let Value::Map(m) = &args[0] else {
                return Err(error("put_resp_body() expects conn map"));
            };
            let body = text(&args[1])?;
            let mut out = (**m).clone();
            out.insert("resp_body".into(), Value::string(body));
            Ok(Value::map(out))
        }

        "text_response" => {
            let Value::Map(m) = &args[0] else {
                return Err(error("text_response() expects conn map"));
            };
            let status = args[1].clone();
            let body = text(&args[2])?;
            let mut out = (**m).clone();
            out.insert("status".into(), status);
            let mut headers = match out.get("resp_headers") {
                Some(Value::Map(h)) => (**h).clone(),
                _ => BTreeMap::new(),
            };
            headers.insert(
                "content-type".into(),
                Value::string("text/plain; charset=utf-8"),
            );
            out.insert("resp_headers".into(), Value::map(headers));
            out.insert("resp_body".into(), Value::string(body));
            out.insert("halted".into(), Value::Bool(true));
            Ok(Value::map(out))
        }

        "json_response" => {
            let Value::Map(m) = &args[0] else {
                return Err(error("json_response() expects conn map"));
            };
            let status = args[1].clone();
            let body = super::json::stringify(&args[2])?;
            let mut out = (**m).clone();
            out.insert("status".into(), status);
            let mut headers = match out.get("resp_headers") {
                Some(Value::Map(h)) => (**h).clone(),
                _ => BTreeMap::new(),
            };
            headers.insert("content-type".into(), Value::string("application/json"));
            out.insert("resp_headers".into(), Value::map(headers));
            out.insert("resp_body".into(), Value::string(body));
            out.insert("halted".into(), Value::Bool(true));
            Ok(Value::map(out))
        }

        "html_response" => {
            let Value::Map(m) = &args[0] else {
                return Err(error("html_response() expects conn map"));
            };
            let status = args[1].clone();
            let body = text(&args[2])?;
            let mut out = (**m).clone();
            out.insert("status".into(), status);
            let mut headers = match out.get("resp_headers") {
                Some(Value::Map(h)) => (**h).clone(),
                _ => BTreeMap::new(),
            };
            headers.insert(
                "content-type".into(),
                Value::string("text/html; charset=utf-8"),
            );
            out.insert("resp_headers".into(), Value::map(headers));
            out.insert("resp_body".into(), Value::string(body));
            out.insert("halted".into(), Value::Bool(true));
            Ok(Value::map(out))
        }

        "plug_send" => {
            let Value::Map(conn_map) = &args[0] else {
                return Err(error("plug_send() expects conn map"));
            };
            let stream_handle = &args[1];

            let status = conn_map.get("status").cloned().unwrap_or(Value::Int(200));
            let headers = conn_map
                .get("resp_headers")
                .cloned()
                .unwrap_or_else(|| Value::map(BTreeMap::new()));
            let body = conn_map
                .get("resp_body")
                .cloned()
                .unwrap_or_else(|| Value::string(""));

            let resp = call(_rt, "format_http_response", &[status, headers, body])?;
            let _ = call(_rt, "socket_write", &[stream_handle.clone(), resp])?;
            let _ = call(_rt, "socket_close", std::slice::from_ref(stream_handle));
            Ok(args[0].clone())
        }

        _ => Err(error(format!("Unknown socket builtin '{name}'"))),
    }
}
