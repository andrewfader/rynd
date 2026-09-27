# Interactive development

## REPL

Start `rynd repl`. Functions and variables persist across evaluations. Functions,
blocks, lists, and strings can span lines. `_` holds the last successful result.

| Command | Action |
| --- | --- |
| `:help` | Show available commands. |
| `:vars [text]` | Inspect globals and their types, optionally filtering names. |
| `:complete prefix` | List matching global and builtin names. |
| `:type expression` | Show the expression's runtime type. |
| `:history` / `:! N` | List evaluated source / evaluate entry N again. |
| `:save path` | Save evaluated source history as a script. |
| `:load path` / `:reload` | Evaluate a file and its modules / evaluate the last loaded file again. |
| `:time expression` | Evaluate once and report timing and result. |
| `:bench N expression` | Measure N evaluations. |
| `:bench` | Run the full benchmark suite. |
| `:cancel` | Discard pending multiline input. |
| `:reset` | Clear script state; preserve registered host callbacks. |
| `:quit` | Exit. |

## Source debugger

```sh
rynd debug examples/debug_rules.rynd
rynd debug application.rynd -- input.json
```

The debugger stops before executing the first source location. Commands accept
an optional leading colon; short aliases resemble Ruby debuggers.

| Command | Action |
| --- | --- |
| `break 5` / `b 5` | Break on line 5 of the current file. |
| `break helper.rynd:12 if count > 0` | Conditional breakpoint in a source file; relative paths also resolve beside the current file. |
| `breaks` / `delete N` | List breakpoints / delete breakpoint N. |
| `continue` / `c` | Run to a breakpoint, exception, or completion. |
| `step` / `s` | Stop at the next source-line visit, entering calls. |
| `next` / `n` | Advance at the current call depth, stepping over calls. |
| `finish` / `out` | Run until the current function returns. |
| `locals` | Inspect parameters, lexical locals, and captured values. |
| `globals [text]` | Inspect global values, optionally filtering names. |
| `bt` / `where` | Show the call stack. |
| `frame N` | Select a stack frame for inspection; zero is innermost. |
| `list` / `l` | Show nearby source with the selected location marked. |
| `p expression` | Evaluate an expression; bare expressions also work. |
| `type expression` | Inspect its runtime type. |
| `time expression` / `bench N expression` | Time expressions in the selected frame. |
| `complete prefix` | List matching globals, locals, and captures. |
| `history` / `! N` | List commands / repeat command N. |
| `save path` | Save debugger command history for replay through stdin. |
| `help` / `quit` | Show help / stop execution. |

Empty input repeats the last stepping command. Expressions support multiline
input and `:cancel`. Evaluation uses a snapshot of the selected frame's bindings;
script declarations in that evaluation belong to the snapshot. Calls perform
their ordinary process I/O and host effects. Conditional breakpoints use the
same expression evaluator. Inspection errors leave the debug session active.

Runtime exceptions pause with the failing frame available. Continuing propagates
the original error, including through an enclosing `attempt`. Breakpoints still
apply during `next` and `finish`. Source stepping follows line visits; repeated
callbacks and recursive calls on a single source line are distinct visits.

Normal execution uses the VM loop compiled without debugger hooks. Debug
compilation produces separate local-name metadata, and debug execution selects a separate
loop with hooks. The CLI debugger runs Rynd source through Meso. Generated native
executables retain their shared checked runtime semantics.

Commands also work through piped stdin, which makes a debugging session
reproducible in tests or editor tooling:

```sh
printf 'break 5\ncontinue\nlocals\ndelete 1\ncontinue\n' \
  | rynd debug examples/debug_rules.rynd
```
