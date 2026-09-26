use rynd::{RyndEngine, transpile};
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn run_e2e_transpile_and_verify(test_name: &str, rynd_source: &str) {
    verify(test_name, rynd_source, None, false);
}

fn verify(test_name: &str, source: &str, expected: Option<&str>, expect_error: bool) {
    let mut engine = RyndEngine::new();
    engine.enable_output_capture();
    let result = engine.eval(source);
    let mut vm_stdout = engine.captured_output();
    let (status, stderr) = match result {
        Ok(value) => {
            assert!(!expect_error, "Expected an error for {test_name}");
            if value != rynd::Value::Nil {
                vm_stdout.push_str(&format!("{value}\n"));
            }
            (0, String::new())
        }
        Err(error) => {
            assert!(expect_error, "Unexpected VM error for {test_name}: {error}");
            (1, format!("Execution Error: {error}\n"))
        }
    };
    if let Some(expected) = expected {
        assert_eq!(vm_stdout, expected, "VM oracle: {test_name}");
    }
    let code = transpile(source).expect("Transpilation failed");
    let dir = PathBuf::from("target/e2e_tests").join(std::process::id().to_string());
    fs::create_dir_all(&dir).unwrap();
    let rs = dir.join(format!("{test_name}.rs"));
    let bin = dir.join(format!("{test_name}.bin"));
    fs::write(&rs, code).unwrap();
    let compiled = Command::new("rustc")
        .arg("-O")
        .arg(&rs)
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{test_name}: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let native = Command::new(&bin).output().unwrap();
    assert_eq!(
        native.status.code(),
        Some(status),
        "exit status: {test_name}"
    );
    assert_eq!(native.stdout, vm_stdout.as_bytes(), "stdout: {test_name}");
    assert_eq!(native.stderr, stderr.as_bytes(), "stderr: {test_name}");
}

#[test]
fn test_e2e_arithmetic_transpile() {
    let source = r#"
        let a = 10
        let b = 25
        a * 2 + b
    "#;
    run_e2e_transpile_and_verify("arithmetic", source);
}

#[test]
fn test_e2e_fibonacci_transpile() {
    let source = r#"
        fn fib(n) {
            if n <= 1 {
                n
            } else {
                fib(n - 1) + fib(n - 2)
            }
        }
        fib(10)
    "#;
    run_e2e_transpile_and_verify("fibonacci", source);
}

#[test]
fn test_e2e_pipeline_transpile() {
    let source = r#"
        [1, 2, 3, 4, 5]
        |> filter(\x -> x > 2)
        |> map(\x -> x * 10)
        |> sum()
    "#;
    run_e2e_transpile_and_verify("pipeline", source);
}

#[test]
fn test_e2e_comprehension_transpile() {
    let source = r#"
        let numbers = [1, 2, 3, 4, 5, 6]
        let evens_doubled = [x * 2 for x in numbers if x % 2 == 0]
        sum(evens_doubled)
    "#;
    run_e2e_transpile_and_verify("comprehension", source);
}

#[test]
fn test_e2e_elvis_transpile() {
    let source = r#"
        let missing = nil ?: "fallback_value"
        missing
    "#;
    run_e2e_transpile_and_verify("elvis", source);
}

#[test]
fn regression_readme_pipeline() {
    verify(
        "regression_readme_pipeline",
        "let numbers = range(1, 21) let result = numbers |> filter(\\x -> x % 2 == 0) |> map(\\x -> x * 10) |> sum() println(result)",
        Some("1100\n"),
        false,
    );
}

#[test]
fn regression_readme_comprehension() {
    verify(
        "regression_readme_comprehension",
        "let data = [1, 2, 3, 4, 5, 6, 7, 8] let evens_squared = [x * x for x in data if x % 2 == 0] evens_squared",
        Some("[4, 16, 36, 64]\n"),
        false,
    );
}

#[test]
fn regression_readme_null_safety() {
    verify(
        "regression_readme_null_safety",
        "let user = {\"name\": \"Grace Hopper\"} let title = user?.title ?: \"Engineer\" title",
        Some("Engineer\n"),
        false,
    );
}

