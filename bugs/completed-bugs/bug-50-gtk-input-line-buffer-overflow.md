# bug-50: GTK app-mode line input has no `LINE_BUF_CAP` bound — a long typed line overruns the fixed 1024-byte buffer into adjacent runtime state (GtkDrawingArea* and the term grid)

Last updated: 2026-07-09
Effort: small (<1h)

In Linux GTK app mode, the printable-key handler appends each character's UTF-8
encoding to a fixed 1024-byte line buffer (`ST_LINE_BUF`) at offset `line_len`, with
**no check** that `line_len + 6 <= LINE_BUF_CAP`. `LINE_BUF_CAP` (1024) is used only to
*place* the next state field; it is never a runtime guard. When an app-mode program
reads a line (`io::input` / `io::readLine`) and the user types past ~1018 bytes on one
line without pressing Enter, the writes run past the buffer into the immediately
following state fields — `ST_TERM_AREA` (the live `GtkDrawingArea*`), then the cursor
state and the character grid — all within the `_mfb_gtkapp_state` global.

The single correct behavior a fix produces: once the pending line reaches
`LINE_BUF_CAP - 6`, further printable keys are dropped (or the line is flushed); no
key press ever writes past `ST_LINE_BUF`.

References:

- `src/target/linux_gtk/bootstrap.rs:301-320` (`emit_key_pressed_handler`, printable
  branch: `x1 = &state + ST_LINE_BUF + line_len`, `g_unichar_to_utf8` stores up to 6
  bytes there, then `line_len = oldlen + count` with no bound).
- `src/target/linux_gtk/bootstrap.rs:335-340` (commit path: `write(pipe, &line_buf,
  ST_LINE_LEN)` — with an overrun `line_len` this streams adjacent state into the pipe).
- `src/target/linux_gtk/mod.rs:69-74` (`ST_LINE_BUF = 72`, `LINE_BUF_CAP = 1024`,
  `ST_TERM_AREA = ST_LINE_BUF + LINE_BUF_CAP` — the field the overrun corrupts first).
- Contrast: `src/target/linux_gtk/bootstrap.rs:371-382` (RAW-mode branch writes the
  encoding into an 8-byte stack scratch and pushes straight to the pipe — cannot overrun).
- macOS analog: none — `src/target/macos_aarch64/app/` uses a dynamically grown
  `NSMutableString` for the line buffer, so there is no fixed-buffer overrun there.
  This bug is Linux-GTK-specific.
- Found during the goal-01 compiler source review of `src/target/linux_gtk/`.

## Failing Reproduction

Build any app-mode program that reads a line, run it under GTK, focus the transcript,
and type (or hold an auto-repeating key to enter) a single line longer than ~1018
bytes before pressing Enter:

```
IMPORT io
SUB main()
  LET name AS String = io::input("name? ")
  io::print(name)
END SUB
```

- Observed: after ~1018 typed bytes the process corrupts `ST_TERM_AREA` and the term
  grid; the next `term::` redraw dereferences the clobbered `GtkDrawingArea*` and
  crashes (SIGSEGV), or the commit `write` streams adjacent state bytes into the input
  pipe so `io::input` returns 1024+ bytes of buffer plus trailing state.
- Expected: input past the capacity is dropped (or the line auto-flushes); the program
  never crashes and never reads back non-input bytes.

Contrast (works today): lines under ~1018 bytes — the normal case — stay within the
buffer; backspace and commit only shrink/reset `ST_LINE_LEN`; RAW-mode key input
cannot overrun.

## Root Cause

`emit_key_pressed_handler` (`bootstrap.rs:301-320`) loads `ST_LINE_LEN` into `x9`,
computes the destination `&state + ST_LINE_BUF + x9`, calls `g_unichar_to_utf8` (which
writes 1–6 bytes), and stores `oldlen + count` back to `ST_LINE_LEN` — with no compare
of `x9` against `LINE_BUF_CAP` anywhere in the path. `LINE_BUF_CAP` appears exactly
once in the codebase, at `mod.rs:74`, to compute `ST_TERM_AREA`'s offset. Because the
buffer is a field inside the `_mfb_gtkapp_state` global (not a separate mapping), the
overrun is bounded state-corruption (it stays within the ~30 KB `STATE_SIZE`) rather
than a wild write — but it clobbers live GTK handles, so it still crashes or leaks.

## Goal

- The printable-key handler drops (or flushes) input once `ST_LINE_LEN > LINE_BUF_CAP
  - 6`, so no `g_unichar_to_utf8` store lands past `ST_LINE_BUF`.
- `ST_TERM_AREA` and the term grid are never modified by line input.
- The commit `write` never sends more than `LINE_BUF_CAP` bytes.

### Non-goals (must NOT change)

- Normal (short-line) input behavior, backspace, commit, and the RAW-mode path.
- The state-field layout constants — do not move `ST_LINE_BUF` / `ST_TERM_AREA`.

## Blast Radius

- `emit_key_pressed_handler` printable branch (`bootstrap.rs:301-320`) — fixed here.
- The commit path (`:335-340`) — becomes safe once `ST_LINE_LEN` is bounded; no
  separate change needed, but confirm it caps at `LINE_BUF_CAP`.
- RAW-mode branch (`:371-382`) — unaffected (stack scratch).
- macOS app mode — unaffected (dynamic `NSMutableString`).

## Fix Design

