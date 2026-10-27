# bug-1120: `fs::deleteDirectory` goes through a directory symbolic link (trailing separator; Windows)

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: MED
Class: Runtime (filesystem, platform parity)

Status: Open
Regression Test: —

`mfb man fs deleteDirectory` says it never removes a symbolic link and that a
link to a directory is refused with `ErrInvalidPath`. Two cases break that:

1. **Trailing separator (macOS, measured).** With `target/` a directory and
   `link -> target`, `fs::deleteDirectory(link)` raises `ErrInvalidPath` as
   documented, but `fs::deleteDirectory(link & "/")` succeeds: the host resolves
   the path through the link, removes `target`, and leaves `link` dangling.
2. **Windows (read, not run).** `src/target/win_x86_64/code.rs:emit_fs_path_operation`
   maps `FsPathOperation::Rmdir` to `RemoveDirectoryW` with no reparse-point check;
   `RemoveDirectoryW` removes a directory symbolic link or junction itself.

**The correct behavior a fix produces:** on every target a `path` whose last
named entry is a symbolic link (with or without a trailing separator) raises
`ErrInvalidPath` and removes nothing.

References:

- Found by plan-125-D Phase 5, findings `fs-deleteDirectory` 1 and 3; probe
  `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-deleteDirectory/p_probe`.
