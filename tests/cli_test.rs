use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn cli(args: &[&str], input: &str) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_rynd"))
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn cli_errors_are_nonzero_and_help_is_available() {
    for args in [
        vec!["run"],
        vec!["compile"],
        vec!["eval"],
        vec!["unknown"],
        vec!["run", "/nonexistent/rynd/file"],
        vec!["compile", "/nonexistent/rynd/file"],
        vec!["compile", "README.md", "-o"],
        vec!["compile", "README.md", "--typo"],
        vec!["eval", "1 / 0"],
    ] {
        let out = cli(&args, "");
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        assert!(!out.stderr.is_empty());
    }
    for args in [vec![], vec!["--help"], vec!["-h"], vec!["help"]] {
        let out = cli(&args, "");
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("USAGE"));
    }
    assert_eq!(cli(&["eval", "3.14159"], "").stdout, b"3.14159\n");
}

#[test]
fn repl_retains_functions_and_recovers_after_errors() {
    let out = cli(
        &["repl"],
        "\nfn double(x) { x * 2 }\ndouble(21)\n1 / 0\ndouble(5)\nquit\n",
    );
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("=> 42\n"));
    assert!(stdout.contains("=> 10\n"));
    assert!(
        String::from_utf8(out.stderr)
            .unwrap()
            .contains("Division by zero (at 1:3)")
    );
    assert!(cli(&["repl"], "").status.success());
    assert!(cli(&["repl"], "exit\n").status.success());
}

