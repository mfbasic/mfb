### 1. Terminal status is not ANSI-capability detection
UNIT:      man-page:io/isOutputTerminal
CLAIM:     "Use it to decide whether emitting ANSI colour, progress bars, or cursor tricks is appropriate, and to fall back to plain text when output is being captured."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/io/gen_is_terminal.rs:lower_is_terminal` calls only `isatty(1)` and converts its nonzero result to `TRUE`; it does not inspect terminal capabilities or `TERM`. The compiled scratch example run in a pseudo-terminal with `TERM=dumb` still printed `\x1b[32mStatus: OK\x1b[0m`.
SUGGESTED: Use it to detect whether output is attached to a terminal. Configure ANSI colour, progress bars, and cursor control separately when the target terminal supports them.

### 2. App-mode explanation is inaccurate and leaks implementation detail
UNIT:      man-page:io/isOutputTerminal
CLAIM:     "In app mode the program has no real standard streams — output is rendered by the application transcript window, which is treated as an interactive console — so this call returns TRUE without probing a descriptor."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/io/gen_is_terminal.rs:lower_is_terminal` does return `TRUE` in app mode, but `src/target/macos_aarch64/app/app_io.rs:emit_app_io_write` and `src/target/linux_gtk/app_io.rs:emit_app_io_write` both fall back to stdout/stderr file descriptors when no transcript is attached. “Probing a descriptor” is also implementation detail prohibited by `.ai/man-content.md` §3.
SUGGESTED: In app mode, this call returns `TRUE` because MFBASIC treats the application window as an interactive console.