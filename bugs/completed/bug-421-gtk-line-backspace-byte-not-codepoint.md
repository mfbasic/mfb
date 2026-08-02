# bug-421: Linux-GTK app-mode LINE input Backspace deletes one byte, not one code point → invalid UTF-8 committed for multi-byte input

Last updated: 2026-07-28
Effort: small (<1h)
Severity: LOW
Class: Correctness (UTF-8 boundary) — acknowledged in-code TODO(plan-05)

Status: FIXED (4a951ad0e)
Regression Test: `src/target/linux_gtk/bootstrap.rs` emit-inspection tests
(`backspace_removes_whole_codepoint_not_one_byte`,
`line_echo_backspace_erases_transcript_glyph`,
`delete_last_char_helper_is_codepoint_granular`). GTK app mode cannot be emulated
(`.ai/compiler.md` §Validation), so the regression is pinned at the emitted-
instruction level (the Windows/macOS codegen bug pattern): the Backspace handler
must scan line-buffer bytes for the UTF-8 continuation-byte boundary and, in
LINE_ECHO, call the transcript delete helper.

## STATUS: FIXED (4a951ad0e)

The Linux-GTK key handler's Backspace branch now removes one whole UTF-8 code
point instead of one byte: it scans back over continuation bytes
(`(b & 0xC0) == 0x80`) to the lead byte
(`do { len--; } while (len > 0 && (line_buf[len] & 0xC0) == 0x80)`), so a
multi-byte character (`é` = 0xC3 0xA9) is dropped whole and the committed line
stays valid UTF-8. In LINE_ECHO mode it calls a new `_mfb_gtkapp_delete_last_char`
helper that erases one character from the transcript via GtkTextIter's char-
granular `gtk_text_iter_backward_char` + `gtk_text_buffer_delete`, keeping the
echo in sync with the buffer. Two GTK imports (`gtk_text_iter_backward_char`,
`gtk_text_buffer_delete`) were declared for the helper, which is wired into both
the aarch64 and x86 app function lists.

Deviation from the doc: verified by emit-inspection tests + the full `cargo test`
(3635 passed), not by a live GTK run — Linux app mode is unemulatable, so no
end-to-end reproduction on a GTK box was performed (the doc's reproduction was
already source-only for the same reason).

Commit: 4a951ad0e (`bug-421: GTK app LINE Backspace deletes a whole UTF-8 code point`)

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