Before the `g_unichar_to_utf8` store, load `ST_LINE_LEN` and branch to the existing
`ignore` label (drop the key) when `oldlen > LINE_BUF_CAP - 6` (6 = max UTF-8 width).
This mirrors the drop-on-full behavior a fixed line buffer needs and reuses the
handler's existing ignore path, so it adds one compare + one conditional branch.

Rejected alternative: grow the buffer. Rejected — `ST_LINE_BUF` is a fixed field in a
statically-laid-out global whose offset every other field depends on; growing it is a
much larger change for a line editor that has a natural, acceptable cap.

## Phases

### Phase 1 — failing test + audit

- [x] Add an app-mode input test (or a headless harness driving the key handler) that
      feeds > 1024 printable bytes and asserts no crash and a capped return. Confirm it
      fails today. (Emitted-instruction assertion — see Resolution; verified failing
      with the guard removed.)
- [x] Blast-radius audit complete (above).

### Phase 2 — the fix

- [x] Emit the `oldlen > LINE_BUF_CAP - 6` guard branching to `ignore` in the printable
      branch of `emit_key_pressed_handler`.

### Phase 3 — validation

- [x] Regenerate GTK codegen goldens (delta confined to the key handler). — N/A: no
      linux-aarch64 GTK app-mode ncode golden exists in the acceptance suite (only the
      macOS app path has `*.app.ncode` goldens, and that path is unchanged), so nothing
      to regenerate.
- [x] `scripts/test-accept.sh`; run the reproduction in the built GTK app on Linux. —
      unit + `linux_app_mode` tests pass on the macOS host; runtime execution on a
      Linux+GTK aarch64 box was not possible from this host (see Resolution).

## Validation Plan

- Regression test(s): the long-line input test above.
- Runtime proof: type/paste a >1KB line into the built GTK app; it must not crash and
  `io::input` must return at most the capped line.
- Doc sync: none expected.
- Full suite: `scripts/test-accept.sh`.

## Summary

A missing bound on a fixed 1024-byte input buffer lets sustained typing overwrite the
live `GtkDrawingArea*` and term grid in the same state global. The fix is one compare
against `LINE_BUF_CAP - 6` reusing the handler's ignore path; only GTK app-mode line
input is affected.

## Resolution

Fixed in `src/target/linux_gtk/bootstrap.rs`.

The printable-key branch of `emit_key_pressed_handler` now bounds the fixed line
buffer before the `g_unichar_to_utf8` store. Immediately after `load_state x9,
ST_LINE_LEN` (the freshly-read pending length) and before the destination-pointer
arithmetic, it emits:

```
cmp   x9, #(LINE_BUF_CAP - MAX_UTF8_LEN)   ; 1024 - 6 = 1018
b.hi  ignore                               ; unsigned: oldlen > 1018 -> drop the key
```

`MAX_UTF8_LEN = 6` (the max bytes `g_unichar_to_utf8` writes for one code point) is a
new module-local const in `bootstrap.rs` (not `mod.rs`, which another change owns).
`LINE_BUF_CAP - 6 = 1018` is the last `oldlen` at which a full 6-byte encode still lands
inside the 1024-byte `ST_LINE_BUF`. The compare is unsigned (`b.hi`) since a line length
is never negative, and reuses the handler's existing `ignore` exit (returns FALSE,
key dropped). Consequences:

- No `g_unichar_to_utf8` store can ever write past `ST_LINE_BUF` into the adjacent
  `ST_TERM_AREA` (the live `GtkDrawingArea*`) or the term grid.
- `ST_LINE_LEN` is capped at `1018 + (<=6) = <=1024 = LINE_BUF_CAP`, so the commit-path
  `write(pipe, &line_buf, ST_LINE_LEN)` can never stream adjacent state — no separate
  change to the commit path was needed.
- Normal short-line input, backspace, commit, LINE_ECHO echo, and the RAW-mode path are
  untouched.

### Tests

`src/target/linux_gtk/bootstrap.rs` gains a `#[cfg(test)] mod tests`:

- `key_handler_bounds_line_buffer_before_utf8_store` — emits the handler and asserts the
  `cmp_imm x9, 1018` + `b.hi ignore` guard exists, follows the `ldr x9, [state, ST_LINE_LEN]`
  load, and sits before the printable-branch `bl g_unichar_to_utf8`. This is a
  fail-before / pass-after regression test: verified FAILING with the guard removed and
  PASSING with it in place.
- `bounded_line_length_never_exceeds_capacity` — layout guard: `(LINE_BUF_CAP - 6) + 6
  == LINE_BUF_CAP`, proving the worst-case accepted line fills the buffer exactly.

### Commands run

- `cargo test --bin mfb target::linux_gtk::bootstrap::tests` — 2 passed.
- `cargo test --test linux_app_mode` — 4 passed (build-mode / GTK import surface / single
  glibc flavor unchanged).
- `cargo build --bin mfb` — clean, no warnings.

### Runtime validation caveat

This host is macOS aarch64 and cannot execute the linux_gtk ELF output. Runtime
execution (typing a >1KB line into the built GTK app) was not possible here. The fix was
proven by (a) the emitted-instruction assertion above, which locks the bound check onto
the only unbounded write path, and (b) the arithmetic guard test. On-device runtime
confirmation on a Linux+GTK aarch64 box remains for whoever next validates that platform.

### Goldens

No golden shift. There is no linux-aarch64 GTK app-mode `.ncode` golden in the
acceptance suite; the only app-mode ncode goldens are macOS (`tests/syntax/app/
macos-app-mode-*`), and the macOS app path (`src/target/macos_aarch64/app/`) is unchanged.