#[test]
fn regression_readme_match() {
    verify(
        "regression_readme_match",
        "fn classify(temp) { match temp { t if t < 0 => \"freezing\", t if t <= 20 => \"cool\", t if t <= 30 => \"warm\", _ => \"hot\" } } [classify(-5), classify(15), classify(25), classify(38)]",
        Some("[freezing, cool, warm, hot]\n"),
        false,
    );
}

#[test]
fn regression_float_precision() {
    verify(
        "regression_float_precision",
        "3.14159",
        Some("3.14159\n"),
        false,
    );
}

#[test]
fn regression_mixed_numeric_compare() {
    verify(
        "regression_mixed_numeric_compare",
        "[1 == 1.0, 2 > 1.0]",
        Some("[true, true]\n"),
        false,
    );
}

#[test]
fn regression_block_local_result() {
    verify(
        "regression_block_local_result",
        "fn f() { let x = 10 x + 1 } f()",
        Some("11\n"),
        false,
    );
}

#[test]
fn regression_capturing_global_lambda() {
    verify(
        "regression_capturing_global_lambda",
        "let offset = 10; [1, 2] |> map(\\x -> x + offset)",
        Some("[11, 12]\n"),
        false,
    );
}

#[test]
fn regression_capturing_local_lambda() {
    verify(
        "regression_capturing_local_lambda",
        "fn add_offset(offset) { [1, 2] |> map(\\x -> x + offset) } add_offset(10)",
        Some("[11, 12]\n"),
        false,
    );
}

#[test]
fn regression_named_function_global() {
    verify(
        "regression_named_function_global",
        "let offset = 10 fn add(x) { x + offset } add(1)",
        Some("11\n"),
        false,
    );
}

#[test]
fn regression_tuple_destructure() {
    verify(
        "regression_tuple_destructure",
        "let (a, b) = (1, 2) a + b",
        Some("3\n"),
        false,
    );
}

#[test]
fn regression_list_destructure() {
    verify(
        "regression_list_destructure",
        "let [a, b] = [1, 2] a + b",
        Some("3\n"),
        false,
    );
}

#[test]
fn regression_variant_payload() {
    verify(
        "regression_variant_payload",
        "match Some(42) { Some(x) => x, _ => 0 }",
        Some("42\n"),
        false,
    );
}

#[test]
fn regression_list_match_shape() {
    verify(
        "regression_list_match_shape",
        "match [1, 2] { [9, 9] => 1, _ => 2 }",
        Some("2\n"),
        false,
    );
}

#[test]
fn regression_tuple_match_shape() {
    verify(
        "regression_tuple_match_shape",
        "match (1, 2) { (9, 9) => 1, _ => 2 }",
        Some("2\n"),
        false,
    );
}

#[test]
fn regression_string_pattern_guard() {
    verify(
        "regression_string_pattern_guard",
        "match \"hello\" { \"hello\" if true => 1, _ => 0 }",
        Some("1\n"),
        false,
    );
}

#[test]
fn regression_logical_or_side_effect() {
    verify(
        "regression_logical_or_side_effect",
        "fn f() { println(\"called\") true } f() || false",
        Some("called\ntrue\n"),
        false,
    );
}

#[test]
fn regression_logical_and_side_effect() {
    verify(
        "regression_logical_and_side_effect",
        "fn f() { println(\"called\") false } f() && true",
        Some("called\nfalse\n"),
        false,
    );
}

#[test]
fn regression_unicode_string_index() {
    verify(
        "regression_unicode_string_index",
        "\"héllo\"[1]",
        Some("é\n"),
        false,
    );
}

#[test]
fn regression_reduce_builtin() {
    verify(
        "regression_reduce_builtin",
        "[1, 2, 3] |> reduce(0, \\acc, x -> acc + x)",
        Some("6\n"),
        false,
    );
}

#[test]
fn regression_named_map_callback() {
    verify(
        "regression_named_map_callback",
        "fn double(x) { x * 2 } [1, 2] |> map(double)",
        Some("[2, 4]\n"),
        false,
    );
}

#[test]
fn regression_match_binding_scope() {
    verify(
        "regression_match_binding_scope",
        "let t = 99 let result = match 1 { t => t } t",
        Some("99\n"),
        false,
    );
}

#[test]
fn regression_immutable_list_push() {
    verify(
        "regression_immutable_list_push",
        "let xs = [1, 2]; let ys = push(xs, 3); [xs, ys]",
        Some("[[1, 2], [1, 2, 3]]\n"),
        false,
    );
}

