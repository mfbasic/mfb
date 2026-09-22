# bug-669: with mouse reporting on, a lone Esc is held forever and the key after an Esc is lost

Last updated: 2026-09-21
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/runtime/rt_native_term_runtime.rs
(`native_term_mouse_poll_input_keeps_the_byte_after_an_escape`,
`native_term_mouse_escape_at_end_of_input_is_delivered`,
`native_term_mouse_lone_escape_is_reported_without_a_following_key`)

After `term::enableMouse(TRUE)`, every stdin byte goes through the SGR mouse
decoder. An `ESC` might be the start of a mouse report, so the decoder buffers
it until the next byte decides. Three things go wrong with that:

1. **A lone Esc keypress is never delivered on its own.** The Esc key sends one
   byte and then nothing. `io::pollInput` reads the ESC, sees it buffered, and
   answers FALSE, even with a 2 s timeout, because nothing else ever arrives.
   A blocking `io::readChar` blocks until the user presses another key. So a TUI
   that uses the mouse cannot bind Esc (found adding Esc-to-pause to
   `examples/bugs/tui`).
2. **The byte after an Esc is dropped by `io::pollInput`.** When `ESC x`
   arrives together, the decoder flushes the prefix: ESC is delivered now and
   `x` is owed. `pollInput`'s verify path then loses `x`. `a ESC x b ESC [ Z c`
   reaches a `pollInput`/`readChar` loop as `a ESC b ESC c`. Arrow keys lose
   their `[A` tails, and the game hung on Esc then `q`.
3. **An ESC right before EOF is lost.** The pump reads EOF with the ESC still
   buffered, and the held prefix is never handed back.

**Correct behavior:** every byte that is not part of a complete SGR mouse report
reaches the program, in order. A held escape prefix is handed back once input
has gone quiet for a short escape delay (25 ms) or has ended, whether the
program reads with `pollInput` or blocks in `readChar`/`readByte`/`readLine`.

References:

- `src/codegen/io/mouse/decode.rs` module doc: "a byte that is not part of a
  recognised mouse report reaches the program unchanged, in order — including
  a bare `ESC`".
- `mfb man io pollInput`: TRUE means a following read will not block. FALSE
  with a key waiting breaks the other half of that promise.
- plan-94-B (the decoder, ring and pump).

## Failing Reproduction

`cargo test --release --test rt_native_term_runtime mouse` at c3ddf0379:

- `native_term_mouse_poll_input_keeps_the_byte_after_an_escape`: stdin
  `a\x1bxb\x1b[Zc` through `DO WHILE io::pollInput(2000) … readChar`.
  Observed `CHARS:a<ESC>b<ESC>c`; expected `CHARS:a<ESC>xb<ESC>[Zc`.
- `native_term_mouse_escape_at_end_of_input_is_delivered`: stdin `a\x1b` then
  EOF, two `readChar`s. Observed `CHARS:a`; expected `CHARS:a\x1b`.
- `native_term_mouse_lone_escape_is_reported_without_a_following_key`: stdin
  `\x1b` with the pipe held open for 3 s. Observed `CHARS:` (pollInput FALSE
  straight away); expected `CHARS:<ESC>`. The blocking-`readChar` half is
  expected to return `GOT:ESC` while stdin is still open.

Contrast cases that pass today and must keep passing:
`native_term_mouse_leaves_other_input_untouched` (a bare ESC or `ESC [ Z` read
by a blocking `readChar` loop *when more bytes follow*), and
`native_term_mouse_keeps_poll_input_honest` (reports are swallowed by
`pollInput(0)` and still land in the ring). Without `enableMouse` there is no
decoder and a lone ESC works. Checked with a probe: `[ESC]` read fine with the
mouse off.

| Environment | Result |
| --- | --- |
| macOS aarch64, console, pipe or pty | fails ✗ (measured) |
| Linux / Windows console, app mode | same emitted logic (`emit_stdin_byte_read` / `lower_poll_input` are shared); guess: fails |

## Root Cause

- **(1) There is no escape delay.** `decode.rs:emit_decode_byte` returns
  `DECODE_BUFFERED` for an ESC and can only resolve it on the *next* byte.
  - In `func_poll_input.rs:emit_mouse_ready_verify`, a buffered byte takes the
    `consumed` arm, which forces the timeout to 0 and rechecks readiness.
    Nothing is ready, so pollInput returns FALSE with the ESC still held.
  - In `gen_read_family.rs:emit_pump_epilogue`, a buffered byte loops back to a
    blocking read.
  - Neither path ever releases a prefix because time has passed.
- **(2) The verify path damages the replay queue.**
  - `emit_mouse_ready_verify` asks "is a byte owed?" by calling
    `emit_drain_pending`. That call *advances* the drain cursor, so the owed
    byte is used up by the question and never read.
  - On a `DECODE_PASS` produced by a flush, `emit_pushback_byte` rewrites the
    buffer as a single byte (`len = 1, pos = 1`), throwing away the owed tail
    that the flush had just queued (`x`, or `[Z`).
- **(3) EOF with a prefix held.** `emit_pump_epilogue` passes a 0 count
  straight to the caller as EOF and never looks at the buffer.
- **(4) pollInput can't see owed bytes** (found during the fix). The owed-byte
  check lived only inside `emit_mouse_ready_verify`, which runs only after the
  log or OS poll says ready. Owed bytes live in the decoder, not the log. So
  after `readChar` took the ESC of an arrow key, `pollInput(30)` reported FALSE
  with `[A` still owed. The arrow read as Esc and the game unpaused.
- **(5) Stale drain cursor** (found during the fix). `emit_drain_pending`
  reset the queue only on the drain call *after* the last owed byte. The old
  pollInput made that call itself; a peek doesn't, so the stale one-past-end
  cursor read as "owed", the pushback rewound into stale bytes, and the decoder
  saw a stale `len` as mid-sequence.

