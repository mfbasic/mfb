# bug-110 — GTK app-mode exit-code formatter lacks the bug-70 mask → garbage transcript digits

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G7).
**Severity:** MED — divergent, garbled exit-code display on Linux GTK app mode.
**Class:** correctness (platform divergence).

## Finding

`src/target/linux_gtk/bootstrap.rs:522-549` (`emit_format_exit_code`, used by
`emit_finish_helper`). The macOS sibling
(`macos_aarch64/app/bootstrap.rs:770-786`) masks the exit code with
`and x9, x9, 255` (the bug-70 fix) before computing hundreds/tens/ones. The GTK
copy has no mask: a code ≥ 1000 makes hundreds ≥ 10 and emits `'0'+10 = ':'`
garbage; a negative code (u64 wrap) emits garbage for all three digits; a code
of 300 prints "300" on Linux but "44" on macOS — divergent from what `_exit`
actually delivers (exit codes are truncated to 8 bits).

## Trigger

Linux GTK app-mode (`mfb build -app`, GUI): program `RETURN 1000` → transcript
shows "Program exited with code :00". Same program on macOS shows "232".

## Fix

Add the `and …, 255` mask to `emit_format_exit_code` before the digit
computation, matching macOS.

## Prior art

bug-70 fixed macOS only; no doc mentions the GTK sibling.
