//! HTTP/1.1 for Rynd: a fiber-per-connection server, a client, Plug-style
//! connection helpers, routing, and URL encoding.
use super::{
    concurrency::{self, blocking},
    runtime::{Runtime, error},
    sockets::{self, io},
    value::Value,
};
use crate::error::RyndResult;
use std::collections::BTreeMap;
use std::convert::TryFrom;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

pub const BUILTINS: &[(&str, usize)] = &[
    ("parse_http_request", 1),
    ("format_http_response", 3),
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
    ("serve", 2),
    ("read_http_request", 1),
    ("http_request", 1),
    ("fetch", 1),
    ("url_encode", 1),
    ("url_decode", 1),
    ("parse_query", 1),
    ("query_string", 1),
];

/// Request heads beyond this are rejected with 431.
const MAX_HEAD: usize = 64 * 1024;
/// Request and response bodies beyond this are rejected (413 on the server).
const MAX_BODY: usize = 16 * 1024 * 1024;
/// Keep-alive connections close after this long without a new request.
const IDLE_TIMEOUT: Duration = Duration::from_secs(5);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(30);

fn text(value: &Value) -> RyndResult<&str> {
    <&str>::try_from(value)
}

fn conn_map(value: &Value, builtin: &str) -> RyndResult<BTreeMap<String, Value>> {
    match value {
        Value::Map(m) => Ok((**m).clone()),
        _ => Err(error(format!("{builtin}() expects conn map"))),
    }
}

pub fn reason(status: i64) -> &'static str {
    match status {
        100 => "Continue",
        101 => "Switching Protocols",
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        206 => "Partial Content",
        301 => "Moved Permanently",
        302 => "Found",
        303 => "See Other",
        304 => "Not Modified",
        307 => "Temporary Redirect",
        308 => "Permanent Redirect",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        409 => "Conflict",
        410 => "Gone",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Content",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

// ---------------------------------------------------------------- URLs

fn is_unreserved(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~')
}

pub fn url_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for &b in input.as_bytes() {
        if is_unreserved(b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Decode `%XX` escapes (and `+` as space when `form` is set). Malformed
/// escapes pass through unchanged; invalid UTF-8 is replaced.
pub fn url_decode(input: &str, form: bool) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
                match hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                    Some(b) => {
                        out.push(b);
                        i += 3;
                        continue;
                    }
                    None => out.push(b'%'),
                }
            }
            b'+' if form => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn parse_query(query: &str) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(
            url_decode(key, true),
            Value::string(url_decode(value, true)),
        );
    }
    out
}

fn split_target(target: &str) -> (&str, &str) {
    target.split_once('?').unwrap_or((target, ""))
}

// ---------------------------------------------------------------- conns

fn base_conn(method: &str, target: &str) -> BTreeMap<String, Value> {
    let (path, query) = split_target(target);
    let mut conn = BTreeMap::new();
    conn.insert("method".into(), Value::string(method.to_uppercase()));
    conn.insert(
        "path".into(),
        Value::string(if path.is_empty() { "/" } else { path }),
    );
    conn.insert("query_string".into(), Value::string(query));
    conn.insert("query".into(), Value::map(parse_query(query)));
    conn.insert("params".into(), Value::map(BTreeMap::new()));
    conn.insert("headers".into(), Value::map(BTreeMap::new()));
    conn.insert("body".into(), Value::string(""));
    conn.insert("version".into(), Value::string("HTTP/1.1"));
    conn.insert("status".into(), Value::Int(200));
    let mut resp_headers = BTreeMap::new();
    resp_headers.insert(
        "content-type".into(),
        Value::string("text/html; charset=utf-8"),
    );
    conn.insert("resp_headers".into(), Value::map(resp_headers));
    conn.insert("resp_body".into(), Value::string(""));
    conn.insert("halted".into(), Value::Bool(false));
    conn
}

/// A parsed request head plus body, before it becomes a script value.
struct Request {
    method: String,
    target: String,
    version: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }
    fn into_conn(self) -> Value {
        let mut conn = base_conn(&self.method, &self.target);
        conn.insert("version".into(), Value::string(self.version));
        let headers = self
            .headers
            .into_iter()
            .map(|(k, v)| (k, Value::string(v)))
            .collect();
        conn.insert("headers".into(), Value::map(headers));
        conn.insert(
            "body".into(),
            Value::string(String::from_utf8_lossy(&self.body).into_owned()),
        );
        Value::map(conn)
    }
}

