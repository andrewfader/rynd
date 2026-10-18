//! Behavior specific to fibers being coroutines on one thread: cheap to
//! create, multiplexed I/O, and waits that cancellation can interrupt.
use rynd::{RyndEngine, Value};
use std::time::{Duration, Instant};

fn eval(source: &str) -> Value {
    RyndEngine::new().eval(source).unwrap()
}

#[test]
fn ten_thousand_fibers_run_concurrently() {
    let start = Instant::now();
    let total = eval(
        r"let fibers = range(0, 10000) |> map(\i -> spawn(\ -> { yield_fiber() i * 2 }))
          fibers |> map(\f -> await_fiber(f)) |> sum()",
    );
    assert_eq!(total, Value::Int(99990000));
    assert!(start.elapsed() < Duration::from_secs(20));
}

#[test]
fn sleeping_fibers_overlap() {
    let start = Instant::now();
    let done = eval(
        r"nursery(\n -> range(0, 500) |> each(\i -> n.spawn(\ -> { sleep_ms(100) i }))) |> len()",
    );
    assert_eq!(done, Value::Int(500));
    let elapsed = start.elapsed();
    assert!(elapsed >= Duration::from_millis(100), "{elapsed:?}");
    assert!(
        elapsed < Duration::from_secs(3),
        "sleeps did not overlap: {elapsed:?}"
    );
}

#[test]
fn fibers_recurse_deeply_on_their_own_stacks() {
    let value = eval(
        r"fn depth(n) { if n == 0 { 0 } else { 1 + depth(n - 1) } }
          await_fiber(spawn(\ -> depth(9000)))",
    );
    assert_eq!(value, Value::Int(9000));
    let error = RyndEngine::new()
        .eval(
            r"fn forever(n) { 1 + forever(n + 1) }
              await_fiber(spawn(\ -> forever(0)))",
        )
        .unwrap_err();
    assert!(
        error.to_string().contains("Call depth limit exceeded"),
        "{error}"
    );
}

#[test]
fn nursery_cancels_a_sibling_blocked_on_a_socket() {
    let start = Instant::now();
    let outcome = eval(
        r#"let listener = tcp_listen("127.0.0.1:0")
           let result = attempt(\ -> nursery(\n -> {
               n.spawn(\ -> tcp_accept(listener))
               n.spawn(\ -> { sleep_ms(20) 1 / 0 })
           }), [])
           socket_close(listener)
           result.tag"#,
    );
    assert_eq!(outcome, Value::string("Err"));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn timeouts_wake_waiters_and_messages_still_arrive() {
    let value = eval(
        r"let mb = create_mailbox()
          let timed_out = range(0, 50) |> map(\_ -> receive_timeout(mb, 1)) |> filter(\r -> r.tag == 'None') |> len()
          let sender = spawn(\ -> { sleep_ms(10) send(mb, 7) })
          let got = receive_timeout(mb, 5000)
          await_fiber(sender)
          [timed_out, got.value]",
    );
    assert_eq!(value, Value::list(vec![Value::Int(50), Value::Int(7)]));
}

#[test]
fn deadlocks_are_reported_from_the_waiting_fiber() {
    let error = RyndEngine::new()
        .eval(
            r"let ch = channel()
              let waiter = spawn(\ -> channel_recv(ch[1]))
              await_fiber(waiter)",
        )
        .unwrap_err();
    assert!(error.to_string().contains("Deadlock"), "{error}");
}

#[cfg(unix)]
#[test]
fn blocking_work_runs_beside_other_fibers() {
    let start = Instant::now();
    let value = eval(
        r#"let slow = spawn(\ -> run_process("sleep", ["0.3"], "").status)
           let ticks = spawn(\ -> range(0, 20) |> map(\i -> { sleep_ms(10) i }) |> len())
           [await_fiber(slow), await_fiber(ticks)]"#,
    );
    assert_eq!(value, Value::list(vec![Value::Int(0), Value::Int(20)]));
    assert!(
        start.elapsed() < Duration::from_millis(1500),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn fibers_left_running_resume_at_the_next_wait() {
    let mut engine = RyndEngine::new();
    engine
        .eval(r"let mb = create_mailbox() let bg = spawn(\ -> send(mb, 42))")
        .unwrap();
    assert_eq!(engine.eval("receive(mb)").unwrap(), Value::Int(42));
}

#[test]
fn a_panicking_host_function_fails_only_its_fiber() {
    let mut engine = RyndEngine::new();
    engine.register_closure("boom", 0, |_| panic!("host bug"));
    let value = engine
        .eval(
            r"let bad = attempt(\ -> await_fiber(spawn(\ -> boom())), [])
              let good = await_fiber(spawn(\ -> 5))
              [bad.tag, good]",
        )
        .unwrap();
    assert_eq!(
        value,
        Value::list(vec![Value::string("Err"), Value::Int(5)])
    );
}

#[test]
fn engines_on_separate_threads_have_separate_schedulers() {
    let handles: Vec<_> = (0..4)
        .map(|t| {
            std::thread::spawn(move || {
                let source = format!(
                    r"let fs = range(0, 200) |> map(\i -> spawn(\ -> {{ sleep_ms(1) i + {t} }}))
                      fs |> map(\f -> await_fiber(f)) |> sum()"
                );
                RyndEngine::new().eval(&source).unwrap().to_string()
            })
        })
        .collect();
    for (t, handle) in handles.into_iter().enumerate() {
        assert_eq!(handle.join().unwrap(), (19900 + 200 * t).to_string());
    }
}

#[test]
fn one_thread_serves_a_hundred_concurrent_clients() {
    let start = Instant::now();
    let value = eval(
        r#"let listener = tcp_listen("127.0.0.1:0")
           let addr = tcp_local_addr(listener)
           fn app(conn) { json_response(conn, 200, {"path": conn.path}) }
           let server = spawn(\ -> serve({"listener": listener, "max_connections": 120}, app))
           let clients = range(0, 120) |> map(\i -> spawn(\ -> fetch("http://" + addr + "/item/" + to_string(i))))
           let bodies = clients |> map(\f -> await_fiber(f)) |> filter(\r -> r.status == 200) |> map(\r -> parse_json(r.body).path)
           [len(bodies), bodies[119], await_fiber(server)]"#,
    );
    assert_eq!(
        value,
        Value::list(vec![
            Value::Int(120),
            Value::string("/item/119"),
            Value::Int(120)
        ])
    );
    assert!(
        start.elapsed() < Duration::from_secs(20),
        "{:?}",
        start.elapsed()
    );
}
