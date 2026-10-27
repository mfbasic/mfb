# bug-1123: `fs::readAll`/`fs::readAllBytes` on a write-only `File` return empty instead of failing

Last updated: 2026-10-26
Effort: small (≤1h)
Severity: LOW
Class: Runtime (error reporting)

Status: Open
Regression Test: —

A `File` opened with `fs::open(path, "w")` or `"a"` cannot be read. `fs::readLine`
on it raises `ErrReadFailed`, but `fs::readAllBytes` (and `fs::readAll`) return an
empty value with no error, and `fs::eof` reports `TRUE` — so a program that
opened with the wrong mode silently sees an empty file.

Repro (plan-125 reviewer probe `probe2.mfb`): write `"data"` to a file, open it
with `"w"`... or, to keep contents, with `"a"`; `fs::readAllBytes(h)` → `[]`,
no error; `fs::readLine(h)` → `77020001 Read operation failed`.

**The correct behavior a fix produces:** every reader on a handle not opened for
reading raises `ErrReadFailed`, matching `fs::readLine`.

References:

- Found by plan-125-D Phase 5, finding `fs-open` 2; probe
  `/tmp/plan-125-scratch/D-phase5-current/man-page-fs-open/probe2.mfb`.
