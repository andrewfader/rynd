use rynd::{RyndEngine, Value};

#[test]
fn test_fiber_spawn_and_await() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let f = spawn(\ -> 10 + 32)
        await_fiber(f)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::Int(42));
}

#[test]
fn test_fiber_multiple_and_yield() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let f1 = spawn(\ -> {
            yield_fiber()
            100
        })
        let f2 = spawn(\ -> 200)
        let r1 = await_fiber(f1)
        let r2 = await_fiber(f2)
        r1 + r2
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::Int(300));
}

#[test]
fn test_structured_nursery_basic() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let results = nursery(\n -> {
            n.spawn(\ -> 10)
            n.spawn(\ -> 20)
            n.spawn(\ -> 30)
        })
        results
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![Value::Int(10), Value::Int(20), Value::Int(30)])
    );
}

#[test]
fn test_structured_nursery_failure_cascading() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let outcome = attempt(\ -> {
            nursery(\n -> {
                n.spawn(\ -> 1)
                n.spawn(\ -> 10 / 0)
            })
        }, [])
        outcome.tag
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::string("Err"));
}

#[test]
fn test_actor_mailbox_communication() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let worker = spawn_actor(\mb -> {
            let x = receive(mb)
            let y = receive(mb)
            x * y
        })
        send(worker, 6)
        send(worker, 7)
        await_fiber(worker)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::Int(42));
}

#[test]
fn test_actor_receive_timeout() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let worker = spawn_actor(\mb -> {
            receive_timeout(mb, 20)
        })
        await_fiber(worker)
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::variant("None", vec![]));
}

#[test]
fn test_csp_channel_communication() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let ch = channel()
        let tx = ch[0]
        let rx = ch[1]

        let empty = channel_try_recv(rx)
        channel_send(tx, "message_via_channel")
        let received = channel_recv(rx)

        [empty, received]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::variant("None", vec![]),
            Value::string("message_via_channel")
        ])
    );
}

#[test]
fn test_fiber_pipeline_workflow() {
    let mut engine = RyndEngine::new();
    let code = r#"
        let ch = channel()
        let tx = ch[0]
        let rx = ch[1]

        let producer = spawn(\ -> {
            [1, 2, 3, 4]
            |> each(\x -> channel_send(tx, x * 10))
        })
        await_fiber(producer)

        let a = channel_recv(rx)
        let b = channel_recv(rx)
        let c = channel_recv(rx)
        let d = channel_recv(rx)

        [a, b, c, d]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Int(10),
            Value::Int(20),
            Value::Int(30),
            Value::Int(40)
        ])
    );
}

#[test]
fn test_self_id_returns_fiber_handle() {
    // `self_id` should return a Fiber(id) handle so it composes with the rest
    // of the fiber API. On the main thread the id is 0.
    let mut engine = RyndEngine::new();
    let code = r#"
        let id = self_id()
        [id.tag, id.value]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![Value::string("Fiber"), Value::Int(0),])
    );
}

#[test]
fn test_self_id_inside_spawned_fiber() {
    // A spawned fiber sees its own id; the main thread continues to see 0.
    let mut engine = RyndEngine::new();
    let code = r#"
        let f = spawn(\ -> self_id().value)
        let child_id = await_fiber(f)
        let main_id = self_id().value
        [main_id, child_id]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    if let Value::List(items) = res {
        assert_eq!(items[0], Value::Int(0)); // main thread
        if let Value::Int(child) = items[1] {
            assert!(child > 0, "spawned fiber should have a non-zero id");
        } else {
            panic!("expected int child id, got {:?}", items[1]);
        }
    } else {
        panic!("expected list, got {res:?}");
    }
}

#[test]
fn test_close_channel_releases_queue() {
    // Closing either side of a channel removes the underlying queue. After
    // close, `is_channel_closed` reports true and `channel_recv` errors.
    let mut engine = RyndEngine::new();
    let code = r#"
        let ch = channel()
        let tx = ch[0]
        let rx = ch[1]
        channel_send(tx, "hello")
        let before_close = is_channel_closed(rx)
        let removed = close_channel(tx)
        let after_close = is_channel_closed(rx)
        [before_close, removed, after_close]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
        ])
    );
}

#[test]
fn test_close_mailbox_releases_queue() {
    // Manually created mailboxes can be closed explicitly; subsequent send or
    // receive should fail because the queue is gone.
    let mut engine = RyndEngine::new();
    let code = r#"
        let mb = create_mailbox()
        send(mb, "queued")
        let closed = close_mailbox(mb)
        let outcome = attempt(\ -> receive(mb), [])
        [closed, outcome.tag]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(
        res,
        Value::list(vec![Value::Bool(true), Value::string("Err")])
    );
}

#[test]
fn test_await_fiber_twice_returns_cached_result() {
    // A fiber can be awaited at most once. The first await executes the task,
    // caches the result, and removes the fiber entry. A second await on the
    // same handle must fail with "Invalid or expired Fiber handle".
    let mut engine = RyndEngine::new();
    let code = r#"
        let f = spawn(\ -> 42)
        let first = await_fiber(f)
        let second = attempt(\ -> await_fiber(f), [])
        [first, second.tag]
    "#;
    let res = engine.eval(code).expect("Eval failed");
    assert_eq!(res, Value::list(vec![Value::Int(42), Value::string("Err")]));
}

