# bug-368: the general `is*` predicates are not usable as function values anywhere, and fail three different ways depending on the call site

Last updated: 2026-07-19
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Missing capability + internal error leaked to the user

Status: FIXED (2026-07-19)
Regression Test: `tests/rt-behavior/general/builtin-predicate-as-value-rt`

`isEven`, `isOdd`, `isPositive`, `isNegative`, `isZero`, `isNumeric`, `isEmpty`
and `isNotEmpty` are ordinary single-argument predicates in the language surface
(§18, "general built-ins"). They are **not** usable as function values. A
reference to one in a value position fails — and *which* failure you get depends
on the call site, not on anything the user wrote:

| expression | result |
|---|---|
| `collections::filter(xs, isEven)` | **works** |
| `collections::transform(xs, isEven)` | `TYPE_UNKNOWN_VALUE` — "Initializer for binding does not have a known type" |
| `collections::any(xs, isEven)` | `error: NIR local reference 'isEven' does not resolve` |
| `collections::all(xs, isEven)` | same internal error |
| `collections::findIndex(xs, isEven)` | same internal error |
| `collections::partition(xs, isEven)` | same internal error |
| `LET f AS FUNC(Float) AS Boolean = isPositive` | `2-203-0043 TYPE_UNKNOWN_VALUE` |

Verified on macos-aarch64, 2026-07-19, for `isEven`, `isOdd`, `isPositive`,
`isNegative` and `isZero`, over `List OF Integer`, `List OF Float` and
`List OF Fixed`. The element type makes no difference; the call site is the only
variable.

**This is a compiler defect, not a language rule.** A function should work
everywhere a function works. That `filter` accepts `isEven` and the neighbouring
`any` does not is the clearest possible statement that the inlining is an
implementation detail leaking into the surface.

`mfb man collections` documented the limitation as though it were intended —

> The numeric/empty filter predicates such as `isEven` are inlined builtins and
> cannot be passed as values; wrap them in a `FUNC` when a predicate argument is
> needed.

— but that sentence described a defect, and did not even describe it
accurately: `filter` took them as values already. It was removed as part of the
fix, not preserved.

## Root cause

Two layers, and both need the fix.

**Lowering.** The predicates are handled purely as *call* special-cases in
`lower_call` ([[src/target/shared/code/builder_values.rs:727-739]]):

```rust
if target == "isEven" && args.len() == 1 {
    return self.lower_integer_parity_predicate("isEven", &args[0], false);
}
if matches!(target.as_str(), "isPositive" | "isNegative" | "isZero") && args.len() == 1 {
    return self.lower_numeric_filter_predicate(target, &args[0]);
}
```

That is the inline fast path, and it is fine — it should stay. What is missing is
the out-of-line counterpart: no callable body is emitted, so no symbol exists for
`NirValue::FunctionRef` to bind, and the reference survives to codegen as an
unresolvable local.

**Types.** `builtin_function_id_for_type`
([[src/builtins/general.rs:150-164]]) is supposed to give a builtin reference a
function type, and it already carries entries:

```rust
(IS_POSITIVE, "Float") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FLOAT),
(IS_POSITIVE, "Fixed") => Some(BUILTIN_FUNCTION_IS_POSITIVE_FIXED),
(IS_NEGATIVE, "Float") => ...
```

with `builtin_function_symbol_for_type` minting `_mfb_builtin_<name>_<type>`
([[src/target/shared/code/data_objects.rs:1230-1237]]). So the machinery for
"a builtin as a first-class value" **already exists**. But even the registered
Float case fails:

```basic
LET f AS FUNC(Float) AS Boolean = isPositive
' error[2-203-0043 TYPE_UNKNOWN_VALUE]: value type could not be determined
```

so nothing reaches it through ordinary inference. `isEven`, `isOdd`, `isEmpty`,
`isNotEmpty` and every `Integer` overload have no entry at all. `filter` works
only because it synthesises the predicate type itself at the call site via
`filter_predicate_type` ([[src/builtins/general.rs:166-171]]) instead of going
through inference — which is exactly why it is the one that works.

## Resolution

Made them real function values. The inline fast path for a direct call is
untouched — it is a genuine optimization and is invisible either way — and the
out-of-line body is emitted on demand wherever the name is used as a value.

The backend turned out to be **already complete**: `lower_builtin_function_wrapper`
emits a body for any unary-Boolean `(name, type)` pair, `builtin_function_refs`
collects exactly those references, and `validate` accepts a `FunctionRef` whose
`builtin_function_id_for_type` resolves. Only the front end was missing, in three
places:

