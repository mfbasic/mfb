# bug-713: `examples/dungeon` quits on any non-UTF-8 input byte

Last updated: 2026-09-25
Effort: small (<1h)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: a console-mode probe reusing dungeon's `getChar`/`ready`/
`readKey` verbatim (see "Validation Plan"); dungeon itself has no test
harness (it is a `--app` canvas program)

`examples/dungeon` is a canvas app that reads keys through three small
functions in `examples/dungeon/src/main.mfb`: `getChar` (wraps
`io::readChar()`), `ready` (wraps `io::pollInput()`), and `readKey` (turns a
sequence of bytes into a logical key or `"EOF"`). `getChar` catches **every**
error `io::readChar()` can raise — including `ErrEncoding`, which
`io::readChar()` raises when the next input byte or bytes are not valid
UTF-8 — and turns all of them into `""`. `readKey` treats an empty string
from `getChar` as "the input source is gone" and returns `"EOF"`, which
`main.mfb`'s `commandFor` maps to `Quit[]`. The result: a single malformed
byte on stdin (a stray non-UTF-8 byte from a misbehaving terminal, a paste
of binary data, anything that isn't a clean UTF-8 stream) quits the game,
identically to the window closing or stdin actually ending.

**Correct behavior:** only a genuine end of input (`ErrEndOfFile`) should be
treated as `"EOF"`/quit. An encoding error is a bad byte, not an end of
input, and should be handled the way the sibling examples `water` and `wind`
already handle it — dropped or substituted, not conflated with quitting.

References:

- `examples/water/src/main.mfb:nextChar` and `examples/wind/src/main.mfb:nextChar`
  — the correct contrast: both explicitly `TRAP` `io::readChar()`,
  distinguish `errorCode::ErrEndOfFile` (return `""`, meaning genuinely done)
  from `errorCode::ErrEncoding` (return the replacement character
  `"\u{FFFD}"`, meaning "bad byte, keep going"), and `PROPAGATE` anything
  else. Neither conflates the two.
- `mfb spec diagnostics error-codes` (`src/docs/spec/diagnostics/02_error-codes.md`):
  `7-702-0004` / `ErrEncoding` "Text encoding or decoding failed." is a
  distinct code from `7-702-0003` / `ErrEndOfFile` "Read operation reached
  end of file where a value was required."
- bug-699 (`bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md`):
  dungeon's entire `getChar`/`ready`/`readKey` mechanism is one of the eight
  hand-rolled Esc decoders scheduled for replacement by `io::readKey` in that
  bug's Phase 6. See "Interaction with bug-699" below.

## Failing Reproduction

`examples/dungeon` is a `--app` canvas program, so its own binary cannot be
driven by piped stdin in a way that exercises this key path headlessly this
session. `getChar`/`ready`/`readKey` (`examples/dungeon/src/main.mfb:28-56`)
do not touch canvas at all, so they were copied verbatim into a `/tmp`
console-mode driver to prove the defect without needing a window:

```basic
' /tmp/bug713/src/main.mfb — getChar/ready/readKey copied verbatim from
' examples/dungeon/src/main.mfb (lines 28-56)
IMPORT io
IMPORT errorCode

FUNC getChar AS String
  LET c AS String = io::readChar() TRAP(e)
    RECOVER ""
  END TRAP
  RETURN c
END FUNC

FUNC ready(ms AS Integer) AS Boolean
  LET r AS Boolean = io::pollInput(ms) TRAP(e)
    RECOVER FALSE
  END TRAP
  RETURN r
END FUNC

FUNC readKey AS String
  LET c AS String = getChar()
  IF c = "" THEN RETURN "EOF"
  IF c <> "\u{1B}" THEN RETURN c
  IF NOT ready(30) THEN RETURN "ESC"
  DO WHILE ready(0)
    LET rest AS String = getChar()
    IF rest = "" THEN RETURN "EOF"
  LOOP
  RETURN ""
END FUNC

FUNC main AS Integer
  MUT key AS String = readKey()
  io::print("readKey() returned: [" & key & "]")
  IF key = "EOF" THEN
    io::print("VERDICT: treated the 0xFF byte as end-of-input (quit)")
  ELSE
    io::print("VERDICT: byte was handled without quitting")
  END IF
  RETURN 0
END FUNC
```

```
$ mfb build /tmp/bug713
Building bug713-driver (executable) for macos-aarch64
Wrote executable to /tmp/bug713/build/bug713-driver.out
$ printf '\xffabc' | /tmp/bug713/build/bug713-driver.out
readKey() returned: [EOF]
VERDICT: treated the 0xFF byte as end-of-input (quit)
```

- Observed: `readKey()` returns `"EOF"` for a single stray `0xFF` byte
  followed by three perfectly good ASCII bytes (`abc`) still sitting unread
  on stdin. In `examples/dungeon/src/main.mfb`'s `commandFor`
  (`CASE "ESC", "EOF" : RETURN Quit[]`), this quits the game.
- Expected: the malformed byte is skipped and the next real key (`a`) is
  read normally; the game is unaffected by one bad byte.

Confirming the byte really is `ErrEncoding`, not `ErrEndOfFile`, isolating
the exact mechanism:

```basic
FUNC main AS Integer
  LET c AS String = io::readChar() TRAP(e)
    io::print("code=" & toString(e.code) & " message=" & e.message)
    io::print("ErrEncoding? " & toString(e.code = errorCode::ErrEncoding))
    io::print("ErrEndOfFile? " & toString(e.code = errorCode::ErrEndOfFile))
    RETURN 0
  END TRAP
  io::print("no error, got: [" & c & "]")
  RETURN 0
END FUNC
```

```
$ printf '\xffabc' | /tmp/bug713/build/bug713-driver.out
code=77020004 message=Text encoding or decoding failed.
ErrEncoding? TRUE
ErrEndOfFile? FALSE
```

`io::readChar()` correctly reports `ErrEncoding` (`7-702-0004`), a code
distinct from `ErrEndOfFile` (`7-702-0003`) — `getChar`'s blanket
`RECOVER ""` is what erases that distinction.

Contrast (works correctly today, same session, not re-run as a full example
build but confirmed by reading the source): `examples/water/src/main.mfb:nextChar`
and `examples/wind/src/main.mfb:nextChar` both `TRAP` `io::readChar()` and
handle `ErrEncoding` and `ErrEndOfFile` as separate cases (see References).

## Root Cause

`examples/dungeon/src/main.mfb:getChar` (lines 28–33):

```basic
FUNC getChar AS String
  LET c AS String = io::readChar() TRAP(e)
    RECOVER ""
  END TRAP
  RETURN c
END FUNC
```

catches every error `io::readChar()` raises and answers `""` for all of
them, with no `e.code` check. `readKey` (lines 45–56) then reads `c = ""` as
"the source produced nothing" and returns `"EOF"`; it has no way to tell
"end of input" apart from "one bad byte", because `getChar` already threw
that information away. `ready` (lines 35–40) has the identical shape —
`RECOVER FALSE` for every error `io::pollInput()` raises — so if
`io::pollInput` were ever to raise `ErrEncoding` too (unconfirmed this
session; not part of this reproduction), it would silently report "nothing
ready" instead of propagating.

`water` and `wind`'s `nextChar` avoid this by checking `e.code` before
deciding what to do:

```basic
FUNC nextChar AS String
  RETURN io::readChar()

  TRAP(e)
    IF e.code = errorCode::ErrEndOfFile THEN RETURN ""
    IF e.code = errorCode::ErrEncoding THEN RETURN "\u{FFFD}"
    PROPAGATE
  END TRAP
END FUNC
```

— genuine EOF becomes `""` (their own callers' quit signal), a bad byte
becomes the replacement character (dropped from key-handling purposes but
not conflated with EOF), and anything else propagates as a real failure
instead of being silently swallowed.

## Goal

- `examples/dungeon`'s `getChar` distinguishes `ErrEndOfFile` from
  `ErrEncoding` from every other error, the way `water`/`wind`'s `nextChar`
  already does.
- A stray non-UTF-8 byte on stdin does not quit the game: it is dropped
  (or substituted, per the fix design) and reading resumes with the next
  byte.
- A genuine end of input (the window/stdin actually closing) still quits,
  unchanged.
- Any other error `io::readChar()`/`io::pollInput()` can raise still
  propagates as a real failure (today it is silently swallowed to `""`/
  `FALSE`; this is also a real defect this fix corrects, since a live bug —
  swallowing every error — must not be left in place once found, per
  `AGENTS.md`'s "never leave a bug you found").

### Non-goals (must NOT change)

- The key-sequence decoding logic in `readKey` (Esc, `[`/`O`, arrow letters,
  `3~`) — untouched by this fix.
- `examples/water`/`examples/wind`'s already-correct `nextChar` — not
  touched, only cited as the pattern to copy.
- **Tempting wrong fix, forbidden:** returning the replacement character
  `"\u{FFFD}"` from `getChar` and letting it flow into `readKey` as if it
  were a real keypress (dungeon has no use for arbitrary Unicode text input,
  only single ASCII command letters and Esc) — the right answer for dungeon
  is to drop the bad byte and keep reading, not to hand it to the game as a
  key. Producing `"\u{FFFD}"` as a *game command* would map to `Ignore[]`
  today (falls through `commandFor`'s `CASE ELSE`) which happens to be
  harmless, but it is accidental, not designed, and should not be relied on.

## Blast Radius

`grep -rn "io::readChar\|io::pollInput" examples/dungeon/src/*.mfb`
(2026-09-25) — only `main.mfb`'s `getChar`/`ready`, both already covered
above; `readKey` calls only those two, not `io::readChar`/`io::pollInput`
directly. No other file in `examples/dungeon` touches input.

Sibling examples with the same blanket-`RECOVER` shape on `getChar`
(`grep -n "io::readChar() TRAP(e)" examples/*/src/main.mfb examples/bugs/*/src/main.mfb`,
2026-09-25): `examples/snake/src/main.mfb:getChar`,
`examples/ai_chat/src/main.mfb:getChar`,
`examples/browser/app/src/main.mfb:getChar`,
`examples/bugs/tui/src/main.mfb:getChar`,
`examples/bugs/canvas/src/main.mfb:getChar`, and
`examples/wind`'s own `Thread`-safe variants where applicable — all `RECOVER
""` unconditionally, same latent hazard as dungeon (a bad byte reads as EOF
and quits the program). **Latent, same hazard, NOT observed to fail — out of
scope for this bug**, which reproduces and fixes dungeon specifically; these
are candidates for the identical one-line fix but are not part of this
bug's blast radius because none was reproduced this session and — per
"Interaction with bug-699" below — most of them are scheduled for deletion
by bug-699's `io::readKey` migration, which does not have this defect by
design (bug-699 Decision 2: invalid bytes are dropped, not raised, so there
is no `ErrEncoding` for a hand-rolled decoder to mishandle in the first
place).

## Interaction with bug-699

bug-699 (`bugs/bug-699-special-keys-arrive-unstandardized-per-surface.md`)
lists `dungeon` in its Blast Radius table as one of the eight examples whose
hand-rolled Esc decoder (exactly `getChar`/`ready`/`readKey`, the functions
this bug fixes) moves to `io::readKey` in its Phase 6. Under bug-699's
design, `io::readKey`'s decoder drops invalid bytes internally and never
raises `ErrEncoding` to the caller at all (Decision 2: "Invalid bytes...are
dropped, not reported — no `ErrEncoding`"), so once dungeon is migrated,
this bug's exact code — and its exact defect — is deleted, not fixed in
place.

**Recommendation: fix this bug now, independently of bug-699.** bug-699 is
`huge (>3d)`, not yet started (`Status: Open`), and gated on
platform-specific measurement work (Phase 1) before any example migrates.
Leaving dungeon quitting on any stray byte until bug-699's Phase 6 lands
would leave a live, easily-triggered correctness bug in place for an
indeterminate time for no benefit — the fix here is a few lines and does not
conflict with or duplicate any part of bug-699's design (it does not touch
the encoding/decoding contract bug-699 is redesigning, only the error
handling around today's `io::readChar()`). When bug-699 Phase 6 migrates
`examples/dungeon` to `io::readKey`, this fix's code is deleted as part of
that migration, same as the seven other hand-rolled decoders — no double
work, no conflict.

## Fix Design

Rewrite `getChar` to match `water`/`wind`'s `nextChar` pattern exactly:

```basic
FUNC getChar AS String
  RETURN io::readChar()

  TRAP(e)
    IF e.code = errorCode::ErrEndOfFile THEN RETURN ""
    IF e.code = errorCode::ErrEncoding THEN RETURN ""
    PROPAGATE
  END TRAP
END FUNC
```

Dungeon's `getChar` cannot reuse `water`/`wind`'s `"\u{FFFD}"` substitution
verbatim, because dungeon's `readKey` treats `getChar() = ""` as "nothing
useful happened, try again" only inside the Esc-sequence loop — at the top
level, `""` means EOF (see Non-goals: this is exactly the ambiguity to
avoid). The fix therefore needs `readKey` itself to distinguish "no more
input" from "one bad byte, keep going" instead of overloading `""` for both.
Recommended: `getChar` returns a sentinel distinguishable from both a normal
character and true EOF (e.g. keep `getChar`'s existing `""`-for-EOF contract
and add a second helper, or thread the error code up through `readKey`
directly instead of down through `getChar`'s return string). This needs a
small design choice by the implementer in Phase 1 before Phase 2 lands —
recorded as an Open Decision below, since two designs both work and the
choice affects `readKey`'s call sites.

Also add the same `errorCode` import dungeon does not currently have
(`examples/dungeon/src/main.mfb` imports only `app`, `canvas`, `io` — check
`IMPORT errorCode` is added).

`ready` gets the identical treatment for `ErrEncoding` if `io::pollInput` is
confirmed (Phase 1) to be able to raise it; if it cannot, `ready`'s existing
blanket `RECOVER FALSE` for every other error remains a separate, smaller
defect (silently swallowing a real failure as "nothing ready") worth a
one-line note but not blocking this fix.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Commit the `/tmp/bug713` driver (verbatim `getChar`/`ready`/`readKey`
      copy) as the regression probe — recommended home:
      `tools/dungeon-key-probe/` per the `scripts/`-vs-`tools/` split in
      `AGENTS.md` (a reusable probe, not a one-off), with a README.
      Confirm it fails today (`readKey()` returns `"EOF"` for a leading
      `0xFF` byte) — reproduced above.
- [ ] Confirm whether `io::pollInput()` can raise `ErrEncoding` (read its
      registry descriptor / lowering, or probe it directly) to settle
      whether `ready` needs the same fix.
- [ ] Settle the `getChar`/`readKey` sentinel design (Fix Design) and record
      the decision here.

Acceptance: the probe fails for the documented reason; the `pollInput`
question is answered; the sentinel design is chosen.
Commit: —

### Phase 2 — the fix

- [ ] `examples/dungeon/src/main.mfb`: add `IMPORT errorCode`; rewrite
      `getChar` (and `ready`, if Phase 1 found it needs it) to distinguish
      `ErrEndOfFile` from `ErrEncoding` from everything else, per the chosen
      sentinel design; adjust `readKey` accordingly so a bad byte is dropped
      and reading continues, while true EOF still quits.

Acceptance: the Phase 1 probe (rebuilt against the fixed functions) shows
`readKey()` skipping the `0xFF` byte and returning the next real key instead
of `"EOF"`; true EOF (empty stdin) still returns `"EOF"`.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `bash scripts/build-examples.sh` exits 0.
- [ ] Re-run the probe with: a clean UTF-8 stream (unchanged), a leading bad
      byte (now recovers), and empty stdin (still `"EOF"`).

Acceptance: full examples build passes; all three probe cases behave as
documented.
Commit: —

## Validation Plan

- Regression test: the `/tmp/bug713` driver, committed to `tools/dungeon-key-probe/`
  (or `tests/` if the project's convention for example-adjacent probes
  differs — check `scripts/README.md` at Phase 1 time).
- Runtime proof: the driver's before/after output for a bad-byte stream and
  a true-EOF stream, shown above for "before".
- Doc sync: none expected — this is example code, not a spec/man surface.
- Full suite: `bash scripts/build-examples.sh`.

## Open Decisions

- The `getChar`/`readKey` sentinel design for "bad byte, keep going" vs.
  "true EOF" (Fix Design) — implementer decides in Phase 1 and records the
  choice here before Phase 2.
- Whether to also fix the five sibling examples with the identical blanket-
  `RECOVER` shape (Blast Radius) now or leave them for bug-699's Phase 6
  migration — recommended: leave them, since bug-699 deletes this whole
  mechanism for those examples and a parallel one-line fix would be thrown
  away almost immediately; revisit if bug-699 stalls significantly past this
  session.

## Summary

`examples/dungeon`'s `getChar` conflates "no more input" with "one
malformed byte" by catching every `io::readChar()` error into the same `""`
sentinel; `readKey` then quits the game on that sentinel. `water` and
`wind` already show the fix — check `e.code`, treat `ErrEncoding` and
`ErrEndOfFile` differently — and it is a small, self-contained change. The
same mechanism is scheduled for deletion (not repair) by bug-699's
`io::readKey` migration, but that migration is `huge` effort and not
started, so this bug should land now rather than wait.
