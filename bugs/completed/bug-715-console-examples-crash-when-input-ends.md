# bug-715: console examples exit 255 with a raw error code when stdin ends early

STATUS: FIXED (cab39baef)

Last updated: 2026-09-26
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Closed
Regression Test: `tools/example-eof-probe/probe.py` — piped-stdin runs of each
affected example asserting exit status and stderr text.

Four examples call `io::input`/`io::readChar` without a `TRAP`, at least
once, on a path reachable during normal use (a name prompt, a menu choice, a
"press any key"). When stdin ends before that read is satisfied — a script
piping input that runs out, `</dev/null`, a terminal killed mid-session —
`io::readChar`/`io::input` raise the uncaught runtime error `ErrEndOfFile`
(`7-702-0003`), which every one of them lets propagate all the way out:
the process exits 255 and prints `Error: 7-702-0003` /
`Read operation reached end of file where a value was required.` to
stdout/stderr with no example-specific message, and — for the two that run
in the terminal's alternate screen (`wide-demo`, `hangman`) — that raw error
text is written *before* the alternate screen is torn down
(`[?1049l`), so on a real terminal the user never sees it: it scrolls by
inside the alternate screen and is gone the instant the screen restores.

**Correct behavior:** a clean, example-specific message and a defined exit
status (not the compiler's raw error-code text), with the terminal restored
to its normal mode *before* anything is printed — matching the pattern
`examples/snake` and `examples/life` already use (an `isInputTerminal`/
`isOutputTerminal` guard up front, and a function-level `TRAP` around `main`
that restores the terminal before reporting the error).

References:

- `src/docs/spec/diagnostics/02_error-codes.md` — `7-702-0003` /
  `ErrEndOfFile`: "Read operation reached end of file where a value was
  required."
- `examples/snake/src/main.mfb:main`, `examples/life/src/main.mfb:main` —
  the correct contrast pattern (see "Root Cause" and "Blast Radius").
- bug-699 (`bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md`)
  — three of the four affected examples (`hello_input`, `wide-demo`,
  `hangman`) have their `io::readChar()` call sites scheduled for migration
  to `io::readKey`/`io::readScalar` in that bug's Phase 6. See "Interaction
  with bug-699" below.

## Failing Reproduction

All four run today at HEAD (`7609b46fa`), rebuilt fresh into `/tmp`.

**`hello_input`** (`examples/hello_input/src/main.mfb:5,11` — `io::input` at
line 5, `io::readChar()` at line 11, neither `TRAP`ped):

```
$ mfb build /tmp/bug715/hello_input
Building hello_input (executable) for macos-aarch64
Wrote executable to /tmp/bug715/hello_input/build/hello_input.out
$ printf 'Bob\nw' | /tmp/bug715/hello_input/build/hello_input.out; echo "EXIT=$?"
What is your name? Hello Bob!
Press W/A/S/D to move, or Q to quit.
↑
Error: 7-702-0003
Read operation reached end of file where a value was required.
EXIT=255
```

**`wide-demo`** (`examples/wide-demo/src/main.mfb:76` —
`LET key AS String = io::readChar()`, not `TRAP`ped, called after
`term::on()` puts the terminal in the alternate screen):

```
$ mfb build /tmp/bug715/wide-demo
Building wide-demo (executable) for macos-aarch64
Wrote executable to /tmp/bug715/wide-demo/build/wide-demo.out
$ /tmp/bug715/wide-demo/build/wide-demo.out < /dev/null > /tmp/bug715/wide_out.txt 2>&1; echo "EXIT=$?"
EXIT=255
$ cat -v /tmp/bug715/wide_out.txt | tail -c 400
...^[[13;43H^[[?25lError: 7-702-0003
Read operation reached end of file where a value was required.
^[[6;13H^[[38;2;200;200;255m^[[48;2;0;0;0mé^[[7;10H👨‍👩‍👧‍👦^[[0m^[[1;1H^[[?25h^[[?25h^[[?1049l^[[0m
```

**`hangman`** (`examples/hangman/src/main.mfb:134,164` —
`strings::upper(io::readChar())`, not `TRAP`ped, inside the alternate
screen):

```
$ mfb build /tmp/bug715/hangman
Building hangman (executable) for macos-aarch64
Wrote executable to /tmp/bug715/hangman/build/hangman.out
$ /tmp/bug715/hangman/build/hangman.out < /dev/null > /tmp/bug715/hangman_out.txt 2>&1; echo "EXIT=$?"
EXIT=255
$ cat -v /tmp/bug715/hangman_out.txt | tail -c 200
...^[[13;43H^[[?25lError: 7-702-0003
Read operation reached end of file where a value was required.
^[[0m^[[13;43H^[[?25l^[[?25h^[[?1049l^[[0m
```

**`audio`** (`examples/audio/src/main.mfb:57,84` — `io::input("Device >")`
and `io::input("Choose> ")`, neither `TRAP`ped; builds cleanly against the
vendored `libsnd` package, no separate installation was needed this
session):

```
$ mfb build /tmp/bug715/audio
uses libsnd - [Verified]
Building audio (executable) for macos-aarch64
Wrote executable to /tmp/bug715/audio/build/audio.out
$ /tmp/bug715/audio/build/audio.out < /dev/null > /tmp/bug715/audio_out.txt 2>&1; echo "EXIT=$?"
EXIT=255
$ cat /tmp/bug715/audio_out.txt
Select the output device: 
  0) Justin's AirPods
  1) MacBook Pro Speakers
Device >Error: 7-702-0003
Read operation reached end of file where a value was required.
```

- Observed (all four): exit status 255, stdout/stderr shows the compiler's
  raw `Error: 7-702-0003` / `ErrEndOfFile` message text, not an
  example-authored message.
- Observed (`wide-demo`, `hangman` only — the two alternate-screen apps):
  the raw error text is emitted *before* the final `^[[?1049l` (exit
  alternate screen) sequence, interleaved with more alternate-screen draw
  commands that follow it (visible in the raw byte dumps above — the error
  text is followed by more `^[[...H` cursor moves and glyph writes, and only
  then `^[[?1049l`). On a real terminal, the alternate screen is still
  active when the error prints, so the message is overwritten by the
  subsequent draws and erased the instant `?1049l` restores the normal
  screen — the user sees nothing.
- Expected: a clean, example-specific message on the restored, normal
  screen, and a defined (non-255, or an example-documented) exit status.

**Contrast, correct today** — `examples/snake` and `examples/life`, run by
reading the source (not re-built this session; the pattern is what matters):
both guard entry with

```basic
IF NOT io::isOutputTerminal() OR NOT io::isInputTerminal() THEN
  io::printError("snake: run this from an interactive terminal.")
  RETURN 1
END IF
```

(`examples/snake/src/main.mfb:197`, `examples/life/src/main.mfb:263`) and
wrap the entire `main` body in a function-level `TRAP`:

```basic
FUNC main AS Integer
  ...
  term::showCursor()
  term::off()
  io::print("Thanks for playing Snake!")
  RETURN 0

  TRAP(e)
    term::showCursor()
    term::off()
    io::printError("snake: " & e.message)
    RETURN 1
  END TRAP
END FUNC
```

(`examples/snake/src/main.mfb:308-313`; `examples/life/src/main.mfb` has the
equivalent at lines ~355-360). Any uncaught error — including
`ErrEndOfFile` from `getChar`/`readKey` inside the loop, since those already
wrap `io::readChar()`/`io::pollInput()` in their own `TRAP` and turn a
failure into `""`/`FALSE` rather than propagating — is caught once at the
top, the terminal is restored (`term::showCursor()`, `term::off()`) *before*
anything is printed, and the message goes to the normal screen via
`io::printError`, with a defined exit code (`1`).

| Example | Read call | TRAP present | Alternate screen | Exit code | Error visible to user |
| --- | --- | --- | --- | --- | --- |
| `hello_input` | `io::input`, `io::readChar` | no | no | 255 | yes (raw code) |
| `wide-demo` | `io::readChar` | no | yes (`term::on`) | 255 | no (erased by `?1049l`) |
| `hangman` | `io::readChar` ×3 | no | yes (`term::on`) | 255 | no (erased by `?1049l`) |
| `audio` | `io::input` ×2+ | no | no | 255 | yes (raw code) |
| `snake` (contrast) | `io::readChar` (via `getChar`, itself `TRAP`ped) | yes, function-level | yes | 1 | yes, clean, after restore |
| `life` (contrast) | `io::readChar` (via `getChar`, itself `TRAP`ped) | yes, function-level | yes | 1 | yes, clean, after restore |

## Root Cause

`io::input`/`io::readChar` raise `ErrEndOfFile` (`7-702-0003`,
`src/docs/spec/diagnostics/02_error-codes.md`) when stdin has no more bytes
to satisfy the read. `hello_input`, `wide-demo`, `hangman`, and `audio` each
call one of these directly, at least once, with no `TRAP` anywhere on the
call or in an enclosing function:

- `examples/hello_input/src/main.mfb:5` (`io::input`) and `:11`
  (`io::readChar()`, inside the `DO...LOOP UNTIL`).
- `examples/wide-demo/src/main.mfb:76` (`io::readChar()`, after
  `term::on()`/`term::clear()` at lines 27-28 have already entered the
  alternate screen).
- `examples/hangman/src/main.mfb:134` and `:164` (`io::readChar()`, also
  inside the alternate screen — `drawScreen` uses `term::` drawing
  throughout).
- `examples/audio/src/main.mfb:57` and `:84` (`io::input`, plain console
  mode, no alternate screen).

With no `TRAP`, the runtime's default behavior for an uncaught error is to
print `Error: <code>` / the error's message and exit 255 — exactly what
every reproduction above shows. `hello_input` and `audio` are plain console
programs, so the message at least reaches the user (garbled with the raw
code, but visible). `wide-demo` and `hangman` are worse: `term::on()`
switches the terminal into the alternate screen (`?1049h`) before the
unguarded read, and the runtime's default error print happens *before* the
exit path's own terminal-restore sequence runs (`?1049l` appears only at the
very end of each captured byte stream, after the error text and after
further alternate-screen draws) — so on a real terminal the error is
written into a screen buffer that is about to be discarded, and the user's
terminal simply returns to their shell prompt with no visible explanation.

