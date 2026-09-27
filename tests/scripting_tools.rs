use rynd::{RyndEngine, Value};
use std::{fs, path::PathBuf, process::Command};

#[test]
fn collection_boundaries_callback_order_and_failure_recovery() {
    let mut engine = RyndEngine::new();
    engine.enable_output_capture();
    assert_eq!(
        engine
            .eval(r"[1, 2, 3] |> each(\x -> println(x))")
            .unwrap()
            .to_string(),
        "[1, 2, 3]"
    );
    assert_eq!(engine.take_output(), "1\n2\n3\n");
    assert_eq!(engine.eval("[chunks([], 2), windows([1], 2), zip([], [1]), scan([], 7, \\a,b -> a+b), partition([], \\x -> true)]").unwrap().to_string(), "[[], [], [], [], ([], [])]");
    for source in [
        "chunks([1], 0)",
        "windows([1], -1)",
        "chunks([1], true)",
        "flatten([1])",
        "each([], 42)",
        "partition([], \\a,b -> true)",
        "zip_with([], [], \\x -> x)",
        "scan([], 0, \\x -> x)",
        "grep('x', 1)",
        "replace('a', 'a', 1)",
        "cat([1])",
        "path_join([1])",
        "parse_json_lines('bad')",
        "to_json_lines([Some(1)])",
        "benchmark(\\ -> 1, 0)",
        "benchmark(\\x -> x, 1)",
        "benchmark(\\ -> 1 / 0, 1)",
        "sleep_ms(-1)",
        "assert(false, 'failure')",
        "assert(true, 1)",
    ] {
        assert!(engine.eval(source).is_err(), "{source}");
        assert_eq!(engine.eval("6 * 7").unwrap(), Value::Int(42));
    }
    assert!(
        engine
            .eval("parse_json_lines(\"1\\nbad\")")
            .unwrap_err()
            .to_string()
            .contains("JSON line 2")
    );
    assert_eq!(
        engine.eval("uniq([1, 1.0, 2])").unwrap().to_string(),
        "[1, 2]"
    );
}

#[test]
fn files_bytes_processes_and_json_lines_work_on_both_targets() {
    let root = PathBuf::from(format!("target/scripting-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let program = env!("CARGO_BIN_EXE_rynd");
    let source = format!(
        r#"
        let root = {root:?}
        mkdir_all(path_join([root, 'nested']))
        let path = path_join([root, 'nested', 'data.txt'])
        write_text(path, "a\nb\n")
        append_text(path, "c\n")
        assert(read_lines(path) == ['a', 'b', 'c'], 'lines')
        assert(cat([path, path]) == "a\nb\nc\na\nb\nc\n", 'cat')
        assert(file_info(path).size == 6, 'size')
        assert(file_info(path).is_file and not file_info(path).is_dir, 'file')
        assert(exists(path) and not exists(path_join([root, 'missing'])), 'exists')
        assert(len(list_dir(path_join([root, 'nested']))) == 1, 'listing')
        write_bytes(path, [0, 127, 128, 255])
        assert(read_bytes(path) == [0, 127, 128, 255], 'bytes')
        assert(len(cwd()) > 0, 'cwd')
        assert(env('RYND_TEST_MISSING_928189') == nil, 'missing environment')
        sleep_ms(0)
        let result = run_process({program:?}, ['eval', 'read_stdin()'], 'hello')
        assert(result.success and result.status == 0, 'process success')
        assert(result.stdout == "hello\n" and result.stderr == '', 'process output')
        assert(result.stdout_bytes == [104, 101, 108, 108, 111, 10], 'process bytes')
        let failed = run_process({program:?}, ['eval', '1 / 0'], '')
        assert(not failed.success and failed.status == 1, 'process failure')
        assert(contains(failed.stderr, 'Division by zero'), 'process error')
        'ok'
    "#,
        root = root.to_string_lossy()
    );
    assert_eq!(
        RyndEngine::new().eval(&source).unwrap(),
        Value::string("ok")
    );
    let rust = rynd::transpile(&source).unwrap();
    let binary = root.join("native");
    rynd::build::native(&rust, &binary).unwrap();
    let output = Command::new(binary).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ok\n");
    let mut engine = RyndEngine::new();
    engine.set_global("path", Value::string(root.join("absent").to_string_lossy()));
    for source in [
        "read_lines(path)",
        "cat([path])",
        "read_bytes(path)",
        "list_dir(path)",
        "file_info(path)",
        "write_bytes(path, [256])",
        "write_bytes(path, [-1])",
        "run_process(path, [], '')",
        "run_process('unused', [1], '')",
    ] {
        assert!(engine.eval(source).is_err(), "{source}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn process_pipes_drain_while_writing_large_input() {
    let mut engine = RyndEngine::new();
    engine.set_global("program", Value::string(env!("CARGO_BIN_EXE_rynd")));
    engine.set_global("input", Value::string("a".repeat(200_000)));
    let result = engine.eval(r#"run_process(program, ['eval', 'range(0, 100000) |> map(\x -> "x") |> join("") |> print(); len(read_stdin())'], input)"#).unwrap();
    let Value::Map(fields) = result else { panic!() };
    assert_eq!(fields["status"], Value::Int(0));
    let output = <&str>::try_from(&fields["stdout"]).unwrap();
    assert_eq!(output.len(), 100007);
    assert!(output.ends_with("200000\n"));
}
