# bug-421: Linux-GTK app-mode LINE input Backspace deletes one byte, not one code point → invalid UTF-8 committed for multi-byte input

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Correctness (UTF-8 boundary) — acknowledged in-code TODO(plan-05)

Status: Open
Regression Test: tests/ — a GTK-app-mode `io::readLine` receiving a multi-byte
character followed by Backspace must commit a valid UTF-8 line (the whole character
removed), and the transcript echo must match.

In Linux-GTK app mode, the LINE-mode Backspace branch (`src/target/linux_gtk/
bootstrap.rs:599`) does `ST_LINE_LEN -= 1` — decrementing the line buffer length by
**one byte**, not one UTF-8 code point. Typing a multi-byte character (e.g. `é` =
2 UTF-8 bytes) in `io::input`/`io::readLine`, then pressing Backspace once, leaves a
stray continuation byte in `ST_LINE_BUF`; on Enter the committed line (written to
the pipe → fd 0 → the console `readLine`) contains an invalid/partial UTF-8
sequence. Separately, in LINE_ECHO the echoed glyph is not removed from the
transcript, so the display disagrees with the committed line.

Both are explicitly deferred by the in-code `TODO(plan-05)` at `bootstrap.rs:598`.
Filed here so the gap is tracked as a defect (not just a source TODO). Latent for
ASCII-only input; Linux-GTK-app-mode only.

References:

- `src/target/linux_gtk/bootstrap.rs:598` (`TODO(plan-05)`), `:599`
  (`ST_LINE_LEN -= 1`). Found during goal-07.

## Failing Reproduction

Requires a live GTK box (Linux app mode cannot be emulated per `.ai/compiler.md`
§Validation); not run. Confirmed by source: the Backspace branch subtracts a fixed
1 from the byte length with no continuation-byte scan.

- Observed: type `é`, Backspace, Enter → committed line has a lone `0xA9`
  continuation byte (invalid UTF-8); transcript still shows `é`.
- Expected: Backspace removes the whole `é` (both bytes); transcript matches.

## Root Cause

The Backspace handler treats `ST_LINE_LEN` as a code-point count and decrements by
1 byte, ignoring UTF-8 multi-byte encoding.

## Goal

- LINE-mode Backspace removes one whole UTF-8 code point (scan back over
  continuation bytes `0x80..0xBF` to the lead byte) and erases the corresponding
  echoed glyph from the transcript.

### Non-goals (must NOT change)

- ASCII single-byte behavior (already correct). The RAW/other input modes.

## Blast Radius

- `src/target/linux_gtk/bootstrap.rs:599` (LINE Backspace) and the LINE_ECHO
  transcript-erase path. Compare with the macOS/Windows app backends' Backspace
  handling for a consistent code-point-aware approach.
