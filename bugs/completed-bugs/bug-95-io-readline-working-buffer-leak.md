# bug-95 — `io::readLine`/`io::input` leak the working line buffer on every call

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G4).
**Severity:** MED — unbounded arena growth in long-running line-processing programs.
**Class:** memory-safety (leak in emitted runtime helper).

## Finding

`src/target/shared/code/io_helpers.rs:1528-1873` — `lower_io_read_line_helper`
arena-allocates a 32-byte working buffer (doubling as the line grows), copies
it into the freshly allocated result String, and returns. The **grow** path
frees the *old* buffer via `ARENA_FREE_SYMBOL` (lines 1798-1804 — the only
free in the file), but the final working buffer — dead the moment the result
copy finishes at `result_copy_done` — is never freed, on the success path or
any error path. Scope-drop frees only cover user values, so nothing reclaims
it.

## Trigger

```
WHILE TRUE
  LET s = io::readLine()
  ...
WEND
```

Each iteration permanently loses max(32, ~2× line length) bytes of arena. A
long-running line-processing program's arena grows without bound proportional
to lines read, even though every `s` is scope-dropped correctly.

## Fix sketch

After `result_copy_done` (and on each error exit that owns the buffer), emit
an `ARENA_FREE_SYMBOL` call on the working-buffer pointer before returning,
mirroring the existing grow-path free.

## Prior art

plan-01-arena-update §8 Phase 3 landed only the grow-path free and noted
"further internal temporaries can follow" — the tail buffer was never
included. bug-01 / plan-25-A cover different leak sites.
