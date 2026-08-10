<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-440: A record-returning function called in a `WHILE` condition, combined with `term::` writes in the loop body, miscompiles — the loop's later `term::setBold`/`setUnderline` receive corrupted (garbage) arguments

Last updated: 2026-08-09
Effort: unknown (native regalloc/clobber root-cause; needs `.ncode`/objdump inspection)
Severity: MEDIUM — silent wrong output (wrong cell attributes / miscount), not a crash; needs a specific code shape
Class: Correctness (native codegen — register allocation / call-clobber)

Status: Open (discovered while implementing `term::drawText(x, y, AttributedString)`; worked around there by not using the triggering shape)
Regression Test: none yet — see Acceptance

## Symptom

A loop that (a) resolves a small record from a helper **inside the `WHILE`
condition** and (b) makes `term::` writer calls (`setBold`/`setUnderline`/
`drawText`) in the body produces **wrong terminal attributes on the last run**:
an attribute (e.g. underline) is applied that was never set anywhere in the
program, and/or a run's bold/underline is stale. The exact corruption is
**build-profile dependent** (debug vs release differ, and the cell count can even
differ) — the fingerprint of a register-allocation-sensitive miscompile.

The identical loop with the identical control flow but `io::print` instead of the
`term::` calls prints the **correct** values, so the loop logic, the record `=`
comparison, and the record field reads are all fine — the corruption is confined
to the interaction between the record-returning call in the loop condition and the
`term::` writer codegen.

## Root Cause

Not yet localized. Observations that bound it (each is a minimal delta from a
working variant, native macOS-aarch64):

- Trigger REQUIRES a **record-returning** helper called in the `WHILE` condition.
  The same loop with **Boolean**-returning helpers in the condition renders
  correctly. (Booleans vs a 2-field record is the only change.)
- Trigger REQUIRES the helper call to be **in the loop condition**. Calling the
  record helper only in the loop *body* (per-iteration, no inner `WHILE`) renders
  correctly.
- Trigger REQUIRES `term::` writer calls in the body. Replacing them with
  `io::print` of the same record fields prints correct values.
- An extra allocating call (e.g. `astrings::getAttributes`) placed between the
  resolve and the `term::` calls does **not** trigger it on its own.

This points at a callee-clobbered / mis-allocated register or stack slot around
the record temporary materialized for the loop-condition call, exposed when the
`term::` writer helpers (which load the arena state base and store the flag
argument) run in the same loop. The garbage `underline` (a flag never set to
`TRUE` in the program) indicates the Boolean argument register handed to
`term::setUnderline` is corrupted, not the resolved record value.

## Failing Reproduction

`/tmp` project, `IMPORT term` only (no astrings needed — this is a pure
codegen bug):

```mfbasic
IMPORT term

TYPE TStyle
  bold AS Boolean
  underline AS Boolean
END TYPE

FUNC styleAt(index AS Integer) AS TStyle
  IF index = 1 THEN
    RETURN TStyle[TRUE, FALSE]
  END IF
  RETURN TStyle[FALSE, FALSE]
END FUNC

FUNC main AS Integer
  term::on()
  MUT col AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 3
    LET st AS TStyle = styleAt(i)
    MUT j AS Integer = i + 1
    WHILE j < 3 AND styleAt(j).bold = st.bold AND styleAt(j).underline = st.underline
      j = j + 1
    END WHILE
    term::setBold(st.bold)
    term::setUnderline(st.underline)
    term::drawText(col, 0, "?")
    col = col + 1
    i = j
  END WHILE
  term::sync()
  term::off()
  RETURN 0
END FUNC
```

Build and run under a PTY (`script -q /dev/null <exe> < /dev/null`). The three
runs resolve to styles plain / bold / plain, so the presented row should be
`?`(plain) `?`(bold) `?`(plain). Instead the **last** cell carries a stray
attribute (observed: underline `\x1b[4m` — never set anywhere), and under the
release profile the cell count/attribution differs again.

Control (localizes the bug to codegen, not logic): replace the three `term::`
calls with
`io::print("[" & toString(i) & "," & toString(j) & ") bold=" & toString(st.bold) & " underline=" & toString(st.underline))`.
It prints the correct three runs (`bold=FALSE/TRUE/FALSE`, `underline=FALSE`
throughout).

## Proposed Fix

Root-cause in native codegen: dump `.ncode` (and objdump) for the reproduction and
localize which register/slot the loop-condition record temporary and the `term::`
writer argument share. Likely a missing save/restore or a vreg-allocation-order
issue around a record-returning call whose result is consumed across a subsequent
call in the same loop. Fix the allocation/clobber, then confirm the reproduction
renders plain/bold/plain in **both** debug and release.

## Acceptance

- The reproduction above renders `?`(plain) `?`(bold) `?`(plain) in both profiles.
- A regression test (native PTY, like `tests/rt_native_term_runtime.rs`) asserting
  the last run's cell carries no stray bold/underline.
- Full `cargo test` green.

## Notes / Scope

- Found while adding `term::drawText(x, y, AttributedString)`. That feature was
  written to AVOID this shape: its bridge
  (`src/builtins/term_astrings_bridge.mfb`) resolves bold/underline with two
  **Boolean**-returning helpers (`__term_boldAt` / `__term_underlineAt`) compared
  in the run-scan `WHILE` condition, rather than one record-returning resolver, so
  it does not hit this miscompile. This bug is independent of that feature — the
  reproduction uses only `term` + a user record.
- The variance between debug and release output is itself diagnostic: the same
  source, same front-end IR, different native register allocation → different
  wrong output. Any fix must be verified in both profiles.
