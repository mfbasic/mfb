# bug-73: A collection of function values type-checks but cannot be lowered — `List OF FUNC(...)` fails codegen with an internal, location-less error

Last updated: 2026-07-09
Effort: medium (1h–2h) to reject cleanly; large (3h–1d) to support

A `List` or `Map` whose element/value type is a function value passes the whole
front end — parse, resolve, typecheck, monomorphization, IR verification — and then
dies in the native backend with an internal message that carries no source
location, no diagnostic code, and no guidance:

```
error: native collection packed payload does not support type 'FUNC(Integer, Integer) AS Integer'
       while lowering bind fns AS List OF FUNC(Integer, Integer) AS Integer
```

Nothing in the language specification says a collection may not hold a function
value. `04_types.md` lists function/lambda types only as *non-defaultable* (so a
`MUT` binding needs an initializer), and `List OF T` is otherwise unrestricted. A
user who writes a list of callbacks therefore gets a compiler-internal failure at
the very last stage, after every diagnostic layer has approved the program.

This is not a memory-safety or wrong-answer bug: the build fails, loudly. It is a
**contract** bug — the front end accepts a program the backend cannot lower — and a
**diagnostics** bug: the failure is an unstructured `error:` string rather than a
located diagnostic.

The single correct behavior a fix produces: either (a) a collection of function
values lowers and runs, or (b) it is rejected by the front end with a located,
coded diagnostic naming the element type — and the specification states which. The
current state, where the front end accepts what the backend refuses, is what must
end.

References:

- `src/docs/spec/language/04_types.md` §"Defaultability" — the only place function
  types are restricted, and only with respect to defaults.
- `src/docs/spec/memory/…` (`memory_layouts.md`, Scalar Storage) — the packed
  collection payload rules the backend enforces.
- Found while fixing bug-26 (`function_parts` mis-parsing a nested `FUNC` type
  argument): the natural reproduction for that bug — `collections::transform` over a
  `List OF FUNC(A, B) AS C` — cleared overload resolution after the fix and then hit
  this wall.
- Related but distinct: bug-72 (calling a function value never links). A fix to
  bug-72 does not touch this; a fix to this does not touch bug-72.
- Memory note: `bugs-20-39-fixed`.

## Failing Reproduction

```basic
' src/main.mfb
IMPORT io

FUNC add(a AS Integer, b AS Integer) AS Integer
  RETURN a + b
END FUNC

FUNC main() AS Integer
  MUT fns AS List OF FUNC(Integer, Integer) AS Integer = [add]
  io::print(toString(len(fns)))
  RETURN 0
END FUNC
```

```
$ mfb build .
error: native collection packed payload does not support type 'FUNC(Integer, Integer) AS Integer' while lowering bind fns AS List OF FUNC(Integer, Integer) AS Integer
```

- Observed: exit 1 from the native backend; no source line, no diagnostic code, no
  `.out`. `mfb build -ast -ir` on the same program **succeeds**, proving every
  front-end stage accepts it.
- Expected: either the program builds and prints `1`, or `mfb build` reports a
  located diagnostic such as "a collection element may not be a function value" at
  `src/main.mfb:8`.

Every collection shape fails the same way:

| Program | Result |
| --- | --- |
| `MUT fns AS List OF FUNC(Integer, Integer) AS Integer = [add]` | fails ✗ (at the bind) |
| `MUT fns AS List OF FUNC(Integer) AS Integer = []` then `collections::append(fns, inc)` | fails ✗ (at the append) |
| `LET m AS Map OF String TO FUNC(Integer, Integer) AS Integer = Map … { "a" := add }` | fails ✗ (at the bind) |

Contrast cases that work correctly today, and bound the bug:

- A **record field** of function type: `TYPE Handler` with `fn AS FUNC(Integer) AS
  Integer`, constructed as `Handler[inc]`, builds and runs. Record fields are 8-byte
  slots and accept the function value without ceremony.
- Binding, returning, capturing, and passing function values
  (`tests/rt-error/functions/lambda-capture-valid`).
- Every other element type: scalars, `String`, records, unions, resources, and
  nested collections all have a packed-payload rule.

Platform-independent: the rejection is in the shared code builder, before any
per-arch emission.

## Root Cause

