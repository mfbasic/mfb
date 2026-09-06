# bug-487 — an operand that mutates a resource's STATE frees the block a sibling operand points into

STATUS: FIXED (see "Fix" below)
FOUND: plan-121-D Phase 1 (2026-09-03), while settling the STATE reachability question
REPRO: `bugs/repro/bug-487-state-mutating-operand-uaf.mfb`

## Summary

```basic
f.state.xs = collections::append(f.state.xs, sideEffect(f))
```

where `sideEffect(RES f …)` appends to `f.state.xs`. Argument 0 lowers to a
pointer **into f's STATE block**; argument 1 grows that same block, which
reallocates and **frees** the allocation argument 0 points into. `append` then
reads freed memory.

This is a use-after-free, not a wrong value. The symptom is whatever the freed
memory happens to hold:

| path taken | compiler | symptom |
|---|---|---|
| in-place STATE arm (`f.state.xs = append(…)`) | worktree P-121 @ plan-121-D Phase 1 | `exit 139` (SIGSEGV) at 120 rounds, `Allocation failed` (7-701-0001) at 3 |
| in-place STATE arm | **`56b368996`** (pre-plan-121) | `Allocation failed` (7-701-0001) |
| copying rebuild (two-field `WITH`, declines `G14`) | worktree P-121 | `ErrIndexOutOfRange` (7-705-0001) |

**Pre-existing, and not confined to the in-place work.** It reproduces on the
pre-plan-121 compiler, and it reproduces on the *copying* path that takes no
in-place arm at all. Three distinct symptoms, one mechanism.

## Not a program error

`mfb spec language resource-management` §15: *"Because a `RES` is an alias to one
live resource, a state update made through a `RES` parameter (an alias, not a
copy) is visible to the owner after the call."* Making that update from inside an
operand is not excluded anywhere in §15 or §8.

**The defined answer is `len == rounds`, every element 222** — argument 0 is a
snapshot taken before `sideEffect` runs, so the outer `append` overwrites the
nested one. *Losing* the nested append is correct value semantics. Crashing is
the bug. Do not "fix" this by trying to make the nested append survive.

## Two halves, tracked separately

1. **The in-place STATE arms** capture the STATE pointer into `block_slot`
   *before* lowering the operand (`open_inplace_state_dest`, called ahead of
   `lower_value` in every `try_inplace_state_*` arm). If the operand reallocates,
   that snapshot is stale and the `O4` write-back republishes a freed pointer.
   The existing comment justifies the early snapshot as `O-order-4`, "the
   operand's own lowering must not observe a stale STATE pointer" — but reading
   the pointer into a slot cannot change what the operand observes, so the
   ordering appears to be defensive against a hazard that does not exist, while
   creating one that does. **The record-field arms already do the opposite** and
   take the address *after* the operand. Fixing this half is local.
2. **The copying path** fails independently, before any in-place arm is reached,
   so fixing (1) does not fix the program. The root cause there is in how a call
   argument that is a pointer into a live block is kept alive across the
   evaluation of later arguments — a general argument-aliasing question, not a
   STATE one. **This half is the reason this is a bug report rather than a
   plan-121-D task**: it is outside that plan's blast radius and needs its own
   design.

### CORRECTION (2026-09-06) — half 2 was fixed by bug-496

Half 2 no longer reproduces. `a075a589b` (bug-496) added
`src/codegen/engine/value/operand_snapshot.rs`, whose
`operand_reachable_by_later_call` already treats a resource-handle local and any
`MemberAccess` of one as reachable, so the copying path deep-copies
`f.state.xs` before the later operand's call runs. Measured at `8f0ebfeb8` with
the repro rewritten as a two-field `WITH` (which declines `G14` and so takes the
copying path):

```
len=3
n=3
e0=222  e1=222  e2=222   ' exit 0
```

Only half 1 was still live at `8f0ebfeb8`, and only in the in-place arms. The
`.ncode` for the repro confirms it: `_mfb_fn_main` allocates `inline_state_ptr`
and `inline_state_rhs` and **no** `operand_snapshot` slot — the in-place arm
never lowers operand 0, so it never reaches bug-496's seam at all.

### A THIRD arm, found while auditing the other eight

`try_inplace_state_scalar_assign` (`engine/control/builder_control.rs`) does not
go through the shared STATE container matcher. It re-loads the STATE pointer
*after* the operands, so it never dangled — but it has the same *divergence*:

```
f.state.n = f.state.n + sideEffect(f)   ' sideEffect appends to f.state.xs
```

