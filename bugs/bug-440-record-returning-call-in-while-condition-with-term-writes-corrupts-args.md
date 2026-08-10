<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-440: A record-returning function called in a `WHILE` condition, combined with `term::` writes in the loop body, miscompiles — the loop's later `term::setBold`/`setUnderline` receive corrupted (garbage) arguments

Last updated: 2026-08-09
Effort: small fix (one zero-store per owned-value drop); wide golden churn (`.ncode`/`.ncodesum` for every fixture with an owned-value drop)
Severity: MEDIUM — silent wrong output / arena corruption, not a reliable crash; needs a specific code shape
Class: Correctness (native codegen — owned-value drop double-free)

Status: FIXED — root cause is NOT register clobber (the original title's guess); it is a
double free from an owned-value-drop cleanup slot that is zero-initialized only once at
the prologue and never re-nulled after a free, so a drop re-reached across a loop
back-edge without an intervening store frees a stale pointer again. Fix: free-and-null in
`emit_owned_value_drop` (and `emit_closure_drop`).
Regression Test: `tests/codegen_owned_drop_free_and_null.rs` (codegen inspection — the
runtime symptom is entropy-scrub-flaky, so a black-box rt test is unsound here).

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

**A double free of an owned-value-drop cleanup slot, NOT a register clobber.** The
original title/observations (register-allocation-sensitive) were a red herring —
the profile/tty-dependence is just the arena's entropy-scrub of the double-freed
block surfacing (or not) through a live alias.

`styleAt(j)` in the inner-`WHILE` condition returns a record; that owned temp is
dropped by `emit_owned_value_drop` (`src/target/shared/code/builder_owned_cleanup.rs`)
at the inner-loop scope. The drop is null-guarded — `if slot != 0 { arena_free(slot) }`
— and the guard is sound only if the slot reads 0 on every path that reaches the
drop without a store. The slot is zero-initialized **once at the prologue**
(`function_lowering.rs`, `owned_value_slots` splice) and **never re-nulled after a
free**. Across outer-loop iterations that breaks:

- iter i=1: inner condition calls `styleAt(j)` → slot = T1; at inner-loop exit the
  drop frees T1. The slot still holds T1 (freed).
- iter i=2 (last): `j < 3` is false, so the `AND` short-circuits and `styleAt(j)`
  is **never called** — the slot is not re-stored. The drop runs anyway and frees
  T1 **again**.

The second free is a *non-immediate* double free (other allocs intervened, so the
arena's immediate-double-free idempotency guard in `arena.rs` does not catch it):
it re-inserts an already-freed block, and if a live block (`st` for i=2, or any
16-byte record) had reused T1's storage, the free scrubs it — `st.underline`
(never set) then reads entropy-poison garbage, so the last cell renders underlined.
`io::print` instead of `term::` renders correctly only because its different
allocation order happens not to reuse the double-freed block; that is why the
symptom looked term- and profile-specific.

The confirming `.ncode` (`main`, macOS-aarch64): the inner-condition temp slot is
`str xzr` zeroed once in the prologue, assigned inside the inner while, and freed
at `while_end` (`bl _mfb_arena_free`) with **no** following store — so the stale
pointer survives to the next iteration's free.

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

## Fix

Free-and-null the owned-value drop: in `emit_owned_value_drop`
(`src/target/shared/code/builder_owned_cleanup.rs`), immediately after
`emit_arena_free_call()` (the freed path, before the skip label), zero the slot —
`self.emit(abi::store_u64(abi::ZERO, abi::stack_pointer(), cleanup.stack_offset))`.
This restores the drop's documented invariant ("the slot reads 0 on every path that
reaches this drop without a store") for loop re-entry, so a re-reached drop with no
intervening store reads 0 and skips instead of double-freeing. The null path already
read 0, so only the freed path needs it. The same free-and-null was added to
`emit_closure_drop` (its `object_slot`) for the closure-temp-in-loop analogue.

The added store uses only `abi::ZERO`/`abi::stack_pointer()` (no `temporary_vreg()`/
`allocate_register()`), so it does NOT perturb vreg numbering — the `.ncode` delta is
purely the additive zero-stores (one per owned-value drop), reviewable as such.

## Acceptance

- `tests/codegen_owned_drop_free_and_null.rs` asserts every `owned_value_free_skip*`
  cleanup ends with a zero-store to its slot (RED before the fix: the label is
  preceded by the bare `bl _mfb_arena_free`). Deterministic; the runtime symptom is
  entropy-scrub-flaky (0/20 under a pipe even unfixed), so a black-box rt assertion
  is unsound — this is the codegen-inspection test the memory note prescribes for a
  slot/double-free fix.
- Goldens regenerated for the `.ncode`/`.ncodesum` shift (the added zero-stores);
  diff confirmed to be ONLY those additions.
- Full `cargo test` green (aside from the pre-existing, independent bug-438 GTK
  grid-size failure).

## Notes / Scope

- Found while adding `term::drawText(x, y, AttributedString)`. That feature had
  originally been written to AVOID this shape (two Boolean-returning helpers
  compared in the run-scan `WHILE` condition instead of one record-returning
  resolver). As part of this fix the workaround is REMOVED: the bridge
  (`src/builtins/term_astrings_bridge.mfb`) now uses the natural single
  record resolver `__term_styleAt` returning a `__TermStyle` record and compares
  it in the `WHILE` condition — the exact shape that used to miscompile, now
  correct (and one `getAttributes` per scalar instead of two). The record type is
  named with the internal `__` sigil (`__TermStyle`) so the injected type cannot
  collide with a user's own `TermStyle`. This bug is independent of that feature —
  the reproduction uses only `term` + a user record.
- The variance between debug and release output is itself diagnostic: the same
  source, same front-end IR, different native register allocation → different
  wrong output. Any fix must be verified in both profiles.