/// Method, target, version, and lowercase-named headers of a request head.
type Head = (String, String, String, Vec<(String, String)>);

fn parse_head(head: &str) -> RyndResult<Head> {
    let mut lines = head.split("\r\n").flat_map(|l| l.split('\n'));
    let first = lines.next().unwrap_or("").trim();
    let mut parts = first.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Err(error(format!("Malformed HTTP request line {first:?}")));
    };
    let version = parts.next().unwrap_or("HTTP/1.1");
    let mut headers = Vec::new();
    for line in lines {
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_lowercase(), v.trim().to_string()));
        }
    }
    Ok((method.into(), target.into(), version.into(), headers))
}

/// Why a message could not be read: the transport failed (close quietly) or
/// the peer sent something malformed (answer with `status`).
enum ReadError {
    Io(std::io::Error),
    Closed(&'static str),
    Protocol(i64, String),
}
type ReadResult<T> = Result<T, ReadError>;

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> Self {
        ReadError::Io(e)
    }
}
impl From<ReadError> for crate::error::RyndError {
    fn from(e: ReadError) -> Self {
        match e {
            ReadError::Io(e) => io(e),
            ReadError::Closed(what) => error(format!("Connection closed inside the {what}")),
            ReadError::Protocol(_, message) => error(message),
        }
    }
}

/// Buffered reads from a socket, keeping bytes past one message for the next
/// (keep-alive and pipelining).
struct Reader {
    stream: Arc<TcpStream>,
    buf: Vec<u8>,
}

