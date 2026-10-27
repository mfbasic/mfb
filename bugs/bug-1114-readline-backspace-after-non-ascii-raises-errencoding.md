# bug-1114: `io::readLine` on a terminal raises `ErrEncoding` when Backspace follows a non-ASCII character

Last updated: 2026-10-26
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Runtime (terminal input)

Status: Open
Regression Test: —

`mfb man io readLine` says Backspace erases the last character. On a terminal
it erases one **byte**: after `é` (`c3 a9`) a Backspace leaves the lone lead
byte `c3`, and the line fails UTF-8 decoding with `ErrEncoding` (`77020004`).
A decomposed `e` + `U+0301` fails the same way. ASCII text is unaffected.

**The correct behavior a fix produces:** typing `é`, Backspace, `Z`, Return
returns the line `Z`; typing `e`, `U+0301`, Backspace returns `e`, as a UTF-8
aware terminal line discipline does.

References:

- Found by plan-125-D Phase 6, finding `io-readLine` 1
  (`planning/plan-125-findings/D-phase6-current/man-page-io-readLine.md`).
- Lead, not a diagnosis: the pty and macOS default is `-iutf8`, so the kernel's
  canonical-mode erase removes one byte. `emit_configure_stdin_terminal`
  (`src/codegen/io/terminal/io_terminal.rs`) configures the terminal for
  `readLine`, and `grep -rn IUTF8 src` finds nothing.

## Failing Reproduction

Program `plines` (a loop of `io::readLine()` printing `[line] len=N`), run
under a pty (`/tmp/p125-ex/d6-io/ptyrun.py`):

```
python3 -I ptyrun.py ./plines/build/plines.out 'ab\x7fc\n' '\xc3\xa9\x7fZ\n' 'q\n'
```

- Observed: `[ac] len=2`, then `ERR 77020004`, then `[q] len=1`.
- Expected: `[ac] len=2`, `[Z] len=1`, `[q] len=1`.
