use rynd::{RyndEngine, Value};

#[test]
fn test_metaprogramming_dynamic_apply() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let sum_res = apply("sum", [[10, 20, 30]])
        let lambda_res = apply(\a, b -> a * b + 2, [6, 7])
        [sum_res, lambda_res]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::list(vec![Value::Int(60), Value::Int(44)]));
}

#[test]
fn test_metaprogramming_call_method_on_map() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let account = {
            "balance": 100,
            "deposit": \self, amount -> {
                put(self, "balance", self.balance + amount)
            },
            "summary": \self -> "Balance: #{self.balance}"
        }

        let updated = call_method(account, "deposit", [50])
        let msg = call_method(updated, "summary", [])

        [updated.balance, msg]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![Value::Int(150), Value::string("Balance: 150")])
    );
}

#[test]
fn test_map_metaprogramming_primitives() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let m1 = {"a": 1, "b": 2}
        let m2 = {"b": 20, "c": 30}

        let merged = merge(m1, m2)
        let modified = put(merged, "d", 40)
        let deleted = delete(modified, "a")

        let val_c = get(deleted, "c", 0)
        let fallback = get(deleted, "missing", 999)
        let has_b = has_key(deleted, "b")
        let has_a = has_key(deleted, "a")

        [deleted.b, deleted.d, val_c, fallback, has_b, has_a]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Int(20),
            Value::Int(40),
            Value::Int(30),
            Value::Int(999),
            Value::Bool(true),
            Value::Bool(false),
        ])
    );
}

#[test]
fn test_route_pattern_matching() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let m1 = match_route("/users/42", "/users/:id")
        let m2 = match_route("/posts/2026/dev/intro", "/posts/:year/*slug")
        let m3 = match_route("/unrelated/path", "/users/:id")

        let id = if m1.tag == "Some" { m1.value.id } else { "" }
        let year = if m2.tag == "Some" { m2.value.year } else { "" }
        let slug = if m2.tag == "Some" { m2.value.slug } else { "" }
        let tag3 = m3.tag

        [id, year, slug, tag3]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::string("42"),
            Value::string("2026"),
            Value::string("dev/intro"),
            Value::string("None"),
        ])
    );
}

#[test]
fn test_plug_pipeline_and_json_response() {
    let mut engine = RyndEngine::new();
    let code = r#"
        fn plug_logger(conn) {
            put_resp_header(conn, "x-request-id", "req-12345")
        }

        fn plug_cors(conn) {
            put_resp_header(conn, "access-control-allow-origin", "*")
        }

        fn router(conn) {
            if conn.halted { return conn }

            let m = match_route(conn.path, "/api/v1/users/:id")
            if m.tag == "Some" and conn.method == "GET" {
                return json_response(conn, 200, {
                    "id": m.value.id,
                    "name": "Ada Lovelace",
                    "admin": true
                })
            }

            text_response(conn, 404, "Route Not Found")
        }

        let connection = conn("GET", "/api/v1/users/99")
            |> plug_logger()
            |> plug_cors()
            |> router()

        [
            connection.status,
            connection.halted,
            connection.resp_headers["x-request-id"],
            connection.resp_headers["access-control-allow-origin"],
            connection.resp_headers["content-type"],
            parse_json(connection.resp_body).name
        ]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Int(200),
            Value::Bool(true),
            Value::string("req-12345"),
            Value::string("*"),
            Value::string("application/json"),
            Value::string("Ada Lovelace"),
        ])
    );
}

#[test]
fn test_plug_mounted_engines_and_subapps() {
    let mut engine = RyndEngine::new();
    let code = r#"
        # Sub-app / Engine
        fn admin_engine(conn) {
            if conn.halted { return conn }

            let m = match_route(conn.path, "/dashboard")
            if m.tag == "Some" {
                return html_response(conn, 200, "<h1>Admin Dashboard</h1>")
            }

            let m_stats = match_route(conn.path, "/metrics")
            if m_stats.tag == "Some" {
                return json_response(conn, 200, {"uptime": 99.9})
            }

            text_response(conn, 404, "Admin 404")
        }

        # Engine mount helper
        fn mount(conn, prefix, engine_handler) {
            if conn.halted { return conn }
            if contains(conn.path, prefix) {
                let sub_path = split(conn.path, prefix)[1]
                let target_path = if sub_path == "" { "/" } else { sub_path }
                let sub_conn = conn |> put("path", target_path)
                let handled = engine_handler(sub_conn)
                return handled |> put("path", conn.path)
            }
            conn
        }

        # Root app pipeline
        fn app(conn) {
            conn
            |> mount("/admin", admin_engine)
            |> (\c -> {
                if c.halted { return c }
                let m = match_route(c.path, "/")
                if m.tag == "Some" {
                    return text_response(c, 200, "Welcome Home")
                }
                text_response(c, 404, "Global 404")
            })()
        }

        let admin_req = conn("GET", "/admin/dashboard") |> app()
        let home_req = conn("GET", "/") |> app()
        let missing_req = conn("GET", "/not-existing") |> app()

        [
            admin_req.status,
            admin_req.resp_body,
            home_req.status,
            home_req.resp_body,
            missing_req.status
        ]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Int(200),
            Value::string("<h1>Admin Dashboard</h1>"),
            Value::Int(200),
            Value::string("Welcome Home"),
            Value::Int(404),
        ])
    );
}

#[test]
fn test_socket_streaming_http_chunks() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let server = tcp_listen("127.0.0.1:0")
        let addr = tcp_local_addr(server)
        let client = tcp_connect(addr)
        let accepted = tcp_accept(server)
        let server_stream = accepted[0]

        # Send HTTP chunked stream
        socket_write_http_chunk(client, "chunk one; ")
        socket_write_http_chunk(client, "chunk two")
        socket_finish_http_chunks(client)

        let received = socket_read(server_stream, 1024)

        socket_close(client)
        socket_close(server_stream)
        socket_close(server)

        received
    "#;
    let res = engine.eval(code).expect("Eval failed");
    if let Value::String(s) = res {
        assert!(s.contains("B\r\nchunk one; \r\n"));
        assert!(s.contains("9\r\nchunk two\r\n"));
        assert!(s.ends_with("0\r\n\r\n"));
    } else {
        panic!("Expected string result, got {res:?}");
    }
}