#[test]
fn regression_newline_list_statement() {
    verify(
        "regression_newline_list_statement",
        "let offset = 10\n[1, 2] |> sum()",
        Some("3\n"),
        false,
    );
}

#[test]
fn regression_elvis_falsy() {
    verify(
        "regression_elvis_falsy",
        "[0 ?: 99, false ?: true, \"\" ?: \"fallback\"]",
        Some("[99, true, fallback]\n"),
        false,
    );
}

#[test]
fn regression_elvis_option_result() {
    verify(
        "regression_elvis_option_result",
        "[Some(42) ?: 99, Ok(42) ?: 99, Err(42) ?: 99]",
        Some("[Some(42), Ok(42), 99]\n"),
        false,
    );
}

#[test]
fn regression_division_by_zero() {
    verify("regression_division_by_zero", "1 / 0", None, true);
}

#[test]
fn regression_type_error() {
    verify("regression_type_error", "1 + true", None, true);
}

#[test]
fn regression_map_comprehension() {
    verify(
        "regression_map_comprehension",
        "{to_string(x): x * x for x in 1..=4 if x % 2 == 0}",
        Some("{\"2\": 4, \"4\": 16}\n"),
        false,
    );
}

#[test]
fn regression_nested_closures() {
    verify(
        "regression_nested_closures",
        "fn make(x) { \\y -> \\z -> x + y + z } let a = make(1); let b = a(2); b(3)",
        Some("6\n"),
        false,
    );
}

#[test]
fn regression_nested_patterns() {
    verify(
        "regression_nested_patterns",
        "match Some((1, [2, 3])) { Some((a, [b, c])) => a + b + c, _ => 0 }",
        Some("6\n"),
        false,
    );
}

#[test]
fn regression_local_destructure() {
    verify(
        "regression_local_destructure",
        "fn f(a) { let (x, y) = (a, 2); x + y } f(10)",
        Some("12\n"),
        false,
    );
}

#[test]
fn regression_expression_stack() {
    verify(
        "regression_expression_stack",
        "1 + { let x = 10; x + 1 }",
        Some("12\n"),
        false,
    );
}

#[test]
fn regression_early_return() {
    verify(
        "regression_early_return",
        "fn f(x) { if x > 0 { return x + 1 } 0 } [f(2), f(0)]",
        Some("[3, 0]\n"),
        false,
    );
}

#[test]
fn regression_recursive_local() {
    verify(
        "regression_recursive_local",
        "fn outer() { fn fact(n) { if n <= 1 { 1 } else { n * fact(n - 1) } } fact(5) } outer()",
        Some("120\n"),
        false,
    );
}

#[test]
fn regression_whitespace_stdout() {
    verify(
        "regression_whitespace_stdout",
        "print(\"  a\\n\"); println(\" b \"); \" c \"",
        Some("  a\n b \n c \n"),
        false,
    );
}

#[test]
fn regression_output_before_error() {
    verify(
        "regression_output_before_error",
        "println(\"before\"); 1 / 0",
        None,
        true,
    );
}

#[test]
fn regression_overflow() {
    verify("regression_overflow", "9223372036854775807 + 1", None, true);
}

#[test]
fn regression_overflow_div() {
    verify(
        "regression_overflow_div",
        "(-9223372036854775807 - 1) / -1",
        None,
        true,
    );
}

#[test]
fn regression_mismatched_binding() {
    verify(
        "regression_mismatched_binding",
        "let [a,b] = [1]; a",
        None,
        true,
    );
}

#[test]
fn regression_wrong_arity() {
    verify("regression_wrong_arity", "sum()", None, true);
}

#[test]
fn regression_wrong_callback_arity() {
    verify(
        "regression_wrong_callback_arity",
        "[1] |> map(\\a, b -> a + b)",
        None,
        true,
    );
}

#[test]
fn regression_mixed_large_ints() {
    verify(
        "regression_mixed_large_ints",
        "[9007199254740993 == 9007199254740992.0, 9007199254740993 > 9007199254740992.0]",
        Some("[false, true]\n"),
        false,
    );
}

