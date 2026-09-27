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
