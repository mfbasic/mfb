<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-435: a re-exported union's transitively-referenced field types (records/enums from the owner package) are dropped from a package's type-export closure

Last updated: 2026-08-08
Effort: medium (2h–4h)
Severity: MEDIUM
Class: Incompleteness (package type-closure serialization is non-transitive — blocks a valid cross-package pattern; hard compile error, no wrong runtime output)

Status: Open
Regression Test: tests/syntax/packages/reexport-union-owner-record-field-* (new)

When package **B** re-exports a type from a dependency **A** (by naming A's type in an exported
function/record signature), B's `.mfp` stores that type as a *foreign marker* and the importer
resolves its real definition from A's sibling `.mfp` at read time (the bug-390 design). That
resolution is **shallow**: it fills in the foreign type's own fields/variants but does **not**
recursively pull in the *other* user types that those fields/variants reference. So if A's
re-exported union `Node` has a variant `ElementNode` whose field is a **record** `Style` (which in
turn references **enums**), then `Style` and the enums are never added to B's resolved type-export
closure. B's package is therefore **not self-contained**, and any importer that walks B's exported
`Node` hits `Style` as an unknown type and rejects the whole package:

```
error[6-605-0001 PACKAGE_INVALID]: package container is malformed or incompatible
  Imported package `.../display.mfp` has exported union `Node` that references unknown type `Style`.
```

The single correct behavior a fix produces: **a package's resolved type-export closure is transitive
— every user type (record or enum) reachable through a re-exported type's fields/variants is included
(resolved from its owner), so the package is self-contained and importable regardless of import order
or whether the importer also imports the owner package.**

This is exactly the limitation the `dom` example package worked around by making `StyleNode` a union
*variant* rather than a record field (a variant's layout travels with the union; a record reached
transitively does not). It stayed latent until a variant gained a field whose type is a user record.

References:

- Producer: `src/binary_repr/builder.rs:233` `package_type_exports` iterates only
  `package.project.abi.exports` (lines 251-257); a re-exported dependency type is emitted as a
  FOREIGN marker with empty fields/variants (bug-390, lines 264-278).
- Reader: `src/binary_repr/mod.rs:592` `read_package_type_exports_resolved`; the resolve loop
  (lines 610-641) fills each foreign marker's def from the owner `.mfp` via
  `find(|candidate| candidate.name == export.name)` (632-640) — it never walks the resolved def's
  field/variant types to pull *their* referenced user types.
- Consumer: `src/syntaxcheck/mod.rs:665-680` emits the `PACKAGE_INVALID` "references unknown type"
  when `type_infos.get(name)` misses; enums are treated as leaves (`:682`,
  `validate_imported_package_type` `:558`). Imports are installed-then-validated **interleaved,
  per-import, in source order** (`:444-454`), which makes the failure additionally sensitive to
  import order.

## Failing Reproduction

**Measured** in the `browser` example (branch `worktree-browser`) after adding `style AS Style` to
`dom`'s `ElementNode` (`examples/browser/dom/src/style.mfb` defines `Style` + enums `Display`,
`FlexDirection`, `FlexWrap`, `Justify`, `Align`):

```sh
# dom builds; so do fetch and display (each re-exports dom::Node in its API):
target/release/mfb build examples/browser/dom
target/release/mfb build examples/browser/fetch     # LoadResult.document AS Node
target/release/mfb build examples/browser/display    # render(n AS Node), tree(n AS Node)
# app imports dom, fetch, display — and FAILS:
target/release/mfb build examples/browser/app
# error[6-605-0001 PACKAGE_INVALID] ... display.mfp has exported union `Node`
#   that references unknown type `Style`.
```

Direct evidence the closure is incomplete (`display.mfp` carries `Node`'s variants but not the
record/enums they reference):

```sh
strings examples/browser/display/display.mfp | grep -E 'Style|FlexDirection|FlexWrap|Justify|Align'
# -> only "Style" (from the StyleNode variant); NO FlexDirection/FlexWrap/Justify/Align, no
#    standalone Style record, no defaultStyle.
```

### Minimal distilled repro (author this as the RED test)

Always fails regardless of import order, because the app never imports the owner:

- pkg **leaf** (owner): `EXPORT ENUM Kind { Alpha, Beta }`; `EXPORT TYPE Meta { kind AS Kind, n AS
  Integer }`; `EXPORT TYPE Box { meta AS Meta }`; `EXPORT TYPE Leaf { text AS String }`;
  `EXPORT UNION Node { Box, Leaf }`; plus a constructor returning a `Node`.
- pkg **mid** (re-exporter): `IMPORT leaf`; `EXPORT FUNC describe(n AS Node) AS String` (MATCHes the
  variants). This re-exports `Node` in its ABI.
- app **useonly-mid**: `IMPORT mid` only (NOT leaf). Building it must fail with `PACKAGE_INVALID`
  "references unknown type `Meta`" (or `Kind`). After the fix it must build.

## Root Cause