kept the nested `xs` growth (`xs=1`), while the whole-state `WITH` that
statement is shorthand for discards it (`xs=0`, measured on a STATE shape whose
updated field is inlined so the arm declines). Same root cause, same gate, fixed
in the same change.

## Fix

**`G25` — decline any in-place `RES … STATE` arm whose operands can reach a
`STATE` assignment.**

The contract the fix realizes is `mfb spec language resource-management` §15:

> It is updated either by assigning a single field in place
> (`s.state.field = value`) or by assigning a whole-state `WITH` update
> (`s.state = WITH s.state { field := value }`); **the former is shorthand for
> the latter.**

The `WITH` form *reads* `s.state`, and `mfb spec language memory-semantics`
§14.6 says *"Reads produce owned values, not aliases into the buffer"* — so the
record stored at the end of the statement is built from a value taken **before**
the operands ran, and a `STATE` write performed during operand evaluation is
overwritten. §14 permits the in-place strategy — *"The compiler may choose stack
storage, inline storage, heap allocation, or destructive update, but those
choices cannot change the ownership behavior described here"* — only while
nothing else writes the block in between.

So the gate asks exactly one question: **can any operand reach a
`NirOp::StateAssign`?** It is transitive over the module call graph, and fails
closed on a call whose body it cannot see (an indirect `FUNC`-value call, a
higher-order builtin handed a callback, a separately-lowered symbol).

It is deliberately *not* "does the operand call user code at all". That broader
guard would have rejected `append(f.state.xs, clamp(v))` — a valid program, and
the exact 20 000× cliff `tests/codegen_inplace_append_call_result.rs` measures.
Both halves are pinned.

Seams (one shared, one arm-local, both pure declines that emit and allocate
nothing, so `O-order-1` holds):

* `resolve_inplace_state_field` (`collection/assign/inplace_dest.rs`) — the
  shared container matcher all eight STATE collection arms go through.
* `try_inplace_state_scalar_assign` (`engine/control/builder_control.rs`).

`O-order-4` is left exactly as it was: the fix does not reorder the STATE-pointer
load, it removes the statements that could invalidate it.

### Why an operand's callee can hold an alias the operand never names

Measured at `8f0ebfeb8`, so the gate could not be narrowed to "the operand
mentions the resource":

| escape route | result |
| --- | --- |
| `LAMBDA() -> sideEffect(f)` | rejected — `2-203-0019 TYPE_LAMBDA_CAPTURE_UNSUPPORTED` |
| `RES h AS fs::File STATE St = f` | rejected — `2-203-0055 TYPE_USE_AFTER_MOVE` |
| a global `List OF RES fs::File STATE St` that a callee stashes into, read back by a later callee | **compiles, and reproduces the same `7-701-0001`** with an operand that never names `f` |

The third row is why the gate is a call-graph question rather than a
name-occurrence one.

## Reproduce


```
mfb build bugs/repro/                # as a scratch project with entry main
./build/<name>.out                   # exit 139, or 7-701-0001, or 7-705-0001
```

The count matters only for which symptom appears, not whether it fails: 3 rounds
already fails.

## Verification

* **RED**: `tests/rt-behavior/resources/bug487_state_mutating_operand` — at
  `8f0ebfeb8` it exits 255 with `Error: 7-701-0001 / Allocation failed.`; with
  the fix it exits 0 and its golden `build.log` pins the full expected output.
  Three codegen pins in `tests/rt_res_state_inplace_mutation.rs` are RED at
  `8f0ebfeb8` and green after:
  `state_field_append_whose_operand_reaches_a_state_assign_declines`,
  `state_field_append_whose_operand_reaches_a_state_assign_transitively_declines`,
  `scalar_state_field_whose_operand_reaches_a_state_assign_declines`.
* **POSITIVE pins** (green *before* the fix too, so they pin what must not
  change): `state_field_append_whose_operand_call_chain_never_assigns_state_grows_in_place`
  and `scalar_state_field_whose_operand_never_assigns_state_stores_in_place`,
  plus the pre-existing
  `codegen_inplace_append_call_result::state_field_append_of_a_user_call_result_grows_in_place`.
* **Independent oracle.** The same fixture program, with the STATE record given
  a trailing `String` field so `xs` is no longer last-inlined and `G17` declines
  every statement to the copying whole-record replace (four `state_assign_value`
  slots in `_mfb_fn_main`), prints output **identical** to the fixed in-place
  compiler's. The two strategies now agree.
* **Artifact gate**: 1394 tests, 1560 builds, **1939 goldens checked, 0 diffs** —
  the only goldens that moved in the whole tree are the new fixture's own. No
  `.ncodesum` regeneration was needed, because no existing fixture emits the
  shape the gate declines.
