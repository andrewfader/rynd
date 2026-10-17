//! HTTP server, client, and URL helpers over real loopback sockets.
use rynd::{RyndEngine, Value};
use std::io::{Read, Write};
use std::net::TcpStream;

#[path = "common/socket_lock.rs"]
mod socket_lock;

fn eval(code: &str) -> Value {
    RyndEngine::new()
        .eval(code)
        .unwrap_or_else(|e| panic!("{e}\n{code}"))
}

#[test]
fn a_program_serves_and_fetches_from_itself() {
    let _guard = socket_lock::acquire().unwrap();
    let result = eval(
        r##"
        let listener = tcp_listen("127.0.0.1:0")
        let base = "http://#{tcp_local_addr(listener)}"
        fn echo(conn) {
            json_response(conn, conn.method == "POST" ? 201 : 200, {
                "path": conn.path, "query": conn.query, "body": conn.body,
                "agent": conn.headers["user-agent"]
            })
        }
        let server = spawn(\ -> serve({"listener": listener, "max_connections": 2}, echo))
        let got = fetch("#{base}/hello%20there?x=1&name=a+b%26c")
        let posted = http_request({"method": "POST", "url": "#{base}/items", "body": {"k": [1, 2]}})
        let served = await_fiber(server)
        [got.status, parse_json(got.body), posted.status, parse_json(posted.body).body,
         posted.headers["content-type"], served]
    "##,
    );
    let text = result.to_string();
    assert!(text.starts_with("[200, {"), "{text}");
    assert!(text.contains(r#""path": /hello%20there"#), "{text}");
    assert!(
        text.contains(r#""query": {"name": a b&c, "x": 1}"#),
        "{text}"
    );
    assert!(text.contains(r#""agent": rynd"#), "{text}");
    assert!(
        text.ends_with(r#"201, {"k":[1,2]}, application/json, 2]"#),
        "{text}"
    );
}

fn raw_exchange(addr: &str, request: &[u8]) -> String {
    let mut stream = TcpStream::connect(addr).unwrap();
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(20)))
        .unwrap();
    stream.write_all(request).unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

/// Run a server program on its own thread and hand back its address.
fn background_server(
    handler: &str,
    connections: usize,
) -> (String, std::thread::JoinHandle<String>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    drop(listener);
    let code =
        format!(r#"serve({{"addr": "{addr}", "max_connections": {connections}}}, {handler})"#);
    let thread = std::thread::spawn(move || eval(&code).to_string());
    for _ in 0..200 {
        if TcpStream::connect(&addr).is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    (addr, thread)
}

#[test]
fn server_handles_keep_alive_chunked_bodies_and_bad_requests() {
    let _guard = socket_lock::acquire().unwrap();
    let handler =
        r##"\conn -> text_response(conn, 200, "#{conn.method} #{conn.path} [#{conn.body}]")"##;
    // The readiness probe in background_server uses one connection.
    let (addr, server) = background_server(handler, 5);

    // Two pipelined requests on one keep-alive connection, the second chunked.
    let both = raw_exchange(
        &addr,
        b"POST /a HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhelloPUT /b HTTP/1.1\r\nHost: x\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n3\r\nabc\r\n2;ext=1\r\nde\r\n0\r\n\r\n",
    );
    assert!(both.contains("POST /a [hello]"), "{both}");
    assert!(both.contains("PUT /b [abcde]"), "{both}");
    assert_eq!(both.matches("HTTP/1.1 200 OK").count(), 2, "{both}");

    let head = raw_exchange(
        &addr,
        b"HEAD /h HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
    );
    assert!(head.starts_with("HTTP/1.1 200 OK"), "{head}");
    assert!(head.contains("Content-Length: 10"), "{head}");
    assert!(head.ends_with("\r\n\r\n"), "HEAD has no body: {head}");

    let bad = raw_exchange(&addr, b"POST /x HTTP/1.1\r\nContent-Length: nope\r\n\r\n");
    assert!(bad.starts_with("HTTP/1.1 400 Bad Request"), "{bad}");

    let http10 = raw_exchange(&addr, b"GET /old HTTP/1.0\r\n\r\n");
    assert!(http10.contains("connection: close"), "{http10}");
    assert!(http10.contains("GET /old []"), "{http10}");
    assert_eq!(server.join().unwrap(), "4");
}

#[test]
fn failing_handlers_return_500_and_the_server_keeps_going() {
    let _guard = socket_lock::acquire().unwrap();
    let handler = r#"\conn -> conn.path == "/boom" ? 1 / 0 : text_response(conn, 200, "fine")"#;
    let (addr, server) = background_server(handler, 3);
    let boom = raw_exchange(&addr, b"GET /boom HTTP/1.1\r\nConnection: close\r\n\r\n");
    assert!(
        boom.starts_with("HTTP/1.1 500 Internal Server Error"),
        "{boom}"
    );
    let fine = raw_exchange(&addr, b"GET / HTTP/1.1\r\nConnection: close\r\n\r\n");
    assert!(fine.ends_with("fine"), "{fine}");
    server.join().unwrap();
}

#[test]
fn handlers_can_stream_chunked_responses_on_the_socket() {
    let _guard = socket_lock::acquire().unwrap();
    let handler = r#"\conn -> {
        socket_write(conn.socket, "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
        ["one ", "two"] |> each(\part -> socket_write_http_chunk(conn.socket, part))
        socket_finish_http_chunks(conn.socket)
        put(conn, "sent", true)
    }"#;
    let (addr, server) = background_server(handler, 2);
    let streamed = raw_exchange(&addr, b"GET / HTTP/1.1\r\n\r\n");
    assert!(
        streamed.ends_with("4\r\none \r\n3\r\ntwo\r\n0\r\n\r\n"),
        "{streamed:?}"
    );
    server.join().unwrap();

    // And a client can decode them chunk by chunk.
    let result = eval(
        r#"
        let server = tcp_listen("127.0.0.1:0")
        let client = tcp_connect(tcp_local_addr(server))
        let peer = tcp_accept(server)[0]
        socket_write_http_chunk(client, "alpha")
        socket_write_http_chunk(client, "beta")
        socket_finish_http_chunks(client)
        [socket_read_chunk(peer, 100), socket_read_chunk(peer, 100), socket_read_chunk(peer, 100)]
    "#,
    );
    assert_eq!(result.to_string(), "[alpha, beta, nil]");
}

#[test]
fn urls_queries_and_routes_decode_consistently() {
    let result = eval(
        r#"
        [
            url_encode("a b/c?d=é"),
            url_decode("a%20b%2Fc%3Fd%3D%C3%A9"),
            url_decode("100%"),
            parse_query("?tag=rust+lang&x=%26&empty"),
            query_string({"q": "a&b", "n": 2}),
            match_route("/articles/hello%20world/comments", "/articles/:slug/comments").value.slug,
            match_route("/files/a/b%2Fc", "/files/*rest").value.rest,
            conn("get", "/search?q=x").query.q,
            conn("get", "/search?q=x").path
        ]
    "#,
    );
    assert_eq!(
        result.to_string(),
        r#"[a%20b%2Fc%3Fd%3D%C3%A9, a b/c?d=é, 100%, {"empty": , "tag": rust lang, "x": &}, n=2&q=a%26b, hello world, a/b/c, x, /search]"#
    );
}

#[test]
fn clients_report_unreachable_and_unsupported_urls() {
    let _guard = socket_lock::acquire().unwrap();
    let mut engine = RyndEngine::new();
    for (code, message) in [
        (r#"fetch("https://example.com")"#, "TLS"),
        (r#"fetch("ftp://example.com")"#, "http://"),
        (r#"http_request({"method": "GET"})"#, "url"),
    ] {
        let error = engine.eval(code).unwrap_err().to_string();
        assert!(error.contains(message), "{code}: {error}");
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let closed = listener.local_addr().unwrap();
    drop(listener);
    assert!(
        engine
            .eval(&format!(r#"fetch("http://{closed}/")"#))
            .is_err()
    );
}
