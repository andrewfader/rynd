use rynd::{RyndEngine, Value};

#[test]
fn test_parse_http_request_get() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let raw = "GET /users?sort=asc HTTP/1.1\r\nHost: example.com\r\nAccept: text/html\r\n\r\n"
        let req = parse_http_request(raw)
        [req.method, req.path, req.version, req.headers.host, req.body]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("GET"),
            Value::string("/users?sort=asc"),
            Value::string("HTTP/1.1"),
            Value::string("example.com"),
            Value::string(""),
        ])
    );
}

#[test]
fn test_parse_http_request_post_with_body() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let raw = "POST /api/deploy HTTP/1.1\r\nHost: api.cluster.local\r\nContent-Type: application/json\r\n\r\n{\"nodes\": 3}"
        let req = parse_http_request(raw)
        let body_json = parse_json(req.body)
        [req.method, req.path, req.headers["content-type"], body_json.nodes]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("POST"),
            Value::string("/api/deploy"),
            Value::string("application/json"),
            Value::Int(3),
        ])
    );
}

#[test]
fn test_format_http_response() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let body = '{"status": "ok"}'
        let headers = {"Content-Type": "application/json"}
        format_http_response(200, headers, body)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    if let Value::String(s) = res {
        assert!(s.contains("HTTP/1.1 200 OK"));
        assert!(s.contains("Content-Type: application/json"));
        assert!(s.contains("Content-Length: 16"));
        assert!(s.ends_with("\r\n\r\n{\"status\": \"ok\"}"));
    } else {
        panic!("Expected string response, got {res:?}");
    }
}

#[test]
fn test_format_http_response_404() {
    let mut engine = RyndEngine::new();
    let code = r#"
        format_http_response(404, {}, "Not Found")
    "#;
    let res = engine.eval(code).expect("Eval failed");
    if let Value::String(s) = res {
        assert!(s.contains("HTTP/1.1 404 Not Found"));
        assert!(s.contains("Content-Length: 9"));
        assert!(s.ends_with("\r\n\r\nNot Found"));
    } else {
        panic!("Expected string response, got {res:?}");
    }
}

#[test]
fn test_tcp_socket_loopback_roundtrip() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let server = tcp_listen("127.0.0.1:0")
        let addr = tcp_local_addr(server)
        let client = tcp_connect(addr)
        let accepted = tcp_accept(server)
        let server_stream = accepted[0]

        let sent = socket_write(client, "ping")
        let received = socket_read(server_stream, 1024)

        socket_write(server_stream, "pong")
        let client_received = socket_read(client, 1024)

        socket_close(client)
        socket_close(server_stream)
        socket_close(server)

        [sent, received, client_received]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Int(4),
            Value::string("ping"),
            Value::string("pong"),
        ])
    );
}

#[test]
fn test_socket_nonblocking_and_bytes() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let server = tcp_listen("127.0.0.1:0")
        socket_set_nonblocking(server, true)
        let non_accepted = tcp_accept(server)

        let addr = tcp_local_addr(server)
        let client = tcp_connect(addr)
        let accepted = tcp_accept(server)
        let server_stream = accepted[0]

        socket_write(client, [1, 2, 3, 4])
        let bytes = socket_read_bytes(server_stream, 10)

        socket_close(client)
        socket_close(server_stream)
        socket_close(server)

        [non_accepted, bytes]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Nil,
            Value::list(vec![
                Value::Int(1),
                Value::Int(2),
                Value::Int(3),
                Value::Int(4)
            ]),
        ])
    );
}