#[test]
fn cli_runs_and_transpiles_checked_in_examples() {
    let out = cli(&["run", "examples/fibonacci.rynd"], "");
    assert_eq!(out.stdout, b"Recursive fib(10):\n55\nMatch fib(10):\n55\n");
    let out = cli(&["compile", "examples/fibonacci.rynd"], "");
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("fn rynd_program"));
    let dir =
        std::path::PathBuf::from("target").join(format!("rynd-cli-test-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("test.rs");
    assert!(
        cli(
            &[
                "compile",
                "examples/fibonacci.rynd",
                "-o",
                source.to_str().unwrap()
            ],
            ""
        )
        .status
        .success()
    );
    assert!(source.exists());
    assert_eq!(
        cli(
            &[
                "compile",
                "examples/fibonacci.rynd",
                "-o",
                dir.to_str().unwrap()
            ],
            ""
        )
        .status
        .code(),
        Some(1)
    );
    let malformed = dir.join("bad.rynd");
    fs::write(&malformed, "let =").unwrap();
    assert_eq!(
        cli(&["compile", malformed.to_str().unwrap()], "")
            .status
            .code(),
        Some(1)
    );
    fs::write(&malformed, "1 / 0").unwrap();
    assert_eq!(
        cli(&["run", malformed.to_str().unwrap()], "").status.code(),
        Some(1)
    );
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn benchmark_success_checks_native_output() {
    let out = cli(&["bench"], "");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.contains("in-process, excluding startup"));
    assert!(stdout.contains("Handwritten typed Rust"));
    assert!(stdout.contains("Benchmark suite passed successfully."));
}

#[cfg(unix)]
#[test]
fn benchmark_compiler_failure_and_missing_compiler_are_errors() {
    use std::os::unix::fs::PermissionsExt;
    let dir = std::path::PathBuf::from("target")
        .join(format!("rynd-failed-rustc-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let rustc = dir.join("rustc");
    fs::write(
        &rustc,
        "#!/bin/sh\necho intentional-compiler-failure >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&rustc, fs::Permissions::from_mode(0o755)).unwrap();
    for path in [dir.clone(), dir.join("missing")] {
        let out = Command::new(env!("CARGO_BIN_EXE_rynd"))
            .arg("bench")
            .env("PATH", path)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(!String::from_utf8_lossy(&out.stdout).contains("passed successfully"));
        assert!(String::from_utf8_lossy(&out.stderr).contains("Benchmark failed"));
    }
    fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn scripts_accept_stdin_arguments_and_check_without_execution() {
    assert_eq!(
        cli(&["run", "-", "--", "a b", "é"], "args |> join(\"|\")").stdout,
        "a b|é\n".as_bytes()
    );
    assert_eq!(
        cli(
            &["eval", "read_stdin() |> lines() |> map(parse_int) |> sum()"],
            "10\n20\n"
        )
        .stdout,
        b"30\n"
    );
    assert_eq!(
        cli(&["eval", "args[0]", "--", "hello"], "").stdout,
        b"hello\n"
    );
    assert!(cli(&["check", "-"], "println(1 / 0)").status.success());
    assert!(!cli(&["check", "-"], "let =").status.success());
    for args in [
        vec!["eval", "1", "2"],
        vec!["repl", "extra"],
        vec!["--version", "extra"],
        vec!["check"],
        vec!["check", "-", "extra"],
    ] {
        assert!(!cli(&args, "").status.success(), "{args:?}");
    }
    assert_eq!(cli(&["--version"], "").stdout, b"rynd 0.1.0\n");
}

#[test]
fn repl_handles_multiline_input_and_cancel_without_prompt_noise() {
    let out = cli(
        &["repl"],
        "fn double(x) {\n x * 2\n}\ndouble(21)\n[1,\n2] |> sum()\n1 +\n2\n\"{\"\nlet x = [\n:cancel\n4\n:reset\n5\n:quit\n",
    );
    assert!(out.status.success());
    assert_eq!(out.stderr, b"");
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(
        stdout.contains("=> 42\n=> 3\n=> 3\n=> {\n=> 4\n=> 5\n"),
        "{stdout}"
    );
    assert!(!stdout.contains("rynd>"));
    assert!(!cli(&["repl"], "fn f() {\n").status.success());
    assert!(cli(&["repl"], ":help\n").status.success());
}

#[test]
fn sales_report_works_from_files_stdin_and_native_binary() {
    let data = fs::read_to_string("examples/data/sales.tsv").unwrap();
    let expected = b"Sales: 3\nTotal cents: 7100\nCustomers at or above 2000 cents: Grace, Linus\n";
    let vm = cli(
        &[
            "run",
            "examples/sales_report.rynd",
            "--",
            "examples/data/sales.tsv",
        ],
        "",
    );
    assert!(
        vm.status.success(),
        "{}",
        String::from_utf8_lossy(&vm.stderr)
    );
    assert_eq!(vm.stdout, expected);
    let piped = cli(&["run", "examples/sales_report.rynd"], &data);
    assert!(piped.status.success());
    assert_eq!(piped.stdout, expected);
    let dir =
        std::path::PathBuf::from("target").join(format!("sales-workflow-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let rs = dir.join("report.rs");
    let bin = dir.join("report");
    assert!(
        cli(
            &[
                "compile",
                "examples/sales_report.rynd",
                "-o",
                rs.to_str().unwrap()
            ],
            ""
        )
        .status
        .success()
    );
    let compiled = Command::new("rustc")
        .arg("-O")
        .arg(&rs)
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let native = Command::new(&bin)
        .arg("examples/data/sales.tsv")
        .output()
        .unwrap();
    assert!(native.status.success());
    assert_eq!(native.stdout, expected);
    let mut child = Command::new(&bin)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(data.as_bytes())
        .unwrap();
    let piped = child.wait_with_output().unwrap();
    assert!(piped.status.success());
    assert_eq!(piped.stdout, expected);
    assert!(
        !cli(&["run", "examples/sales_report.rynd"], "Ada\tnot-money\n")
            .status
            .success()
    );
    assert!(
        !cli(&["run", "examples/sales_report.rynd"], "wrong columns\n")
            .status
            .success()
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn build_produces_executable_and_preserves_input_on_failures() {
    let dir =
        std::path::PathBuf::from("target").join(format!("native-build-{}", std::process::id()));
    fs::create_dir_all(&dir).unwrap();
    let source = dir.join("program.rynd");
    let binary = dir.join("program");
    fs::write(&source, "args |> join(\"|\")").unwrap();
    let built = cli(
        &[
            "build",
            source.to_str().unwrap(),
            "-o",
            binary.to_str().unwrap(),
        ],
        "",
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let out = Command::new(&binary)
        .args(["hello world", "é"])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, "hello world|é\n".as_bytes());
    let original = fs::read(&source).unwrap();
    assert!(
        !cli(
            &[
                "build",
                source.to_str().unwrap(),
                "-o",
                source.to_str().unwrap()
            ],
            ""
        )
        .status
        .success()
    );
    assert_eq!(fs::read(&source).unwrap(), original);
    let before = fs::read(&binary).unwrap();
    let failed = Command::new(env!("CARGO_BIN_EXE_rynd"))
        .args([
            "build",
            source.to_str().unwrap(),
            "-o",
            binary.to_str().unwrap(),
        ])
        .env("PATH", dir.join("no-compiler"))
        .output()
        .unwrap();
    assert!(!failed.status.success());
    assert_eq!(fs::read(&binary).unwrap(), before);
    assert!(
        !cli(
            &[
                "compile",
                source.to_str().unwrap(),
                "-o",
                source.to_str().unwrap()
            ],
            ""
        )
        .status
        .success()
    );
    assert_eq!(fs::read(&source).unwrap(), original);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cli_json_pipe() {
    use std::io::Write;
    use std::process::Stdio;
    let bin = env!("CARGO_BIN_EXE_rynd");
    let mut child = Command::new(bin)
        .args([
            "eval",
            "read_stdin() |> parse_json() |> sort_by(\\x -> x.n) |> to_json()",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"[{\"n\":2},{\"n\":1}]")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success());
    assert_eq!(out.stdout, b"[{\"n\":1},{\"n\":2}]\n");
}

#[test]
fn repl_introspection_history_loading_and_measurement() {
    let dir = format!("target/repl-tools-{}", std::process::id());
    fs::create_dir_all(&dir).unwrap();
    let script = format!("{dir}/loaded.rynd");
    let saved = format!("{dir}/saved.rynd");
    fs::write(&script, "let loaded = 7\nloaded").unwrap();
    let input = format!(
        "let amount = 21\namount * 2\n_ + 1\n:vars amount\n:complete amo\n:type amount\n:time amount * 2\n:bench 3 amount + 1\n:history\n:! 2\n:save {saved}\n:load {script}\n:reload\nloaded\n:bench zero 1\n:! 999\n:unknown\nquit\n"
    );
    let out = cli(&["repl"], &input);
    assert!(out.status.success());
    let stdout = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "=> 42",
        "=> 43",
        "amount: int = 21",
        "amount\n",
        "=> int",
        "\"iterations\": 3",
        "\"result\": 22",
        "1: let amount = 21",
        "=> 7",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
    assert!(fs::read_to_string(saved).unwrap().contains("amount * 2"));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("iterations must be positive"));
    assert!(stderr.contains("no such history entry"));
    assert!(stderr.contains("Unknown REPL command"));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn streaming_lines_transform_filter_number_and_report_errors() {
    let out = cli(
        &["lines", "upper(line) if contains(line, 'warn')"],
        "ok\nwarn one\r\nwarn two\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(out.stdout, b"WARN ONE\nWARN TWO\n");
    assert_eq!(cli(&["-p", "line_number"], "a\n\nb").stdout, b"1\n2\n3\n");
    assert_eq!(cli(&["-n", "println(line)"], "a\nb\n").stdout, b"a\nb\n");
    assert!(cli(&["-n", "line"], "a\n").stdout.is_empty());
    let out = cli(
        &["lines", "parse_json(line).name"],
        "{\"name\":\"Ada\"}\nbad\n",
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(out.stdout, b"Ada\n");
    assert!(String::from_utf8_lossy(&out.stderr).contains("-:2:"));
    let dir = format!("target/line-tools-{}", std::process::id());
    fs::create_dir_all(&dir).unwrap();
    let a = format!("{dir}/a");
    let b = format!("{dir}/b");
    fs::write(&a, "one\ntwo\n").unwrap();
    fs::write(&b, "three\n").unwrap();
    let out = cli(
        &["lines", "[line_number, record_number, line]", "--", &a, &b],
        "",
    );
    assert_eq!(out.stdout, b"[1, 1, one]\n[2, 2, two]\n[1, 3, three]\n");
    assert!(out.status.success());
    assert_eq!(
        cli(&["lines", "line", "missing-rynd-test-file"], "")
            .status
            .code(),
        Some(1)
    );
    fs::remove_dir_all(dir).unwrap();
}

#[cfg(not(feature = "jit"))]
#[test]
fn jit_flag_explains_how_to_enable_it() {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rynd"))
        .args(["eval", "--jit", "1"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("--features jit"));
}
