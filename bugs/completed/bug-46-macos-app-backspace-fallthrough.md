# bug-46: macOS app-mode transcript keyDown handler — line-echo Backspace falls through into the raw-input path and injects the DEL/BS byte into the input pipe

Last updated: 2026-07-09
Effort: small (<1h)

In the macOS AppKit app-mode transcript keyDown handler
(`emit_key_down_helper`), the `kd_backspace` block deletes the last character from
the input buffer and the transcript, but has **no terminating `branch("kd_done")`**.
Execution falls straight into the `kd_raw` label that immediately follows, which
fetches the pipe write fd and `write()`s the backspace key event's own UTF-8 bytes
(`0x7f`/`0x08`) into the input pipe. When the line is later committed, `io::input` /
`io::readLine` returns a line with a stray DEL/BS byte embedded ahead of the visible
text.

The single correct behavior a fix produces: pressing Backspace in a line-input field
erases a character and injects **nothing** into the input pipe — exactly as the
structurally identical TUI handler already does.

References:

- `src/target/macos_aarch64/app/term_view.rs:139-163` (`emit_key_down_helper`,
  `kd_backspace` block: deletes from buffer `x25` and transcript storage `x22`, ends
  at the second `deleteCharactersInRange:` msgSend with **no branch**).
- `src/target/macos_aarch64/app/term_view.rs:167` (`kd_raw` label — the fallthrough
  target; writes `[chars UTF8String]` to the pipe).
- Correct sibling: `src/target/macos_aarch64/app/term_view.rs:1164-1176`
  (`emit_term_key_down_helper`, `tkd_backspace` ends with `abi::branch("tkd_done")`).
- The other completed paths in the same function terminate correctly: default at
  `:97`, commit at `:136`, both `branch("kd_done")`.
- Found during the goal-01 compiler source review of `src/target/macos_aarch64/app/`.

## Failing Reproduction

Build any app-mode program that reads a line, run it on macOS, type into the
transcript, and press Backspace mid-line:

```
IMPORT io
SUB main()
  LET s AS String = io::input("> ")
  io::print("[" + s + "]")
END SUB
```

Type `ab`, press Backspace, type `c`, press Enter.

- Observed: the committed line contains a stray `0x7f` (or `0x08`) byte before the
  visible text — e.g. `io::input` returns `"a\x7fc"` rather than `"ac"`. The transcript
  looks correct (the character was visually erased), so the corruption is silent.
- Expected: `io::input` returns `"ac"`.

Contrast (works today): the TUI-surface handler (`emit_term_key_down_helper`) handles
Backspace correctly because `tkd_backspace` ends with `branch("tkd_done")`. Raw-mode
(mode 2) Backspace in the transcript handler is also fine — it *intends* to reach
`kd_raw` and write the key once. The bug is specific to **line-echo mode
(`INPUT_MODE_LINE_ECHO = 1`) Backspace/Delete** in the transcript keyDown handler.

## Root Cause

`emit_key_down_helper` lays out its dispatch so every completed branch ends with
`branch("kd_done")`. The `kd_backspace` block (`term_view.rs:139-163`) omits that
terminating branch, so after the transcript-storage `deleteCharactersInRange:` msgSend
at `:163`, control falls into `label("kd_raw")` at `:167`. In line-echo mode the
handler's top-of-function raw check (`:74-75`) does not divert, so `kd_raw` should only
be reachable in genuine raw mode; the missing branch defeats that invariant. `kd_raw`
then reads the still-live key event string in `x21`, takes its UTF-8 bytes, and writes
them to the pipe fd — appending the control byte to the pending input.

## Goal

- Backspace/Delete in line-echo mode erases one character and writes nothing to the
  input pipe.
- `io::input` after editing returns exactly the visible text.

### Non-goals (must NOT change)

- Raw-mode (mode 2) key handling, which correctly reaches `kd_raw`.
- The commit and default paths, and the TUI handler.

## Blast Radius

