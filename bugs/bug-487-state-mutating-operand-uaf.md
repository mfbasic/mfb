# bug-487 — an operand that mutates a resource's STATE frees the block a sibling operand points into

STATUS: OPEN
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

## Reproduce

```
mfb build bugs/repro/                # as a scratch project with entry main
./build/<name>.out                   # exit 139, or 7-701-0001, or 7-705-0001
```

The count matters only for which symptom appears, not whether it fails: 3 rounds
already fails.
