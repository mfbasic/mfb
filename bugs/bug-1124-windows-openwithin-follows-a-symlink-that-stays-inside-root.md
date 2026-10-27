# bug-1124: Windows `fs::openWithin` follows a symbolic link that stays inside `root`

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: MED
Class: Runtime (platform parity, path-containment contract)

Status: Open
Regression Test: —

`mfb man fs openWithin` says a `relPath` that traverses a symbolic link at any
component is refused with `ErrAccessDenied`, matching `fs::openFileNoFollow`. On
macOS and Linux that holds. On Windows (read, not run)
`src/codegen/builtins/fs/gen_open.rs:lower_fs_open_within_helper`'s Windows arm
does a plain open and then `platform.emit_verify_within`
(`src/target/win_x86_64/code.rs:emit_verify_within`), which accepts any open whose
final path is still under the root. A link or junction inside `root` that points
elsewhere inside `root` is followed.

**The correct behavior a fix produces:** on Windows, as on the other hosts, a
`relPath` passing through any reparse point raises `ErrAccessDenied`
(as `fs::openFileNoFollow` does there, `src/target/win_x86_64/plan.rs`).

References:

- Found by plan-125-D Phase 5, finding `fs-openWithin` 2 (source only).
