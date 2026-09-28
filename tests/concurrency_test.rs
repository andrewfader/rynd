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
