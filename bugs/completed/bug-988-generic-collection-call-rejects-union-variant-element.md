# bug-988: `collections::append`/`prepend`/`insert`/`set` (and every generic `T` slot) reject a union VARIANT value where the collection's element type is the union

Last updated: 2026-10-07
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Fixed (bs18-agent-j), including `reduce`/`reduceRight` (owner decision 2026-10-10)
Regression Test: tests/rt-behavior/collections/union-variant-element-insert-rt, tests/syntax/collections/func_collection_append_union_list_var_invalid, `codegen::registry::tests::a_top_level_variable_bound_to_a_union_accepts_its_variant`, tests/rt-behavior/collections/union-variant-reduce-rt, `codegen::registry::tests::reduce_widens_a_variant_initial_value_and_never_narrows_a_union`

A generic collections member whose element slot is a bare type variable `T` rejects a value of a
union's **variant** type when the collection's element type is that union. With
`MUT shapes AS List OF Shape` (`UNION Shape  Circle  Square`), `collections::append(shapes, Square[2.0])`
fails to compile with `TYPE_CALL_ARGUMENT_MISMATCH`, as do `prepend`, `insert`, `set` (list and map
forms), `getOr` (list and map forms), `repeat`, and `reduce` with a variant initial value. The same
value is accepted everywhere else a `Shape` is expected: a list literal `[Circle[1.0], Square[2.0]]`
into `List OF Shape`, `LET s AS Shape = Square[2.0]`, a user `FUNC take(s AS Shape)` called with a
`Square`, and a map literal `{ "b" := Square[2.0] }` into `Map OF String TO Shape`. Built-in
(registry) unions fail the same way: `collections::append(items, canvas::Rectangle[...])` into
`List OF canvas::DrawItem` is rejected. The user-side workaround is to bind the value to the union
first (`LET d AS Shape = Square[2.0]` then `append(shapes, d)`).

**Correct behavior:** at a top-level `T` parameter, a value whose type is a variant of the union that
`T` is bound to is accepted, exactly as it is at a `LET`/user-argument/list-literal slot. The call
compiles, lowering receives `T`'s binding (`Shape`) as the argument's expected type and wraps the
value in the union's tag, and at runtime the inserted element is a `Shape` that `MATCH` routes to its
`Square` arm. A `List OF Square` **value** (a variable, not a literal) passed where `List OF Shape` is
expected is NOT made acceptable by this fix: no conversion exists for it (see Blast Radius).

References:

- `src/docs/spec/architecture/22_type-inference.md` § "Call arguments — the parameter type IS the
  expected type", rule 3: "for a **generic** parameter (`collections::append`'s `List OF T`), the type
  the registry's own overload selection resolves it to from the call's actual argument types, with
  the call's own expected type seeding the unification (`resolved_parameter_type`); an ambiguous
  position — one where the surviving overloads disagree — yields no expected type rather than a
  guess". The same section, rule 2: the agreed type "is what decides union wrapping, so a union-typed
  parameter still receives a tagged union".
- `src/docs/spec/architecture/22_type-inference.md` § "Structural Assignability", IR-verifier rule 5:
  "Accept an actual variant of the expected union, using its qualified identity or built-in bare
  leaf. Otherwise reject." Every *assignment* position, including "call argument", asks `assignable`
  (= `compatible` + literal coercions), so a variant into a union-typed argument slot is assignable.
- `src/docs/spec/architecture/22_type-inference.md` opening paragraph: "the only "widening" steps are
  literal coercion (asymmetric, literal-shapes only) and union-variant subsumption" — union-variant
  subsumption is a sanctioned widening; the generic matcher is the one seam that does not apply it.
- Found while building `examples/canvas-demo` (appending `canvas::Picture` etc. to
  `List OF canvas::DrawItem`).

## Failing Reproduction

`/tmp/cdrepro2/project.json`:

```json
{ "name": "cdrepro2", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
```

`/tmp/cdrepro2/src/main.mfb`:

```
IMPORT collections
IMPORT io

TYPE Circle
  r AS Float
END TYPE

TYPE Square
  side AS Float
END TYPE

UNION Shape
  Circle
  Square
END UNION

FUNC main AS Integer
  MUT shapes AS List OF Shape = [Circle[1.0]]
  shapes = collections::append(shapes, Square[2.0])
  io::print(toString(len(shapes)))
  RETURN 0
END FUNC
```

```
target/release/mfb build /tmp/cdrepro2      # built at 468ff0f94, macOS aarch64
```

- Observed (exit 1):
  ```
  /tmp/cdrepro2/src/main.mfb:19 error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]: function call argument type does not match parameter type
                 Call to `collections.append` has argument type(s) (List OF Shape, Square), expected List OF T, T or List OF T, List OF T.
  ```
- Expected: builds; running prints `2`; the appended element `MATCH`es `CASE Square(q)`.

Variant matrix (each a one-function `main` over the declarations above, built with the same binary;
harness `/tmp/bug988/run.sh`, not checked in). `P` = `MUT shapes AS List OF Shape = [Circle[1.0]]`.

| Call | Result today |
| --- | --- |
| `append(shapes, Square[2.0])` | `TYPE_CALL_ARGUMENT_MISMATCH` (List OF Shape, Square) ✗ |
| `append(shapes, [Square[2.0]])` (list literal) | `TYPE_CALL_ARGUMENT_MISMATCH` (List OF Shape, List OF Square) ✗ |
| `prepend(shapes, Square[2.0])` | `TYPE_CALL_ARGUMENT_MISMATCH` ✗ |
| `insert(shapes, 0, Square[2.0])` | `TYPE_CALL_ARGUMENT_MISMATCH` ✗ |
| `set(shapes, 0, Square[2.0])` | `TYPE_CALL_ARGUMENT_MISMATCH` ✗ |
| `set(m, "b", Square[2.0])`, `m AS Map OF String TO Shape` | `TYPE_CALL_ARGUMENT_MISMATCH` ✗ |
| `getOr(shapes, 5, Square[2.0])` | `TYPE_CALL_ARGUMENT_MISMATCH` ✗ |
| `getOr(m, "z", Square[2.0])` (map) | `TYPE_CALL_ARGUMENT_MISMATCH` ✗ |
| `repeat(shapes, Square[2.0], 2)` | `TYPE_CALL_ARGUMENT_MISMATCH` ✗ |
| `reduce(shapes, Square[2.0], LAMBDA(acc AS Shape, s AS Shape) -> s)` | `TYPE_CALL_ARGUMENT_MISMATCH` (List OF Shape, Square, FUNC(Shape, Shape) AS Shape) ✗ |
| `append(items, canvas::Rectangle[...])`, `items AS List OF canvas::DrawItem` (app mode) | `TYPE_CALL_ARGUMENT_MISMATCH` (List OF canvas.DrawItem, canvas.Rectangle) ✗ |

Contrast cases that work today (regression guards):

| Program fragment | Result |
| --- | --- |
| `LET shapes AS List OF Shape = [Circle[1.0], Square[2.0]]` | builds, prints `2` ✓ |
| `LET s AS Shape = Square[2.0]` | builds ✓ |
| user `FUNC take(s AS Shape)` called as `take(Square[2.0])` | builds, prints `1` ✓ |
| `Map OF String TO Shape { "a" := Circle[1.0], "b" := Square[2.0] }` | builds, prints `2` ✓ |
| `LET d AS Shape = Square[2.0]` then `append(shapes, d)` | builds, prints `2` ✓ |
| `LET d AS Shape = …` then `getOr(shapes, 5, d)` / `repeat(shapes, d, 2)` / map `set(m, "b", d)` | builds ✓ |

## Root Cause

Two seams run the same registry unifier and both refuse the variant.

1. **Acceptance (shape pass).** `src/ir/shape.rs:check_builtin_call`, `collections` branch, calls
   `builtins::resolve_call_return_type_with_kinds(canonical, &arg_types, true, …)` with the
   arguments' bottom-up types (`List OF Shape`, `Square`). For a registry member that routes to
   `src/codegen/registry/mod.rs:resolve_call_typed` → `resolved_return_type` → `RegistryFunction::resolve`
   → `match_overload`, which runs `unify(&param.ty, arg, …)` per parameter. Parameter 0
   (`List OF T`) binds `T := Shape`. Parameter 1 (`T`) meets the existing binding in
   `src/codegen/registry/mod.rs:unify_at`, `ParameterType::Var` arm:

   ```rust
   Some(bound) => resource_base_eq(bound, concrete),
   ```

   `resource_base_eq(Shape, Square)` is plain `==` after stripping `STATE`, so it is `false`; every
   overload fails and the shape pass emits `TYPE_CALL_ARGUMENT_MISMATCH`. A re-occurring type
   variable is compared by **identity**, and union-variant subsumption (shape `compatible`'s union arm
   in `src/ir/shape.rs:compatible`; registry `value_type_accepts`) is never consulted for it.
   `value_type_accepts` *does* implement variant acceptance, but only in `leaf_matches`, for a
   parameter spelled as a concrete registry nominal, never for a `Var` rebind. That is why
   `canvas.DrawItem` fails too.

   The unifier also cannot answer for a **user** union by construction. The `TypeKinds` oracle that
   `ShapeKinds` passes (`src/codegen/builtins/mod.rs:TypeKinds`) carries only `is_enum`, and
   `resolve_call_return_type_with_kinds` does not forward it to `registry::resolve_call_typed` at all
   (only `general` uses it). The registry has no view of `UNION Shape`'s variant list.

2. **Expected type (lowering).** Even if the shape pass accepted the call,
   `src/ir/lower.rs:call_argument_expected_type` rule 3 → `resolved_generic_argument_type` →
   `src/codegen/registry/mod.rs:resolved_parameter_type` runs the same `unify` (lenient). The call's
   expected type seeds `T := Shape` (the assignment target), and then `Square` fails the same rebind,
   so every overload `continue`s, the function returns `None`, and argument 1 lowers with **no**
   expected type. That leaves a bare `Square` record, not a tagged `Shape`.
   `src/codegen/builtins/collections/gen_mutate.rs:collection_argument_as_list_slot` then requires
   `item.type_ == element_type` exactly and fails with the internal
   `native collection list item must be Shape, got Square`. So the fix must make rule 3 answer
   `Shape` for the variant position. Widening acceptance alone would turn a clean diagnostic into an
   internal codegen error.

Why the contrast cases are immune: a `LET`, a user-function argument and a list/map-literal element
each have a **concrete** expected type (`Shape`) independent of unification. They go through
`expression_compatible`/`compatible` (union arm) for acceptance and `lower_expression_with_expected`
for wrapping. The workaround `LET d AS Shape` makes argument 1's bottom-up type `Shape`, so the rebind
is `Shape == Shape`.

The rebind rule dates from `e82a9c653` ("codegen(collections): migrate onto the clean-room
registry", `git log -S 'Some(bound) => resource_base_eq(bound, concrete)'`). Its comment cites
`general::element_accepts_item` as the mirrored contract, but no such function exists any more
(`grep -rn 'fn element_accepts_item' src` → no hits; only the two comments in
`src/codegen/registry/mod.rs` mention it). The fix should correct that dangling citation.

## Goal

- The reproduction and every ✗ row of the variant matrix for which `T` is not required to be
  comparable (`append` element and list-literal forms, `prepend`, `insert`, list/map `set`, list/map
  `getOr`, `repeat`, `canvas::DrawItem` append) build, and the inserted value is a tagged union
  element. `MATCH` on it takes the variant's arm and reads the variant's fields.
- `resolved_parameter_type("collections.append", 1, [List OF Shape, Square], Some(List OF Shape))`
  answers `Shape` (with a user-union-aware oracle), so lowering wraps.
- `reduce` with a variant initial value either builds (per the Open Decision) or keeps its current
  diagnostic by recorded decision. It must not be left silently inconsistent.

### Non-goals (must NOT change)

- **No list covariance without conversion.** `append(shapes, sq)` with `sq AS List OF Square` (a
  variable) must stay a compile error. No runtime conversion of a `List OF Square` payload into
  tagged `Shape` elements exists, so accepting it would produce the silent miscompile described under
  Blast Radius. The tempting wrong fix is to make the `Var` rebind accept a variant at **any**
  depth. Inside `List OF T` that admits exactly this case, so it is forbidden. The variant
  allowance applies only where lowering can wrap the value: a top-level `T` argument, or a list
  **literal** whose elements are lowered against the expected element type.
- Comparability gates are unchanged. `contains`/`find`/`replace`/Set `add`/`remove` over a union
  element type stay rejected, with `TYPE_REQUIRES_COMPARABLE` instead of the argument mismatch (see
  Blast Radius).
- The STATE/ownership-agnostic resource rebind (bug-427) and the `Unknown`-refinement rule in
  `unify_at` are unchanged. Two unrelated nominals (`Circle` vs `Square` with `T` bound to `Circle`)
  still mismatch, and a union bound where `T` holds a **different** union still mismatches.
- No least-upper-bound inference: `append([Circle[1.0]], Square[2.0])` with no union anywhere in the
  call (or its expected type) stays a mismatch. The union must come from an argument or the seed;
  it is never invented.
- The existing `func_collection_*_invalid` syntax goldens keep their diagnostics.

## Blast Radius

Search: every registry parameter spelled with a bare `ParameterType::var(..)` (`grep -c 'ty:
ParameterType::var("[A-Z]")' src/codegen/builtins/collections/func_*.rs`, plus `grep -rln
'ty: ParameterType::var(' src/codegen/builtins` outside `collections/`). Verdicts measured with the
variant matrix above.

- `collections::append` (element and list-literal overloads), `prepend`, `insert`, `set` (list and
  map), `getOr` (list and map), `repeat`: fixed by this bug (all observed ✗ today, all accept a
  `Shape`-typed value today).
- `collections::reduce` / `reduceRight` (`U` bound to the variant by the initial value, then
  contradicted by the callback's `Shape`): same root cause, opposite binding order (variant first,
  union second). In scope per the Open Decision below.
- `collections::contains`, `find`, `replace` (list), `add`/`remove` (set): same mismatch today with a
  variant argument, but the union element type is not comparable. A `Shape`-typed argument already
  gets `TYPE_REQUIRES_COMPARABLE` (`contains`, `find`, `replace`, and `Set OF Shape { c }` measured).
  After the fix these must report `TYPE_REQUIRES_COMPARABLE`, not build. That is a diagnostic change
  only, and it must be checked against the goldens.
- `get`, `hasKey`, `removeKey`: the bare `Var` is a key/`K` position or is bound only once. Not
  affected by the variant rebind unless a key type is a union, which is not comparable.
- `testing::expectTrap` / `expectNTrap` (`src/codegen/builtins/testing/func_expect_trap.rs`,
  `func_expect_n_trap.rs`): a single `T` occurrence, so there is no rebind. Unaffected.
- **Latent, related, NOT this bug: list covariance is silently accepted with no conversion.**
  `LET sq AS List OF Square = [Square[2.0], Square[3.0]]` then `LET sh AS List OF Shape = sq`
  builds, and a `FOR EACH s IN sh` / `MATCH s` with `CASE Circle(c)` / `CASE Square(q)` arms prints
  **nothing** (neither arm fires; exit 0). The same loop over a literal
  `LET sh AS List OF Shape = [Square[2.0], Square[3.0]]` prints `2.00` and `3.00` (both measured
  with `/tmp/bug988/{list_var_cov_read,control_read}`). Cause: `src/ir/shape.rs:compatible` recurses
  `ListOf` element-wise into its union-variant arm, and the IR verifier's `compatible` does the same
  (spec "Structural Assignability" rules 2+5). Both are representation-unsound because the payload
  is not rewrapped. Out of scope here because it is a different seam (assignability, not generic
  unification) with its own fix choice (reject vs. convert). Filed as bug-989. This bug's
  Non-goal "No list covariance" exists so the fix does not extend the hole into `append`.

## Fix Design

Give the unifier a union-membership oracle and let a **top-level** `T` occurrence accept a variant of
the union `T` is bound to, keeping the binding as the union.

1. Extend `src/codegen/builtins/mod.rs:TypeKinds` with
   `fn union_accepts(&self, union: &ParameterType, actual: &ParameterType) -> bool`. Implement it in
   every oracle: `ShapeKinds` (`src/ir/shape.rs`) from `self.types`' variant lists, reusing the
   qualified/bare spelling logic in `compatible`'s union arm; `TypeIndex` (`src/ir/lower.rs`);
   `TypeEnv` (`src/ir/verify/compat.rs`); `TypeModel` (`src/codegen/engine/validation/validation.rs`);
   `Monomorphizer` (`src/monomorph/lower.rs`). For a registry union, fall back to the existing
   `registry::value_type_accepts`. `NoTypeKinds` answers registry unions only.
2. Thread the oracle into `registry::resolve_call_typed`, `RegistryFunction::resolve`/`dispatch`/
   `match_overload`, `resolved_parameter_type` and `unify`/`unify_at`. Have
   `resolve_call_return_type_with_kinds` forward its `kinds` to the registry path (today it drops it).
   `resolved_argument_type`/`resolved_generic_argument_type` in lowering pass `TypeIndex`.
3. In `unify_at`'s `Var` arm, track whether the occurrence is **top-level** (the parameter *is*
   the variable) or nested (inside `ListOf`/`MapOf`/`Func`/…). On `Some(bound)`: keep
   `resource_base_eq`. Add: at a top-level occurrence, accept when `kinds.union_accepts(bound,
   concrete)` and leave the binding as `bound`. Lowering's `substitute` then yields the union for
   that position, and `lower_expression_with_expected` wraps the variant.
4. The list-literal form (`append(shapes, [Square[2.0]])`): the shape pass types the literal
   `List OF Square` bottom-up, so it reaches the nested `T` and is (correctly) refused by step 3. In
   `src/ir/shape.rs:check_builtin_call`, when a resolution fails and an argument is a
   `HirExpression::ListLiteral` at a generic position, retype it with the parameter type
   `resolved_parameter_type` answers from the *other* arguments plus the call's expected type, and
   validate it with `expression_compatible` (the same literal path a `LET … AS List OF Shape = [...]`
   takes). Lowering already passes that expected type to the literal (rule 3), so its elements wrap.
5. Fix the dangling `general::element_accepts_item` citations in `unify_at` and `resource_base_eq`
   to name the real contract (bug-427, `resource_base_eq`).

Rejected alternatives:

- **Accept a variant at any depth in `unify_at`.** This admits `append(shapes, sqVariable)`, which
  has no conversion. Forbidden by Non-goals.
- **Accept in the shape pass only** (e.g. retry with `Square` replaced by `Shape`). Lowering's
  `resolved_parameter_type` still declines, the argument lowers unwrapped, and codegen fails with the
  internal `native collection list item must be Shape, got Square`. Acceptance and expected type must
  come from the same matcher.
- **Rebind `T` to the variant and widen the list.** Not expressible: the list is already
  `List OF Shape`, and the spec has no LUB.

Expected output shift: none on existing goldens (they contain no accepted variant-into-generic call)
except any `*_invalid` golden that today expects an argument mismatch for exactly this shape. The
grep in Phase 1 decides whether one exists. `contains`/`find`/`replace` over a union element switch
from `TYPE_CALL_ARGUMENT_MISMATCH` to `TYPE_REQUIRES_COMPARABLE`, which is intended.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/rt-behavior/collections/union-variant-element-insert-rt` (layout as its siblings:
      `project.json`, `src/main.mfb`, `golden/`). Include: `append` element form, `append` list-literal
      form, `prepend`, `insert`, list `set`, map `set`, list/map `getOr` with a variant default,
      `repeat`, a `canvas`-free registry-union case if one exists (else note `canvas::DrawItem` needs
      app mode and cover it with a unit test), and `reduce` with a variant initial value. After each
      insert, `FOR EACH` + `MATCH` prints the arm and field, to prove the element is tagged. Confirm it
      fails today with `TYPE_CALL_ARGUMENT_MISMATCH`.
- [ ] Add registry unit tests next to
      `resolved_parameter_type_infers_a_generic_position_from_the_call` in
      `src/codegen/registry/mod.rs`. Cover: (a) `resolved_parameter_type("collections.append", 1,
      [List OF U, V], Some(List OF U))` answers `U` under an oracle where `V ∈ U`; (b) `[List OF U,
      List OF V]` (non-literal) stays `None`/rejected; (c) `[List OF U, W]` with `W ∉ U` stays
      rejected.
- [ ] Add a syntax-invalid guard
      `tests/syntax/collections/func_collection_append_union_list_var_invalid`:
      `append(shapes, sq)` with `sq AS List OF Square` must keep `TYPE_CALL_ARGUMENT_MISMATCH`.
- [ ] `grep -rln 'Square\|UNION' tests/syntax/collections/*/src` and the
      `func_collection_*_invalid` goldens, to confirm that no golden pins the current mismatch for an
      accepted shape. Record the verdict here.

Acceptance: the new rt fixture and unit test (a) fail for the documented reason; (b), (c) and the
invalid guard pass today and must keep passing.
Commit: —

### Phase 2 — the fix

- [ ] `TypeKinds::union_accepts` + implementations (Fix Design 1).
- [ ] Thread the oracle through the registry matcher and `resolved_parameter_type`. Forward `kinds`
      from `resolve_call_return_type_with_kinds` (Fix Design 2).
- [ ] Top-level-only variant acceptance in `unify_at` (Fix Design 3), plus the variant-first/union-later
      order for `reduce` if the Open Decision keeps it in scope.
- [ ] List-literal retyping in `check_builtin_call` (Fix Design 4).
- [ ] Correct the dangling `general::element_accepts_item` citations (Fix Design 5).
- [ ] Spec sync (Hard Completion Gate): in `src/docs/spec/architecture/22_type-inference.md` rule 3,
      state that a top-level generic position accepts a variant of the union its variable is bound
      to, and that a nested position does not. Check `mfb man collections append` prose ("Any other
      type is a compile-time error") and add the union-variant sentence there per
      `.ai/man-content.md`.

Acceptance: the Phase 1 rt fixture and unit tests pass; the invalid guard still fails to build with
`TYPE_CALL_ARGUMENT_MISMATCH`; the contrast cases still behave as documented; nothing in Non-goals
changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate only goldens the fix moves. The expected delta is `contains`/`find`/`replace`
      over a union element, if any golden has one, switching to `TYPE_REQUIRES_COMPARABLE`. Any other
      delta is a bug-hunt trigger.
- [ ] Run the full suite (never one module) per `.ai/testing-gates.md`.
- [ ] Rebuild `/tmp/cdrepro2` and the `examples/canvas-demo` call sites. Run them and confirm they
      print the expected output.

Acceptance: full suite green; golden deltas are exactly the intended change; the reproduction and
the canvas-demo append compile and run.
Commit: —

## Validation Plan

- Regression test(s): `tests/rt-behavior/collections/union-variant-element-insert-rt`; registry unit
  tests (a)–(c) in `src/codegen/registry/mod.rs`; invalid guard
  `tests/syntax/collections/func_collection_append_union_list_var_invalid`.
- Runtime proof: `mfb build /tmp/cdrepro2 && /tmp/cdrepro2/build/cdrepro2.out` prints `2`, and a
  `MATCH` over the list prints the `Circle` arm, then the `Square` arm with `2.00`.
- Doc sync: `src/docs/spec/architecture/22_type-inference.md` rule 3; the `collections::append`
  (and siblings') man prose.
- Full suite: the repo's full gate per `.ai/testing-gates.md`.

## Owner decision (2026-10-10, after review): the bucket fixes `T`; it never widens to fit the item

"In general for UNIONs: you can fit a member type into a UNION bucket. You can't fit a UNION into a
member bucket." For `append(List OF T, T)`, **`T` must match**: the collection argument is the
bucket and fixes `T`.
- `append(shapes, aSquare)` works: `T` is `Shape`, and a `Square` is wrapped into it.
- `append(squares, aShape)` is a **compile error** (as on main), whatever the `Shape` holds. The
  programmer narrows with `MATCH` and appends in the `Square` arm. There is no implicit run-time
  narrowing (option A; the owner considered and declined a run-time "wrong type" check).
- The result type is never changed to the union to make the call fit. The bs18-agent-j build that
  typed `append(squares, aShape)` as `List OF Shape` (measured by the coordinator:
  `typeName(r) = List OF Shape, len = 2`) is wrong and must be undone.
- The same holds for every member where one argument is the collection being added to or searched
  (`prepend`, `insert`, `set`, `contains`, `find`, `getOr`'s default, map `set`, …): a type
  variable bound from a collection argument's element (or key/value) type is never rebound to a
  union by a later argument.
- `reduce`/`reduceRight` are unchanged by this: there the accumulator the callback declares is the
  bucket, so a variant initial value fits a union accumulator (decision below).

## Owner decision (2026-10-10): `reduce`/`reduceRight` with a variant initial value works

It follows from the bug-989 decision (a variant is accepted wherever its union is expected): when
the initial value is a variant and the callback's accumulator is the union, the accumulator type is
the union and the initial value is wrapped. Implement the "in scope" option below.

## Open Decisions (the `reduce` item is settled above)

- **`reduce`/`reduceRight` with a variant initial value** (`U` bound to `Square` first, then `Shape`
  from the callback). Recommended: in scope. When the bound value is a variant reached at a
  **top-level** occurrence and a later occurrence is a union that accepts it, rebind to the union.
  Every variant-typed value is then still at a wrappable position, so lowering's `substitute` gives
  argument 1 the expected type `Shape`. Alternative: leave it out and document it, since it needs
  order-independent bookkeeping (remember which occurrences were top-level) that the
  append/prepend case does not.
- **List-literal argument** (`append(shapes, [Square[2.0]])`). Recommended: accept via Fix Design 4,
  because it is the same literal shape a `LET … AS List OF Shape = [...]` accepts. Alternative:
  accept only the element form and keep the literal form an error, which is simpler but leaves a
  visible asymmetry.

## Summary

The risk is in the unifier change. It must widen at exactly the positions lowering can wrap: a
top-level `T` (and a list literal), never a nested `T` inside `List OF T`, because the adjacent
list-covariance hole proves that accepting a variant without a conversion silently miscompiles.
Acceptance (shape pass) and the expected type (lowering rule 3) share `unify`, so one oracle-aware
change fixes both. Changing only one of them turns the clean diagnostic into an internal codegen
error. Comparability gates, resource STATE-agnostic rebinding, and the no-LUB rule are untouched.

## Resolution

Fix Design 1–5, with one change to step 4 (below).

1. `TypeKinds::union_accepts(union, actual)` (`src/codegen/builtins/mod.rs`). Its
   default answers the built-in packages' unions (`registry::registry_union_accepts`,
   over `value_type_accepts`), so `NoTypeKinds` and every test oracle need nothing.
   Overridden by `TypeIndex` (lowering: exactly `variant_belongs_to_union`, the
   membership `wrap_union_value` wraps by), `ShapeKinds` (shape pass: the union arm's
   qualified-or-bare variant match), `TypeEnv` (verifier: `union_variants`),
   `TypeModel` (codegen) and `Monomorphizer` (declared unions through `INCLUDES`).
   `FieldTypes` keeps the default: codegen sees the call after lowering wrapped the
   item. Monomorph's `builtin_call_return_type` keeps its kind-less answer and falls
   back to the union-aware one (and the literal relaxation) only for a non-`general`
   call that otherwise does not type, so `firstOf(collections::prepend(base,
   mk(19.0)))` infers `T = Shape` (measured: without it, `cannot infer template
   arguments from Unknown`).
2. The oracle is threaded through `unify_with_kinds` / `unify_at`, `match_overload`
   (`resolve_with_kinds` / `dispatch_with_kinds`), `resolve_call_typed_with_kinds` and
   `resolved_parameter_type`. `resolve_call_return_type_with_kinds` now forwards its
   `kinds` to the registry path (it dropped it). The kind-less entry points
   (`unify`, `resolve`, `dispatch`, `resolve_call_typed`) are unchanged and pass
   `NoTypeKinds`.
3. `unify_at` carries a `Position { kinds, var: VarPosition }`. A `Var` that IS the
   whole parameter, already bound to a union, also accepts a value of one of its
   variants and stays bound to the union; every container, map, result, user-generic,
   callable and thread-slot recursion passes `VarPosition::Nested`, where only
   `resource_base_eq` applies. Lowering's `resolved_generic_argument_type` passes
   `TypeIndex`, so rule 3 answers `Shape` for the item and it lowers wrapped (bug-990's
   wrap does the rest).
4. Changed: the list-literal relaxation lives in ONE function,
   `builtins::union_literal_relaxed_arg_types` (adapted to HIR arguments by
   `ir::lower::union_literal_call_types`), used by every seam that must agree: the
   shape pass (`check_builtin_call`'s collections branch), lowering's return-type oracle
   (`generic_builtin_call_type`, the `abi_inline` and registry arms of
   `expression_type`), rule 3 (`resolved_generic_argument_type`) and monomorph's
   built-in typing. Only when the
   call does not resolve as typed, every non-empty list-literal argument is retyped
   `List OF Unknown`; the call is then accepted only if it resolves AND every element
   of each relaxed literal is the element type the call gives that position or a
   variant of it. A list VALUE is never relaxed. The doc's step 4 ran the shape pass's
   `expression_compatible` instead; that predicate accepts `List OF Square` for
   `List OF Shape` only through bug-989's leak, so it would have tied this fix to the
   hole bug-989 closes, and it is not visible to lowering's typing.
5. The dangling `general::element_accepts_item` citations are gone (`unify_at`,
   `resource_base_eq` now cite bug-427).

Open Decisions:

- **List-literal argument** — accepted, as the Goal lists it
  (`append(shapes, [Square[2.0]])` and the mixed `[Square[4.0], Circle[5.0]]`).
- **`reduce`/`reduceRight` with a variant initial value — NOT changed.** The doc leaves
  it to a recorded decision and none is recorded (`bugs/status.md` "Owner decisions
  recorded" does not list 988). It still reports `TYPE_CALL_ARGUMENT_MISMATCH` (+
  `TYPE_UNKNOWN_VALUE`), same as main (`/tmp/bug988/reduce`). The workaround is
  `LET init AS Shape = Square[2.0]`. The registry unit test pins only the general
  rule it rests on (no least upper bound: a variable bound to the variant does not
  widen to a later union).

### Runtime proof

- `tests/rt-behavior/collections/union-variant-element-insert-rt`: `append`
  (constructor, call result, list literal, mixed literal), `prepend`, `insert`, `set`,
  `repeat`, list `getOr` (hit and miss), map `set` and map `getOr` (hit and miss), and
  the in-place `xs = op(xs, …)` forms; every element `MATCH`es its own arm and prints
  its field. It also covers an unannotated binding of each form and a template
  argument inferred from such a call. `MFB_OPT=0` and `MFB_OPT=3
  scripts/test-accept.sh` both pass. Main: every call is `TYPE_CALL_ARGUMENT_MISMATCH`.
- `--debug`, one program per form (`/tmp/bs18j/b_*.mfb`), once and in a 200-iteration
  loop: `alloc_calls == free_calls`, `live_bytes 0`, `double_free_skips 0` for all 22
  (e.g. `b_append_loop` 1025 / 1025 / 0, `b_appendlit_loop` 2227 / 2227 / 0,
  `b_mapset_loop` 2010 / 2010 / 0).
- `/tmp/bug988` matrix: every ✗ row now builds and runs (`append_list_lit`,
  `append_member`, `prepend`, `insert`, `set`, `map_set`, `getOr`, `map_getOr`,
  `repeat`); `contains`/`contains_member`/`find`/`replace` now report
  `TYPE_REQUIRES_COMPARABLE` (main: `TYPE_CALL_ARGUMENT_MISMATCH`), as intended;
  `append_list_var` still `TYPE_CALL_ARGUMENT_MISMATCH`; `reduce` unchanged; every
  contrast row prints what it did on main.
- `canvas::DrawItem` (app mode, `/tmp/bs18j/m/cv`): `collections::append(items,
  dotc(3.0))` and `append(items, canvas::Rectangle[…])` build, with
  `unionWrap(canvas.DrawItem <- canvas.Circle / canvas.Rectangle)` in the IR; main
  rejects both. Not run (needs a window). `examples/canvas-demo` builds.
- A resource-union variant (`append(hs AS List OF RES Handle, openOne())`) works (both
  elements `MATCH` `fs::File` and are writable) and leaks the 16 B box bug-1012
  describes (272 B live vs 240 B for the bound-union form on main and here).

### Found on the way (not this bug)

`/tmp/bug988/canvas_registry_union` declares a user `TYPE Circle` in an app-mode
program. With it, ANY `canvas::DrawItem` append fails the build with the internal
`canvas: the record 'Circle' has no field 'paint'` — also on main with the
`LET d AS canvas::DrawItem = …` workaround, so it is not caused by this fix. Fixed
separately (see its own commit).

## Golden impact

- `scripts/artifact-gate.sh <mfb> all`: 1651 tests, 2310 goldens, 0 diffs.
- `scripts/diag-set-diff.sh <mfb>`: 652 fixtures with diagnostics, 652 SAME — no
  existing `*_invalid` golden pinned the old mismatch for an accepted shape, and no
  existing golden has a `contains`/`find`/`replace` of a variant over a union list.
- New goldens only: the two new fixtures.
- The registry unit test `a_top_level_variable_bound_to_a_union_accepts_its_variant`
  lands with the fix, not in the RED commit: it calls the oracle-taking entry points
  the fix adds. The RED proof is the rt fixture (main: `TYPE_CALL_ARGUMENT_MISMATCH` on
  every call).

## Resolution, part 2 (2026-10-10): `reduce`/`reduceRight`, and one rule with bug-989

Two things in the resolution above are superseded:

- **The list-literal relaxation is gone.** bug-989 (convert) makes a `List` element a
  convertible position, so `append(shapes, [Square[2.0], Circle[3.0]])` is accepted by
  the same rule that accepts `append(shapes, squares)`, and the literal is lowered
  against the union's list. `union_literal_relaxed_arg_types` /
  `union_literal_call_types` are deleted (bug-989's commit). The fixture's IR is
  byte-identical before and after.
- **"A nested `T` never accepts a variant" is now "a `T` at a position lowering
  cannot convert never does".** `VarPosition::{Subsuming, Invariant}` in
  `src/codegen/registry/mod.rs`.

`reduce(List OF T, U, FUNC(U, T) AS U)` with a variant initial value binds `U` to the
variant first and meets the union in the callback, which is an `Invariant` position.
The unifier now keeps a `pinned` set per overload attempt: a variable is pinned once
it has been bound or matched at an `Invariant` position. A variable that is NOT
pinned (every occurrence so far was convertible) and is bound to a variant form is
rebound to a union form a later occurrence brings; the earlier values then convert.
So `U = Shape`, rule 3 answers `Shape` for the initial value, and it lowers wrapped.

One direction only. A variable bound to the union never accepts the variant at an
`Invariant` position, and a pinned variable never widens:
`reduce(squares, aShape, FUNC(Square, Square) AS Square)` and the same over a `List OF
Shape` are `TYPE_CALL_ARGUMENT_MISMATCH` (unit test, and the one-way syntax fixture).
A callback that returns the variant for a union accumulator is rejected too (callable
types do not convert).

The same rule makes `collections::append(squares, aShape)` type-check as `T = Shape`:
the list is converted and the call's type is `List OF Shape`. An `Arg(n)` return now
reports the parameter's resolved type when argument `n` was accepted by conversion;
echoing the argument's type would have typed that call `List OF Square` while it
held a `Shape`. Storing it back into `squares` is `TYPE_ASSIGNMENT_MISMATCH` (pinned
in the one-way fixture).

### Runtime proof

- `tests/rt-behavior/collections/union-variant-reduce-rt`: `reduce` and `reduceRight`
  over a `List OF Shape` with a `Square` initial value (constructor and call result),
  named and `LAMBDA` reducers, bound, unannotated and empty-list forms; each result
  `MATCH`es the right arm with the right payload. Passes at the default level,
  `MFB_OPT=0` and `MFB_OPT=3`. Before: every call `TYPE_CALL_ARGUMENT_MISMATCH`.
- `--debug` on that program: 98 allocs / 98 frees / 0 live / `double_free_skips 0`.
- It needed the separate `reduce` fix on this branch (`4d7d92c37`): on main a bound
  `reduce` result that is still the initial value is a freed block, whatever the
  types, and a variant initial value is always a fresh temporary.

## Golden impact (part 2)

- `scripts/artifact-gate.sh <mfb> all`: 0 diffs; `diag-set-diff.sh`: all SAME (run on
  the combined tree).
- New goldens: `union-variant-reduce-rt`; two more lines in
  `types-union-subsumption-one-way-invalid`.

## Resolution, part 3 (2026-10-10): the bucket fixes `T`

Part 2's rebinding was too wide: it let ANY not-yet-invariant variable widen, so
`append(squares, aShape)` bound `T = Square` from the list, rebound it to `Shape` from
the item, converted the list and typed the call `List OF Shape`. Undone per the owner
decision above.

`VarPosition` now has three values (`src/codegen/registry/mod.rs`): `Value` (the
variable is the whole parameter), `Element` (a `List` element or `Map` value type, at
any depth) and `Invariant`. A variable bound or matched at an `Element` or `Invariant`
position is **pinned**: the collection argument is the bucket. Only a variable bound
solely from `Value` occurrences can be rebound to a union a later occurrence brings
(`reduce`'s initial value). A union bucket still accepts a member at a `Value` or
`Element` position (`append(shapes, aSquare)`, `append(shapes, squares)`).

- `tests/syntax/collections/func_collection_union_into_variant_bucket_invalid`:
  `append`, `prepend`, `insert`, `set` (list and map), `getOr` (list and map), `repeat`,
  `contains`, `find` of a `Shape` into/over a collection of `Square`, and
  `append(squares, shapes)`. Its golden is main f6b4099c9's own output, byte for byte:
  eleven `TYPE_CALL_ARGUMENT_MISMATCH`. Before this part nine of them type-checked.
- The `TYPE_ASSIGNMENT_MISMATCH` store-back line is removed from
  `types-union-subsumption-one-way-invalid`: the call itself is the error now.
- `/tmp/bs18-rev/ap` (`LET r = collections::append(squares, aTriangle)`) is
  `TYPE_CALL_ARGUMENT_MISMATCH` again.
- Unit test `a_top_level_variable_bound_to_a_union_accepts_its_variant` asserts the
  rejection for each member, strict and lenient.

### Every other member, main vs this branch (`/tmp/bs18j/pr/run.sh`, one call per program)

Changed, all intended by bug-988/989:

| Call | main f6b4099c9 | branch |
| --- | --- | --- |
| `append(shapes, squares)` | mismatch | `List OF Shape` |
| `append(shapes, aSquare)`, `getOr(shapes, 0, aSquare)`, `getOr(shapeMap, "z", aSquare)`, `set(shapeMap, "k", aSquare)` | mismatch | works, union-typed result |
| `reduce(shapes, aSquare, FUNC(Shape, Shape) AS Shape)` | mismatch | `Shape` |
| `reduce(squares, aSquare, FUNC(Shape, Square) AS Shape)` | mismatch | `Shape` (same rule: `U` from the initial value, then the callback's `Shape`) |
| `replace(shapes, aSquare, aSquare)`, `contains(shapes, aSquare)` | mismatch | `TYPE_REQUIRES_COMPARABLE` |

Unchanged (same result on both): `append(squares, shapes)`, `append(squares, aShape)`,
`replace(squares, aShape, aSquare)`, `filter(squares, isShape)`, `filter(shapes,
isSquare)`, `any`, `transform`, `sortBy`, `findIndex` with a callback over the other
form (all mismatch: a callback is an `Invariant` position), `reduce(squares, aShape,
FUNC(Shape, Square) AS Shape)` (works), `reduce(squares, aSquare, FUNC(Shape, Shape) AS
Shape)` (mismatch: the list fixes `T`), `zip(squares, shapes)` (works, two variables).

### Not decided here (owner's call)

These go through monomorphization's own unifier (`monomorph::helpers::unify_type`),
which this bug did not touch. They are rejected on main and on the branch, in BOTH
argument orders, so nothing regressed, but they are now inconsistent with `append`:

- `collections::merge(shapeMap, squareMap, TRUE)` and `merge(squareMap, shapeMap,
  TRUE)`: `TYPE_CALL_ARGUMENT_MISMATCH` ("cannot infer template arguments"). `merge`
  is a source-companion template, not a registry-native member. By the bucket rule
  the first map would be the bucket: `merge(shapeMap, squareMap)` would work and
  `merge(squareMap, shapeMap)` would not.
- User templates: `pick OF T(x AS T, xs AS List OF T)` with `pick(aSquare, shapes)`,
  and `both OF T(n, a AS List OF T, b AS List OF T)` with `both(1, shapes, squares)`:
  mismatch in every order. `pick(aShape, shapes)` works.

Proposal: a follow-up that gives `unify_type` the same three-position rule, so a
template call behaves like a registry member. Until then a template never accepts a
variant for its union, which is the safe side.
