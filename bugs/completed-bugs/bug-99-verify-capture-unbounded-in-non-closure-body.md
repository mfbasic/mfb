# bug-99 — `Capture` in a function that is never a closure body escapes bounds check → OOB env read

**Status:** OPEN. Filed 2026-07-10 (goal-02 review, G3).
**Severity:** MED — untrusted `.mfp` can drive an out-of-bounds env-relative
load past `ir::verify`.
**Class:** memory-safety (trust boundary — crafted package).

## Finding

`src/ir/verify/mod.rs:673-675` (`closure_slot_count`) and :3707-3723
(`check_value_captures`).

`check_value_captures` early-returns when `slots` is `None`.
`closure_slot_count` returns `None` in two cases: (a) an ambiguous body — the
case bug-32 fixed, now `iter().min()`; and (b) `closure_counts.get(function)`
is absent, i.e. the function is never targeted by any `Closure` node. In case
(b) a `Capture{index: N}` sitting in that function is bounds-checked by
nothing.

Codegen then lowers the `Capture` to
`load_u64(CLOSURE_ENV_REGISTER, index*8)`
(`src/target/shared/code/builder_values.rs:567-575`) — an env-relative load off
whatever `CLOSURE_ENV_REGISTER` happens to hold in a non-closure frame, at an
attacker-chosen offset.

The legitimate front end never emits a `Capture` in a non-closure-body
function (zero-capture lambdas lower to `FunctionRef` with no `Capture` nodes),
so bounding/rejecting this never rejects valid IR.

## Trigger

Craft a `.mfp` whose exported/called function `f` (named by no `Closure`) has a
body containing `Capture{index: 9999}`. `merge_packages` → `ir::verify::check`
accepts it (closure_slot_count("f") = None → skip), and codegen emits an
out-of-bounds env read.

## Fix sketch

In `check_value_captures`, treat "function has a `Capture` but is not a known
closure body" as a hard verify error (a `Capture` outside any closure context
is malformed IR), rather than skipping the check when `slots` is `None`.

## Prior art

Same OOB-env class as bug-32 / audit-1 PKG-02, but a distinct still-open
trigger: bug-32 addressed the ambiguous-arity `None` path (now `min()`); this
is the *absence* `None` path (function never a closure body), still unguarded.
