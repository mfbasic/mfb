# bug-708: `examples/life` header comment claims keys need Return; they don't

STATUS: FIXED (f9c2d75f4)

Last updated: 2026-09-26
Effort: small (<1h)
Severity: HIGH
Class: Other

Status: Closed
Regression Test: `tools/pty-probe/life_quit_no_return.py` (landed under the
shared `tools/pty-probe/` tool rather than a new `tools/example-pty-smoke/`,
since bug-703 landed in the same session and needed the identical pty-driving
primitives — see Open Decisions).

`examples/life/src/main.mfb`'s header comment tells the developer reading the
example that the terminal "stays in cooked/line mode" and a key "takes effect
when you press it followed by Return." That is false: the very first thing
`main` does is call `term::on()`, which — per its own man page — puts the
terminal into single-key mode specifically so a keypress is seen immediately,
with no Return required. A developer copying this example's mental model into
their own program would defend against the wrong input model (buffering
line-based input, or expecting a trailing newline byte) and be surprised when
their program reacts to bare keystrokes.

References:

- `examples/life/src/main.mfb:9-30` — the header comment (verbatim quoted below).
- `mfb man term on` — documents step 4 of `term::on`, the mechanism that
  contradicts the header.
- `examples/wide-demo/src/main.mfb:74-76` — a sibling example with the correct
  claim, worded almost identically to what life's header should say.

## Failing Reproduction

Header comment, verbatim (`examples/life/src/main.mfb:21-22`):

```
' Controls (the console terminal stays in cooked/line mode, so a key takes effect
' when you press it followed by Return):
```

Build fresh from current `main` and drive it over a pty, sending a single `q`
keystroke with **no** trailing Return/newline:

```
mfb build examples/life
python3 /tmp/life_pty_test5.py   # pty driver: os.openpty via python's pty.fork(),
                                   # sets a 24x80 winsize, waits 1s, os.write(fd, b"q")
                                   # with no \n or \r, then waitpid()s with a 3s budget
```

- Observed: the child process is confirmed alive immediately before the send
  (`waitpid(..., WNOHANG)` returns `(0, 0)`); one byte `b"q"` is written with no
  Return; the process exits **normally with code 0 within ~0.01s** of the send
  (`os.WIFEXITED(status)` true, `os.WEXITSTATUS(status) == 0`), and the captured
  pty output ends with `Thanks for playing Life!` (the `main` function's normal
  quit-path message, `examples/life/src/main.mfb:352`). No Return, Enter, or
  newline byte was ever sent.
- Expected (per the header comment as written): the process should still be
  waiting for a trailing Return after `q`, i.e. it should NOT have exited within
  the 3-second wait budget.

The build used for this reproduction was fresh, not the stale top-level
`examples/life/life.out` (Jul 13) or `examples/life/build/life.out` (Sep 4)
already on disk — `mfb build examples/life` was re-run today (2026-09-25),
producing a new `examples/life/build/life.out` (828696 bytes), and that binary
is what was driven over the pty.

Contrast case that already documents the correct behavior:
`examples/wide-demo/src/main.mfb:74-76` — "term::on has already put the tty
into single-key mode, so one keypress returns without waiting for Return" —
same mechanism (`term::on`), opposite (correct) claim.

## Root Cause

`examples/life/src/main.mfb:21-22 (header comment)` asserts cooked/line-buffered
input. But `main` (`examples/life/src/main.mfb:259` area) calls `term::on()`
before entering the game loop, and per `mfb man term on`, step 4 of that call:

> Puts the terminal into single-key mode, so an `io::pollInput` + `io::readChar`
> loop sees each keypress as it happens instead of waiting for Return, and keys
> are not echoed as the user types.

The game loop then reads input via exactly that pattern —
`io::pollInput(frameDelayMs())` gating a call to `readKey()`
(`examples/life/src/main.mfb:323-325`), which wraps `io::readChar()`
(`examples/life/src/main.mfb:252-256`). `io::readChar`'s own man page confirms
it "does not wait for a newline." So every layer — the terminal mode `term::on`
configures, and the read call the loop uses — agrees the input is unbuffered,
single-keystroke. The header comment simply contradicts the code beneath it.

Likely origin: the comment reads like it was copied from, or never updated
after, an earlier revision of the example (or a different example) that used
line-buffered input before the example was switched to `term::on` +
`io::pollInput`/`io::readChar`. There is no code path in the current file that
produces cooked-mode behavior.

## Goal

- The header comment accurately describes the input model: keys take effect as
  soon as pressed, no Return needed — matching `wide-demo`'s wording for the
  same mechanism.

### Non-goals (must NOT change)

- The input-handling code itself (`term::on`, `io::pollInput`, `io::readChar`,
  `readKey`) — it is correct today; only the stale comment is wrong.
- Any other prose in the header (board/status-bar description, resize handling)
  — unaffected by this bug.

## Blast Radius

Grepped every example's `.mfb` sources and `README.md` files for "Return"/"Enter"
near "key"/"press" (`grep -rn -i "return" examples/*/src/*.mfb examples/*/README.md
examples/*/*/src/*.mfb | grep -i -E "key|press"` and the equivalent for "enter"):

