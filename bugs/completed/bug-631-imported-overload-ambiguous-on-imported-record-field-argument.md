# bug-631: an imported overloaded call is `TYPE_OVERLOAD_AMBIGUOUS` when its argument is a field of an imported record

Last updated: 2026-09-15
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: Fixed
Regression Test: tests/runtime/rt_imported_overload_imported_field_argument.rs

A consumer that calls an **overloaded function exported by a package** fails to build when the
argument's type comes from **a field of a record type the consumer imported**. That covers the
field read directly, an untyped `LET` bound to it, a `FOR EACH` variable over a list field, and
`collections::get` of a list field:

```
error[2-203-0101 TYPE_OVERLOAD_AMBIGUOUS]: return-type overload cannot be resolved without an expected type
    Call to `ov.show` matches 4 imported overloads; annotate the argument types (an untyped `[]` selects none of them) to choose one.
```

The same argument copied into a local with a declared type builds and runs. So does the same call
to a non-overloaded package function, and the same overloads and loop inside one program with no
package boundary. The frontend accepts the program; overload resolution fails on a valid call.

This is the natural way to use a package that exports a tree and overloaded functions over it:
`FOR EACH n IN doc.children` / `xml::stringify(n)` (plan-138-B's API) fails.

**The single correct behavior a fix produces:** an argument read from an imported record's field
has the field's declared type during imported-overload resolution, exactly as a field of a local
record does, so the call resolves to the one matching overload and the program builds and runs.

## STATUS: FIXED (73ab94edc, c30373da7)

`src/monomorph/helpers.rs:collect_imported_records` decodes each imported package's `.mfp`
record layouts (via `imported_type_defs_from_files`) into a separate `imported_records` map.
`record_fields` consults it after `concrete_types`, under the type's own spelling and then its
package-qualifier-stripped one. `resolve_imported_overload`, `types_compatible` and bug-36's
ambiguity rule are untouched. Deviations from the design:

- A separate map, as the Open Decision defaulted (`concrete_types` is emitted from).
- The local-overload and generic latent sites were confirmed failing and are fixed by the same
  change. The imported-record constructor site was already working; it is kept as a guard.
- The consumer-local `TYPE A` collision row is a different, pre-existing bug: the package's own
  `A[1]` is checked against the consumer's `A` even with no overloaded call. Filed as bug-632 and
  removed from this bug's test.
- `tests/guards/no_type_strings.rs` `declared_sites`/`monomorph` 8 -> 9, justified in the table
  the same way as the `ir` row's identical `.mfp`-name-as-table-key entry.
- plan-138-B's `packages/xml` consumer check cannot run until plan-138-A lands; its prerequisite
  row is marked partially met.

Verification: `cargo test --no-fail-fast` over the whole tree (`artifact_gate_all ... ok`, 186
test binaries `ok`). The only two failures were the stale collision case and the budget row. After
fixing them, `no_type_strings` (7 passed), the regression test (11 passed) and
`cargo test --bin mfb monomorph` (63 passed) were re-run green. Since the full run, the only source
change is `cargo fmt` on one unrelated match arm. The reproduction rebuilt with the fixed compiler
prints `A, A, A A, U, A, A`.

References:

- `bugs/completed/bug-36-monomorph-imported-overload-unknown-ambiguity.md` — made
  `resolve_imported_overload` report ambiguity for `Unknown`-wildcard matches instead of silently
  taking the first export. That fix is correct; this bug is the `Unknown` that should never have
  reached it.
- `aa3a77745` "fix(ir): type imported package types so keys()/values() work on their fields" and
  `tests/runtime/rt_imported_record_map_field_keys.rs` — the same gap (imported record layouts
  missing from a type table) closed for IR lowering's `TypeIndex`, not for the monomorphizer.
- Found while verifying plan-138's API as a package
  (`planning/plan-138-A-xml-tree-builder-and-reader.md`, Verified properties); prerequisite of
  `planning/plan-138-B-xml-serializer-and-docs.md`.

## Failing Reproduction

Built with `target/release/mfb` at `80967b7c6` (rebuilt 2026-09-15), macOS aarch64.

Package `ov` (`ov/project.json`: kind `package`, sources `src/**/*.mfb`), `ov/src/lib.mfb`:

```
EXPORT TYPE A
  x AS Integer
END TYPE

EXPORT TYPE B
  y AS Integer
END TYPE

EXPORT TYPE Leaf
  s AS String
END TYPE

EXPORT UNION U
  Leaf
  A
END UNION

EXPORT TYPE Holder
  items AS List OF A
  nodes AS List OF U
  first AS A
END TYPE

EXPORT FUNC show(a AS A) AS String
  RETURN "A"
END FUNC

EXPORT FUNC show(b AS B) AS String
  RETURN "B"
END FUNC

EXPORT FUNC show(u AS U) AS String
  RETURN "U"
END FUNC

EXPORT FUNC show(h AS Holder) AS String
  RETURN "H"
END FUNC

EXPORT FUNC one(a AS A) AS String
  RETURN "one"
END FUNC

EXPORT FUNC holder() AS Holder
  LET u AS U = Leaf["s"]
  RETURN Holder[[A[1], A[2]], [u], A[3]]
END FUNC
```

Consumer (executable, `packages: [{"name":"ov","version":"=0.1.0","source":"file:packages/ov.mfp"}]`),
`src/main.mfb` is this frame with one BODY from the table below:

```
IMPORT ov
IMPORT io
IMPORT collections

FUNC main() AS Integer
  LET h AS ov::Holder = ov::holder()
  <BODY>
  RETURN 0
END FUNC
```

```
mfb build ov && cp ov/ov.mfp app/packages/ && mfb build app && ./app/build/<name>.out
```

| BODY | Result |
|---|---|
| `io::print(ov::show(h.first))` | ✗ `TYPE_OVERLOAD_AMBIGUOUS` |
| `LET f = h.first` / `io::print(ov::show(f))` | ✗ `TYPE_OVERLOAD_AMBIGUOUS` |
| `FOR EACH a IN h.items` / `io::print(ov::show(a))` / `NEXT` | ✗ `TYPE_OVERLOAD_AMBIGUOUS` |
| `FOR EACH n IN h.nodes` / `io::print(ov::show(n))` / `NEXT` | ✗ `TYPE_OVERLOAD_AMBIGUOUS` |
| `io::print(ov::show(collections::get(h.items, 0)))` | ✗ `TYPE_OVERLOAD_AMBIGUOUS` |
| `LET a = collections::get(h.items, 0)` / `io::print(ov::show(a))` | ✗ `TYPE_OVERLOAD_AMBIGUOUS` |
| `io::print(ov::show(h))` | ✓ prints `H` |
| `io::print(ov::one(h.first))` (not overloaded) | ✓ prints `one` |
| `FOR EACH a IN h.items` / `io::print(ov::one(a))` / `NEXT` (not overloaded) | ✓ prints `one one` |
| `LET xs AS List OF ov::A = h.items` / `FOR EACH a IN xs` / `io::print(ov::show(a))` / `NEXT` | ✓ prints `A A` |
| `LET us AS List OF ov::U = h.nodes` / `FOR EACH n IN us` / `io::print(ov::show(n))` / `NEXT` | ✓ prints `U` |
| `LET xs AS List OF ov::A = h.items` / `LET a = collections::get(xs, 0)` / `io::print(ov::show(a))` | ✓ prints `A` |
| `FOR EACH a IN h.items` / `LET t AS ov::A = a` / `io::print(ov::show(t))` / `NEXT` | ✓ prints `A A` |

- Observed: every ✗ row fails to build with the diagnostic above.
- Expected: `A`, `A`, `A A`, `U`, `A`, `A` respectively.

Contrast, no package boundary (one executable declaring `Text`/`Comment`/`UNION Node`/`Document`,
`FUNC show(doc AS Document)` and `FUNC show(n AS Node)`, then `FOR EACH n IN doc.children` /
`io::print(show(n))`) → builds, prints `doc:2`, `node`, `node`.

## Root Cause

1. `src/monomorph/lower.rs:Monomorphizer::new` fills `concrete_types` only from the consumer's own
   HIR: `for file in &source.files` → `HirItem::Type(type_decl)` →
   `concrete_types.insert(ParameterType::declared(&type_decl.name), …)`. Types declared in an
   imported package are never inserted.
2. `src/monomorph/lower.rs:record_fields` reads only `concrete_types`.
3. `src/monomorph/lower.rs:expression_type`, arm `HirExpression::MemberAccess { target, member }`,
   types a field as `self.record_fields(&target_type)?…field.type_`. For a target whose type is an
   imported record, `record_fields` is `None`, so the field's type is `None`. A `FOR EACH` variable
   over that field, a `collections::get` of it, and an untyped `LET` bound to either inherit the
   missing type.
4. The call-lowering path in `expression_type`'s caller (the `arg_types` vector built just before
   `let public_callee = callee.clone()`) maps `None` to `ParameterType::Unknown`.
