# bug-99 — `Capture` in a never-a-closure function escapes ir::verify's bounds check → OOB env read

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G3).
**Severity:** MED — untrusted `.mfp` can drive an out-of-bounds env-relative
load in generated code; not front-end-reachable.
**Class:** memory-safety / security (trust boundary: `.mfp` decode → verify → codegen).

## Finding

`src/ir/verify/mod.rs:3707-3723` (`check_value_captures`) early-returns when
`slots` is `None`. `closure_slot_count` (mod.rs:673-675) returns `None` in two
cases:

1. body arity ambiguous — **fixed** by bug-32 (now `iter().min()`);
2. `closure_counts.get(function)` absent — the function is never targeted by
   any `Closure` node. **Unguarded.**

So a `Capture{index: N}` sitting in a function that no `Closure` references is
bounds-checked by nothing. Codegen lowers it to
`load_u64(CLOSURE_ENV_REGISTER, index*8)`
(`src/target/shared/code/builder_values.rs:567-575`)