impl Reader {
    fn fill(&mut self) -> std::io::Result<usize> {
        let mut chunk = [0u8; 8192];
        let n = (&*self.stream).read(&mut chunk)?;
        self.buf.extend_from_slice(&chunk[..n]);
        Ok(n)
    }
    /// The head up to the blank line, or `None` on a clean close before any byte.
    fn head(&mut self) -> ReadResult<Option<String>> {
        loop {
            if let Some(end) = find(&self.buf, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&self.buf[..end]).into_owned();
                self.buf.drain(..end + 4);
                return Ok(Some(head));
            }
            if self.buf.len() > MAX_HEAD {
                return Err(ReadError::Protocol(431, "HTTP head too large".into()));
            }
            if self.fill()? == 0 {
                return if self.buf.iter().all(|b| b.is_ascii_whitespace()) {
                    Ok(None)
                } else {
                    Err(ReadError::Closed("HTTP head"))
                };
            }
        }
    }
    fn exact(&mut self, n: usize) -> ReadResult<Vec<u8>> {
        while self.buf.len() < n {
            if self.fill()? == 0 {
                return Err(ReadError::Closed("HTTP body"));
            }
        }
        Ok(self.buf.drain(..n).collect())
    }
    fn line(&mut self) -> ReadResult<String> {
        loop {
            if let Some(end) = find(&self.buf, b"\r\n") {
                let line = String::from_utf8_lossy(&self.buf[..end]).into_owned();
                self.buf.drain(..end + 2);
                return Ok(line);
            }
            if self.buf.len() > MAX_HEAD {
                return Err(ReadError::Protocol(
                    400,
                    "HTTP chunk header too long".into(),
                ));
            }
            if self.fill()? == 0 {
                return Err(ReadError::Closed("chunk framing"));
            }
        }
    }
    fn chunked(&mut self) -> ReadResult<Vec<u8>> {
        let mut body = Vec::new();
        loop {
            let line = self.line()?;
            let size_text = line.split(';').next().unwrap_or("").trim();
            let size = usize::from_str_radix(size_text, 16).map_err(|_| {
                ReadError::Protocol(400, format!("Invalid chunk size {size_text:?}"))
            })?;
            if size == 0 {
                while !self.line()?.is_empty() {}
                return Ok(body);
            }
            if body.len() + size > MAX_BODY {
                return Err(ReadError::Protocol(413, "HTTP body too large".into()));
            }
            body.extend(self.exact(size)?);
            self.exact(2)?;
        }
    }
    fn read_to_close(&mut self) -> ReadResult<Vec<u8>> {
        while self.fill()? > 0 {
            if self.buf.len() > MAX_BODY {
                return Err(ReadError::Protocol(413, "HTTP body too large".into()));
            }
        }
        Ok(std::mem::take(&mut self.buf))
    }
    /// Body framed by `content-length` or chunked transfer encoding.
    fn body(&mut self, headers: &[(String, String)], until_close: bool) -> ReadResult<Vec<u8>> {
        let get = |name: &str| {
            headers
                .iter()
                .rev()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.as_str())
        };
        if get("transfer-encoding").is_some_and(|te| te.to_ascii_lowercase().contains("chunked")) {
            return self.chunked();
        }
        match get("content-length") {
            Some(len) => {
                let len: usize = len.trim().parse().map_err(|_| {
                    ReadError::Protocol(400, format!("Invalid Content-Length {len:?}"))
                })?;
                if len > MAX_BODY {
                    return Err(ReadError::Protocol(413, "HTTP body too large".into()));
                }
                self.exact(len)
            }
            None if until_close => self.read_to_close(),
            None => Ok(Vec::new()),
        }
    }
    fn request(&mut self) -> ReadResult<Option<Request>> {
        let Some(head) = self.head()? else {
            return Ok(None);
        };
        let (method, target, version, headers) =
            parse_head(&head).map_err(|e| ReadError::Protocol(400, e.to_string()))?;
        let body = self.body(&headers, false)?;
        Ok(Some(Request {
            method,
            target,
            version,
            headers,
            body,
        }))
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn header_map(value: Option<&Value>) -> BTreeMap<String, Value> {
    match value {
        Some(Value::Map(m)) => (**m).clone(),
        _ => BTreeMap::new(),
    }
}

/// Serialize a response; `Content-Length` is added unless already present.
fn format_response(
    status: &Value,
    headers: &BTreeMap<String, Value>,
    body: &[u8],
    extra: &[(&str, &str)],
    head_only: bool,
) -> RyndResult<Vec<u8>> {
    let status_line = match status {
        Value::Int(code) => match reason(*code) {
            "" => format!("{code}"),
            phrase => format!("{code} {phrase}"),
        },
        Value::String(s) => s.as_str().to_string(),
        _ => return Err(error("Status must be an integer code or string")),
    };
    let mut out = format!("HTTP/1.1 {status_line}\r\n");
    let mut has_length = false;
    for (k, v) in headers {
        if k.eq_ignore_ascii_case("content-length") {
            has_length = true;
        }
        let value = match v {
            Value::String(s) => s.as_str().to_string(),
            other => other.to_string(),
        };
        if k.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err(error(
                "HTTP header names and values cannot contain newlines",
            ));
        }
        out.push_str(&format!("{k}: {value}\r\n"));
    }
    for (k, v) in extra {
        if !headers.keys().any(|h| h.eq_ignore_ascii_case(k)) {
            out.push_str(&format!("{k}: {v}\r\n"));
        }
    }
    if !has_length {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("\r\n");
    let mut bytes = out.into_bytes();
    if !head_only {
        bytes.extend_from_slice(body);
    }
    Ok(bytes)
}

fn with_response(
    conn: &Value,
    builtin: &str,
    status: &Value,
    content_type: &str,
    body: Value,
) -> RyndResult<Value> {
    let mut out = conn_map(conn, builtin)?;
    out.insert("status".into(), status.clone());
    let mut headers = header_map(out.get("resp_headers"));
    headers.insert("content-type".into(), Value::string(content_type));
    out.insert("resp_headers".into(), Value::map(headers));
    out.insert("resp_body".into(), body);
    out.insert("halted".into(), Value::Bool(true));
    Ok(Value::map(out))
}

fn response_bytes(
    conn: &BTreeMap<String, Value>,
    head_only: bool,
    close: bool,
) -> RyndResult<Vec<u8>> {
    let status = conn.get("status").cloned().unwrap_or(Value::Int(200));
    let headers = header_map(conn.get("resp_headers"));
    let body = match conn.get("resp_body") {
        Some(Value::String(s)) => s.as_bytes().to_vec(),
        Some(Value::Nil) | None => Vec::new(),
        Some(other) => other.to_string().into_bytes(),
    };
    let connection = if close { "close" } else { "keep-alive" };
    format_response(
        &status,
        &headers,
        &body,
        &[("connection", connection)],
        head_only,
    )
}

// ---------------------------------------------------------------- server

struct ServeOptions {
    max_connections: Option<usize>,
}

/// Serve one connection: read requests, run the handler, write responses,
/// until the client closes, asks to close, or idles past the timeout.
fn handle_connection(
    rt: &mut dyn Runtime,
    socket: &Value,
    peer: &str,
    handler: &Value,
) -> RyndResult<Value> {
    let stream = sockets::stream(socket)?;
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let mut reader = Reader {
        stream: stream.clone(),
        buf: Vec::new(),
    };
    let mut served = 0;
    loop {
        let request = match blocking(|| reader.request()) {
            Ok(Some(request)) => request,
            // Clean close, idle timeout, reset, or truncated message.
            Ok(None) | Err(ReadError::Io(_) | ReadError::Closed(_)) => break,
            Err(ReadError::Protocol(status, _)) => {
                let bytes = format_response(
                    &Value::Int(status),
                    &BTreeMap::new(),
                    reason(status).as_bytes(),
                    &[("connection", "close"), ("content-type", "text/plain")],
                    false,
                )?;
                let _ = blocking(|| (&*stream).write_all(&bytes));
                break;
            }
        };
        served += 1;
        let head_only = request.method.eq_ignore_ascii_case("HEAD");
        let wants_close = match request.header("connection") {
            Some(c) => c.eq_ignore_ascii_case("close"),
            None => request.version == "HTTP/1.0",
        };
        let label = format!("{} {}", request.method, request.target);
        let mut conn = match request.into_conn() {
            Value::Map(m) => Rc::unwrap_or_clone(m),
            _ => unreachable!(),
        };
        conn.insert("remote_addr".into(), Value::string(peer));
        conn.insert("socket".into(), socket.clone());
        let outcome = rt.call_ref(handler, &[Value::map(conn)]);
        let response = match outcome {
            Ok(Value::Map(conn)) => {
                if conn.get("sent").is_some_and(Value::is_truthy) {
                    // The handler wrote (and may have streamed) its own response.
                    break;
                }
                let close = wants_close
                    || header_map(conn.get("resp_headers"))
                        .get("connection")
                        .is_some_and(|c| c.to_string().eq_ignore_ascii_case("close"));
                (response_bytes(&conn, head_only, close)?, close)
            }
            other => {
                match &other {
                    Ok(v) => eprintln!(
                        "rynd serve: {label}: handler returned {} instead of a conn map",
                        v.type_name()
                    ),
                    Err(e) => eprintln!("rynd serve: {label}: {e}"),
                }
                let mut conn = base_conn("GET", "/");
                conn.insert("status".into(), Value::Int(500));
                let mut headers = BTreeMap::new();
                headers.insert("content-type".into(), Value::string("text/plain"));
                conn.insert("resp_headers".into(), Value::map(headers));
                conn.insert("resp_body".into(), Value::string("Internal Server Error"));
                (response_bytes(&conn, head_only, true)?, true)
            }
        };
        let (bytes, close) = response;
        if blocking(|| {
            let mut writer = &*stream;
            writer.write_all(&bytes).and_then(|_| writer.flush())
        })
        .is_err()
            || close
        {
            break;
        }
    }
    sockets::close(socket)?;
    Ok(Value::Int(served))
}

fn serve(rt: &mut dyn Runtime, target: &Value, handler: &Value) -> RyndResult<Value> {
    let mut options = ServeOptions {
        max_connections: None,
    };
    let listener = match target {
        Value::String(addr) => {
            sockets::register_listener(std::net::TcpListener::bind(addr.as_str()).map_err(io)?)
        }
        Value::Map(opts) => {
            if let Some(max) = opts.get("max_connections") {
                options.max_connections = Some(
                    usize::try_from(i64::try_from(max)?)
                        .map_err(|_| error("max_connections must be non-negative"))?,
                );
            }
            match (opts.get("listener"), opts.get("addr")) {
                (Some(listener), _) => listener.clone(),
                (None, Some(Value::String(addr))) => sockets::register_listener(
                    std::net::TcpListener::bind(addr.as_str()).map_err(io)?,
                ),
                _ => return Err(error("serve() options need \"listener\" or \"addr\"")),
            }
        }
        other => other.clone(),
    };
    let socket_listener = sockets::listener(&listener)?;
    let bounded = options.max_connections.is_some();
    let mut fibers = Vec::new();
    let mut accepted = 0usize;
    while options.max_connections.is_none_or(|max| accepted < max) {
        let (stream, peer) = match blocking(|| socket_listener.accept()) {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                concurrency::sleep(Duration::from_millis(5))?;
                continue;
            }
            Err(e) => return Err(io(e)),
        };
        let _ = stream.set_nodelay(true);
        accepted += 1;
        let socket = sockets::register_stream(stream);
        let handler = handler.clone();
        let peer = peer.to_string();
        let task = Value::Compiled {
            name: Rc::from("http_connection"),
            arity: 0,
            func: Rc::new(move |rt, _, _| handle_connection(rt, &socket, &peer, &handler)),
        };
        // Unbounded servers detach connection fibers; bounded ones join them.
        fibers.push(concurrency::spawn_fiber(rt, task, !bounded)?);
        if !bounded {
            fibers.clear();
        }
    }
    let mut served = 0;
    for id in fibers {
        let fiber = Value::variant("Fiber", vec![Value::Int(id)]);
        if let Ok(Value::Int(n)) = concurrency::call(rt, "await_fiber", &[fiber]) {
            served += n;
        }
    }
    Ok(Value::Int(served))
}

