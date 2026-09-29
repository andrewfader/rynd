//! Run with: cargo run --example embedded_rules
use rynd::{RyndEngine, RyndResult, Value, Verdict};
use std::{cell::RefCell, rc::Rc};

rynd::record! {
    #[derive(Debug, Clone)]
    struct Policy { minimum: i64, currency: String }
}

fn main() -> RyndResult<()> {
    let events = Rc::new(RefCell::new(Vec::new()));
    let recorded = events.clone();
    let mut engine = RyndEngine::sandboxed();
    // Policy fields become script globals: `minimum`, `currency`.
    engine.install(&Policy {
        minimum: 2000,
        currency: "USD".into(),
    })?;
    engine.register("record", move |accepted: bool| {
        recorded.borrow_mut().push(accepted)
    });
    engine.eval(
        r##"
        fn approve(cents) {
            let accepted = cents >= minimum
            record(accepted)
            if accepted { Accept("#{cents} #{currency}") } else { Reject("below minimum") }
        }
    "##,
    )?;
    // Compile once, supply typed data on every call. No source interpolation.
    assert_eq!(
        engine.filtermap("approve", &[Value::Int(3500)])?,
        (Verdict::Accept, Value::from("3500 USD"))
    );
    assert_eq!(
        engine.filtermap("approve", &[Value::Int(500)])?,
        (Verdict::Reject, Value::from("below minimum"))
    );
    assert_eq!(*events.borrow(), vec![true, false]);
    println!("Decisions: {:?}", events.borrow());
    Ok(())
}
