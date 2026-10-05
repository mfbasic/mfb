# bug-824: built-in functions reject an `ISOLATED FUNC` where a `FUNC` callback is expected

Last updated: 2026-10-04
Effort: small (guess)
Severity: MEDIUM
Class: Correctness (type rules)

Status: Closed (2026-10-04)
Regression Test: `codegen::registry::tests::bug_824_plain_callback_param_accepts_isolated_concrete`
(registry dispatch, plus the sanity/negative directions); runtime fixtures
`tests/rt-behavior/collections/builtin-callback-isolated-widening-rt` (forEach,
transform, filter, reduce, sortBy) and
`tests/rt-behavior/json/json-parse-reviver-isolated-rt` (the reviver); the
pre-existing `tests/syntax/threads/thread-start-non-isolated-entry-invalid`
pins the untouched negative direction (a plain `FUNC` still rejected where
`ISOLATED` is required).

Found while fixing bug-822 (bug-staging-12).

## Rule (owner, 2026-10-04)

"an ISOLATED FUNC is a FUNC. a FUNC is NOT an ISOLATED FUNC."

## Failing Reproduction

`collections::forEach(xs, isolatedFn)` (and every other built-in with a plain
`FUNC` callback parameter: `transform`, `filter`, `reduce`, `sortBy`, `groupBy`,
`json::parse`'s reviver, …) gives `TYPE_CALL_ARGUMENT_MISMATCH` when `isolatedFn`
is an `ISOLATED FUNC`.

## Root Cause

The registry's `unify_at` deliberately accepts only a plain concrete function
for a plain callback parameter ("a plain callback param only a plain concrete").
That is inconsistent with bug-786, which lets an `ISOLATED FUNC` stand in for a
`FUNC` of the same shape in user calls.

## Goal

- An `ISOLATED FUNC` of the right shape is accepted wherever a built-in expects a
  `FUNC`, and it runs as usual.
- A plain `FUNC` is still rejected where a built-in requires `ISOLATED FUNC`
  (e.g. `thread::start`'s worker).
- One rule shared by user calls and built-ins.
- Spec (language functions / type rules) updated to state the rule once.

## Resolution (2026-10-04)

**The fix.** `unify_at`'s `Func`/`Func` arm (`src/codegen/registry/mod.rs`)
required `isolated == concrete_isolated` exactly. Replaced with the directional
check `!isolated || *concrete_isolated`: a plain pattern (`isolated == false`)
now accepts either an isolated or a plain concrete; an isolated pattern
(`thread::start`'s worker) still accepts only an isolated concrete. This is the
same direction `ir::verify::compat::Scope::assignable` already applies for
`LET`/argument/return/field positions (bug-786) — now one rule, stated once in
`mfb spec language functions` §"Isolation widens in one direction, everywhere a
callable type is matched" and cross-referenced from `mfb spec language
templates`.

Two native-lowering guards hard-coded the old non-isolated shape and had to
widen too, now that the registry lets an isolated argument reach them:
`CodeBuilder::require_direct_callable` (`src/codegen/builtins/collections/gen_flow.rs`,
shared by `filter`/`transform`/`reduce`/`reduceRight`/`sortBy`/`partition`/
`mapValues`/`findLastIndex` and their in-place variants) and `lower_for_each`'s
own duplicated inline check (`src/codegen/builtins/collections/func_for_each.rs`).
Both now match `ParameterType::Func(_, _, _)` — isolated or not is the same
capture-free closure-struct shape to `emit_direct_callable_branch`, matching
bug-822's finding that a call through an `ISOLATED FUNC` value needs no special
ABI handling. `groupBy` needed no change: it is written in MFBASIC and calls
`transform` internally, so it inherits the fix through the ordinary call path.
`json::parse`'s reviver needed no native-lowering change either — the reviver
call itself (`__json_revive`) is an ordinary function-value call already
isolated-aware since bug-822; only the registry's outer `unify_at` dispatch for
`json::parse` gated it.

**Validation.** RED confirmed on the parent compiler before the fix (both the
new unit test and both new fixtures fail for the documented
`TYPE_CALL_ARGUMENT_MISMATCH` reason); GREEN after.
`cargo test --release --bin mfb` (targeted): `codegen::registry::` (57 passed),
every `isolated`-named test (19 passed), every `collections`-named test (41
passed) — all green, no existing test edited.
`scripts/test-accept.sh` over
`*isolated* *collections* *forEach* *transform* *filter* *reduce* *sortBy*
*groupBy* *json* *callback* *thread* *func* *lambda* *closure* *overload*
*callable* *template* *generic*`: 756 ran, passed (one transient mismatch on an
earlier run did not reproduce on rerun).

**Reach check.** Every project under `tests/` and `examples/` (1660 total: 1658
common + the 2 new fixtures) built with `mfb build -q -ast -ir` on both the
parent and the fixed compiler; the 1658 common projects produced byte-identical
exit status and AST+IR+stderr on both compilers — no existing build, AST or IR
changed. Only the 2 new fixtures differ (absent from the parent tree; pass on
the fixed one). Expected golden moves: none.