// ---------------------------------------------------------------- client

struct Url {
    host: String,
    port: u16,
    target: String,
}

fn parse_url(url: &str) -> RyndResult<Url> {
    let rest = if let Some(rest) = url.strip_prefix("http://") {
        rest
    } else if url.starts_with("https://") {
        return Err(error(
            "https:// needs TLS, which the standard library does not provide; register a TLS client from Rust",
        ));
    } else {
        return Err(error(format!("Expected an http:// URL, found {url:?}")));
    };
    let (authority, target) = match rest.find(['/', '?']) {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let target = if target.starts_with('?') {
        format!("/{target}")
    } else {
        target.to_string()
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !h.is_empty() && !p.contains(']') => (
            h.to_string(),
            p.parse::<u16>()
                .map_err(|_| error(format!("Invalid port in {url:?}")))?,
        ),
        _ => (authority.to_string(), 80),
    };
    if host.is_empty() {
        return Err(error(format!("Missing host in {url:?}")));
    }
    Ok(Url { host, port, target })
}

fn http_request(opts: &BTreeMap<String, Value>) -> RyndResult<Value> {
    let url = text(
        opts.get("url")
            .ok_or_else(|| error("http_request() needs a \"url\""))?,
    )?;
    let url = parse_url(url)?;
    let method = match opts.get("method") {
        Some(m) => text(m)?.to_uppercase(),
        None => "GET".into(),
    };
    let mut headers = header_map(opts.get("headers"));
    let body = match opts.get("body") {
        None | Some(Value::Nil) => Vec::new(),
        Some(Value::String(s)) => s.as_bytes().to_vec(),
        Some(other) => {
            if !headers
                .keys()
                .any(|k| k.eq_ignore_ascii_case("content-type"))
            {
                headers.insert("content-type".into(), Value::string("application/json"));
            }
            super::json::stringify(other)?.into_bytes()
        }
    };
    let timeout = match opts.get("timeout_ms") {
        Some(ms) => Duration::from_millis(
            u64::try_from(i64::try_from(ms)?).map_err(|_| error("timeout_ms must be positive"))?,
        ),
        None => CLIENT_TIMEOUT,
    };
    let host_header = if url.port == 80 {
        url.host.clone()
    } else {
        format!("{}:{}", url.host, url.port)
    };
    let mut head = format!("{method} {} HTTP/1.1\r\n", url.target);
    let has = |name: &str| headers.keys().any(|k| k.eq_ignore_ascii_case(name));
    let defaults = [
        ("host", host_header.as_str()),
        ("user-agent", "rynd"),
        ("accept", "*/*"),
        ("connection", "close"),
    ];
    let mut extra = Vec::new();
    for (k, v) in defaults {
        if !has(k) {
            extra.push(format!("{k}: {v}\r\n"));
        }
    }
    let sends_body = !body.is_empty() || matches!(method.as_str(), "POST" | "PUT" | "PATCH");
    if sends_body && !has("content-length") {
        extra.push(format!("content-length: {}\r\n", body.len()));
    }
    for (k, v) in &headers {
        let value = match v {
            Value::String(s) => s.as_str().to_string(),
            other => other.to_string(),
        };
        if k.contains(['\r', '\n']) || value.contains(['\r', '\n']) {
            return Err(error(
                "HTTP header names and values cannot contain newlines",
            ));
        }
        head.push_str(&format!("{k}: {value}\r\n"));
    }
    for line in extra {
        head.push_str(&line);
    }
    head.push_str("\r\n");
    let mut request = head.into_bytes();
    request.extend_from_slice(&body);

    let addr = format!("{}:{}", url.host, url.port);
    let stream = sockets::connect(&addr, timeout)?;
    let (status, version_reason, headers, body) = blocking(|| -> RyndResult<_> {
        stream.set_read_timeout(Some(timeout)).map_err(io)?;
        stream.set_write_timeout(Some(timeout)).map_err(io)?;
        (&stream).write_all(&request).map_err(io)?;
        let mut reader = Reader {
            stream: Arc::new(stream),
            buf: Vec::new(),
        };
        loop {
            let head = reader
                .head()?
                .ok_or_else(|| error("Server closed the connection without a response"))?;
            let mut lines = head.split("\r\n");
            let status_line = lines.next().unwrap_or("");
            let mut parts = status_line.splitn(3, ' ');
            let _version = parts.next();
            let status: i64 = parts
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| error(format!("Malformed HTTP status line {status_line:?}")))?;
            let phrase = parts.next().unwrap_or("").to_string();
            let headers: Vec<(String, String)> = lines
                .filter_map(|l| l.split_once(':'))
                .map(|(k, v)| (k.trim().to_lowercase(), v.trim().to_string()))
                .collect();
            if (100..200).contains(&status) {
                continue;
            }
            let no_body = method == "HEAD" || status == 204 || status == 304;
            let body = if no_body {
                Vec::new()
            } else {
                reader.body(&headers, true)?
            };
            return Ok((status, phrase, headers, body));
        }
    })?;
    let mut response = BTreeMap::new();
    response.insert("status".into(), Value::Int(status));
    response.insert("reason".into(), Value::string(version_reason));
    response.insert(
        "headers".into(),
        Value::map(
            headers
                .into_iter()
                .map(|(k, v)| (k, Value::string(v)))
                .collect(),
        ),
    );
    response.insert(
        "body".into(),
        Value::string(String::from_utf8_lossy(&body).into_owned()),
    );
    Ok(Value::map(response))
}

