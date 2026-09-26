//! BDD (Behavior-Driven Development) Specifications for Rynd
//! Given-When-Then examples covering the language's public behavior.

use rynd::{RyndEngine, Value};

/// User Story:
/// As a data engineer writing in Rust,
/// I want an ergonomic pipeline operator `|>` to transform collections cleanly
/// So that my code reads linearly from input to output without nesting or temporary variables.
#[test]
fn scenario_pipeline_data_transformation() {
    // Given a dataset of numeric metrics
    let script = r#"
        let raw_metrics = [10, 15, 20, 25, 30, 35, 40]

        # When the metrics are filtered for values >= 20, doubled, and aggregated
        let processed = raw_metrics
            |> filter(\x -> x >= 20)
            |> map(\x -> x * 2)
            |> sum()

        processed
    "#;

    // When the Rynd engine evaluates the pipeline
    let mut engine = RyndEngine::new();
    let result = engine.eval(script).expect("Pipeline evaluation failed");

    // Then the output should equal the sum of [40, 50, 60, 70, 80] = 300
    assert_eq!(result, Value::Int(300));
}

/// User Story:
/// As a frontend/fullstack developer transitioning to Rust,
/// I want list comprehensions with inline conditionals
/// So that I can transform lists declaratively like in CoffeeScript or Python.
#[test]
fn scenario_declarative_list_comprehension() {
    // Given a list of integers from 1 to 10
    let script = r#"
        let numbers = range(1, 11)

        # When filtering for odd numbers and squaring them
        let odd_squares = [x * x for x in numbers if x % 2 != 0]

        odd_squares
    "#;

    // When evaluated
    let mut engine = RyndEngine::new();
    let result = engine
        .eval(script)
        .expect("Comprehension evaluation failed");

    // Then the result is [1, 9, 25, 49, 81]
    assert_eq!(
        result,
        Value::list(vec![
            Value::Int(1),
            Value::Int(9),
            Value::Int(25),
            Value::Int(49),
            Value::Int(81),
        ])
    );
}

/// User Story:
/// As a developer dealing with nullable/optional data,
/// I want Kotlin/Groovy-style Elvis (`?:`) and safe navigation (`?.`) operators
/// So that I can handle defaults safely without deeply nested Option unwrapping.
#[test]
fn scenario_null_safety_and_elvis_coalescing() {
    // Given user records with some missing profile fields
    let script = r#"
        let complete_user = {"name": "Grace Hopper", "title": "Rear Admiral"}
        let partial_user = {"name": "Anonymous"}
        let empty_user = nil

        let title1 = complete_user?.title ?: "Ensign"
        let title2 = partial_user?.title ?: "Civilian"
        let title3 = empty_user?.title ?: "Unknown"

        [title1, title2, title3]
    "#;

    // When evaluated
    let mut engine = RyndEngine::new();
    let result = engine.eval(script).expect("Elvis evaluation failed");

    // Then safe defaults are applied accurately
    assert_eq!(
        result,
        Value::list(vec![
            Value::string("Rear Admiral"),
            Value::string("Civilian"),
            Value::string("Unknown"),
        ])
    );
}

/// User Story:
/// As an algorithmic programmer,
/// I want pattern matching over values and variants with guard expressions
/// So that I can write declarative recursive algorithms with clarity.
#[test]
fn scenario_pattern_matching_with_guards() {
    // Given a pattern-matching function with guard clauses
    let script = r#"
        fn classify_temperature(temp) {
            match temp {
                t if t < 0 => "freezing",
                t if t <= 20 => "cool",
                t if t <= 30 => "warm",
                _ => "hot"
            }
        }

        [
            classify_temperature(-5),
            classify_temperature(15),
            classify_temperature(25),
            classify_temperature(38)
        ]
    "#;

    // When evaluated
    let mut engine = RyndEngine::new();
    let result = engine.eval(script).expect("Match evaluation failed");

    // Then the categories match the temperature intervals
    assert_eq!(
        result,
        Value::list(vec![
            Value::string("freezing"),
            Value::string("cool"),
            Value::string("warm"),
            Value::string("hot"),
        ])
    );
}

/// User Story:
/// As a Rust systems engineer,
/// I want to embed Rynd in my Rust service and register custom native Rust functions
/// So that non-technical users or script authors can safely script business rules.
#[test]
fn scenario_rust_host_embedding_with_native_callbacks() {
    // Given a RyndEngine with a registered host native function (calculating tax)
    let mut engine = RyndEngine::new();
    engine.register_fn("calculate_tax", 1, |args| match &args[0] {
        Value::Int(subtotal) => {
            let tax = (*subtotal as f64 * 0.08) as i64;
            Ok(Value::Int(tax))
        }
        Value::Float(subtotal) => {
            let tax = *subtotal * 0.08;
            Ok(Value::Float(tax))
        }
        _ => Err(rynd::RyndError::RuntimeError {
            message: "Expected number".to_string(),
        }),
    });

    // When running Rynd script that calls the native host function in a pipeline
    let script = r#"
        let item_prices = [100, 250, 400]
        let taxes = item_prices |> map(\price -> calculate_tax(price))
        sum(taxes)
    "#;

    let result = engine
        .eval(script)
        .expect("Host embedding execution failed");

    // Then taxes on 100 is 8, 250 is 20, 400 is 32; sum = 60
    assert_eq!(result, Value::Int(60));
}
