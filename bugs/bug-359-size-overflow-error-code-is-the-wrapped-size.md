# bug-359: three string size-overflow exits report the *wrapped size* as the error code instead of `ErrOutOfMemory`

Last updated: 2026-07-19
Effort: small (<1h)
Severity: LOW
Class: Correctness (wrong error code on an overflow path)

Status: Open
Regression Test: tests/ (new) — a size-overflow exit surfaces code 77050005
(`ErrOutOfMemory`), not an arbitrary integer

Three allocation-size overflow labels call `emit_allocation_error_return()`,
which passes `RESULT_TAG_REGISTER` (**= `abi::RET[0]` = x0**,
`error_constants.rs:25`) as the *error code register*. That contract is correct
on the allocation-failure path, where `_mfb_arena_alloc` leaves a tag in x0 —
`emit_error_register_return`'s own comment says so ("the allocation path passes
it in x0"). It is wrong at an overflow label, because the preceding
`emit_checked_size_add_immediate(abi::return_register(), …)` wrote the **wrapped
sum** into x0 and branched on the wrap. The program therefore fails with an
error whose code is a garbage size value.

This contradicts the intent bug-60 recorded for exactly these guards: *"All
overflow branches raise the allocation error the codebase already uses for an
oversized request (`emit_allocation_error_return` / `ERR_OUT_OF_MEMORY_CODE` /
`ERR_INVALID_ARGUMENT_CODE`), never a silent clamp."* The comment at each of the
three sites repeats that claim. The claim is false at these three sites and true
at the other 34.

The single correct behavior a fix produces: every allocation size-overflow exit
fails with `ErrOutOfMemory` (`ERR_OUT_OF_MEMORY_CODE`), the same as the 34 sites
that already get it right.

References:

- `bugs/completed-bugs/bug-60-size-arith-before-alloc-cluster.md` — added these
  guards and states the intended error code.
- Found while auditing bug-322's blast radius (the arena-allocation cleanup),
  not by looking for it.

## Failing Reproduction

Not reachable from source today: it needs a string operation whose computed
output length is within 9 bytes of `u64::MAX`, which allocation limits make
unreachable — the guards are defense-in-depth, as bug-60 says. The defect is
therefore established by reading the emitted code rather than by running it, and
the regression test should drive the emitter directly (assert the overflow label
is followed by a `move_immediate` of `ERR_OUT_OF_MEMORY_CODE` into the code arg,
not a `move_register` from x0).

## Root Cause

`src/target/shared/code/builder_strings.rs:207`, `:441`, and
`src/target/shared/code/builder_strings_builtins.rs:1409` each read:

```rust
self.emit_checked_size_add_immediate(abi::return_register(), output_len, 9, &overflow);
...
// A 64-bit wrap in the size computation raises the same catchable
// allocation error as an oversized request (defense-in-depth; bug-60).
self.emit(abi::label(&overflow));
self.emit_allocation_error_return()?;
```

`emit_allocation_error_return` is `emit_error_register_return(RESULT_TAG_REGISTER, …)`
(`builder_codegen_primitives.rs:298`), and `RESULT_TAG_REGISTER` is x0 — the
register holding the wrapped size at that label.

The 34 correct sites (e.g. `builder_collection_mutate.rs`, 9 of them) instead use
`emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)`, which
materialises the code as an immediate and does not depend on x0.

## Goal

- The three sites emit `ErrOutOfMemory`, matching the other 34 and bug-60's
  stated contract.

### Non-goals (must NOT change)

- The allocation-*failure* path's use of `emit_allocation_error_return`. There
  x0 legitimately holds the tag from `_mfb_arena_alloc`; that is the helper's
  reason to exist.
- `emit_checked_size_add_immediate`'s choice of destination register — changing
  it would move goldens across every caller.

## Blast Radius

- `src/target/shared/code/builder_strings.rs:207,441` — fixed here.
- `src/target/shared/code/builder_strings_builtins.rs:1409` — fixed here.
- The 34 sites already using `emit_error_code_return` — unaffected.
- Goldens: these are error-path instruction sequences, so `.ncode`/`.nobj`
  goldens for the affected helpers **will** shift. That is a real change, not
  churn to be waved through; regenerate only after confirming the sole delta is
  the code immediate.

## Fix Design

Replace `self.emit_allocation_error_return()?` at the three overflow labels with
`self.emit_error_code_return(ERR_OUT_OF_MEMORY_CODE, ERR_ALLOCATION_MESSAGE)?`,
and correct the comment at each site, which currently asserts the behavior the
code does not have.

Rejected: making `emit_allocation_error_return` load the immediate itself —
it would break the allocation-failure path, which is its actual caller and
depends on the register contract.

## Validation Plan

- Regression: an emitter-level test asserting the overflow exits carry the
  `ERR_OUT_OF_MEMORY_CODE` immediate.
- Full suite: `cargo test`, `scripts/artifact-gate.sh` (expect a scoped diff at
  these helpers only), `scripts/test-accept.sh`.

## Summary

A register-contract mismatch: an error-return helper that takes its code from x0
is called at a label where x0 holds a wrapped size. Unreachable in practice, but
it is a wrong error code on a path whose comment and originating bug both claim
it produces `ErrOutOfMemory` — and 34 sibling sites do exactly that.
