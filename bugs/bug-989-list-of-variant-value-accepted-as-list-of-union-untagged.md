# bug-989: a `List OF Square` value is accepted where `List OF Shape` (a union over `Square`) is expected, and its elements stay untagged, so `MATCH` misroutes, reads garbage, or crashes

Last updated: 2026-10-06
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/syntax/types/types-union-container-covariance-invalid (new; name suggested), plus `TypeEnv::compatible` unit tests beside `resource_union_parameter_widening_is_directional` in `src/ir/verify/tests.rs`

A collection **value** whose element type is a union variant is accepted where the same
collection over the union is expected, with no conversion. Take `UNION Shape  Circle  Square` and
`LET sq AS List OF Square = [...]`. Then `LET sh AS List OF Shape = sq` compiles, and so do passing
`sq` to a `List OF Shape` parameter, `RETURN sq` from a `List OF Shape` function, `Holder[sq]` into a
`List OF Shape` field, `sh = sq` onto a `MUT` binding, the `Map OF K TO Square` → `Map OF K TO Shape`
forms, nested `List OF List OF Square` → `List OF List OF Shape`, and the resource form
`List OF RES fs::File` → `List OF RES Handle`. The lowered IR copies the payload as-is
(`"bind" … "type": "List OF Shape", "value": { "kind": "local", "name": "sq" }`, no `unionWrap`).
The list therefore holds bare `Square` records, which the program then reads as `{tag, size, data}`
union blocks. Depending on the bytes it reads, a `MATCH` over an element:

- fires **no** arm, so the program continues silently and exits 0;
- fires `CASE ELSE` for a value that is a `Square`;
- fires the **wrong** arm and binds garbage (`circle -5028363935145493149`);
- skips elements, because the iterator steps by a misread `size` word;
- traps with `7-701-0001 Allocation failed.` when the element is copied (passed by value to a
  function, or read with `collections::get`).

The same program with a list **literal** (`LET sh AS List OF Shape = [Square[2.0], Square[3.0]]`)
is correct. Lowering wraps each literal element in the union tag.

**Correct behavior:** union-variant subsumption is a per-**value** conversion. Lowering realizes it
by inserting a `UnionWrap` (`src/ir/lower.rs:wrap_union_value`), and nothing exists that rewraps a
container payload. So it applies only at a position where a single variant-typed value is lowered
against the union, never through a container's element, key, or value type. Every
non-literal `List OF Square` (or `Map OF K TO Square`, nested container, `List OF RES Variant`) in a
slot that expects the union-element form is a compile error, with that position's existing mismatch
rule: `TYPE_BINDING_MISMATCH`, `TYPE_ASSIGNMENT_MISMATCH`, `TYPE_CALL_ARGUMENT_MISMATCH`,
`TYPE_RETURN_MISMATCH`, `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH`. The rule is the same one an unrelated
element type already gets (`List OF Square` into `List OF Circle`, measured below). List, set and map
**literals** keep working, because their elements are lowered one at a time against the expected
element type.

The bug is dangerous because it is silent. The build is clean and the IR verifier accepts the
program. A `MATCH` that should be exhaustive falls through with exit 0 or binds memory as the wrong
record.

References:

- `src/docs/spec/memory/03_heap-values.md` § "Union": a data union is a self-describing
  `{tag, size, data}` block, "Constructing a variant wraps its built record block at `+16`; `MATCH`
  dispatches on `tag@0`". "A `List`/`Map` of a data union stores each union block inline by its
  runtime `size`". A `List OF Square` payload has neither word, so reading it as `List OF Shape`
  violates the representation contract.
- `src/docs/spec/architecture/22_type-inference.md` opening paragraph: "the only "widening" steps are
  literal coercion (asymmetric, literal-shapes only) and union-variant subsumption". Covariant
  containers are not listed, and there is no least-upper-bound. The `wrap_trap_slot_value` doc comment in
  `src/ir/lower.rs` states the mechanism: "Every other binding and assignment path runs its value
  through [`wrap_union_value`], which inserts the `UnionWrap` that gives the value its `{tag, payload}`
  union representation."