- **`src/builtins/general.rs`** — `isNumeric` was absent from
  `builtin_function_id` while the other seven were present. That single omission
  is why it alone failed even in `filter`. Added `BUILTIN_FUNCTION_IS_NUMERIC`.
- **`src/syntaxcheck/inference.rs`** — new `builtin_predicate_value_type`: an
  identifier naming a general built-in resolves to the function type **expected**
  at that position. Expected-type-driven because a bare `isPositive` is genuinely
  ambiguous across `Integer`, `Float` and `Fixed`; nothing in the reference
  chooses.
- **`src/ir/lower.rs`** — new `builtin_predicate_ref_type`, so the reference
  lowers to `IrValue::FunctionRef` instead of surviving as an undefined `Local`.
  Both consult `filter_predicate_type`, so the type the checker assigns and the
  type the `FunctionRef` carries cannot diverge.

And the reason only `filter` ever worked: it had a **hardcoded special case** in
both `syntaxcheck::check_collections_builtin_call` and `ir::lower`. Those gates
are now the set `builtins::collections::unary_callback_member`
(`filter`/`transform`/`forEach` — `reduce` is excluded, its callback is binary).

### A real defect caught by the suite mid-fix

Widening the `ir::lower` gate to `forEach` broke two existing tests
(`mut_capture_in_foreach_is_by_ref`, `lowers_lambda_mut_byref_capture_in_foreach`).
The special-cased branch bypassed the general argument path, which is what sets
`context.nonescaping_callback` — the licence for a lambda to slot-borrow a `MUT`
capture. `filter` never noticed because it is not a non-escaping callback
position; `forEach` is.

The gate now diverts **only** when the argument is an identifier that actually
resolves to a built-in predicate; a lambda, a named `FUNC` or an already-typed
function value falls through to the general path exactly as before. The tests
were right and the first version of the change was wrong.

### Verification

`tests/rt-behavior/general/builtin-predicate-as-value-rt` — 57 checks: all eight
predicates across `filter`, `transform`, `any` (a source generic, unlike
`filter`), and a bare `LET f AS FUNC(T) AS Boolean` with an indirect call, over
`Integer`/`Float`/`Fixed`/`String`, plus passing a bound value on to a
higher-order call. Every check states its expected result independently, so a fix
that merely compiled but bound the wrong predicate would still fail. 57/57.

3105 Rust tests, 1013 acceptance tests, and `artifact-gate` (1217 goldens,
0 diffs — the inline direct-call path is byte-identical) all pass.

### Documentation

The claim was stated in **ten** places and all were corrected: the seven
per-function man pages, `mfb man general` and `mfb man collections` package
pages, and `18_builtin-functions.md`. They now say the predicates are lowered
inline at a direct call site and out of line as a value, and that a value-position
reference resolves against the expected type.

## Follow-up

`error: NIR local reference '<x>' does not resolve` reaching a user at all is a
second, broader defect — an internal invariant message from NIR lowering with no
code and no source location, the same class as bug-363's `has no data object`.
This bug removed one path to it; the others are unaudited.

A user override that can never be selected under gap-fill (below) is accepted
silently and its body is dead. That deserves its own diagnostic and is not filed.

### Not a bug: gap-fill override

Note in passing, so the next person does not re-file it. A user-defined
`FUNC isPositive(n AS Integer) AS Boolean` is accepted, and then **never runs** —
a direct `isPositive(5)` returns the built-in's answer, not the user's:

```
isPositive(5)  = TRUE      ' user FUNC returns FALSE unconditionally; body never ran
```

That is **specified behavior**: general-builtin resolution is *gap-fill*
(`18_builtin-functions.md:99-103`) — the built-in stays authoritative for the
types it already supports, and an override is consulted only where the built-in
rejects the argument types. Since the built-in handles `Integer`, the user's
overload is correctly unselectable.

It is worth a separate diagnostic that an override can never be selected (it is
silently dead code today), but it is not this bug and the resolution rule should
not be changed.

## Validation Plan

Carried out as written; see §Resolution → Verification for results. The one
change: the regression test landed at `tests/rt-behavior/general/`, not
`tests/rt-behavior/functions/`, to sit with the other general-builtin fixtures.

## Notes

Found while adding artifact coverage for the `collections::` builtins reachable
only from `tests/acceptance/`. The fixture used a predicate named `isPositive`
and hit the internal error; renaming to `isPos` sidesteps it, which is how the
underlying gap stayed invisible.