5. `src/monomorph/lower.rs:resolve_imported_overload` filters candidates with `types_compatible`,
   where `(Unknown, other) | (other, Unknown) => is_leaf(other)`. Every nominal record/union
   parameter is a leaf, so an `Unknown` argument matches every same-arity overload taking one. Two
   or more match → `TYPE_OVERLOAD_AMBIGUOUS` (the bug-36 behavior, correct for a genuinely
   untyped argument).

Why the contrast cases are immune:

- **Typed local / typed list local:** `expression_type` types the local from the function context's
  declared type, so no `MemberAccess` on an imported record is involved.
- **Non-overloaded callee:** `resolve_imported_overload` returns `None` for a non-overloaded import
  (`self.imported_overloads.get(callee)?`), and the later passes type the call without needing the
  argument's type here.
- **Whole record `h`:** `h` is a typed local (`ov::Holder`), and `ov.show(Holder)` is the only
  candidate compatible with a *known* `Holder`.
- **No package boundary:** the record types are local, so they are in `concrete_types`.

IR lowering had the same hole and was fixed separately: `aa3a77745` added
`src/manifest/package.rs:imported_type_defs` / `imported_type_defs_from_files`, which decode the
record/union/enum layouts from each installed `.mfp` and fold them into IR lowering's `TypeIndex`.
The monomorphizer never received those layouts.