- `emit_key_down_helper` `kd_backspace` block — fixed here.
- `emit_term_key_down_helper` `tkd_backspace` — already correct; the template for the fix.
- No other handler falls through: default/commit both branch to `kd_done`.

## Fix Design

Insert `asm.push(abi::branch("kd_done"));` immediately after the transcript-storage
`deleteCharactersInRange:` msgSend (after `term_view.rs:163`), mirroring
`tkd_backspace:1176`. One instruction.

## Phases

### Phase 1 — failing test

- [x] Add a keyDown-handler emitted-instruction harness that emits
      `emit_key_down_helper()` and asserts the instruction immediately before the
      `kd_raw` label is an unconditional `b kd_done` (i.e. `kd_backspace` cannot
      fall through into `kd_raw`). Confirmed fails today (the instruction before
      `kd_raw` was the transcript-delete `bl _objc_msgSend`, a `BranchLink`).
      A full app-mode runtime input test is not achievable: the transcript
      keyDown/pipe path runs only in the GUI window mode (`MFB_MACAPP_GUI=1`,
      idle machine + Accessibility), and the headless input case uses real
      stdin, bypassing `emit_key_down_helper` entirely.

### Phase 2 — the fix

- [x] Add the `branch("kd_done")` at the end of `kd_backspace`.

### Phase 3 — validation

- [ ] Regenerate macOS app codegen goldens (delta = one branch in the keyDown
      helper). Orchestrator-owned; goldens WILL shift for every macOS app-mode
      binary (one added `b kd_done` plus downstream offset shifts within the
      keyDown helper).
- [x] `scripts/test-macapp.sh target/debug/mfb` (non-GUI, headless) passes;
      app-mode build/codegen unaffected. The interactive backspace reproduction
      in a built `.app` is GUI-only and not automatable in this environment.
      `scripts/test-accept.sh` is orchestrator-owned (not run here).

## Validation Plan

- Regression test(s): the backspace-in-input test above.
- Runtime proof: interactively backspace in a built macOS app and confirm `io::input`
  returns clean text.
- Doc sync: none expected.
- Full suite: `scripts/test-accept.sh`.

## Summary

A single missing `branch("kd_done")` makes line-mode Backspace leak the DEL/BS byte
into the input pipe on macOS. The fix is one instruction, copied from the TUI
handler's correct sibling; only line-echo transcript input is affected.

## Resolution

Fixed in `src/target/macos_aarch64/app/term_view.rs`. Inserted
`asm.push(abi::branch("kd_done"));` immediately after the transcript-storage
`deleteCharactersInRange:` `bl _objc_msgSend` in the `kd_backspace` block (right
before the `kd_raw` label), mirroring `tkd_backspace`'s terminating
`branch("tkd_done")`. This terminates the line-echo Backspace/Delete path so it no
longer falls through into `kd_raw`, which would otherwise write the Backspace key's
own UTF-8 byte (`0x7f`/`0x08`) into the input pipe. Register lifetimes are
unaffected — the added instruction is a pure control-flow terminator that holds no
live value across a call.

Regression tests (same file, `#[cfg(test)] mod tests`):
`kd_backspace_does_not_fall_through_into_kd_raw` asserts the instruction directly
before the `kd_raw` label is an unconditional `b kd_done`; a sibling anchor test
`tkd_backspace_does_not_fall_through_into_tkd_raw` pins the already-correct TUI
handler. Fail-before was proven by temporarily removing the branch (the guarded
assertion reported `BranchLink` instead of `Branch`); both pass after the fix.

Validation: `cargo test --bin mfb kd_backspace` (2 passed).
`scripts/test-macapp.sh target/debug/mfb` passes (headless app-mode build +
runtime unaffected; GUI keyDown/backspace path is not automatable here).
Goldens: macOS app-mode codegen goldens will shift (one added branch + downstream
offset shifts within the keyDown helper); regeneration + `scripts/test-accept.sh`
are orchestrator-owned.