The serialized/resolved type-export closure of a package is **not transitive**. A re-exported type is
resolved (its own fields/variants filled from the owner), but the resolver stops there — it does not
treat the user types *inside* those fields/variants as things that must also be present. Before a
union variant ever had a user-record field, every re-exported closure was self-contained by accident
(field types were all builtins — `String`/`Map`/`List` — plus the union itself), so the gap never
showed.

## Goal

`read_package_type_exports_resolved` (and therefore every importer's installed `type_infos`) returns
the full transitive closure of user types reachable from a package's exported types: for each
resolved record field / union variant field / function param+return, any `Type::User(name)` that is
not already in the returned list is resolved from its owner `.mfp` and appended (records recursing
into their fields, unions into their variants' fields; enums are leaves). Cycle-guarded
(`List OF Node` self-reference must terminate).

### Non-goals (must NOT change)

- No change to the `.mfp` on-disk format is required if the fix lives in the **reader** (preferred):
  the owner `.mfp` already carries `Style`/enums in its own exports; the reader just isn't pulling
  them. Do not gratuitously bump the container version.
- Do not change how a package references builtins or `Node`-self; only user types reached
  transitively are newly included.
- Do not weaken the `PACKAGE_INVALID` check itself — a genuinely missing owner `.mfp` must still be
  reported.

## Blast Radius

- Every package import flows through `read_package_type_exports_resolved` → `install_package_type_info`
  → `validate_imported_package_type`. Pulling more types means more `type_infos` installs; installs
  are last-wins `HashMap` (`src/binary_repr/reader.rs:656` note), so duplicates from multiple import
  paths are safe.
- Interacts with the per-import install/validate ordering in `src/syntaxcheck/mod.rs:444-454`. A
  reader-side closure fix makes each package self-contained, so order no longer matters; optionally
  also hoist all installs before all validations as defense-in-depth.
- Re-export chains (mid re-exports A which re-exports …): the resolver already recurses with a
  `MAX_REEXPORT_DEPTH` guard (`mod.rs:596`); the closure walk needs its own `seen` set keyed by type
  name to avoid re-resolving and to terminate on self-referential unions.

## Fix Design

Primary (reader-side, least invasive): in `read_package_type_exports_resolved`, after the existing
foreign-marker fill, compute the transitive closure. Maintain a `seen: HashSet<String>` of type names
already in `exports`. Walk each export's resolved fields/variants collecting `Type::User` names; for
any name not in `seen`, resolve it from the appropriate owner `.mfp` (the owner recorded on the
foreign marker, or the current package for locally-defined types), append the resolved
`BinaryReprTypeExport`, add to `seen`, and continue until no new names appear. Enums resolve to a
leaf export (members only). This mirrors the recursion already in
`syntaxcheck::validate_package_metadata_type` but on the *producer/read* side so the data is present
before validation runs.

Defense-in-depth (consumer-side): split `src/syntaxcheck/mod.rs:444-454` so all imported packages'
`install_package_type_info` run before any `validate_imported_package_type`, removing the
source-order sensitivity even for partially-resolved inputs.

Alternative (writer-side): have `package_type_exports` emit a foreign marker for the full transitive
closure of re-exported types (not just the directly-named ones). Keeps the reader simple but touches
the write path and every re-exporting package's bytes. Prefer the reader-side fix unless the closure
is cheaper to compute at write time.

## Phases

### Phase 1 — failing test + audit (no behavior change)
- Add the minimal `leaf`/`mid`/`useonly-mid` package fixtures under `tests/syntax/packages/`; assert
  the current `PACKAGE_INVALID` "unknown type" failure (RED).
- Confirm which owner each transitively-referenced type belongs to in a multi-hop chain.

### Phase 2 — the fix
- Implement the reader-side transitive closure in `read_package_type_exports_resolved` (cycle-guarded).
- Optionally hoist consumer installs before validations.

### Phase 3 — validation
- Minimal repro app now builds; enums resolve as leaves; self-referential unions terminate.
- The `browser` example (`worktree-browser`) builds end to end (dom→fetch/display→app) once
  `ElementNode` carries `style AS Style`.
- Full `cargo test` green; existing package/cross-package tests unchanged.

## Validation Plan

- New syntax fixtures (above) go RED→GREEN.
- `cargo test` (esp. `src/binary_repr/tests/cross_package_tests.rs`) stays green.
- Re-run the browser reproduction command block; the `strings display.mfp` check now shows the
  enums/record present in the resolved closure (or, if reader-only, `app` build succeeds without them
  needing to be in `display.mfp` bytes).

## Open Decisions

- Reader-side closure vs. writer-side foreign-marker enumeration (default: reader-side).
- Whether to also fix the consumer install/validate ordering now or leave it (default: fix as
  defense-in-depth, since it's one loop split and removes a latent order dependency).

## Summary

A package that re-exports a dependency's union/record is not self-contained when that type's
fields/variants reference other user records/enums from the owner: the reader resolves the named type
but not its transitive type closure, so importers reject the package with `PACKAGE_INVALID:
references unknown type`. Fix by making the resolved type-export closure transitive (reader-side,
cycle-guarded), optionally hoisting consumer installs before validation. Discovered while adding a
resolved-`Style` record field to the browser example's `dom::ElementNode`.