`snake` and `life` avoid all of this with two mechanisms working together:
an upfront `isInputTerminal`/`isOutputTerminal` guard (so a non-interactive
invocation gets a clean message before the alternate screen is ever
entered), and a function-level `TRAP` around the whole of `main` that
restores the terminal (`term::showCursor()`, `term::off()`) before calling
`io::printError`, so *any* uncaught error — not just `ErrEndOfFile` —
prints to the normal screen with the terminal already sane, and exits `1`.

## Goal

- `hello_input`, `wide-demo`, `hangman`, and `audio` each report a clean,
  example-specific message when stdin ends, and exit with a defined,
  documented status (recommend `1`, matching `snake`/`life`'s convention;
  the exact code is the implementer's call, but it must not be the raw
  255/`Error: <code>` default).
- `wide-demo` and `hangman` restore the terminal (`term::off()` /
  equivalent) *before* printing that message, so it is visible on the
  restored, normal screen — matching `snake`/`life`.
- `audio` and `hello_input` (already plain console) simply need the message
  to be example-authored instead of the raw error text; no alternate-screen
  ordering issue applies to them.

### Non-goals (must NOT change)

- Any behavior of the four examples on their happy path (input available,
  no EOF) — this bug is scoped to what happens when input runs out.
- `snake`/`life`'s existing, already-correct handling — cited only as the
  pattern to copy, not touched by this bug.
