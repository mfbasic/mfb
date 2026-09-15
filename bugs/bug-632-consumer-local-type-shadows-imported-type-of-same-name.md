# bug-632: a consumer-local type with the same bare name as an imported package type breaks the package's own constructors

Last updated: 2026-09-15
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: tests/runtime/rt_imported_type_name_collision.rs (to add, Phase 1)

A consumer that declares its own `TYPE A` while importing a package that exports a different
`TYPE A` fails to build. The error is raised against code the consumer never wrote: the package's
own `A[1]` constructor is checked against the consumer's `A` layout.

```
error: TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type Integer, expected String for field `z`.
```

The diagnostic carries no file or line, so the user cannot tell that the conflict is with a
package's type. Two packages can't be expected to coordinate type names with every consumer, so
any common name (`Node`, `Item`, `Error`-like records) can break a build.

**The single correct behavior a fix produces:** a consumer's `TYPE A` and an imported `pkg::A`
are distinct types. The package's code keeps using the package's layout, the consumer's code uses
its own, and the program builds and runs.

References:

- Found while fixing bug-631 (its Blast Radius "consumer-local `TYPE A`" row).
- `aa3a77745` — IR lowering's `TypeIndex` folds imported layouts in with "a locally-declared type
  always wins", and bug-631 kept that rule for the monomorphizer's `record_fields`. Both are keyed
  by bare name, which is the same collision on the typing side.

## Failing Reproduction

Built with `target/release/mfb` from `main` at `d7645c14d` (without bug-631's fix), macOS aarch64.
The package is bug-631's `ov` (exports `TYPE A` with `x AS Integer`, a `Holder` with
`first AS A`, `FUNC one(a AS A) AS String`, and `FUNC holder() AS Holder`, which constructs
`A[1]`, `A[2]`, `A[3]`).

Consumer `src/main.mfb`:

```
IMPORT ov
IMPORT io
IMPORT collections

TYPE A
  z AS String
END TYPE

FUNC main() AS Integer
  LET h AS ov::Holder = ov::holder()
  LET mine AS A = A["z"]
  io::print(mine.z)
  io::print(ov::one(h.first))
  RETURN 0
END FUNC
```

```
mfb build ov && cp ov/ov.mfp app/packages/ && mfb build app
```

- Observed: `error: TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type Integer, expected String for field `z`.`
- Expected: builds; prints `z`, `one`.

Contrast: rename the consumer's type to `Mine` → builds and runs (bug-631's guard cases).

## Root Cause

Hypotheses, most likely first:

1. The emitted diagnostic comes from `src/ir/verify/compat.rs` (the constructor check using
   `record_field_lists.get(type_)`), which keys record layouts by bare nominal. After the package's
   code is merged into the consumer, the package's `A[1]` and the consumer's `A` share the key
   `A`, and the consumer's layout wins. Confirm by finding where `record_field_lists` is populated
   and whether the package's functions reach verification with an unqualified or unprefixed `A`.
2. The package merge identity-prefixes function symbols but not the nominal type names inside the
   merged bodies, so the collision is created at merge time rather than in verification. Confirm by
   dumping `-ir` for the consumer and checking the type name on the package's constructor.

## Goal

- The reproduction builds and prints `z`, `one`; a consumer can also pass `h.first` to an
  overloaded `ov::show` while declaring its own `A` (bug-631's collision row).

### Non-goals (must NOT change)

- bug-631's fix and `aa3a77745`'s layout folding for consumers without a name collision.
- The `.mfp` package format, unless the design shows it has to change. If it does, that becomes
  its own decision in this doc.
- **Tempting wrong fix, forbidden:** rejecting a consumer type whose name matches an imported one.
  The program is valid.

## Blast Radius

To audit in Phase 1, by search (`grep -rn "record_field_lists\|TypeIndex::new\|imported_records" src`):

- `src/ir/verify/compat.rs` constructor check — observed failing site.
- `src/ir/lower.rs:TypeIndex::new` — local-wins bare-name keying; unverified.
- `src/monomorph/lower.rs:record_fields` — local-wins bare-name keying (bug-631); unverified.
- Union variants and enums with colliding bare names — unverified.

## Fix Design

To be decided in Phase 1 once the hypothesis is confirmed. The likely shape is to key imported
nominals by their package-qualified identity wherever package code and consumer code share a type
table. This doc does not choose yet.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add `tests/runtime/rt_imported_type_name_collision.rs` (modelled on
      `tests/runtime/rt_imported_overload_imported_field_argument.rs`) with the reproduction, plus
      bug-631's collision row (`ov::show(h.first)` with a local `TYPE A`). Confirm both are RED for
      the documented reason.
- [ ] Confirm or eliminate each Root Cause hypothesis; complete the Blast Radius verdicts.

Acceptance: the tests fail with `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` for the documented reason; the
audit has a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Per the Fix Design chosen in Phase 1.

Acceptance: the Phase 1 tests pass; bug-631's regression test still passes.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Run the artifact gate; inspect any golden diff fixture by fixture.
- [ ] Run the full suite (`cargo test --no-fail-fast`).
- [ ] Re-run the reproduction end to end.

Acceptance: full suite green; golden deltas none or exactly explained.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_imported_type_name_collision.rs`.
- Full suite: `cargo test --no-fail-fast`.

## Summary

Local and imported types share a bare-name key in at least one type table, so a consumer's
`TYPE A` silently replaces a package's `A` layout. The workaround until then is renaming the
consumer's type.