#[test]
fn regression_float_infinity() {
    verify(
        "regression_float_infinity",
        "[1.0 / 0.0 == 2.0 / 0.0, 0.0 / 0.0 == 0.0 / 0.0]",
        Some("[true, false]\n"),
        false,
    );
}

#[test]
fn regression_builtin_as_value() {
    verify(
        "regression_builtin_as_value",
        "let f = abs; f(-2)",
        Some("2\n"),
        false,
    );
}

#[test]
fn regression_function_in_list() {
    verify(
        "regression_function_in_list",
        "fn f(x) { x + 1 } let funcs = [f]; funcs[0](2)",
        Some("3\n"),
        false,
    );
}

#[test]
fn regression_shadow_builtin() {
    verify(
        "regression_shadow_builtin",
        "fn sum(x) { x + 100 } sum(1)",
        Some("101\n"),
        false,
    );
}

#[test]
fn regression_map_duplicate() {
    verify(
        "regression_map_duplicate",
        "{\"x\": 1, \"x\": 2}[\"x\"]",
        Some("2\n"),
        false,
    );
}

#[test]
fn regression_rust_keyword_identifier() {
    verify(
        "regression_rust_keyword_identifier",
        "let type = 5; type + 1",
        Some("6\n"),
        false,
    );
}

#[test]
fn regression_unmatched_pattern() {
    verify(
        "regression_unmatched_pattern",
        "match None { Some(x) => x }",
        Some(""),
        false,
    );
}

#[test]
fn regression_negative_pattern() {
    verify(
        "regression_negative_pattern",
        "match -2 { -2 => 7, _ => 0 }",
        Some("7\n"),
        false,
    );
}

#[test]
fn regression_unicode_length() {
    verify(
        "regression_unicode_length",
        "len(\"hé🙂\")",
        Some("3\n"),
        false,
    );
}

#[test]
fn regression_negative_index() {
    verify(
        "regression_negative_index",
        "[\"hé🙂\"[-1], [1,2][-1], (3,4)[-3]]",
        Some("[🙂, 2, nil]\n"),
        false,
    );
}

#[test]
fn regression_empty_map() {
    verify("regression_empty_map", "{}", Some("{}\n"), false);
}

#[test]
fn regression_bad_index() {
    verify("regression_bad_index", "1[0]", None, true);
}

#[test]
fn regression_bad_field() {
    verify("regression_bad_field", "1.name", None, true);
}

#[test]
fn regression_bad_sum() {
    verify("regression_bad_sum", "sum([1, true])", None, true);
}

#[test]
fn regression_bad_range() {
    verify("regression_bad_range", "range(\"a\", 3)", None, true);
}

#[test]
fn regression_safe_scalar_field() {
    verify("regression_safe_scalar_field", "1?.name", Some(""), false);
}

#[test]
fn regression_integer_min_scientific_and_inclusive_max() {
    verify(
        "numeric_boundaries",
        "[-9223372036854775808, 1e3, 2.5e-2, 9223372036854775807..=9223372036854775807]",
        Some("[-9223372036854775808, 1000, 0.025, [9223372036854775807]]\n"),
        false,
    );
}

#[test]
fn regression_recursion_limit_reports_error() {
    verify("recursion_limit", "fn f() { f() } f()", None, true);
}

#[test]
fn regression_all_builtins_and_variants() {
    verify(
        "all_builtins",
        "[sum([1,2.5]), len(\"hé🙂\"), abs(-2.5), min(3,2.0), max(3.0,2), head([]), tail([1,2]), concat([1],[2]), to_map([(\"x\",1)]), Some(1).value, None]",
        Some("[3.5, 3, 2.5, 2, 3, nil, [2], [1, 2], {\"x\": 1}, 1, None]\n"),
        false,
    );
}

#[test]
fn text_processing_native_parity() {
    verify(
        "text_processing",
        r#"
        let rows = " Ada, 12 \r\n Grace, 35 \n" |> lines()
        let values = rows |> map(\row -> split(row, ",")[1] |> parse_int())
        println(values |> sum())
        println(rows |> map(\row -> split(row, ",")[0] |> trim()) |> join(" / "))
        println([contains("café", "fé"), contains([1,2], 2), contains({"x":1}, "x")])
        println(parse_float("1.25e2"))
        println([len(lines("")), split("a,,b", ","), trim("\t hé \n")])
    "#,
        Some("47\nAda / Grace\n[true, true, true]\n125\n[0, [a, , b], hé]\n"),
        false,
    );
}