- **Tempting wrong fix, forbidden:** wrapping only the specific `io::input`/
  `io::readChar` calls in a local `TRAP` that swallows `ErrEndOfFile` into a
  default value (e.g. `""`) without restoring the terminal or reporting
  anything — this would silently continue the program with garbage input
  (an empty name, a phantom keypress) instead of exiting cleanly, which is
  worse than today's crash for `wide-demo`/`hangman` since it hides the
  problem instead of surfacing it after restoring the terminal.

## Blast Radius

`grep -rlE "io::(input|readChar|readLine|readByte)\(" examples --include='*.mfb'`
(2026-09-25) found 14 files; each checked for whether its read call (or an
enclosing function) is `TRAP`ped:

- **Unguarded — fixed by this bug:** `examples/hello_input/src/main.mfb`,
  `examples/wide-demo/src/main.mfb`, `examples/hangman/src/main.mfb`,
  `examples/audio/src/main.mfb` (all four reproduced above).
- **Already guarded — unaffected:** `examples/snake/src/main.mfb`,
  `examples/life/src/main.mfb` (function-level `TRAP` + terminal-restore,
  the contrast pattern above); `examples/ai_chat/src/main.mfb`,
  `examples/browser/app/src/main.mfb`, `examples/swat/tui/src/main.mfb`,
  `examples/swat/canvas/src/main.mfb` (each has a `getChar` wrapping
  `io::readChar() TRAP(e) ... RECOVER "" END TRAP`, so `ErrEndOfFile` is
  caught at the point of the read and turned into `""`, which each
  program's own key-decoding logic already treats as "no more input" — not
  observed to crash); `examples/water/src/main.mfb`,
  `examples/wind/src/main.mfb` (their `nextChar` explicitly `TRAP`s and
  returns `""` for `ErrEndOfFile`, per bug-713's References); `examples/dungeon/src/main.mfb`
  (also `TRAP`ped in `getChar`, though bug-713 shows that same `TRAP`
  over-broadly conflates `ErrEncoding` with EOF — a separate, already-filed
  defect, not a crash-on-EOF one); `examples/emoji/src/main.mfb` (`readKey`
  wraps `io::readChar()` in its own `TRAP` returning `""`).

No file outside these 14 calls `io::input`/`io::readChar`/`io::readLine`/
`io::readByte` at all (confirmed by the grep above being exhaustive over
`examples/**/*.mfb`).

## Interaction with bug-699

bug-699 lists `hello_input`, `wide-demo`, and `hangman` in its own Blast
Radius table as three of the thirteen examples whose `io::readChar()` call
sites migrate — to `io::readScalar`/`io::readGrapheme` for these three,
since none of them decode arrow-key escape sequences — when that bug's
Phase 6 lands. `audio` is not in bug-699's scope at all (it only calls
`io::input`, which bug-699 keeps, only adding an optional `[timeoutMs]`
parameter to it — bug-699's Decision 1 table does not change `io::input`'s
EOF behavior, only its timeout contract).