## Goal

- Every ✗ row in the reproduction table builds and prints its expected output, and the new
  regression test proves it across a real package boundary.

### Non-goals (must NOT change)

- **bug-36's ambiguity rule stays.** `types_compatible`'s `Unknown` leaf wildcard and the
  `TYPE_OVERLOAD_AMBIGUOUS` report for a genuinely untyped argument (`f([])` against
  `f(List OF Integer)` / `f(List OF String)`) must still be reported. Its unit tests
  (`types_compatible_matches_the_token_algorithm` and the `resolve_imported_overload` tests in
  `src/monomorph/lower.rs`) must pass unmodified.
- **Tempting wrong fix, forbidden:** making `resolve_imported_overload` pick a candidate when
  several match via `Unknown` (e.g. "first match wins" or "prefer the union overload"). That
  reintroduces bug-36's silent export-order binding. The fix is to *know the type*, not to guess.
- **Tempting wrong fix, forbidden:** rewriting the regression test to bind a typed local first.
  That is the workaround, not the fix, and exercises the working path.
- Local-overload resolution (`resolve_overload` / `params_match`) semantics.
- IR lowering's `TypeIndex` and `imported_type_defs` contract (`aa3a77745`).
- The `.mfp` package format.

## Blast Radius

Found by `grep -n "record_fields(\|expression_type(" src/monomorph/lower.rs`. Every consumer of the
missing imported layouts in the monomorphizer:

- `resolve_imported_overload` fed by `expression_type` → **fixed by this bug** (the reproduction).
- `resolve_overload` (local overload set) fed by the same `arg_types` — a *local* overloaded function
  called with an imported record's field gets `Unknown` too. **Observed (Phase 1, in scope):**
  `pick(h.first)` against local `pick(ov::A)`/`pick(ov::B)` fails with
  `SYMBOL_UNKNOWN_IDENTIFIER` "Callable `pick` is not a top-level function" — no overload resolved,
  so the unmangled name reached name resolution.
- `instantiate_function` (generic templates) fed by the same `arg_types` — a generic called with an
  imported field would bind its type parameter against `Unknown`. **Observed (Phase 1, in scope):**
  `ov::show(ident(h.first))` fails with `TYPE_OVERLOAD_AMBIGUOUS`.
- `record_fields` at the constructor path (`let field_types = … .and_then(|type_| self.record_fields(&type_).cloned())`)
  — constructing an *imported* record in a consumer gets no expected field types, so an untyped
  `[]` field argument would stay `Unknown`. **Unaffected (Phase 1):** `ov::Holder[[], [], ov::A[1]]`
  builds and runs on the unfixed compiler (later passes type the `[]` fields); kept as a guard.
- `expression_type`'s `HirExpression::Constructor` arm (`else if self.record_fields(type_).is_some()`)
  — a constructor of an imported record types as `None`. Covered by the same guard row.