- `examples/life/src/main.mfb:22` — fixed by this bug (the false claim).
- `examples/wide-demo/src/main.mfb:75` — same mechanism (`term::on` single-key
  mode), but states the **correct** claim ("one keypress returns without
  waiting for Return"); unaffected, and used above as the model for the fix.
- `examples/ai_chat/src/main.mfb:46,861,901,914,918` — "Enter" here names a
  literal `ENTER` keypress recognized by the app's own key decoder while a text
  input field has focus (submitting a typed prompt/command), not a claim that
  single-key commands need a trailing Return. Unaffected — different, and
  accurate, semantics.
- `examples/browser/app/src/main.mfb:58,238` — same pattern as ai_chat: "Enter"
  is the literal key that submits a typed URL in an address-entry field ("Press
  G to type an address, then Enter to load it"); other single-key commands
  (Q, R, L, M, arrows) are not claimed to need Return. Unaffected.
- `examples/browser/fetch/src/lib.mfb:217` — "No address entered. Press G and
  type a URL" is a runtime error message, unrelated to key-buffering claims.
  Unaffected.
- `examples/snake/src/main.mfb:178-181`, `examples/dungeon/src/map.mfb:236,291`,
  `examples/brogue/src/level.mfb:121`, `examples/ai_chat/src/main.mfb:830-979` —
  all `RETURN` statements (the language keyword), not the Return key. Unrelated,
  unaffected.
- `hangman`, `snake`, `dungeon`, `brogue`, `wind`, `water`, `hello_input` header
  comments (lines 1-40 of each `src/main.mfb`) — checked directly for
  cooked/line-mode or single-key-mode claims; none make any such claim at all,
  so none are affected either way.

Only `examples/life` carries the false claim; it is the sole fix site.

## Fix Design

Replace the two-line claim at `examples/life/src/main.mfb:21-22` with wording
matching the accurate mechanism, mirroring `wide-demo`'s phrasing:

```
' Controls (term::on puts the terminal into single-key mode, so a key takes
' effect the instant you press it — no Return needed):
```

No code changes; this is a one-comment documentation fix. The risk is
entirely in wording accuracy, not behavior.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tools/example-pty-smoke/life_quit_no_return.py` (or land the /tmp
      driver used here) asserting: build `examples/life`, send `q` with no
      Return over a pty, process exits 0 within ~1s. Confirm it demonstrates
      the current header comment is false (the process DOES exit without
      Return, contradicting what the comment claims).
- [x] Blast-radius audit above is complete (done in this document).

Acceptance: the smoke script's assertion (exit 0, no Return sent) passes today,
proving the header's claim is false right now.
Commit: f9c2d75f4

### Phase 2 — the fix

- [x] Rewrite `examples/life/src/main.mfb:21-22` per Fix Design above.

Acceptance: the header comment no longer claims cooked/line mode or a Return
requirement; wording matches the single-key-mode mechanism `term::on` actually
provides.
Commit: f9c2d75f4

### Phase 3 — regenerate expected outputs + full validation

- [x] Rebuild `examples/life` (`mfb build examples/life`) and re-run the Phase 1
      smoke script to confirm behavior (already correct) still matches the
      corrected comment.
- [x] No goldens or generated artifacts reference this comment's text, so no
      other regeneration is expected — confirm with a repo-wide grep for the
      old wording before closing.

Acceptance: smoke script passes; grep for the retired phrase
("cooked/line mode", "followed by Return") returns no hits under `examples/`.
Commit: f9c2d75f4

## Validation Plan

- Regression test: `tools/example-pty-smoke/life_quit_no_return.py` (Phase 1).
- Runtime proof: the same pty-driven repro used above, re-run after the fix —
  behavior is unchanged (it was already correct); only the comment moves to
  match it.
- Doc sync: the header comment itself is the doc being fixed; no other doc
  references this claim (grepped above).
- Full suite: none of the compiler's own test suites cover example prose, so
  there is no `cargo test` gate for this fix; the smoke script above is the
  only automated check that exists or is proposed.

## Open Decisions

- Whether to land the pty smoke script under `tools/example-pty-smoke/` now
  (recommended, since it also gives every other `term::`-based example a
  reusable input-behavior probe) or defer it and fix only the comment by
  inspection. **Resolved: landed it, but under the shared `tools/pty-probe/`
  tool** (per the fix-bug lane's shared instructions for this run — bug-703,
  fixed in the same session, needed the identical pty-fork-and-drive
  primitives, so both probes share one reviewed `pty_probe.py` instead of
  each rolling — or naming — their own).

## Summary

Pure documentation bug: the header comment for `examples/life` was stale
relative to the input-handling code beneath it (`term::on` + `io::pollInput`/
`io::readChar`), which has always been single-key, not line-buffered. Confirmed
today by a fresh build and a pty-driven repro (`q` with no Return exits the
process immediately, code 0). No other example repeats the false claim;
`wide-demo` already states the correct version of it. The engineering risk is
zero — this is a two-line comment edit — but severity is HIGH because the
comment actively teaches the wrong input model to anyone using this example as
a reference.
