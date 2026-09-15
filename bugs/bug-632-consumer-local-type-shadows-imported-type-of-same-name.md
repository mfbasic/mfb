# bug-632: two types with the same bare name from different packages (or a package and its consumer) collapse into one at package merge

Last updated: 2026-09-15
Effort: large (3h–1d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/runtime/rt_imported_type_name_collision.rs (to add, Phase 1)

A program fails to build when two of the types that end up in it share a bare name. This happens
when:

- the consumer declares `TYPE A` and imports a package that exports its own `TYPE A`, or
- the consumer imports two packages that each export a `TYPE A`. No consumer type is involved.

The error is raised against code the user never wrote. One `A`'s constructor is checked against
the other `A`'s layout:

```
error: TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type Integer, expected String for field `z`.
```

The diagnostic has no file or line, so the user cannot tell the conflict is between packages.
Packages can't coordinate type names with every consumer and every other package, so any common
name (`Node`, `Item`, `Config`, `Options`) can make two otherwise-valid dependencies unusable
together.

**The single correct behavior a fix produces:** a type keeps its package identity through the
whole build. `ov::A`, `pa::A`, `pb::A` and a consumer's own `A` are four distinct types. Each
package's code uses its own layout, the consumer's code uses its own, and the program builds and
runs.

References:

- Found while fixing bug-631 (`bugs/completed/bug-631-…`, Blast Radius "consumer-local `TYPE A`").
- `src/ir/package.rs:prefix_package_symbols` doc comment: "Types are left unqualified."
- bug-251 — the same collision class for LINK aliases, fixed by folding the alias into the identity
  prefix. That fix is the model here.
- `aa3a77745` and bug-631 — consumer-side imported type layouts in IR lowering's `TypeIndex` and
  the monomorphizer's `record_fields`, both keyed by bare name with "local wins".

## Failing Reproduction

Built with `target/release/mfb` from `main` at `d7645c14d`, macOS aarch64. All projects use
`kind: package` / `kind: executable` manifests with `packages` entries of the form
`{"name":"<pkg>","version":"=0.1.0","source":"file:packages/<pkg>.mfp"}`.

**Case 1 — consumer type vs package type** (`/tmp/b632`)

`ov/src/lib.mfb`:

```
EXPORT TYPE A
  x AS Integer
END TYPE

EXPORT FUNC make() AS A
  RETURN A[1]
END FUNC

EXPORT FUNC one(a AS A) AS String
  RETURN "one"
END FUNC
```

`app/src/main.mfb`:

```
IMPORT ov
IMPORT io

TYPE A
  z AS String
END TYPE

FUNC main() AS Integer
  LET mine AS A = A["z"]
  io::print(mine.z)
  io::print(ov::one(ov::make()))
  RETURN 0
END FUNC
```

```
mfb build ov && cp ov/ov.mfp app/packages/ && mfb build app
```

- Observed: `error: TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type Integer, expected String for field `z`.`
- Expected: builds; prints `z`, `one`.

**Case 2 — package type vs package type** (`/tmp/b632b`)

`pa/src/lib.mfb`:

```
EXPORT TYPE A
  x AS Integer
END TYPE

EXPORT FUNC describe() AS String
  LET a AS A = A[7]
  RETURN "pa:" & toString(a.x)
END FUNC
```

`pb/src/lib.mfb`:

```
EXPORT TYPE A
  s AS String
END TYPE

EXPORT FUNC describe() AS String
  LET a AS A = A["hi"]
  RETURN "pb:" & a.s
END FUNC
```

`app/src/main.mfb` (imports both):

```
IMPORT pa
IMPORT pb
IMPORT io

FUNC main() AS Integer
  io::print(pa::describe())
  io::print(pb::describe())
  RETURN 0
END FUNC
```

- Observed: `error: TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type String, expected Integer for field `x`.`
- Expected: builds; prints `pa:7`, `pb:hi`.

Contrast: rename either colliding type (consumer `A` → `Mine`, or `pb`'s `A` → `B`) → builds and
runs. Two packages that both export a *function* named `describe` never collide (Case 2 shows it):
functions are identity-prefixed.

## Root Cause

Confirmed by reading the merge path. Line references are at `d7645c14d`.

1. `src/target/shared/nir/lower.rs:merge_packages` starts from the consumer's IR
   (`let mut merged = ir.clone()`). For each package, in manifest order, it decodes the package IR,
   calls `crate::ir::prefix_package_symbols(&mut package_ir, &id)`, and then
   `crate::ir::merge_package(&mut merged, package_ir)`.
2. `src/ir/package.rs:prefix_package_symbols` renames the package's functions, globals, entry and
   LINK aliases to `<id>.<package>.<name>` and rewrites every internal reference to match.
   **Types are left unqualified** (its doc comment says so). The package's `TYPE A` stays `A`, and
   so do its constructor `A[1]` and every signature and field that names `A`.
3. `src/ir/package.rs:merge_package` merges types with
   `push_unique(&mut project.types, package.types, |a, b| a.name == b.name)`. The consumer's (or
   an earlier package's) `A` is already in `project.types`, so the incoming package's `A` is
   discarded as a duplicate. Its code still refers to `A`, which now resolves to the other layout.
4. `merge_packages` then runs `crate::ir::verify_semantics(&merged)`.
   `src/ir/verify/mod.rs` builds `record_field_lists` keyed by `ParameterType::declared(&ty.name)`
   from the single surviving `A`, and `src/ir/verify/compat.rs` (the constructor check) reports the
   package's `A[1]` against it. The verifier is correct; it is checking IR in which the second `A`
   no longer exists.

Had verification not caught it, codegen would have laid out the discarded `A`'s values with the
surviving `A`'s layout, which makes this a memory-safety hazard, not just a build failure.

Why the contrasts are immune: distinct bare names never meet `push_unique`'s equality. Functions
are immune because step 2 prefixes them before the merge.

## Goal

- Both reproduction cases build and print their expected output.
- bug-631's collision row builds and prints its output: a consumer declaring `TYPE A` that passes
  `h.first` to an overloaded `ov::show`.
- A diamond import (the same package reached through two dependency paths) still collapses to one
  copy of its types, exactly as it does for functions today.

### Non-goals (must NOT change)

- **Tempting wrong fix, forbidden:** rejecting a program whose consumer or packages declare types
  with the same bare name. The programs are valid.
- **Tempting wrong fix, forbidden:** making the merge "prefer the package's layout", or any other
  rule choosing one of two different layouts. Both types must survive.
- **Tempting wrong fix, forbidden:** relaxing `ir/verify`'s constructor check. The check is the only
  thing standing between this and wrong-layout codegen.
- The `.mfp` package format, unless Phase 1 shows a type's identity cannot be recovered at merge
  time without it. In that case, record it as an explicit decision here before changing it.
- bug-631's behavior for consumers with no name collision, and bug-251's LINK alias prefixing.
- Built-in package types (`json.JsonNull` etc., bug-480's package-qualified builtin identities) —
  confirm unaffected in Phase 1.

## Blast Radius

Phase 1 must map every place a package type's name is stored or compared after decode, by search,
not from memory. Starting points:

- `src/ir/package.rs:prefix_package_symbols` — **fixed by this bug**: it needs to qualify the
  package's own types and every reference to them.
- `src/ir/package.rs:merge_package` type `push_unique` — **fixed by this bug**: once names are
  qualified, identity equality keeps two different `A`s distinct and collapses a diamond.
- `src/ir/package.rs:apply_package_identity` — the consumer's references to `ov::A` (and
  inter-package references) must be rewritten to the qualified name, as function references are
  today. **In scope.**
- IR type references to audit: function params/returns, record fields, union variants and
  `includes`, constructor/`UnionWrap`/`MATCH` ops, casts, and any NIR symbol mangled from a type
  name (overload names like `show$A`, runtime type tags). Verdict per site in Phase 1.
- `src/ir/lower.rs:TypeIndex::new` and `src/monomorph/lower.rs:record_fields` (bug-631) — the
  consumer-side front end keys imported layouts by bare name with "local wins". In Case 1 the
  consumer's `ov::A` is typed with its own `A` layout before the merge is reached. Phase 1 decides
  whether the front end needs the qualified identity too, or whether it only has to hand the merge
  the right *reference* spelling. Verdict required.
- `src/ir/verify/mod.rs` imported-layout seeding (`record_field_lists`, `field_types`, unions,
  enums) — the same bare keys on the consumer-only verify path. Verdict required.
- Built-in packages — their types reach the AST, not the merge. Expected unaffected; confirm.

## Fix Design

Give package types the same identity treatment functions already get, in the same places:

- `prefix_package_symbols` renames each package type declaration to its identity-qualified name
  and rewrites every type reference inside that package's IR.
- `merge_package` then dedups by the qualified name. Two packages' `A`s differ, and a diamond's
  collapses.
- `apply_package_identity` rewrites the consumer's and other packages' references to that
  package's types from the `package.A` spelling to the qualified one.

The consumer's own types stay unqualified, like its functions.

Where the risk concentrates:

- **Coverage of type references.** A type name missed in one IR position leaves a dangling or
  wrong reference. Build a single visitor over every type-bearing IR position, as
  `visit_project_targets_mut` does for function targets, rather than patching sites.
- **The spelling a consumer uses.** The front end currently strips `ov::A` to `A`, so the consumer's
  IR may not say *which* package's `A` it means by the time it reaches the merge. If so, the
  qualifier has to be preserved from lowering onward; that is the biggest open question and
  Phase 1 answers it first.
- **Anything derived from a type's name**: mangled overload symbols, runtime type names in
  diagnostics or reflection, `.ncode` goldens. Expect golden shifts only in fixtures that import
  packages with types; each one is inspected.

Rejected:

- Renaming only on collision. Merge order would decide which copy gets renamed, and a type's
  identity would depend on what else happens to be imported.
- Keying the verifier/codegen tables by `(package, name)` without renaming. Every consumer of
  `IrProject.types` would need the same pair, which is the same audit with more surface.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_imported_type_name_collision.rs` (modelled on
      `tests/runtime/rt_imported_overload_imported_field_argument.rs`) with Case 1, Case 2, bug-631's
      collision row, and a diamond-import guard. Confirm each is RED for the documented reason (the
      diamond guard should already pass).
- [ ] Answer the front-end spelling question: does a consumer's `ov::A` reach the merge as
      something that identifies `ov`?
- [ ] Complete the Blast Radius verdicts.

Acceptance: the collision tests fail with `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` for the documented
reason; the diamond guard passes; every audit site has a verdict.
Commit: —

### Phase 2 — the fix

- [ ] Qualify package types in `prefix_package_symbols`, with one visitor over every type-bearing
      IR position.
- [ ] Rewrite external type references in `apply_package_identity`.
- [ ] Apply to every in-scope site from the audit (front-end spelling included, if Phase 1 says so).

Acceptance: the Phase 1 tests pass; bug-631's regression test still passes; the diamond guard still
passes.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Run the artifact gate; inspect any golden diff fixture by fixture.
- [ ] Run the full suite (`cargo test --no-fail-fast`).
- [ ] Re-run both reproduction cases end to end.

Acceptance: full suite green; golden deltas none or exactly explained.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_imported_type_name_collision.rs`.
- bug-631's `rt_imported_overload_imported_field_argument` unchanged and green.
- Full suite: `cargo test --no-fail-fast`, including `artifact_gate_all`.

## Summary

Package functions are namespaced at merge time; package types are not. The merge keeps the first
type of a given bare name and discards the rest, so two dependencies (or a dependency and its
consumer) that both define `A` can't be used together. The workaround until then is renaming one
of the types, which is only possible when you own one of them.