These are different defects with only partial file overlap, and fixing this
bug first does not conflict with bug-699:

- The fix here is about **catching `ErrEndOfFile` and restoring the
  terminal**, not about *which reader function* is called or *how a key is
  encoded* — bug-699 changes the latter, not the former. A `TRAP` added
  around `io::readChar()` today continues to make sense unchanged after
  bug-699's Phase 6 replaces the call with `io::readScalar`/`io::readKey`
  (both readers raise the same class of "the input is gone" condition —
  `ErrTimeout` under a timeout, or, per bug-699 Decision 1's table, an
  omitted timeout blocks the way `io::readChar` does today, so the same
  `TRAP`-and-restore shape still applies).
- `audio` is entirely outside bug-699's scope, so its fix here is
  independent regardless of order.

**Recommendation: fix this bug now.** It is a small, isolated fix (add a
`TRAP` and a restore/report path to four `main` functions) that improves
real, easily-triggered behavior (any piped or redirected input crashes
today) and does not need to wait on bug-699's `huge`, not-yet-started
migration. When bug-699 Phase 6 rewrites `hello_input`/`wide-demo`/
`hangman`'s read calls, it should preserve the clean-exit-on-EOF behavior
this bug establishes — call this out explicitly in bug-699's Phase 6
checklist when that work starts (not required to edit bug-699's document as
part of this bug; noted here for whoever picks up Phase 6).

## Fix Design

For each of the four examples, add a function-level `TRAP` around `main`
(or, for `hello_input`/`audio`'s simpler structure, around the smallest
enclosing scope that covers every unguarded read) that:

1. For `wide-demo`/`hangman`: restores the terminal first (`term::off()`,
   matching what the happy path already does at the end of `main`), *then*