#[test]
fn invalid_text_conversion_native_parity() {
    verify(
        "invalid_text_conversion",
        "parse_int(\"not a number\")",
        Some(""),
        true,
    );
}

#[test]
fn inspired_expression_semantics() {
    verify(
        "inspired_expressions",
        r##"
        fn branch(label, value) { println(label); value }
        println(true ? branch("yes", 10) : branch("no", 20))
        println(false ? 1 : true ? 2 : 3)
        println([1,2,3] |> sum() > 5 ? "large" : "small")
        println(unless false { "allowed" } else { "denied" })
        let who = "Ada"
        println("Hello #{who}, #{1 + 2}! #{true ? "yes" : "no"}")
        println("map #{ {"x": 3}.x }; nested #{"inner #{2}"}")
        println("literal \#{who}")
        println("#{branch("once", 7)}")
    "##,
        Some(
            "yes\n10\n2\nlarge\nallowed\nHello Ada, 3! yes\nmap 3; nested inner 2\nliteral #{who}\nonce\n7\n",
        ),
        false,
    );
}

#[test]
fn data_pipeline_native_contract() {
    verify(
        "data_pipeline",
        r#"
        let rows = parse_json("[{\"name\":\"Ada\",\"team\":\"compiler\",\"score\":3},{\"name\":\"Grace\",\"team\":\"compiler\",\"score\":5}]")
        println(rows |> sort_by(\row -> -row.score) |> map(\row -> row.name) |> to_json())
        println(rows |> group_by(\row -> row.team) |> keys() |> to_json())
        println({"b":2,"a":1} |> entries() |> to_map() |> to_json())
        println({"b":2,"a":1} |> values() |> to_json())
        println([3,1,2] |> sort() |> skip(1) |> take(1) |> to_json())
        println([1,2] |> enumerate())
        println(zip([1,2], [3]))
        println([1,2] |> flat_map(\x -> [x,x*10]) |> to_json())
        println([0,1,2] |> filter_map(\x -> x % 2 == 0 ? Some(x) : None) |> to_json())
        fn pred(x) { println(x); x == 1 }
        println(any([0,1,2], pred))
        println(all([1,0,2], pred))
        println(find([0,1,2], pred))
        println(parse_json("null"))
    "#,
        Some(
            "[\"Grace\",\"Ada\"]\n[\"compiler\"]\n{\"a\":1,\"b\":2}\n[1,2]\n[2]\n[(0, 1), (1, 2)]\n[(1, 3)]\n[1,10,2,20]\n[0,2]\n0\n1\ntrue\n1\n0\nfalse\n0\n1\nSome(1)\nnil\n",
        ),
        false,
    );
}

#[test]
fn recoverable_errors_native_contract() {
    verify(
        "recoverable_errors",
        r#"
        fn fail(x) { println(x); 100 + 1/0 }
        fn recover(x) { 10 + match attempt(fail, [x]) { Err(e) => 7, _ => 0 } }
        println(map([1,2], recover))
        println(attempt(parse_int, ["no"]).tag)
        println(attempt(abs, [-3]))
        println(attempt(abs, []).tag)
        println(attempt(1, []).tag)
        println(attempt(map, [[3], fail]).tag)
        println(attempt(parse_json, ["[1,]"]).tag)
        println(attempt(sort, [[1,"a"]]).tag)
        println(attempt(sort, [[0.0 / 0.0]]).tag)
        println(attempt(to_json, [Some(1)]).tag)
        println(attempt(filter_map, [[1], abs]).tag)
        println(attempt(flat_map, [[1], abs]).tag)
        println(attempt(group_by, [[1], abs]).tag)
        println(40+2)
    "#,
        Some(
            "1\n2\n[17, 17]\nErr\nOk(3)\nErr\nErr\n3\nErr\nErr\nErr\nErr\nErr\nErr\nErr\nErr\n42\n",
        ),
        false,
    );
}

#[test]
fn json_errors_retain_source_locations_on_both_targets() {
    verify("json_error", "parse_json(\"[1,]\")", Some(""), true);
}