## Goal

- The three regression tests pass and the two contrast tests still pass.
- `examples/bugs/tui`: Esc pauses and resumes, and `q` while paused quits.

### Non-goals (must NOT change)

- Programs that never mention the mouse must keep byte-identical code. The pump
  and verify code stay gated on `mouse_state_offset` / `Some(MousePump)`.
- Mouse reports that arrive whole (the normal case) must still be swallowed
  and enqueued. The escape delay applies only while a prefix is held, and the
  wait is bounded.
- Don't "fix" this by turning the decoder off for a lone ESC or by making
  pollInput report TRUE for any held prefix without waiting. Either one splits
  a real report whose tail is in flight and prints `[<0;…M` garbage.

## Blast Radius

Found with `grep -rn "emit_decode_byte\|emit_drain_pending\|emit_pushback_byte" src`:

- `func_poll_input.rs:emit_mouse_ready_verify`: fixed here (1, 2).
- `gen_read_family.rs:emit_pump_prologue/emit_pump_epilogue`: shared by
  `readChar`, `readByte` and the `readLine` family. Fixed here (1, 3).
- `term/core/term.rs` (`MFB_MOUSE_INJECT` walk): unaffected. It is a test
  hook that feeds a whole NUL-terminated string; a trailing prefix there is a
  malformed injection, not a keypress.
- `term::pollMouse`: unaffected. It only reads the ring, never stdin.

## Fix Design

1. **pollInput verify**
   - Peek `DRAIN_POS` (non-zero means a byte is owed, so report ready) instead
     of draining it.
   - On a PASS where the flush already started a drain, rewind the cursor to 1
     so `buf[0]` is replayed first and the tail follows. Otherwise, push back
     as before.
   - When the byte was BUFFERED, recheck with the escape delay rather than 0.
   - When the recheck finds nothing and a prefix is held with nothing owed,
     set `DRAIN_POS = 1` (the prefix becomes owed bytes) and report TRUE.
2. **Read pump**
   - Before a fresh read, if a prefix is held and nothing is owed, poll stdin
     (the broadcast log first in console mode, then fd 0) for up to the escape
     delay. Retry on EINTR.
   - On timeout, set `DRAIN_POS = 1` and go back to the drain.
   - After a read that returned 0 (EOF) with a prefix held, do the same, so
     the prefix is delivered and the next read reports the EOF again.
3. **Escape delay:** one constant, 25 ms, next to the decoder.

Rejected: a timestamp on the held prefix, checked on the next call. It needs
another state word, and a blocking `readChar` would still hang because nothing
calls back in.

Expected golden shift: only the `.ncode`/`.nir` of fixtures that use the mouse
(for example `app-mouse-surface`). Nothing else.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Three regression tests added; all three fail as documented above.
- [x] Blast-radius audit done.

Acceptance: the tests fail for the documented reasons.
Commit: —

### Phase 2 — the fix

- [x] `func_poll_input.rs`: owed-byte peek at entry (4) and in verify (2),
      escape-delay recheck and release when quiet (1).
- [x] `decode.rs`: `ESCAPE_DELAY_MS`, `emit_branch_unless_prefix_held`,
      `emit_release_prefix`, `emit_branch_if_owed`, pushback rewinds after a
      flush (2), drain resets eagerly (5).
- [x] `gen_read_family.rs`: `emit_pump_escape_wait` before a fresh read (1),
      EOF release in the epilogue (3).
- [x] `stdin_broadcast.rs`: `emit_stdin_poll_ready_check_with` + `PollReadyRegs`,
      so the pump's wait uses caller-minted temporaries (pollInput keeps
      `PollReadyRegs::FIXED`, byte-identical).
- [x] `_poll`/`poll` imported by the four mouse members on macOS/Linux; Windows
      `io::` reads already import the wait's primitives.
- [x] Added a regression assertion for (4): `\x1b[Z` held open reaches a
      pollInput loop whole. Verified red with the entry check removed.

Acceptance: `cargo test --release --test rt_native_term_runtime`: 19 passed.
Commit: — (uncommitted, pending the user's go-ahead)

### Phase 3 — regenerate expected outputs + full validation

- [x] `artifact-gate.sh target/release/mfb all`: 2080 goldens, 4 diffs, all in
      `syntax/app/app-mouse-surface` (the only mouse fixture). The `.nplan` diff
      is exactly the four new `_poll` imports; the fixture reads no keys, so its
      code moved only through the import table. The Windows golden didn't move
      (no import change). Regenerated with `regen-native-goldens.sh`.
- [ ] Full suite (`cargo test --release --no-fail-fast`).
- [x] Cross-target execution of the probe (burst / lone ESC held open / `ESC[Z`
      held open / blocking readChar on a lone ESC): macOS aarch64, Linux
      aarch64 glibc (2223), Linux x86_64 musl (2227), Windows x86_64 (2230).
      All correct. A blocking read returns a lone ESC in 30 ms (Linux) and
      124 ms (Windows) with stdin still open.
- [x] `examples/bugs/tui` through a pty: Esc pauses and timers freeze, an arrow
      key while paused stays paused, Esc resumes, `q` quits from pause, and
      Esc quits from game over.
- [x] Spec `app/04_term-backend.md` and the `term::enableMouse` man page
      document the escape delay; `man-census --memory-scope` 0 unclassified;
      `spec-census --citations` 0 missing.

Commit: —

## Summary

The risk is in the pump's wait, which is a readiness poll emitted inside every
mouse program's read helpers on three platforms. The decoder grammar, the ring
and non-mouse programs are untouched.