#[test]
fn receivers_block_until_another_fiber_sends() {
    // The consumer starts first and parks in channel_recv; only a real
    // suspension lets the producer run and wake it.
    let mut engine = RyndEngine::new();
    let code = r#"
        let (tx, rx) = channel()
        let consumer = spawn(\ -> [channel_recv(rx), channel_recv(rx)])
        let producer = spawn(\ -> {
            channel_send(tx, 1)
            yield_fiber()
            channel_send(tx, 2)
        })
        await_fiber(producer)
        await_fiber(consumer)
    "#;
    assert_eq!(
        engine.eval(code).unwrap(),
        Value::list(vec![Value::Int(1), Value::Int(2)])
    );
}

#[test]
fn actors_ping_pong_in_constant_space() {
    let mut engine = RyndEngine::new();
    let code = r#"
        fn pong(mb, n) {
            if n == 0 { "done" } else {
                let (from, x) = receive(mb)
                send(from, x + 1)
                pong(mb, n - 1)
            }
        }
        fn ping(mb, peer, n, last) {
            if n == 0 { last } else {
                send(peer, (mb, last))
                ping(mb, peer, n - 1, receive(mb))
            }
        }
        let ponger = spawn_actor(\mb -> pong(mb, 20000))
        let me = create_mailbox()
        [ping(me, ponger, 20000, 0), await_fiber(ponger)]
    "#;
    assert_eq!(
        engine.eval(code).unwrap(),
        Value::list(vec![Value::Int(20000), Value::string("done")])
    );
}

#[test]
fn waiting_with_no_one_left_to_wake_us_is_a_deadlock_error() {
    let mut engine = RyndEngine::new();
    for code in [
        "receive(create_mailbox())",
        "let (tx, rx) = channel()\nchannel_recv(rx)",
        "let f = spawn(\\ -> receive(create_mailbox()))\nawait_fiber(f)",
    ] {
        let error = engine.eval(code).unwrap_err().to_string();
        assert!(error.contains("Deadlock"), "{code}: {error}");
    }
    // The engine stays usable afterwards.
    assert_eq!(engine.eval("1 + 1").unwrap(), Value::Int(2));
}

#[test]
fn sleeping_fibers_overlap() {
    let mut engine = RyndEngine::new();
    let started = std::time::Instant::now();
    let code = r#"
        nursery(\n -> {
            n.spawn(\ -> { sleep_ms(300); 1 })
            n.spawn(\ -> { sleep_ms(300); 2 })
            n.spawn(\ -> { sleep_ms(300); 3 })
        })
    "#;
    assert_eq!(
        engine.eval(code).unwrap(),
        Value::list(vec![Value::Int(1), Value::Int(2), Value::Int(3)])
    );
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_millis(800),
        "{elapsed:?}"
    );
}

#[test]
fn a_failing_child_cancels_its_sleeping_siblings() {
    let mut engine = RyndEngine::new();
    let started = std::time::Instant::now();
    let code = r#"
        let ch = channel()
        let outcome = attempt(\ -> nursery(\n -> {
            n.spawn(\ -> { sleep_ms(20000); "late" })
            n.spawn(\ -> channel_recv(ch[1]))
            n.spawn(\ -> { sleep_ms(10); 1 / 0 })
        }), [])
        outcome
    "#;
    let outcome = engine.eval(code).unwrap().to_string();
    assert!(outcome.contains("Division by zero"), "{outcome}");
    let elapsed = started.elapsed();
    assert!(elapsed < std::time::Duration::from_secs(5), "{elapsed:?}");
}

#[test]
fn fibers_get_deep_stacks_and_tail_calls() {
    let mut engine = RyndEngine::new();
    let code = r#"
        fn down(n) { n == 0 ? 0 : 1 + down(n - 1) }
        fn spin(n) { n == 0 ? "spun" : spin(n - 1) }
        let f = spawn(\ -> [down(5000), spin(200000)])
        await_fiber(f)
    "#;
    assert_eq!(
        engine.eval(code).unwrap(),
        Value::list(vec![Value::Int(5000), Value::string("spun")])
    );
}

#[test]
fn fiber_output_is_captured_by_the_engine() {
    let mut engine = RyndEngine::new();
    engine.enable_output_capture();
    engine
        .eval(r#"await_fiber(spawn(\ -> println("from a fiber")))"#)
        .unwrap();
    assert_eq!(engine.captured_output(), "from a fiber\n");
}

#[test]
fn engines_on_separate_threads_schedule_independently() {
    let threads: Vec<_> = (0..4)
        .map(|i| {
            std::thread::spawn(move || {
                let mut engine = RyndEngine::new();
                engine.set_global("seed", Value::Int(i));
                engine
                    .eval(
                        r#"
                        let fibers = range(0, 20) |> map(\k -> spawn(\ -> { yield_fiber(); seed * 100 + k }))
                        fibers |> map(await_fiber) |> sum()
                    "#,
                    )
                    .unwrap()
                    .to_string()
            })
        })
        .collect();
    for (i, thread) in threads.into_iter().enumerate() {
        assert_eq!(thread.join().unwrap(), (i * 2000 + 190).to_string());
    }
}
