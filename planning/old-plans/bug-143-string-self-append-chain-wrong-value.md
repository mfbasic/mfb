# bug-143 — In-place self-append chain re-reads the mutated string → wrong value

**Status:** FIXED (commit e0fa88b8, 2026-07-11).
**Severity:** HIGH — wrong string result when the target reappears later in a
concatenation chain.
**Class:** correctness.

## Finding

`src/target/shared/code/builder_inplace_assign.rs:384-410`
(`try_inplace_concat_assign`) + :417-558 (`lower_string_self_append_one`).
`string_self_append_operands` only requires the *leftmost* leaf to be `name`;
the remaining operands are lowered one-at-a-time *after* earlier operands have
already mutated the buffer. Any chain where `name` reappears as a later operand
(`s = s & x & s`) appends the already-extended value. (It appears correct for
constant-folded strings, masking it.)

Secondary latent defect in the same function: the in-place path reloads `rlen`
from `right_ptr[0]` *after* storing the new length to `ptr[0]` (:549-556), so an
aliased operand over-subtracts the capacity shadow — currently unreachable
(spare < len invariant holds), but a landmine if the grow policy ever leaves
spare ≥ len.

## Trigger (reproduced)

```
u = "ABAB"      # runtime-built, no constant fold
u = u & "x" & u
```
prints `ABABxABABx` (10 chars) instead of `ABABxABAB` (9 chars).

## Fix

When the target `name` appears more than once in the operand chain, materialize
the original value once (snapshot) before the first in-place mutation, or fall
back to the out-of-place concat path.

## Prior art

bug-77 covers only the regrow leak in this function (still unfixed — regrow path
:469-534 never frees the old buffer); the wrong-value chain bug is new.

## Resolution

FIXED in commit e0fa88b8. try_inplace_concat_assign falls back to the out-of-place path when the target reappears in a later operand.

Regression test: `tests/rt-behavior/general/bug143_string_self_append_chain` (fails on the unfixed compiler). Full
acceptance (871) and `cargo test` pass.