- `src/docs/spec/language/12_collections.md` (opening): "List literals use the declared or otherwise
  expected `List OF T` element type … Every element must be compatible with that element type. This
  allows annotated lists of union members, such as `LET shapes AS List OF Shape = [Circle[5],
  Rect[2, 3]]`." The sanctioned form is the per-element literal. Nothing sanctions a whole
  `List OF Circle` value.
- `src/docs/spec/language/04_types.md` § 4.3: unions are closed sums. `INCLUDES` "does not create
  subtyping". The language has no subtyping from which container covariance could follow.
- `src/docs/spec/language/15_resource-management.md` § 15.4: variant→union widening for resources
  is "representation-neutral" only for a single handle "in non-owning parameter position". Nothing
  extends it to a collection.
- **The spec text that must change:** `src/docs/spec/architecture/22_type-inference.md`
  § "Structural Assignability", IR-verifier rule 2 ("Compare `ListOf` and `ResultOf` elements
  recursively; compare `MapOf` keys and values recursively") combined with rule 5 (union-variant
  acceptance) describes exactly the unsound behavior. The HIR-predicate paragraph ("recurses through
  sets and thread-handle components") does the same. Read together with the representation contract
  above, the recursion must not apply rule 5 below the top level. The fix updates this section.
- Related: bug-988 (`bugs/bug-988-generic-collection-call-rejects-union-variant-element.md`). Its
  Blast Radius first recorded this hole, and its Non-goal "No list covariance without conversion"
  exists so that bug's fix does not extend this hole into `collections::append`. The two fixes are
  independent. This one narrows assignability; bug-988 widens generic unification only at a wrappable
  top-level position.
- Prior instance of the same silent symptom from a different seam: bug-642
  (`bugs/completed/bug-642-a-union-with-a-variant-from-an-uncalled-package-fails-to-build.md`). An
  inline-`TRAP` slot was bound without `UnionWrap`, and "`MATCH` on it read a tag that was never
  written and fell through EVERY case, silently".

## Failing Reproduction

Built with `target/release/mfb` at `468ff0f94`, macOS aarch64. Harness: `/tmp/bug989/mk.sh <name>`
(writes `/tmp/bug989/<name>/{project.json,src/main.mfb}` from stdin, builds, runs). Not checked in.

`/tmp/bug989/listvar/project.json`:

```json
{ "name": "listvar", "version": "0.1.0", "mfb": "1.0", "kind": "executable",
  "sources": [{ "root": "src", "role": "main", "include": ["**/*.mfb"] }],
  "entry": "main", "targets": ["native"] }
```

`/tmp/bug989/inline_noelse/src/main.mfb`:

```
IMPORT io
IMPORT collections

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
  LET sq AS List OF Square = [Square[2.0], Square[3.0]]
  LET sh AS List OF Shape = sq
  io::print("len " & toString(len(sh)))
  FOR EACH s IN sh
    io::print("iter")
    MATCH s
      CASE Circle(c)
        io::print("circle " & toString(c.r))
      CASE Square(q)
        io::print("square " & toString(q.side))
    END MATCH
  NEXT
  io::print("after")
  RETURN 0
END FUNC
```

```
target/release/mfb build /tmp/bug989/inline_noelse && /tmp/bug989/inline_noelse/build/inline_noelse.out
```

- Observed (build succeeds, exit 0):
  ```
  len 2
  iter
  iter
  after
  ```
- Expected: the build fails at line 19 with `TYPE_BINDING_MISMATCH` ("Binding `sh` has initializer
  type List OF Square, expected List OF Shape"). The runtime output above must not be producible.

Observed behavior matrix (same declarations; `show(s AS Shape)` is a `FUNC` that `MATCH`es and
prints `circle <r>` / `square <side>`; all build cleanly today):

| Fixture | Fragment | Run result |
| --- | --- | --- |
| `inline_noelse` | `LET sh AS List OF Shape = sq`, inline `MATCH` with `Circle`/`Square` arms | `len 2`, two `iter`, **no arm fires**, exit 0 ✗ |
| `listvar_else` | same, arms `CASE Square(q)` + `CASE ELSE` | prints `else` twice: **`CASE ELSE` fires for a `Square`**, exit 0 ✗ |
| `listvar` | same, element passed to `show(s)` | `len 2`, `iter`, then `Error: 7-701-0001 Allocation failed.`, exit 255 ✗ |
| `tagcollide` | `Circle`/`Square` with `Integer` fields; `sq = [Square[1], Square[0], Square[7]]` | `square 7`, `circle -5028363935145493149`: **2 iterations for 3 elements, wrong arms, garbage field**, exit 0 ✗ |
| `get_elem` | `LET first AS Shape = collections::get(sh, 0)` + `MATCH` | no arm fires, exit 0 ✗ |
| `funcarg` | `walk(sq)`, `walk(sh AS List OF Shape)` | no arm fires, exit 0 ✗ |
| `ret` | `FUNC mk AS List OF Shape` … `RETURN sq` | no arm fires, exit 0 ✗ |
| `field` | `TYPE Holder items AS List OF Shape`; `Holder[sq]` | no arm fires, exit 0 ✗ |
| `mutassign` | `MUT sh AS List OF Shape = [Circle[1.0]]`; `sh = sq` | no arm fires, exit 0 ✗ |
| `mapval` | `LET m AS Map OF String TO Shape = ms` (`ms AS Map OF String TO Square`), loop over `collections::values(m)` | no arm fires, exit 0 ✗ |
| `mapget` | same map; `show(collections::get(m, k))` | `Allocation failed.`, exit 255 ✗ |
| `nested` | `LET nn AS List OF List OF Shape = sq2` (`sq2 AS List OF List OF Square`) | no arm fires, exit 0 ✗ |
| `rescov` | `UNION Handle  fs::File  tcp::Socket`; `walk(files)` with `files AS List OF RES fs::File` into `hs AS List OF RES Handle` | no arm fires, exit 0 ✗ |

Contrast cases that work correctly today (regression guards; measured with the same harness):

| Fixture | Fragment | Result |
| --- | --- | --- |
| `literal` | `LET sh AS List OF Shape = [Square[2.0], Square[3.0]]` | `square 2.00`, `square 3.00` ✓ |
| `literal_of_vars` | `LET a AS Square = …`; `LET sh AS List OF Shape = [a, Square[3.0]]` | `square 2.00`, `square 3.00` ✓ |
| `lit_arg` | `walk([Square[2.0], Circle[1.0]])`; `Holder[[Square[3.0]]]`; `RETURN [Square[4.0]]` from a `List OF Shape` function; `Map OF String TO Shape { "a" := Square[5.0] }` | every arm correct ✓ |
| `var_arg` | `LET a AS Square`; `show(a)`; `LET b AS Shape = a` | `square 2.00` twice ✓ |
| `thenlocal` | `sq` still usable as `List OF Square` after the bad bind | `sq still 2` ✓ |
| `setcov` | `LET s2 AS Set OF Shape = ss` (`ss AS Set OF Square`) | rejected `TYPE_REQUIRES_COMPARABLE` (union not comparable) ✓ |
| `fnret` / `fnparam` | `LET f AS FUNC() AS Shape = mk` (`mk AS Square`); `LET f AS FUNC(Square) AS Nothing = show` | rejected `TYPE_BINDING_MISMATCH` (IR rule 3 requires equal callable types) ✓ |

The diagnostics the fix must produce already exist for a non-union element mismatch. Measured with
`List OF Square` into `List OF Circle` (`neg_bind`, `neg_assign`, `neg_arg2`, `neg_field`, `neg_ret`):
`TYPE_BINDING_MISMATCH`, `TYPE_ASSIGNMENT_MISMATCH`, `TYPE_CALL_ARGUMENT_MISMATCH`,
`TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH`, `TYPE_RETURN_MISMATCH`.

## Root Cause

Both assignability predicates recurse into container element types with the **same** predicate, and
that predicate's last step accepts a union variant. Union-variant subsumption therefore leaks from the
top level, where lowering inserts a `UnionWrap`, into element positions, where nothing converts.

1. **IR verifier (the authoritative gate).** `src/ir/verify/compat.rs:TypeEnv::compatible`:

   ```rust
   (ParameterType::ListOf(e), ParameterType::ListOf(a))
   | (ParameterType::ResultOf(e), ParameterType::ResultOf(a)) => {
       return self.compatible(e, a);
   }
   (ParameterType::MapOf(ek, ev), ParameterType::MapOf(ak, av)) => {
       return self.compatible(ek, ak) && self.compatible(ev, av);
   }
   ```

   The recursive call reaches the tail "A union accepts any of its variants" (`self.union_variants(expected)`
   contains `Square`), so `compatible(List OF Shape, List OF Square)` is `true`.
   `TypeEnv::assignable` (= `expression_compatible` + the isolated-callable conversion) is what every
   assignment position asks, so the binding, assignment, call-argument, return and constructor-field
   checks all pass. The structural recursion dates from `553806aea` ("plan-106-B: ir::verify onto
   ParameterType", `git log -S '(ParameterType::ListOf(e), ParameterType::ListOf(a))' --
   src/ir/verify/compat.rs`). That commit preserved the earlier name-domain prefix cascade's
   behavior, which had the same leak.

2. **HIR shape predicate.** `src/ir/shape.rs:ShapeChecker::compatible` recurses the same way through
   `ListOf`/`SetOf`/`ResultOf`/`MapOf`/`ThreadHandle`, and its `Named` arm accepts a variant
   (`info.variants.iter().any(...)`). The shape pass's declared-function argument check
   (`TYPE_CALL_ARGUMENT_MISMATCH`, the `expression_compatible(&params[index].type_, &actual, argument)`
   loops in `src/ir/shape.rs`) therefore accepts `walk(sq)` too.

3. **Lowering never converts a container.** `src/ir/lower.rs:wrap_union_value` wraps only when
   `variant_belongs_to_union(actual, expected)` holds for the **whole** value's type. That is true
   for `Square` → `Shape` and false for `List OF Square` → `List OF Shape`, so the value is passed
   through unchanged. The IR for the reproduction shows it:
   `{ "op": "bind", "name": "sh", "type": "List OF Shape", "value": { "kind": "local", "name": "sq" } }`
   (`mfb build --ir /tmp/bug989/listvar`, line 164 of `listvar.ir`). Codegen then lays out and reads
   `sh` as a list of inline `{tag, size, data}` blocks (spec `memory heap-values` § Union), while
   the payload holds bare `Square` records. That produces every symptom in the matrix: a float's bits
   or an integer field read as the tag, a field read as the `size` stride, and the copy path
   allocating by a garbage `size` (`Allocation failed.`).

**Why the contrast cases are immune.** A list/map **literal** gets the expected type in lowering
(`lower_expression_with_expected`, `HirExpression::ListLiteral` arm). Each element is lowered with the
expected element type and wrapped, and the literal's IR type is `List OF Shape`, an exact match.
A single variant value (`show(a)`, `LET b AS Shape = a`) goes through `wrap_union_value`. Callables
are immune at the IR seam because `compatible`'s `Func` arm requires equal parameter and return types
(rule 3), with no recursion into the union arm. `Set OF Shape` is rejected earlier because a union is
not comparable.

## Goal

- Each ✗ row of the matrix fails to **build**. It reports its position's existing mismatch rule
  (`TYPE_BINDING_MISMATCH`, `TYPE_ASSIGNMENT_MISMATCH`, `TYPE_CALL_ARGUMENT_MISMATCH`,
  `TYPE_RETURN_MISMATCH`, `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH`), and the message names
  `List OF Square`/`List OF Shape` (or the map/nested/`RES` form).
- `TypeEnv::compatible(List OF Shape, List OF Square)`,
  `compatible(Map OF String TO Shape, Map OF String TO Square)`,
  `compatible(List OF List OF Shape, List OF List OF Square)` and
  `compatible(List OF RES Handle, List OF RES fs.File)` are `false`. `compatible(Shape, Square)` stays
  `true`.
- Every contrast row still builds and prints the same output.

### Non-goals (must NOT change)

- **No runtime conversion.** Do not "fix" this by inserting an element-wise rewrap (a hidden O(n)
  copy that recurses through nested containers and maps, and changes aliasing). The language has no
  implicit conversions of that kind. The user-side spelling stays explicit:
  `collections::map(sq, LAMBDA(q AS Square) AS Shape -> q)` or a literal. Rejected alternative,
  see Fix Design.
- Top-level union-variant subsumption is unchanged at every position (`LET s AS Shape = Square[...]`,
  a `Square` argument into a `Shape` parameter, `RETURN` of a variant from a union-typed function,
  a variant constructor field into a union field), including the resource form (§15.4).
- List, set and map **literals** of variants into a union-element collection keep building, both
  as a `LET` initializer and inline at a call argument, constructor field, or `RETURN` (`lit_arg`).
- `RES`/`STATE` stripping, built-in-qualifier normalization (bug-632), distinct built-in resources
  (plan-110), callable contravariance in the shape predicate (bug-173 A), and the isolated-callable
  conversion (bug-786) are unchanged.
- The annotation-integrity callers of `TypeEnv::compatible` (`src/ir/verify/compat.rs` lines using
  `compatible(&declared, &annotated)` / `compatible(&derived, &annotated)`, and the symmetric checks in
  `src/ir/verify/values.rs`) must keep accepting every annotation the compiler itself emits. A
  stricter element rule could only reject a package annotation that records `List OF Square` for a
  `List OF Shape` value, which is exactly the type confusion to refuse.
- Tempting wrong fix: special-casing `ListOf` only. `MapOf` values, nested lists, and
  `List OF RES <resource union>` leak through the same recursion, and all of them are observed broken
  (`mapval`, `mapget`, `nested`, `rescov`).
- bug-988's generic-unification widening is out of scope here and must not be blocked. It accepts a
  variant only at a top-level `T`, which this fix leaves alone.

## Blast Radius

Found with `grep -n 'compatible(' src/ir/verify/*.rs src/ir/shape.rs` (17 + 11 hits) and the
matrix above.

- `src/ir/verify/compat.rs:TypeEnv::compatible`, `ListOf`/`ResultOf`/`MapOf` arms: fixed by this
  bug. This is the authoritative gate for binding, assignment, call argument, return, constructor
  field, `WITH` update, collection element and `STATE` write (spec 22, "Every *assignment* position
  … asks `assignable`").
- `src/ir/shape.rs:ShapeChecker::compatible`, `ListOf`/`SetOf`/`ResultOf`/`MapOf`/`ThreadHandle`
  arms: fixed by this bug, so the shape pass and the IR verifier agree. The shape pass also checks
  user-function arguments. Here the **literal** contrast (`walk([Square[2.0], Circle[1.0]])`)
  depends on the leak today: `type_of` types the literal bottom-up as `List OF Square` (first
  element; `src/ir/lower.rs:expression_type`, `ListLiteral` arm), and only the recursive union arm
  accepts it. `ShapeChecker::expression_compatible`'s `ListLiteral` arm today handles numeric literals only
  (`numeric_literal_type`). It must be extended to check each element with
  `expression_compatible(expected_element, type_of(element), element)` (and likewise for a map
  literal's values), or the literal guards regress. This is the main risk in the fix.
- `ResultOf`: same recursion. A user cannot spell `Result OF T` (internal fallible-result type,
  spec `language types` § comparable), so it is not observed. Covered by the same change.
- `SetOf` (shape only): unreachable today because a union is not comparable (`setcov` measured).
  Covered by the same change.
- `ThreadHandle` message/resource/output components (shape only): the IR verifier requires exact
  equality for thread handles (spec 22, "Other composites, including `SetOf` and thread handles …
  require exact structural equality"), so this is already rejected at the IR seam. The shape change
  only aligns the two predicates. Not measured.
- `Func` parameter/return in the shape predicate (contravariant/covariant with the union arm):
  rejected at the IR seam today (`fnret`, `fnparam`: `TYPE_BINDING_MISMATCH`). Unaffected. Do not
  widen.
- `src/ir/verify/matching.rs` (`expression_compatible(&scrutinee, &pattern_type, v)`) and the
  equality-operand checks in `src/ir/verify/values.rs` / `src/ir/shape.rs`: both pass top-level types
  (a pattern's type, an operand's type). Unaffected by an element-only change, but the full suite
  must confirm this.
- `src/codegen/registry/mod.rs:unify_at` (generic collection calls): a separate seam. Today
  `append(shapes, sq)` is rejected because of identity rebinding, not this predicate. bug-988 owns it
  and forbids nested-variant acceptance there.
- **Separate bug observed while measuring, NOT this one (distinct root cause, filed as bug-990,
  `bugs/bug-990-variant-call-or-field-result-not-union-wrapped-at-argument-and-element-positions.md`):** a variant-typed **call result or member access** passed directly at an expected-union
  position other than a binding is not wrapped. `show(mk())` with `FUNC mk AS Square` and
  `show(h.sq)` with field `sq AS Square` both build and trap `Allocation failed.`, and
  `LET sh AS List OF Shape = [mk()]` iterates with no arm firing (fixtures `call_arg`, `member_arg`,
  `callelem`). `LET one AS Shape = mk()` and `FUNC mk2 AS Shape … RETURN mk()` are correct
  (`call_let`, `call_ret`). Cause: `src/ir/lower.rs:lower_expression_with_expected` calls
  `wrap_union_value` only in its `Identifier` and `Constructor` arms. Binding/return paths wrap
  every expression after lowering, but argument and list-element paths rely on
  `lower_expression_with_expected` alone. Out of scope here: that is a lowering-wrap gap at a scalar
  position, and this bug is an assignability gap at a container position.

## Fix Design

Make union-variant subsumption a **top-level-only** step in both predicates. Container element/key/value
positions compare with an **invariant** element relation: the same predicate minus the union-variant
step, still peeling `RES`, normalizing built-in qualifiers, and accepting `Unknown`.

1. `src/ir/verify/compat.rs`: split `TypeEnv::compatible` into `compatible` (top level, unchanged
   contract) and a private `element_compatible` that runs the same rules except the trailing
   union-variant acceptance. The `ListOf`/`ResultOf`/`MapOf` arms recurse into `element_compatible`,
   and `element_compatible` recurses into itself. Update the doc comment ("container types recurse; a
   union accepts any of its variants") to say that subsumption applies at the top level only, and why
   (no container rewrap exists).
2. `src/ir/shape.rs`: the same split for `ShapeChecker::compatible`. The container and thread-handle
   arms recurse into the invariant form. Leave the `Func` arm's existing variance alone (it is
   IR-rejected anyway and owned by bug-173 A).
3. `src/ir/shape.rs:ShapeChecker::expression_compatible`: generalize the `ListLiteral` arm to every
   literal element (`self.type_of(value, locals)` per element, falling back to
   `numeric_literal_type` for the existing numeric coercions). Add the analogous `MapLiteral`/`SetLiteral`
   arms if any measured guard needs them. Thread `locals` in, because the arm currently has no access
   to it, and check each caller. This keeps `walk([Square[2.0], Circle[1.0]])` building.
4. Spec sync (Hard Completion Gate): `src/docs/spec/architecture/22_type-inference.md`
   § "Structural Assignability". Rule 2 compares container components with the element relation,
   i.e. rules 1, 3, 4 without rule 5. State this explicitly with the representation reason. The HIR
   paragraph gets the same wording. Add one sentence to `src/docs/spec/language/12_collections.md`
   after the "annotated lists of union members" sentence: a non-literal `List OF Circle` is not a
   `List OF Shape`, and the conversion is spelled with `collections::map`. Check `mfb man` union/list
   guide prose for any claim to the contrary (`grep -rn -i 'List OF Shape' src/docs/man` → no hits today).

Rejected alternatives:

- **Accept and convert** (lowering emits an element-wise `UnionWrap` copy when the static types
  differ only by variant→union in element position). This would introduce the language's first
  implicit O(n) container conversion. It needs recursive handling for maps, nested containers and
  resource lists, where ownership of `RES` elements across the copy is a further hazard. It
  contradicts "no general unification … no least-upper-bound" in spec 22, and §15.4's limit of
  resource widening to a single non-owning handle. The explicit `collections::map` spelling already
  exists.
- **Reject in the shape pass only.** The IR verifier is the gate that package IR and every lowered
  position pass through. A shape-only fix leaves `.mfp` annotations and any non-HIR producer
  unchecked, and the spec requires the two predicates to be read at their own seams.
- **Special-case `ListOf` only.** See Non-goals: `MapOf`, nested and `RES` forms are observed broken.

Expected output shift: no existing golden should move. `grep -rln 'List OF Shape' tests/` finds only
fixtures that use literals (`types-union-list-invalid`, `default-values-rt`, `flat-union`,
`scope-drop-free-union`). Phase 1 confirms with a grep of every `tests/**/src/*.mfb` for a non-literal
variant-element collection bound into a union-element slot. Any golden delta beyond the new fixtures
is a bug-hunt trigger.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/syntax/types/types-union-container-covariance-invalid` (layout as
      `types-union-list-invalid`). One `main` per position or one file with every position: `LET`,
      `MUT` reassignment, user-function argument, `RETURN`, constructor field, `Map OF String TO
      Square` → `Map OF String TO Shape`, nested `List OF List OF Square`. Its golden `build.log`
      records the expected mismatch rule per line. Today it builds (fails the test).
- [ ] Add a resource-union variant (`List OF RES fs::File` into `List OF RES Handle` parameter) to the
      same fixture or a sibling, mirroring `rescov`.
- [ ] Add an rt guard `tests/rt-behavior/types/union-container-literal-rt` covering every contrast row
      (`literal`, `literal_of_vars`, `lit_arg` forms, `var_arg`). Passes today and must keep passing.
- [ ] Unit tests next to `resource_union_parameter_widening_is_directional` in
      `src/ir/verify/tests.rs`: the four `false` cases and the `compatible(Shape, Square)` /
      `compatible(RES Stream, RES fs.File)` `true` cases from Goal.
- [ ] Audit: grep `tests/**/src/*.mfb` and `examples/**/*.mfb` for a non-literal container of a
      variant passed into a union-element slot. Record every hit and its verdict here.

Acceptance: the invalid fixture and the `false` unit tests fail today for the documented reason
(build succeeds / `compatible` answers `true`); the rt guard and the `true` unit tests pass.
Commit: —

### Phase 2 — the fix

- [ ] `element_compatible` split in `src/ir/verify/compat.rs:TypeEnv::compatible` (Fix Design 1).
- [ ] Same split in `src/ir/shape.rs:ShapeChecker::compatible` (Fix Design 2).
- [ ] Per-element literal check in `src/ir/shape.rs:ShapeChecker::expression_compatible`
      (Fix Design 3).
- [ ] Spec sync: `22_type-inference.md` § Structural Assignability, `12_collections.md` (Fix Design 4).

Acceptance: Phase 1 tests pass; every contrast row still builds and prints the same output; nothing
in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate only the new fixtures' goldens. Any other golden delta is a bug-hunt trigger: inspect
      one fixture to localize before concluding.
- [ ] Full suite per `.ai/testing-gates.md` (never one module).
- [ ] Re-run `/tmp/bug989/mk.sh` for every ✗ matrix row. Each must now fail to build with the
      documented rule. Re-run every contrast row; each must print as recorded.
- [ ] Rebuild `examples/canvas-demo` (where bug-988 was found) to confirm it does not depend on the
      leak.

Acceptance: full suite green; golden deltas are exactly the new fixtures; the reproduction is a
compile error.
Commit: —

## Validation Plan

- Regression test(s): `tests/syntax/types/types-union-container-covariance-invalid`,
  `tests/rt-behavior/types/union-container-literal-rt`, and `TypeEnv::compatible` unit tests in
  `src/ir/verify/tests.rs`.
- Runtime proof: `target/release/mfb build /tmp/bug989/inline_noelse` exits 1 with
  `TYPE_BINDING_MISMATCH` at line 19. `/tmp/bug989/literal` and `/tmp/bug989/lit_arg` still print
  their `square`/`circle` lines.
- Doc sync: `src/docs/spec/architecture/22_type-inference.md` § Structural Assignability;
  `src/docs/spec/language/12_collections.md` list-literal paragraph.
- Full suite: the repo's full gate per `.ai/testing-gates.md`.

## Owner decision (2026-10-10): convert, never reject

A `List OF Square` given where a `List OF Shape` is expected **works**: the compiler makes a
converted copy in which every element is wrapped (tagged) as a `Shape`. It is the same rule as
passing a single `Square` where a `Shape` is expected, applied to a list. A compile error is the
wrong answer here. This supersedes the "Recommended: reject" text and the Fix Design built on it.

Scope the coordinator takes this to imply (flag any case that cannot follow it, with evidence):
- It applies at every position where a single variant value is wrapped today (bug-990's matrix):
  a call argument, a `LET`/`MUT` binding or assignment with a declared type, `RETURN`, a record or
  constructor field, a `WITH` field, a collection element, a global initializer.
- It applies to every collection shape whose element (or value) type is the variant and whose
  expected element type is a union accepting it: `List`, `Set`, `Map` values, and nested forms
  (`List OF List OF Square` → `List OF List OF Shape`), converting at each level.
- The source value is unchanged; the converted value is a new collection (lists are values, so
  this has the same meaning as the original). It is a real O(n) copy: do it once per conversion
  position, never per element access, and free it like any other temporary (`--debug` alloc ==
  free).
- A collection that is already of the union type is passed as-is, with no copy.
- The diagnostic-wording question below is moot: nothing is rejected.

## Open Decisions (superseded by the owner decision above)

- **Reject vs. convert.** Recommended: reject (above). Alternative: an implicit element-wise rewrap.
  Rejected in Fix Design, but it is the user-visible language decision, so it is recorded here for
  sign-off.
- **Diagnostic wording.** Recommended: reuse each position's existing rule unchanged (zero new rule
  IDs). Alternative: a hint line ("a `List OF Square` is not a `List OF Shape`; convert with
  `collections::map`") appended to the detail when the mismatch is exactly variant→union in element
  position. This is cheap and helpful, but it touches diagnostic text that goldens may pin.

## Summary

The engineering risk is the shape-pass literal path. Today the literal guards pass the shape pass
only *because of* the leak (a literal's bottom-up type is `List OF <first variant>`), so narrowing
`compatible` without teaching `expression_compatible` to check literal elements one by one would
reject valid programs. The IR-verifier change itself is small and mechanical. Top-level union
subsumption, literals, resource single-handle widening, and every non-union compatibility rule are
left untouched.
