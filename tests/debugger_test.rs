use std::{
    fs,
    io::Write,
    process::{Command, Stdio},
};

fn session(name: &str, source: &str, commands: &str) -> std::process::Output {
    let dir = format!("target/debugger-{}-{name}", std::process::id());
    fs::create_dir_all(&dir).unwrap();
    let file = format!("{dir}/main.rynd");
    fs::write(&file, source).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_rynd"))
        .args(["debug", &file, "--", "test-arg"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(commands.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    fs::remove_dir_all(dir).unwrap();
    output
}
const SOURCE: &str = "fn twice(x) {\n    let y = x * 2\n    y + 1\n}\nlet input = 20\nlet answer = twice(input)\nprintln(answer)\nanswer\n";

#[test]
fn breakpoints_inspection_frames_conditions_and_benchmarks() {
    let out = session(
        "inspect",
        SOURCE,
        "help\nbreak 3 if x == 20\nbreaks\ncontinue\nlocals\np x + y\nbt\nframe 1\np input\nframe 0\nlist\nglobals input\ntype y\ntime y + 1\nbench 3 x + y\np 1 / 0\np x\nhistory\ndelete 1\nfinish\ncontinue\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let output = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "breakpoint 1",
        "x = 20",
        "y = 40",
        "=> 60",
        "#1 nil",
        "Frame #1",
        "=> 20",
        ">    3",
        "input = 20",
        "=> int",
        "\"iterations\": 3",
        "\"result\": 60",
        "Error: Runtime error: Division by zero",
        "Breakpoint deleted",
        "Finished: 41",
    ] {
        assert!(output.contains(expected), "missing {expected}: {output}");
    }
}

#[test]
fn stepping_and_repeated_same_line_callbacks() {
    let out = session(
        "callbacks",
        "fn double(x) { x * 2 }\n[1, 2, 3] |> map(double)\n",
        "break 1 if x > 1\ncontinue\np x\ncontinue\np x\ncontinue\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let output = String::from_utf8(out.stdout).unwrap();
    assert!(output.contains("=> 2"), "{output}");
    assert!(output.contains("=> 3"), "{output}");
    assert_eq!(output.matches("(breakpoint 1)").count(), 2, "{output}");
    let out = session("steps", SOURCE, "step\nnext\n\ncontinue\n");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Finished: 41"));
}

#[test]
fn exception_stops_preserve_locals_and_bad_commands_recover() {
    let out = session(
        "exception",
        "fn bad(x) {\n    1 / x\n}\nbad(0)\n",
        "break nope\ndelete 99\nframe 99\nbench 0 x\nbench\np missing\np { println('before error'); 1 / 0 }\ncontinue\nlocals\nbt\ncontinue\n",
    );
    assert_eq!(out.status.code(), Some(1));
    let output = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "Usage: break",
        "Unknown breakpoint",
        "Invalid frame",
        "iterations must be positive",
        "Usage: bench",
        "Undefined variable",
        "before error\nError:",
        "Exception:",
        "x = 0",
    ] {
        assert!(output.contains(expected), "missing {expected}: {output}");
    }
    assert_eq!(output.matches("Exception:").count(), 1, "{output}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("Division by zero"));
    assert_eq!(session("quit", "1", "quit\n").status.code(), Some(1));
    assert_eq!(session("eof", "1", "").status.code(), Some(1));
}

#[test]
fn debugger_evaluation_keeps_program_bindings_isolated() {
    let out = session(
        "snapshot",
        "let x = 1\nx\n",
        "break 2\ncontinue\np let x = 99; x\np x\ncontinue\n",
    );
    assert!(out.status.success());
    let output = String::from_utf8(out.stdout).unwrap();
    assert!(output.contains("=> 99"), "{output}");
    assert!(output.contains("=> 1"), "{output}");
    assert!(output.contains("Finished: 1"), "{output}");
}

#[test]
fn debugger_multiline_completion_replay_and_captured_values() {
    let source = "fn make(offset) {\n    \\x -> {\n        let result = offset + x\n        result\n    }\n}\nlet f = make(5)\nf(7)\n";
    let output = session(
        "multiline",
        source,
        "break 999\nbreak 4\ncontinue\nlocals\ncomplete off\np {\nlet n = offset + x\nn * 2\n}\np {\n:cancel\np result\n! 999\nhistory\ncontinue\n",
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let out = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "beyond the end",
        "offset = 5",
        "x = 7",
        "offset\n",
        "=> 24",
        "Cancelled",
        "=> 12",
        "No such history entry",
        "Finished: 12",
    ] {
        assert!(out.contains(expected), "missing {expected}: {out}");
    }
}

#[test]
fn quit_cannot_be_swallowed_by_script_error_recovery() {
    let source = "fn work() {\n    println('work')\n}\nattempt(work, [])\nprintln('after')\n";
    let out = session("quit-recovery", source, "break 2\ncontinue\nquit\n");
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        !stdout.contains("\nwork\n") && !stdout.contains("\nafter\n"),
        "{stdout}"
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("Debugger stopped"));
}

#[test]
fn module_breakpoints_expose_source_names_and_private_globals() {
    let dir = format!("target/debugger-{}-modules", std::process::id());
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        format!("{dir}/helper.rynd"),
        "let offset = 2\npub fn twice(x) {\n    let y = x * offset\n    y\n}\n",
    )
    .unwrap();
    let out = session(
        "modules",
        "import 'helper.rynd' as helper\nhelper.twice(21)\n",
        "break helper.rynd:4\ncontinue\nlocals\nglobals offset\np x + offset\np twice(3)\ncomplete off\ncontinue\n",
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    for expected in [
        "x = 21",
        "y = 42",
        "offset = 2",
        "=> 23",
        "=> 6",
        "Finished: 42",
    ] {
        assert!(stdout.contains(expected), "missing {expected}: {stdout}");
    }
}
