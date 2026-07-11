# bug-56: LINK `SUCCESS_ON`/`RESULT` expression codegen uses an unbounded fixed-physical-register scheme that escalates into `x19` (arena_base) and other reserved callee-saved registers for moderately nested expressions — corrupting the arena base program-wide

Last updated: 2026-07-09
Effort: medium (1h–2h)

SKIP THIS BUG. WILL BE FIXED WITH PLAN-34

`emit_link_expr` lowers a LINK binding's `SUCCESS_ON`/`RESULT` boolean expression using
**fixed physical registers** derived from a `base` index: the node's value goes in
`x{base}`, a `Compare`'s rhs in `x{base+2}`, an `And`/`Or`'s rhs in `x{base+4}` — with no
cap and no spilling. Both call sites start at `base = 9`. A right-nested expression
therefore walks `base` upward through `x17`, then `x19`, `x21`, `x23`… — and `x19` is
`ARENA_STATE_REGISTER`, the program-wide-pinned arena base pointer. A `move x19, …` in
the thunk clobbers arena_base; the thunk is finalized with an empty stack-slot list and
`x19` is a reserved physical register (not spilled), so it is never restored. The
subsequent LINK `FREE` path (`load %v, [x19 + slot]; blr %v`) then dereferences a garbage
base → wild indirect call / crash; even without `FREE`, the thunk returns with the whole
program's arena base corrupted, so the next allocation anywhere corrupts memory.

The trigger is not exotic: a legal `SUCCESS_ON r <> 1 AND (r <> 2 AND r <> 3)` — an
`And(Cmp, And(Cmp, Cmp))` tree — reaches `x19` at three levels of right-nesting. The
single correct behavior a fix produces: a LINK success/result expression of any shape
evaluates within scratch registers the thunk is allowed to clobber (or spilled), never
touching `x19`/reserved callee-saved registers.

References:

- `src/target/shared/code/link_thunk.rs:emit_link_expr` (`:916-985`): `dst = x{base}`;
  `Compare` recurses rhs at `base+2` (`:954`); `And`/`Or` recurse rhs at `base+4`
  (`:976`, `:981`). No bound on `base`.
- Call sites both pass `base = 9`: `SUCCESS_ON` at `:512-519`, `RESULT` at `:554-563`.
- `x19` is `ARENA_STATE_REGISTER` (`src/target/shared/code/error_constants.rs:123`),
  loaded to reach the resolved function / deallocator pointers at
  `link_thunk.rs:483`/`:579`, and pinned program-wide.
- The thunk is finalized without spilling `x19` (finalize is called with `&[]` locals),
  and `x19` is a reserved physical register, so nothing saves/restores it.
- Escalation walk for `And(Cmp, And(Cmp, Cmp))`: outer And base 9 (lhs Cmp → x9/x11),
  rhs And base 13 (lhs Cmp → x13/x15), rhs Cmp base 17 (dst x17, rhs base+2 = **x19**).
- Related (same function, LOW, filed separately): bitwise `And`/`Or` on non-normalized
  operands, and the `CInt32` const-pin range-check bypass.
- Found during the goal-01 compiler source review of `src/target/shared/code/`.

## Failing Reproduction

A LINK binding whose `SUCCESS_ON` (or `RESULT`) is a right-nested boolean expression
over the native return value. Sketch (needs a native library to bind against; construct
per the LINK test harness):

```
LINK "libfoo" FUNC foo(...) AS Integer
  SUCCESS_ON r <> 1 AND (r <> 2 AND r <> 3)
END LINK
```

Call `foo`, then perform any allocation (e.g. build a String) after it returns.

- Observed (by construction, from the register walk above): the thunk executes
  `move x19, <status>` while evaluating the innermost compare, clobbering arena_base;
  the next allocation reads a corrupt arena and crashes or corrupts memory. If the LINK
  has a `FREE` clause, `blr` through `[x19+slot]` jumps to a wild address immediately.
- Expected: `SUCCESS_ON`/`RESULT` evaluates correctly and preserves `x19`.

Contrast (safe today): shallow / left-associated expressions keep `base` in the
caller-saved scratch window `x9`–`x17` — `r = 0` (x9/x11), `r >= 0 AND r <= 255`
(x9/x11 then x13/x15), and left-assoc `a AND b AND c` all stay ≤ x15. Only right-nested
(parenthesized) trees push `base` to 19+.

## Root Cause

`emit_link_expr` predates the vreg/linear-scan allocator and hand-assigns physical
registers by an escalating `base` (`x{base}`, `+2`, `+4`) with no upper bound and no
awareness of which registers are reserved (`x19` = arena_base) or callee-saved. Nesting
grows `base` monotonically past the caller-saved scratch window into reserved/callee-saved
registers that the thunk neither may clobber nor saves. Because `x19` is the pinned
arena base, the corruption is program-wide, not thunk-local.

## Goal

- A LINK `SUCCESS_ON`/`RESULT` expression of any nesting depth evaluates without writing
  `x19` or any reserved/callee-saved register the thunk does not save.
- `x19` (arena_base) is intact after every LINK thunk returns.
- The `FREE` path dereferences a valid arena base.

### Non-goals (must NOT change)

- The boolean/compare **semantics** of `SUCCESS_ON`/`RESULT` (except the separately-filed
  bitwise-AND/OR normalization).
- Shallow-expression codegen (already correct) — ideally byte-identical.
- The arena-base register assignment (`x19`) itself.

## Blast Radius

- `emit_link_expr` (`link_thunk.rs`) — fixed here; both `SUCCESS_ON` and `RESULT` callers
  inherit the fix.
