### 1. App-mode explanation invents a false stream model
UNIT:      man-page:io/isInputTerminal
CLAIM:     "In app mode the program has no real standard streams — input is served by the application window, which is treated as an interactive console — so this call returns TRUE."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/io/gen_is_terminal.rs:lower_is_terminal` returns `TRUE` in app mode, while `src/target/macos_aarch64/app/bootstrap.rs:emit_input_pipe_wiring` routes the window-input pipe to standard input; `mfb build --app /tmp/plan-125-scratch/D-phase1/man-page-io-isInputTerminal` succeeded.
SUGGESTED: In app mode, `io::isInputTerminal` returns `TRUE` because the application window is treated as an interactive console.

### 2. Example claims to detect a human, but detects a terminal
UNIT:      man-page:io/isInputTerminal
CLAIM:     "Prompt only when a human is attached, otherwise read the piped stream:"
VERDICT:   misleading
EVIDENCE:  The probe `io::print(toString(io::isInputTerminal()))` printed `FALSE` with stdin redirected from `/dev/null` and `TRUE` under `/usr/bin/script`, demonstrating that `src/codegen/builtins/io/gen_is_terminal.rs:lower_is_terminal` tests terminal status, not whether a human is present.
SUGGESTED: Prompt only when standard input is a terminal; otherwise read the piped input.