# bug-967: console `term::on` while TUI mode is already on allocates a new shadow-grid block without freeing the old one (~445 KB leak per call at 24×80)

Last updated: 2026-10-05
Effort: small (<1h)
Severity: MEDIUM
Class: Memory-safety

Status: Open
Regression Test: (proposed) `--debug` console program calling `term::on()` N times then `term::off()` reports `arena.0.live_bytes 0` for N = 1, 10, 100

`term::on` allocates the console shadow-grid header block (back + front + out buffer +
EGC pool, sized from `terminalSize`) and stores it in term-state slot 48. A second
`term::on` while already on (documented as resetting state; bug-728 made it keep the
saved termios) allocates **another** block and overwrites slot 48, so the previous
block is unreachable. `term::off` / `_mfb_shutdown` free only the last one.

The single correct behavior: a re-entered `term::on` reuses (or frees and replaces) the
existing grid, so live arena bytes after `term::off` are 0 however many times `on`
was called.

References:

- `src/docs/spec/app/04_term-backend.md` "Console shadow-grid header block":
  "`term::on` allocates one arena block ... stores its base pointer in term-state slot
  48 ... `term::off` runs the final present, frees the block, and zeroes slot 48".
- bugs/completed/bug-728 (re-entered `term::on`), tests/runtime/rt_term_on_reentry.rs
  (checks termios only).

## Failing Reproduction

```
IMPORT io
IMPORT term
IMPORT os

FUNC main() AS Nothing
  LET n AS Integer = toInt(os::getEnvOr("N", "10"))
  MUT i AS Integer = 0
  WHILE i < n
    term::on()
    i = i + 1
  END WHILE
  term::off()
END FUNC
```

```
mfb build --debug tl1
N=1|10|100 python3 /tmp/hardening/B17/pty_run.py tl1/build/p1.out 24 80   # report on stderr in the pty
```

- Observed (debug report, 24×80 pty):
  - N=1: `arena.0.alloc_calls 2 arena.0.free_calls 2 arena.0.live_bytes 0`
  - N=10: `alloc_calls 11 free_calls 2 live_bytes 4009968`
  - N=100: `alloc_calls 101 free_calls 2 live_bytes 44109648`
- Expected: `live_bytes 0` in all three.

## Root Cause

`src/codegen/term/core/term.rs:emit_on` calls `term_grid::emit_grid_alloc`
unconditionally before resetting state; it never tests slot 48 (`TERM_STATE_*` grid
header) for an existing block, unlike the termios save which is skipped when the
raw-active flag is set.

## Goal

- Re-entered `term::on` frees (or reuses) the existing grid; N-fold `on` + one `off`
  leaves no live grid bytes.

### Non-goals (must NOT change)

- bug-728's "first `on` saves termios" rule; the full repaint a re-entered `on` does.

## Blast Radius

- App backends keep their grid in the view/state (macOS `TVSTATE`, GTK static arrays),
  not via this path — not affected by this leak (macOS/GTK not measured).