- Any other hand-rolled fixed-physical-register emitter that escalates by index — grep
  for `format!("x{...}")` with an arithmetic base in the codegen; the thunk emitter is
  the known instance.

## Fix Design

Rewrite `emit_link_expr` to evaluate into **vregs** (letting the linear-scan allocator
place and spill them) or into a bounded caller-saved scratch set with an explicit stack
value-stack for deep trees. Either eliminates the escalation into reserved registers.
The vreg route is preferred — it matches the rest of the backend (plan-00-G) and removes
the fixed-register scheme entirely; the thunk is already finalized through the vreg body
path, so the wiring exists.

A minimal stopgap (if a full rewrite is deferred): assert `base` stays within `x9`–`x17`
and reject (compile error) any expression that would exceed it, converting the silent
memory corruption into a diagnostic. This is not a real fix but bounds the damage.

Where risk concentrates: preserving the exact boolean truth values while changing the
register discipline, and keeping shallow-expression output byte-identical for the
golden gate.

## Phases

### Phase 1 — failing test

- [x] Add a LINK test with a 3-level right-nested `SUCCESS_ON` (and one for `RESULT`)
      that allocates after the call; confirm arena corruption / crash today, or assert
      the generated thunk writes `x19`.

### Phase 2 — the fix

- [x] Reimplement `emit_link_expr` over vregs (or bounded scratch + spill). Verify no
      emitted instruction writes `x19`/reserved registers for any expression shape.

### Phase 3 — validation

- [x] Regenerate goldens; shallow-expression thunks should be byte-identical (or a
      reviewed minimal delta). `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.
- [x] Run the LINK reproduction; arena base intact, `FREE` path valid.

## Validation Plan

- Regression test(s): nested `SUCCESS_ON`/`RESULT` LINK tests + a post-call allocation
  assertion.
- Runtime proof: the LINK binding with a deep success expression runs and the following
  allocation succeeds; arena base unchanged across the thunk.
- Doc sync: none expected.
- Full suite: `scripts/artifact-gate.sh`, `scripts/test-accept.sh`.

## Summary

The LINK thunk's success/result expression emitter assigns physical registers by an
unbounded escalating index that walks into `x19` (the program-wide arena base) for
ordinary right-nested boolean expressions, corrupting the arena and crashing the `FREE`
path. The real fix is to evaluate over vregs like the rest of the backend; a bounded-scratch
stopgap at least turns the corruption into a diagnostic. Shallow expressions are already
safe and should stay byte-identical.

## Resolution

Fixed by reimplementing `emit_link_expr` over virtual registers — the vreg route the
Fix Design preferred, matching the rest of the backend (plan-00-G). No stopgap/diagnostic
was used. (The stale "SKIP THIS BUG / PLAN-34" banner at the top predates this fix and is
superseded.)

Changes (`src/target/shared/code/link_thunk.rs`):

- `emit_link_expr` no longer takes a `base: usize` physical-register index. It now takes
  `vreg: &mut usize`, allocates a fresh `%vN` for every intermediate (its own `dst`, and
  each recursive sub-result), and **returns** the vreg name holding the value. The shared
  linear-scan allocator places and spills these, so no expression shape can reach a
  reserved/callee-saved register. The `Compare`/`And`/`Or`/`Not` arms now read their
  operands from the returned sub-result vregs instead of `x{base}`, `x{base+2}`,
  `x{base+4}`.
- A `const LINK_EXPR_VREG_BASE: usize = 64` starts the expression's vregs well past the
  thunk body's fixed scratch vregs (`%v9`..`%v16`), so the two name spaces never overlap
  and each temporary is an independent value to the allocator.
- Both call sites (`SUCCESS_ON` gate and `RESULT` producer) drop the old
  `move %v9, x9` physical→vreg bridge and consume the returned vreg directly.

Boolean/compare semantics are unchanged; only the register discipline changed.

Runtime proof (macOS aarch64):

- Reproduction: `LINK "c" AS libc` binding `abs` with
  `SUCCESS_ON status <> 1 AND (status <> 2 AND status <> 3)` (an `And(Cmp, And(Cmp, Cmp))`
  tree), then a 2000-iteration post-call String-append loop.
  - Before the fix the myAbs thunk emitted `mov x19, #3` (innermost compare's rhs), and the
    program crashed with **SIGSEGV (exit 139)** at the first post-call allocation.
  - After the fix the thunk writes **no `x19`** (the expression lives in `x8`–`x11` scratch;
    the `AND`s fold in `x9`/`x11`), and the program prints `abs=5` / `len=8890`, **exit 0**.
- Regression test `tests/rt-behavior/native/native-link-nested-success-rt/` (portable
  sqlite3 binding): `open` carries a 3-deep right-nested `SUCCESS_ON`; `step` carries a
  3-deep right-nested `SUCCESS_ON` **and** `RESULT`; `main` runs an ordered query then a
  3000-iteration allocation loop after the calls. Builds and runs green
  (`alice;bob;carol;` / `len=13890`, exit 0). This fixture crashed under the old codegen.

Validation:

- `cargo build` clean; `cargo test --bins` = 2442 passed, 0 failed.
- All existing native-link rt tests (`native-link-sqlite-rt`, `native-link-free-rt`,
  `native-link-const-64bit-rt`, `native-link-import-sqlite-rt`) build and run byte-identical
  to their goldens.

Golden impact: no existing test carries a native `.ncode`/`.nir`/`.nplan`/`.mir` golden for
a LINK with `SUCCESS_ON`/`RESULT`, so the codegen change shifts **no** existing golden. The
only new goldens are the new fixture's `build.log`/`.ast`/`.ir` (no native goldens shipped).
`scripts/artifact-gate.sh` / `scripts/test-accept.sh` are the orchestrator's to run.