2. Reports a clean, example-specific message via `io::printError` (not
   `io::print`, matching `snake`'s convention of routing errors to stderr),
   distinguishing `ErrEndOfFile` ("no more input") from any other uncaught
   error if the example wants a more specific message, or reporting `e.message`
   generically (matching `snake`'s `"snake: " & e.message`) — implementer's
   call, consistent within this fix,
3. Returns a defined, non-255 exit status (recommend `1`).

`hello_input` and `audio` need no alternate-screen restore step (they never
call `term::on()`), only steps 2 and 3.

Rejected alternative: add the upfront `isInputTerminal`/`isOutputTerminal`
guard `snake`/`life` use, in addition to the `TRAP`. Considered but not
required by this bug's scope — that guard rejects *starting* a
non-interactive run outright (a stricter policy choice: "this program only
makes sense on a live terminal"), whereas `hello_input` and `audio`'s
existing design accepts piped input as long as it lasts (the reproduction
above shows `hello_input` working correctly through `Bob` and `w` before
the pipe runs dry) — changing that policy is a bigger, separate design
choice than "don't crash when input runs out," so it is left as an Open
Decision rather than folded into the required fix.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Commit piped-stdin probes for all four examples (recommended:
      `tools/example-eof-probe/` per the `scripts/`-vs-`tools/` split in
      `AGENTS.md`, since this is a reusable check across every example with
      a read call, not a one-off) asserting: exit code is not 255, stderr
      does not contain the literal `Error: 7-702-0003`, and — for
      `wide-demo`/`hangman` — the terminal-restore sequence (`?1049l`)
      appears *before* the example's error message in the byte stream.
      Confirm all four probes fail today, reproduced above.
- [x] Complete the Blast Radius audit above with a verdict per of the 14
      files (done above).

Acceptance: all four new probes fail for the documented reason; the audit
has a verdict per file.
Commit: cab39baef

### Phase 2 — the fix

- [x] `examples/hello_input/src/main.mfb`, `examples/audio/src/main.mfb`:
      wrap `main` in a function-level `TRAP` reporting a clean message and
      returning a defined exit code.
- [x] `examples/wide-demo/src/main.mfb`, `examples/hangman/src/main.mfb`:
      same, plus restoring the terminal (`term::off()`) before reporting.

Acceptance: the Phase 1 probes pass for all four examples; each example's
happy path (full input available) is unchanged.
Commit: cab39baef

### Phase 3 — regenerate expected outputs + full validation

- [x] `bash scripts/build-examples.sh` exits 0.
- [x] Re-run the Phase 1 probes against the rebuilt examples; re-run each
      example's happy-path input by hand to confirm no regression.

Acceptance: full examples build passes; all four probes pass; happy paths
unchanged.
Commit: cab39baef

## Validation Plan

- Regression test: the Phase 1 probes, committed to `tools/example-eof-probe/`
  (or `tests/` if the project's convention differs — check
  `scripts/README.md` at Phase 1 time), covering exit code, absence of the
  raw error text, and (for the two alternate-screen apps) restore-before-
  report ordering.
- Runtime proof: the before/after piped-stdin runs shown above for
  "before"; the same commands re-run after the fix.
- Doc sync: none expected — this is example code, not a spec/man surface.
- Full suite: `bash scripts/build-examples.sh`.

## Open Decisions

- Whether `hello_input`/`audio` should also gain an upfront
  `isInputTerminal`/`isOutputTerminal` guard (rejecting non-interactive runs
  outright, `snake`/`life`'s stricter policy) or keep accepting piped input
  until it runs out (their current, more permissive design) — recommended:
  keep the current, more permissive design; only add the `TRAP`/clean-exit
  behavior this bug requires. Revisit if the owner wants uniform policy
  across all console examples.
- Exact exit code and message wording per example — recommended: `1` and
  `"<example>: " & e.message` (or an EOF-specific message), matching
  `snake`'s convention; left to the implementer to keep consistent across
  the four.

## Summary

Four examples let `io::input`/`io::readChar`'s `ErrEndOfFile` propagate
uncaught, so any piped, redirected, or prematurely-closed stdin crashes them
with the compiler's raw error text and exit 255; two of the four
(`wide-demo`, `hangman`) make it worse by printing that raw text while still
inside the terminal's alternate screen, so it never reaches the user at all.
`snake` and `life` already show the fix — a function-level `TRAP` that
restores the terminal before reporting — and it is a small, mechanical
change to apply to the remaining four. It is independent of bug-699's
larger keyboard-input redesign and should land first.