// ---------------------------------------------------------------- routing

fn match_route(path: &str, pattern: &str) -> Value {
    let (path, _) = split_target(path);
    let path_segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let pat_segments: Vec<&str> = pattern.split('/').filter(|s| !s.is_empty()).collect();
    let mut params = BTreeMap::new();
    let mut p_idx = 0;
    for pat in &pat_segments {
        if let Some(name) = pat.strip_prefix('*') {
            let rest = path_segments[p_idx..]
                .iter()
                .map(|s| url_decode(s, false))
                .collect::<Vec<_>>()
                .join("/");
            params.insert(name.to_string(), Value::string(rest));
            p_idx = path_segments.len();
            break;
        }
        if p_idx >= path_segments.len() {
            return Value::variant("None", vec![]);
        }
        if let Some(name) = pat.strip_prefix(':') {
            params.insert(
                name.to_string(),
                Value::string(url_decode(path_segments[p_idx], false)),
            );
        } else if *pat != path_segments[p_idx] {
            return Value::variant("None", vec![]);
        }
        p_idx += 1;
    }
    if p_idx == path_segments.len() {
        Value::variant("Some", vec![Value::map(params)])
    } else {
        Value::variant("None", vec![])
    }
}

pub fn call(rt: &mut dyn Runtime, name: &str, args: &[Value]) -> RyndResult<Value> {
    match name {
        "parse_http_request" => {
            let raw = text(&args[0])?;
            let (head, body) = match raw.split_once("\r\n\r\n") {
                Some(parts) => parts,
                None => raw.split_once("\n\n").unwrap_or((raw, "")),
            };
            let (method, target, version, headers) = parse_head(head)?;
            Ok(Request {
                method,
                target,
                version,
                headers,
                body: body.as_bytes().to_vec(),
            }
            .into_conn())
        }

        "format_http_response" => {
            let Value::Map(headers) = &args[1] else {
                return Err(error("Expected map for HTTP headers"));
            };
            let body = text(&args[2])?;
            let bytes = format_response(&args[0], headers, body.as_bytes(), &[], false)?;
            Ok(Value::string(String::from_utf8_lossy(&bytes).into_owned()))
        }

        "match_route" => Ok(match_route(text(&args[0])?, text(&args[1])?)),

        "conn" => Ok(Value::map(base_conn(text(&args[0])?, text(&args[1])?))),

        "halt" => {
            let mut out = conn_map(&args[0], "halt")?;
            out.insert("halted".into(), Value::Bool(true));
            Ok(Value::map(out))
        }

        "put_status" => {
            let mut out = conn_map(&args[0], "put_status")?;
            out.insert("status".into(), args[1].clone());
            Ok(Value::map(out))
        }

        "put_resp_header" => {
            let mut out = conn_map(&args[0], "put_resp_header")?;
            let mut headers = header_map(out.get("resp_headers"));
            headers.insert(
                text(&args[1])?.to_lowercase(),
                Value::string(text(&args[2])?),
            );
            out.insert("resp_headers".into(), Value::map(headers));
            Ok(Value::map(out))
        }

        "put_resp_body" => {
            let mut out = conn_map(&args[0], "put_resp_body")?;
            out.insert("resp_body".into(), Value::string(text(&args[1])?));
            Ok(Value::map(out))
        }

        "text_response" => with_response(
            &args[0],
            name,
            &args[1],
            "text/plain; charset=utf-8",
            Value::string(text(&args[2])?),
        ),

        "json_response" => with_response(
            &args[0],
            name,
            &args[1],
            "application/json",
            Value::string(super::json::stringify(&args[2])?),
        ),

        "html_response" => with_response(
            &args[0],
            name,
            &args[1],
            "text/html; charset=utf-8",
            Value::string(text(&args[2])?),
        ),

        "plug_send" => {
            let mut conn = conn_map(&args[0], "plug_send")?;
            let stream = sockets::stream(&args[1])?;
            let bytes = response_bytes(&conn, false, true)?;
            blocking(|| {
                let mut writer = &*stream;
                writer.write_all(&bytes).and_then(|_| writer.flush())
            })
            .map_err(io)?;
            sockets::close(&args[1])?;
            conn.insert("sent".into(), Value::Bool(true));
            Ok(Value::map(conn))
        }

        "serve" => serve(rt, &args[0], &args[1]),

        "read_http_request" => {
            let stream = sockets::stream(&args[0])?;
            let mut reader = Reader {
                stream,
                buf: Vec::new(),
            };
            match blocking(|| reader.request()).map_err(crate::error::RyndError::from)? {
                Some(request) => {
                    if !reader.buf.is_empty() {
                        return Err(error(
                            "read_http_request(): pipelined requests need serve()",
                        ));
                    }
                    let mut conn = match request.into_conn() {
                        Value::Map(m) => Rc::unwrap_or_clone(m),
                        _ => unreachable!(),
                    };
                    conn.insert("socket".into(), args[0].clone());
                    Ok(Value::map(conn))
                }
                None => Ok(Value::Nil),
            }
        }

        "http_request" => match &args[0] {
            Value::Map(opts) => http_request(opts),
            _ => Err(error("http_request() expects an options map")),
        },

        "fetch" => {
            let mut opts = BTreeMap::new();
            opts.insert("url".into(), args[0].clone());
            http_request(&opts)
        }

        "url_encode" => Ok(Value::string(url_encode(text(&args[0])?))),
        "url_decode" => Ok(Value::string(url_decode(text(&args[0])?, false))),
        "parse_query" => Ok(Value::map(parse_query(
            text(&args[0])?.trim_start_matches('?'),
        ))),
        "query_string" => {
            let Value::Map(pairs) = &args[0] else {
                return Err(error("query_string() expects a map"));
            };
            Ok(Value::string(
                pairs
                    .iter()
                    .map(|(k, v)| {
                        let v = match v {
                            Value::String(s) => s.as_str().to_string(),
                            other => other.to_string(),
                        };
                        format!("{}={}", url_encode(k), url_encode(&v))
                    })
                    .collect::<Vec<_>>()
                    .join("&"),
            ))
        }

        _ => Err(error(format!("Unknown HTTP builtin '{name}'"))),
    }
}
