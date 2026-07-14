# bug-227: syntaxcheck cleanup — dead empty-else arm, mis-coded ISOLATED diagnostic, dangling comment

Last updated: 2026-07-14
Effort: small (<1h)
Severity: LOW
Class: dead-code / docs

Status: Open

Three minor cleanups in the syntax checker:

- `src/syntaxcheck/checking.rs:423-427` — the `else if else_body.is_empty()`
  branch in the `Statement::If` fallthrough merge is unreachable: `check_block` on
  an empty body always returns `Flow::FallsThrough`, so the preceding
  `if else_flow == Flow::FallsThrough` always wins for an empty else. Fix: delete
  the dead arm.
- `src/syntaxcheck/mod.rs:1629-1638` — an ISOLATED-must-be-project-visible
  violation is reported under the rule code `TYPE_CALL_ARGUMENT_MISMATCH`,
  mis-classifying a declaration/visibility error as a call-argument error. Fix:
  introduce/use a dedicated code (e.g. `TYPE_ISOLATED_NOT_VISIBLE`).
- `src/syntaxcheck/checking.rs:166-168` — a dangling/truncated comment
  ("A `RES` binding whose ownership floats… becomes") describes a check relocated
  to `ir::verify` and no longer present here. Fix: remove or complete it.