- Consumer-local `TYPE A` coexisting with `ov::A`: **out of scope, separate pre-existing bug.** On
  the unfixed compiler `ov::show(h.first)` fails with this bug's `TYPE_OVERLOAD_AMBIGUOUS`; with
  the fix it gets past monomorphization and fails with `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH:
  Argument 1 for `A` has type Integer, expected String for field `z`` (the package's own `A[1]`
  checked against the consumer's `A`). The unfixed compiler fails identically with **no overloaded
  call at all** (`io::print(ov::one(h.first))`), so it is a local/imported bare-name collision,
  not this mechanism. Filed separately; the row is not in this bug's regression test.
- IR lowering (`src/ir/lower.rs`) — **unaffected**: it already folds `imported_type_defs` into
  `TypeIndex` (`aa3a77745`), which is why the non-overloaded rows build.

## Fix Design

Give the monomorphizer the imported record/union/enum layouts the same way IR lowering gets them:
decode each installed dependency `.mfp` with `src/manifest/package.rs:imported_type_defs` and
insert each record as a `HirTypeDecl` into `concrete_types`, keyed by the same `ParameterType`
spelling `normalize_type` produces for the qualified imported name. Then `record_fields` and
`expression_type`'s `MemberAccess` arm type imported fields exactly as local ones. No change to
`resolve_imported_overload` or `types_compatible`.

Where the risk concentrates:

- **Key spelling.** An imported type may be spelled `ov::A`, `ov.A`, or bare `A` depending on how
  the consumer named it and how `normalize_type` strips package qualifiers. A key that does not
  match what `expression_type` produces leaves the bug in place silently. Phase 1's table rows are
  the check.
- **Collision with a local type of the same bare name.** A consumer declaring its own `A` while
  importing `ov::A` must keep both distinct; inserting imported layouts under a bare key would let
  one overwrite the other. Phase 1 adds a reproduction row for that case.
- **`concrete_types` is also iterated for emission** (`let types = self.concrete_types.values()…`).
  Imported layouts must not be re-emitted as consumer-owned type declarations. They may need a
  separate `imported_types` map that `record_fields` consults after `concrete_types`, rather than
  living in `concrete_types`. Phase 2 decides by reading those iteration sites; the separate map is
  the default if any of them emits.

Rejected:

- Resolve ambiguity in favour of a union-typed overload, or the first export — violates bug-36.
- Special-case `FOR EACH` variables — the trigger is any imported field access, not the loop.
- Require users to annotate — the program is valid; the local path needs no annotation.

Expected output shifts: none for programs that build today (they never reached this path with an
imported field argument that mattered). IR goldens are unaffected unless a golden fixture calls an
overloaded or generic function with an imported field argument; Phase 3 checks.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [x] Add `tests/runtime/rt_imported_overload_imported_field_argument.rs`, modelled on
      `tests/runtime/rt_imported_record_map_field_keys.rs` (builds the package and consumer from
      source in a temp root). One case per ✗ row of the reproduction table, each asserting the build
      succeeds and the program prints the expected output; plus the ✓ rows as guards. Register it
      the way `rt_imported_record_map_field_keys.rs` is registered.
- [x] Add rows for the latent blast-radius sites: a local overload set called with `h.first`; a
      generic `FUNC id OF T(x AS T) AS T` called with `h.first`; constructing `ov::Holder` with an
      untyped `[]` field; a consumer-local `TYPE A` coexisting with `ov::A`. Record each observed
      result in this document's Blast Radius section (✗ becomes in-scope; ✓ becomes "unaffected
      because …").
- [x] Confirm every ✗ case fails with `TYPE_OVERLOAD_AMBIGUOUS` (or the latent case's own error).

Acceptance: the new test fails only on the documented cases, each for the documented reason.
  Check: `cargo test --test rt_imported_overload_imported_field_argument` → the ✗ cases fail with
  `TYPE_OVERLOAD_AMBIGUOUS`, the ✓ guards pass (est. 3 min).
Commit: 73ab94edc

### Phase 2 — the fix

- [x] `src/monomorph/lower.rs:Monomorphizer::new` (and whatever constructs it with the project's
      installed packages) — load `imported_type_defs` for the consumer's dependencies and make their
      record layouts visible to `record_fields`, per Fix Design (separate map if `concrete_types` is
      emitted from).
- [x] `src/monomorph/lower.rs:record_fields` — consult the imported layouts after `concrete_types`.
- [x] Apply to every in-scope latent site Phase 1 confirmed.

Acceptance: the Phase 1 test passes; bug-36's tests pass unmodified.
  Check: `cargo test --test rt_imported_overload_imported_field_argument` → all pass;
  `cargo test --bin mfb monomorph` → all pass (est. 5 min).
Commit: 73ab94edc

### Phase 3 — regenerate expected outputs + full validation

- [x] Run the IR golden gate; any diff is inspected fixture by fixture (AGENTS.md: an unexpected
      golden diff is a bug-hunt trigger). No golden is re-baselined without the four answers
      AGENTS.md requires.
- [x] Run the full suite.
- [x] Re-run the reproduction table end to end with the rebuilt `target/release/mfb`; every row
      prints its expected output.
- [ ] Update `planning/plan-138-B-xml-serializer-and-docs.md` Prerequisites status to MET.

Acceptance: full suite green; golden deltas none or exactly explained; every reproduction row
builds and prints its expected output.
  Check: `cargo test` → exit 0 (est. per `.ai/testing-gates.md`); reproduction table re-run → all ✓.
Commit: c30373da7

## Validation Plan

- Regression test: `tests/runtime/rt_imported_overload_imported_field_argument.rs` (failing then
  passing), plus the unchanged bug-36 unit tests in `src/monomorph/lower.rs`.
- Runtime proof: the reproduction table rebuilt and run with the fixed compiler, and plan-138-B's
  `FOR EACH n IN doc.children` / `xml::stringify(n)` consumer building.
- Doc sync: `mfb spec` monomorphization / package-import sections if they describe which types
  overload resolution sees (`scripts/spec-census.sh --citations` after the change; `.ai/specifications.md`).
- Full suite: `cargo test`.

## Open Decisions

- Imported layouts in `concrete_types` vs. a separate map — **separate map** unless Phase 2 shows no
  `concrete_types` iteration emits declarations. The separate map cannot re-emit a foreign type or
  collide with a local bare name. (Fix Design)

## Summary

The real risk is the key spelling and local-name collisions when making imported layouts visible
to `record_fields`; resolution logic and bug-36's ambiguity rule stay untouched. The workaround
until then is a typed local (`LET t AS ov::A = a`).
