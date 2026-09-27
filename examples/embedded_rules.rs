//! Run with: cargo run --example embedded_rules
use rynd::{RyndEngine, RyndResult, Value};
use std::{cell::RefCell, rc::Rc};

fn main() -> RyndResult<()> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    let mut engine = RyndEngine::new();
    engine.set_global("minimum", Value::Int(2000));
    engine.register_closure("record", 1, move |args| {
        recorded.borrow_mut().push(bool::try_from(&args[0])?);
        Ok(Value::Nil)
    });
    engine.eval(
        r#"
        fn approve(cents) {
            let accepted = cents >= minimum
            record(accepted)
            accepted
        }
    "#,
    )?;
    // Compile once, supply typed data on every call. No source interpolation.
    assert_eq!(
        engine.call("approve", &[Value::Int(3500)])?,
        Value::Bool(true)
    );
    assert_eq!(
        engine.call("approve", &[Value::Int(500)])?,
        Value::Bool(false)
    );
    assert_eq!(*events.borrow(), vec![true, false]);
    println!("Decisions: {:?}", events.borrow());
    Ok(())
}