The native collection layout requires every element type to have a *packed payload*
classification, decided by three predicates in
`src/target/shared/code/builder_collection_layout.rs`:

- `is_pointer_collection_payload_type` (`:24`) — resource handles and non-flat
  nested collections; a single 8-byte pointer slot.
- `inline_collection_payload_size` (`:4`) — records and unions; `8 * fields`.
- the scalar arms (`Boolean`/`Byte`/`String`/`Integer`/`Float`/`Fixed`).

A function type matches none of them, so it falls to the `other =>` arm and returns
the error, at all three payload sites: `:1488`, `:1601`, `:1708`. There is no
`FUNC(` case anywhere in the file.

Nothing upstream rejects the type. `ir::verify` checks `is_defaultable` for `MUT`
bindings without initializers, which is a different predicate — a function type is
non-defaultable but the reproduction supplies an initializer, so nothing fires. The
type checker propagates `List OF FUNC(...)` without complaint; `builtins::general`'s
higher-order resolvers happily resolve `transform`/`filter` over such a list (see
bug-26). Every layer says yes until the last one says no.

Why the contrast cases are immune: a record field is stored by
`inline_collection_payload_size`'s `8 * fields.len()` rule, which never inspects the
field's type; a bare `FUNC`-typed local is a register/stack value with no packed
representation involved.

## Goal

Exactly one of the following, decided in Open Decisions before Phase 2:

- **(a) Support.** `List OF FUNC(...)` and `Map OF K TO FUNC(...)` lower, store, and
  read back function values; the reproduction prints `1`, and a stored value can be
  retrieved and called (which additionally requires bug-72).
- **(b) Reject.** A collection element/key/value of function type is rejected by
  `ir::verify` with a located, registered diagnostic code, and
  `src/docs/spec/language/04_types.md` states the restriction. The backend's
  `other =>` arm becomes unreachable-by-construction for this input.

Either way: **no program is accepted by the front end that the backend cannot
lower.**

### Non-goals (must NOT change)

- Record fields of function type (correct today).
- The packed-payload rules for every existing element type; no layout change to any
  collection that compiles today, and no golden shift for them.
- Passing function values to `collections::transform`/`filter`/`reduce` — those take
  the function as an *argument*, not as an element.
- **Tempting wrong fix, forbidden:** deleting or weakening the backend's `other =>`
  error arm so an unsupported type silently lowers as a raw 8-byte word. A closure
  is a pointer to a heap-allocated environment block; storing it as an opaque word
  without copy/drop handling leaks its environment or frees it twice. If option (a)
  is chosen, the ownership work is the fix — not a widened `is_pointer_...` predicate.
- Equally forbidden: "fixing" this by narrowing the higher-order resolvers so
  `transform` over a function-valued list no longer resolves (that would re-break
  bug-26 to hide this bug).

## Blast Radius

Searched for every site that classifies a collection payload type or that could
admit a function-typed element:

- `builder_collection_layout.rs:1488`, `:1601`, `:1708` (the three `other =>` arms) —
  the observed failure; fixed or made unreachable by this bug.
- `builder_collection_layout.rs:24` `is_pointer_collection_payload_type`,
  `:4` `inline_collection_payload_size`, `:45` `collection_payload_alignment` — the
  predicates that would have to learn about function values under option (a). In
  scope for (a); untouched under (b).
- `src/ir/verify/mod.rs:is_defaultable` — adjacent element-type predicate, and the
  natural home for a (b) rejection. Latent: it does not reject function-typed
  elements today, and must not be conflated with defaultability (a function element
  with an initializer is still non-defaultable).
- `src/builtins/general.rs` `resolve_transform`/`resolve_filter`/`resolve_for_each`/
  `resolve_reduce` — resolve over function-valued lists since bug-26. Unaffected as
  code; under option (b) they become resolvable-but-unconstructible, which is fine
  (the *list* is rejected, not the resolver).
- `TYPE` record fields (`builder_*` record paths) — unaffected: a record's payload
  size ignores field types, and a field of function type builds and runs today.
- Thread channels / `Map` keys — a function value is not comparable, so a function
  *key* is separately rejected by the comparable-key rule. Unaffected; the bug is
  about element and map-*value* positions.

## Fix Design

