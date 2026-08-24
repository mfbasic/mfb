# bug-442: record-constructor template inference is poisoned by an `Unknown`-typed field, in field-declaration order

Last updated: 2026-08-15
Effort: small (≤½ day)
Severity: MEDIUM
Class: Compiler correctness (a well-typed, spec-legal generic constructor is rejected; order-dependent, so it also silently "works" for a sibling declaration and misleads)

Status: FIXED
Regression Test: `src/monomorph/helpers.rs::unknown_actual_never_poisons_a_param_binding` (unit) and `src/monomorph/lower.rs::constructor_infers_param_despite_unknown_field_declared_first` (compile). Both RED before the fix, GREEN after.

## STATUS: FIXED

Fixed in `unify_type` (`src/monomorph/helpers.rs`). The doc's preferred **Option A**
(never record an `Unknown` actual as a param binding) was implemented first and
**regressed `collections::flatten`**: `flatten([[], [], ["x"]])`, whose outer literal
types as `List OF List OF Unknown` (only the first, empty, inner is inspected), lost
its provisional `T := Unknown` binding and became a hard `cannot infer template
argument T` compile error. Landed **Option B** instead: an `Unknown` actual is
recorded only as a *provisional* binding that a later concrete actual refines
(`Unknown → concrete`); a concrete binding is never overwritten by an `Unknown`
actual, and two concretes must still agree. This fixes bug-442's order-dependence
while preserving every existing instantiation (including flatten's degenerate
`flatten$Unknown`). Spec updated (`12_monomorphization.md`).

Deviation from the doc: **Option B, not the doc's preferred Option A** — Option A
over-rejected the width-agnostic-native-op case above. The doc's "all-Unknown yields
no substitution" expectation is therefore also not adopted: all-Unknown keeps the
provisional `Unknown`, matching prior behavior (byte-identical goldens).

Verification: bug-442 repro compiles and runs (`0`); `flatten_inline_rt` builds with
byte-identical `.ast`/`.ir` goldens; full `cargo test` green except `artifact_gate_all`
(blocked by cross-session gate concurrency / pre-existing stale cross-arch goldens —
all 9 `.ncode` diffs proven pre-existing via `mine==base` against the pre-fix binary);
acceptance harness's 4 mismatches are a pre-existing test-accept path-corruption bug
on `map-removekey-inplace-rt` (fixture builds fine, goldens byte-identical). Change is
byte-neutral on this tree apart from the intended fix.

## Summary

When a generic record's `[...]` constructor is used inside a generic function,
the monomorphizer infers the record's template argument(s) by unifying each
field's declared type against the corresponding argument's type, accumulating
into one shared substitution map **in field-declaration order**
(`src/monomorph/lower.rs:1327`–`1357`). A field whose argument type is `Unknown`
— most commonly an empty list literal `[]`, which types as `List OF Unknown`
(`src/monomorph/lower.rs:1830`–`1834`) — **records `T := Unknown` as a real
binding** (`src/monomorph/helpers.rs:47`–`53`). Any *later* field that would bind
`T` to a concrete type is then rejected as a conflict (`existing == actual` at
`helpers.rs:48`–`49`, `"Unknown" != "Integer"`), so `T` stays `Unknown` and the
constructor mangles to `<Type>$Unknown`.

The result is **order-dependent**: the exact same fields, reordered so the
concrete-binding field precedes the `Unknown` one, compile and run.

## Minimal reproduction

`Box OF T` has a `List OF T` field and a `FUNC(T) AS Boolean` field. `T` appears
in the constructor only through those two fields; there is no `T`-typed scalar
field.

```basic
IMPORT io

TYPE Box OF T
  items AS List OF T          ' declared FIRST
  fn    AS FUNC(T) AS Boolean
END TYPE

FUNC makeBox OF T(fn AS FUNC(T) AS Boolean) AS Box OF T
  RETURN Box[items := [], fn := fn]
END FUNC

FUNC even(n AS Integer) AS Boolean
  RETURN n MOD 2 = 0
END FUNC

FUNC main AS Integer
  MUT a AS Box OF Integer = makeBox(even)
  io::print(toString(len(a.items)))
  RETURN 0
END FUNC
```

`target/release/mfb build tmp/seq_consumer` reports (the code path is
architecture-neutral front-end monomorphization, so `debug` is expected to match
but was not separately run):

```
error[2-203-0023 TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH]: constructor argument type does not match field type
    Argument 2 for `Box$Unknown` has type FUNC(Integer) AS Boolean, expected FUNC(Unknown) AS Boolean for field `fn`.
error[2-203-0041 TYPE_RETURN_MISMATCH]: return value type does not match function success type
    RETURN value has type Box$Unknown, expected Box$Integer.
```

Note `makeBox` itself inferred `T = Integer` correctly (call-site inference does
descend into the `FUNC(T)` parameter). Only the **constructor inside its body**
failed to resolve `T`, producing `Box$Unknown`.

### Proof it is field-order / `Unknown`-poisoning

Swap the two fields so the concrete-binding field is declared first:

```basic
TYPE Box OF T
  fn    AS FUNC(T) AS Boolean  ' now FIRST — binds T := Integer
  items AS List OF T
END TYPE

FUNC makeBox OF T(fn AS FUNC(T) AS Boolean) AS Box OF T
  RETURN Box[fn := fn, items := []]
END FUNC
```

This **compiles, links, and runs** with no other change. The only difference is
which field the shared substitution map sees first.

## What surfaced it

Building the (pure-MFB, generic) `bindings/simple_event_queue` package. Its
public constructor is exactly this shape:

```basic
EXPORT TYPE EventQueue OF T
  queue    AS List OF T
  dispatch AS FUNC(T) AS Boolean
END TYPE

EXPORT FUNC createQueue OF T(dispatch AS FUNC(T) AS Boolean) AS EventQueue OF T
  RETURN EventQueue[queue := [], dispatch := dispatch]
END FUNC
```

`queue` (a `List`) is declared before `dispatch` (the `FUNC(T)`), so `queue := []`
binds `T := Unknown` and blocks `dispatch`. From the importing executable the
failure surfaces one level out as `TYPE_UNKNOWN_VALUE` on the `createQueue(...)`
initializer ("initializer does not have a known type"), because the package
function's own body cannot instantiate the constructor.

## Root cause

`unify_type` binds a bare template param the first time it sees it and treats
that binding as authoritative thereafter (`src/monomorph/helpers.rs:41`–`53`):

```rust
if params.iter().any(|param| param == pattern) {
    if let Some(existing) = substitutions.get(pattern) {
        return existing == actual;              // <-- refuses to refine
    }
    substitutions.insert(pattern.to_string(), actual.to_string());  // <-- records "Unknown"
    return true;
}
```

Two design points collide:

1. `unify_type` already treats `Unknown` as a **wildcard** in the *non-param*
   tail (`helpers.rs:141`: `pattern == actual || actual == "Unknown"`), but in
   the *param* arm above it records `Unknown` as a concrete binding instead of
   skipping it.
2. Once bound to `Unknown`, a later concrete `actual` is rejected by
   `existing == actual` rather than refining `Unknown → Integer`.

The constructor loop feeds fields in declaration order into one shared map
(`src/monomorph/lower.rs:1337`–`1348`):

```rust
for (field, argument) in fields.iter().zip(lowered_args.iter()) {
    if let Some(actual) = self.expression_type(constructor_arg_value(argument), context) {
        unify_type(&field.type_name, &actual, &template.template_params, &mut inferred);
    }
}
```

and an empty list literal supplies `List OF Unknown`
(`src/monomorph/lower.rs:1830`–`1834`):

```rust
Expression::ListLiteral(values) => values
    .first()
    .and_then(|value| self.expression_type(value, context).map(|t| format!("List OF {t}")))
    .or_else(|| Some("List OF Unknown".to_string())),   // <-- empty list => List OF Unknown
```

So `unify_type("List OF T", "List OF Unknown")` recurses to
`unify_type("T", "Unknown")`, which inserts `T := Unknown`; the subsequent
`unify_type("FUNC(T) AS Boolean", "FUNC(Integer) AS Boolean")` recurses (function
types unify fine, `helpers.rs:130`–`139`) to `unify_type("T", "Integer")`, hits
`existing("Unknown") == "Integer"` → `false`, and leaves `T = Unknown`.

This is not specific to `FUNC(T)` — any pairing where a no-information field
(empty `[]`, empty map/set, or any argument whose `expression_type` is `Unknown`)
is unified for a param **before** a field that carries the concrete type is
enough to poison inference.

## Proposed fix

Either change makes field order irrelevant and fixes both `Box` and
`EventQueue`; option A is the smaller and more targeted:

- **A. Do not record an `Unknown` actual as a param binding.** In the param arm
  of `unify_type` (`helpers.rs:47`–`53`), when `actual == "Unknown"` treat it as
  a wildcard: return `true` without inserting. This mirrors the existing
  wildcard behavior at `helpers.rs:141` and keeps `Unknown` from ever occupying
  a param slot.

- **B. Let a concrete actual refine an existing `Unknown` binding.** When
  `substitutions.get(pattern) == Some("Unknown")` and `actual != "Unknown"`,
  overwrite with `actual` and return `true`.

Prefer **A** (an `Unknown` actual never carries information, so it should never
win a param slot). Confirm A also leaves the genuine-conflict case intact
(`unify T:=Integer` then `unify T:=String` must still fail), and that the
all-`Unknown` case (no field ever supplies a concrete type) still yields no
substitution so the existing expected-type path / diagnostics are unchanged.

Do NOT "fix" this by reordering user field declarations or by adding a dummy
`T`-typed constructor argument — those are workarounds that leave the inference
gap in place for the next generic record.

## Regression test

RED at HEAD, GREEN after the fix. Add both:

1. **Unit (tightest), `src/monomorph/helpers.rs` tests** — assert that
   `unify_type` binds `T := Integer` regardless of the order in which an
   `Unknown` and a concrete actual are unified for the same param:

   ```rust
   // Unknown must not poison a later concrete binding, in either order.
   let mut subs = HashMap::new();
   assert!(unify_type("List OF T", "List OF Unknown", &["T".into()], &mut subs));
   assert!(unify_type("FUNC(T) AS Boolean", "FUNC(Integer) AS Boolean", &["T".into()], &mut subs));
   assert_eq!(subs.get("T"), Some(&"Integer".to_string()));
   ```

2. **Compile test** mirroring the `Box` repro (a monomorph `lower.rs` test in the
   style of `generic_type_inferred_from_constructor_arguments`, or a
   `tests/rt-behavior` fixture) with the `List OF T` field declared *before* the
   `FUNC(T)` field, asserting the constructor instantiates to `Box$Integer`
   rather than `Box$Unknown`.

## Notes / links

- Surfaced while building `bindings/simple_event_queue` (a generic source
  library package). That package is intentionally uncommitted pending this fix —
  its `createQueue` cannot type until the constructor inference is corrected.
- Related, but distinct: bug-142 (completed) was a `FOR EACH` in-place-append
  use-after-free — unrelated mechanism; referenced only because this bug was
  first mis-numbered 142.
```