**Option (b), reject (recommended, medium).** Add a rule to `ir::verify` that a
`List` element type, a `Map` value type, and any nested composition thereof may not
be a function type, reported at the binding/call site with a registered diagnostic
code (new code in the `TYPE_*` family; the Constant Registry in
`src/docs/spec/diagnostics/01_rule-codes.md` is the build input, so it must be added
there in the same change). Document the restriction in `04_types.md` alongside
defaultability. The backend's `other =>` arm stays as a backstop.

**Option (a), support (large).** Function values are 8 bytes (a code pointer, or a
pointer to a closure block). The blocker is not the slot: it is ownership. A closure
stored into a collection must be copied on insert and freed on scope-drop with the
rest of the block, exactly as a `String` or a nested collection is, and the escape
analysis that decides `resource_owners` has no notion of a closure environment
escaping into a collection. Under (a): extend the three predicates to classify
`FUNC(` as a pointer payload, then thread closure environments through
copy-insertion (`lower_value_owned`) and `ActiveCleanup::OwnedValue`. Both bug-72
and a runtime test that *calls* an element are required for (a) to be provable.

The recommendation is (b) now — it closes the contract hole in a session and costs
nothing if (a) is implemented later — with (a) filed as a feature plan if desired.
The correctness risk under (b) is entirely in the reach of the new rule: it must
catch nested compositions (`List OF List OF FUNC(...)`, `Map OF String TO List OF
FUNC(...)`) and must not catch a record whose *field* is a function type.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/syntax/collections/collection-of-function-invalid` with the three
      reproduction shapes (list literal, append, map value) plus the two contrast
      cases (record field, `transform` argument). Confirm today's behavior: the
      front end accepts, `mfb build` fails with the internal message.
- [ ] Add an `ir::verify` unit test asserting the rejection (option b) or a lowering
      test asserting the payload classification (option a).
- [x] Blast-radius audit complete (above).

Acceptance: the fixture reproduces the internal error; the audit has a verdict per
site.
Commit: —

### Phase 2 — the fix

- [ ] Decide (a) vs (b) — see Open Decisions.
- [ ] (b): add the element-type rule to `ir::verify`, register the diagnostic code in
      `src/docs/spec/diagnostics/01_rule-codes.md`, and make it reach nested
      compositions while sparing record fields.
- [ ] (a): classify `FUNC(` as a pointer payload in the three predicates and thread
      closure-environment copy/drop through the collection paths.

Acceptance: the Phase 1 fixture reports a located diagnostic (b) or runs and prints
`1` (a); the contrast cases are unchanged; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate goldens; confirm the deltas are the new fixture only (no existing
      collection's layout moves).
- [ ] Update `src/docs/spec/language/04_types.md` (and the diagnostics spec under b).
- [ ] `scripts/test-accept.sh` and `cargo test`.

Acceptance: full suite green; golden deltas confined to the new fixture; the
reproduction no longer reaches the backend's `other =>` arm.
Commit: —

## Validation Plan

- Regression test(s): `tests/syntax/collections/collection-of-function-invalid`
  (option b, a `build.log` golden with the diagnostic) or
  `tests/rt-behavior/collections/collection-of-function-rt` (option a, a `.run`
  golden). Plus the `ir::verify` unit test.
- Runtime proof: under (a), store two functions in a list, retrieve and call one,
  and print its result — which also requires bug-72. Under (b), the runtime proof is
  that no such program compiles, and the contrast fixtures still run.
- Doc sync: `04_types.md` states the rule; under (b), the diagnostics Constant
  Registry gains the code.
- Full suite: `scripts/test-accept.sh target/debug/mfb target/accept-actual` and
  `cargo test`.

## Open Decisions

- **Support or reject?** Recommended: **(b) reject now**, and file support as a
  feature plan. Supporting function values as collection elements is a real feature
  with an ownership design (closure environments in packed payloads), not a bug fix;
  shipping it under a bug number would smuggle a language extension past review.
  Alternative: (a) directly, accepting the larger scope. Blocked either way on
  bug-72 for the "store then call" story to be usable at all.

## Summary

The engineering risk is in the *decision*, not the code: (b) is a contained verifier
rule plus a diagnostic code, and (a) is a closure-ownership design that touches
copy-insertion and scope-drop. The bug as filed is that the front end accepts what
the backend refuses; either resolution closes it. No existing collection layout or
record-field behavior may move.
