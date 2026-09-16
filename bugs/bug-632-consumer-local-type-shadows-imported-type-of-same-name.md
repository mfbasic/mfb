# bug-632: an imported user-package type has no package identity — two types with the same bare name collapse into one

Last updated: 2026-09-15
Effort: x-large (1d–3d)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: tests/runtime/rt_imported_type_name_collision.rs

A program fails to build when two of its types share a bare name. There are three ways to hit it:

- the consumer declares `TYPE A` and imports a package that exports its own `TYPE A`;
- the consumer imports two packages that each export a `TYPE A`, and declares no type itself;
- the consumer names `ov::A` explicitly while also declaring its own `A`.

The error is raised against code the user never wrote, or against the wrong type:

```
error: TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type Integer, expected String for field `z`.
error[2-203-0045 TYPE_UNKNOWN_FIELD]: record field does not exist
```

Packages can't coordinate type names with every consumer and every other package, so any common
name (`Node`, `Item`, `Config`, `Options`) can make two otherwise-valid dependencies unusable
together. Had verification not caught it, codegen would have laid out one `A`'s values with the
other `A`'s layout.

**The single correct behavior a fix produces:** the language rule in spec §13 holds for user
packages exactly as it already does for built-in ones. An imported type is written `pkg::A`, its
identity is package-qualified all the way through the compiler, a bare `A` means only a local
type, and `ov::A`, `pa::A`, `pb::A` and a consumer's own `A` are four distinct types.

References:

- Spec `src/docs/spec/language/13_modules-and-packages.md:53-78`: "A name defined **locally** …
  needs no prefix. A name reached through an `IMPORT` **requires** one. This holds for every kind
  of name alike: … records, unions, union variants, enums, enum members, and resource types. …
  A bare imported type is refused with `SYMBOL_UNKNOWN_TYPE`; two packages may therefore export
  the same leaf name without colliding (`http::Stream` and `process::Stream` are different
  types)."
- `bugs/completed/bug-480-package-name-resolution.md` — established that rule and implemented it
  for **built-in** packages. Its design, "the package dimension lives in the NAME and no table
  needed a new key", made a built-in value type's declared identity `net.PingStatus`, as plan-97
  did for resources. Its Phase 2 box re-measuring the **user-package** half was left `[~]` and
  deferred to Phase 4b, which then covered built-ins only.
- `8f0ebfeb8` "a qualified imported package TYPE named a type nothing else knew" — made
  `pkg::Note` work for user packages by **de-qualifying it to bare `Note`**, the opposite of
  bug-480's design, because `resolver::packages::install_package_type_names` installs imported
  types bare. That is the convention this bug removes.
- `781a82f07` bug-484 "a bare type name means the LOCAL one, everywhere" — the same rule enforced
  for the built-in type registry.
- bug-251 — the collision class for LINK aliases, fixed by folding the package identity into the
  name.
- Found while fixing bug-631.

## Failing Reproduction

Built with `target/release/mfb` at `330c5d81f` (includes bug-631's fix), macOS aarch64. Every
case is in `tests/runtime/rt_imported_type_name_collision.rs`, which builds the packages and the
consumer from source.

**Case 1 — consumer type vs package type.** `ov` exports `TYPE A` (`x AS Integer`),
`make() AS A` (`RETURN A[1]`) and `one(a AS A) AS String`. The consumer declares
`TYPE A` (`z AS String`), prints `mine.z`, then `ov::one(ov::make())`.

- Observed: `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type Integer, expected String for field `z`.`
- Expected: `z`, `one`.

**Case 2 — package type vs package type.** `pa` exports `TYPE A` (`x AS Integer`) and
`describe()` building `A[7]`. `pb` exports `TYPE A` (`s AS String`) and `describe()` building
`A["hi"]`. The consumer declares no type and prints `pa::describe()` then `pb::describe()`.

- Observed: `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH: Argument 1 for `A` has type String, expected Integer for field `x`.`
- Expected: `pa:7`, `pb:hi`.

**Case 3 — the consumer names both.** Case 1's consumer also writes
`LET theirs AS ov::A = ov::make()` and prints `toString(theirs.x)`.

- Observed: `TYPE_UNKNOWN_FIELD` and `TYPE_CALL_ARGUMENT_MISMATCH` on that line, from the
  consumer's own IR verify (it never reaches the merge).
- Expected: `z`, `1`.

**Case 4 — bug-631's row.** An overloaded `ov::show` called with `h.first` of an imported
`Holder`, while the consumer declares its own `A`.

- Observed: `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH`.
- Expected: `z`, `A`.

**Guard — diamond.** `left` and `right` both depend on `base`, which exports `TYPE P`; the
consumer imports all three. Passes today (`10`, `20`, `30`) and must keep passing.

Contrast: renaming either colliding type makes every case build and run.

## Root Cause

Built-in packages follow bug-480's design. User packages still use the bare-name convention that
design replaced, at every stage:

1. **Parser** — `src/ast/expr.rs:normalize_qualified_type_name` rewrites a type-position `ov::A`
   to bare `A` whenever `ov` exports `A` (`8f0ebfeb8`). From here on the consumer's IR cannot say
   which `A` it means.
2. **Resolver** — `src/resolver/packages.rs:install_package_type_names` installs every imported
   type name bare (its comment rejects bug-301 G1's request to prefix them). A bare imported type
   is therefore accepted, against spec §13.
3. **Imported layouts and signatures** — `src/manifest/package.rs:imported_type_def` /
   `package_export_signature` decode the `.mfp` with the package's own bare spellings, and every
   consumer-side table keys them bare with "local wins": `src/ir/lower.rs:TypeIndex::new`,
   `src/monomorph/helpers.rs:collect_imported_records` (bug-631),
   `src/monomorph/lower.rs:normalize_type` (strips package qualifiers to match `.mfp` overload
   names), and `src/ir/verify/mod.rs`'s imported-layout seeding. This is Case 3's failure.
4. **Merge** — `src/ir/package.rs:prefix_package_symbols` prefixes a package's functions and
   globals but, per its doc comment, "Types are left unqualified", and `merge_package` dedups
   types with `a.name == b.name`. The second `A` is discarded and its code is verified against the
   first. This is Cases 1, 2 and 4.
5. **Compatibility** — `src/ir/verify/compat.rs:compatible` treats two nominals as the same type
   when the segment after their last `.` matches (only distinct built-in resources are exempt). It
   exists because an aliased import used to spell one type two ways. Left as is, it would equate
   `pa.A`, `pb.A` and a local `A` even after the rest is fixed.

Why functions don't collide: stage 4 prefixes them, and stage 1's normalizer leaves a
qualified *identifier* (a call) alone. Why built-in types don't collide: bug-480 Phase 4b made their
declared name package-qualified.

## Goal

- Cases 1–4 build and print their expected output; the diamond guard still passes.
- A bare imported user-package type is refused with `SYMBOL_UNKNOWN_TYPE`, located, printing the
  qualified spelling, exactly as a bare built-in type is.
- `ov::A` and `IMPORT ov AS o` / `o::A` name the same type.
- A package's own source keeps writing its own types bare, and every `.mfp` already on disk keeps
  working without being rebuilt.

### Non-goals (must NOT change)

- **The `.mfp` package format.** Qualification is applied when a package is *read* (its type
  exports, signatures, overloads and IR), not written into the package. The 133 committed `.mfp`
  fixtures are therefore not rebuilt.
- Built-in packages' existing qualified identities (bug-480 Phase 4b, plan-97, bug-484).
- Function and global prefixing (`<id>.<package>.<name>`), bug-251's LINK aliases, and bug-631's
  fix (its layouts become qualified; its behavior stays).
- **Tempting wrong fix, forbidden:** any rule that picks one of two same-named types (local wins,
  first import wins, prefer the package's layout). Both types must exist.
- **Tempting wrong fix, forbidden:** relaxing `ir/verify`'s constructor or field checks. They are
  the only thing standing between this and wrong-layout codegen.
- **Tempting wrong fix, forbidden:** keeping bare imported type names accepted "for
  compatibility". Spec §13 refuses them, and accepting them is what makes a bare `A` ambiguous.

## Blast Radius

To audit by search as each phase lands. Found so far:

- **Parser:** `src/ast/expr.rs:normalize_qualified_type_name` (callers `src/ast/stmt.rs:734`,
  `src/ast/expr.rs:659`, `:1240`, `src/ast/items.rs:425`).
- **Resolver:** `src/resolver/packages.rs:install_package_type_names`,
  `src/resolver/resolution.rs:resolve_package_qualified_name`.
- **`.mfp` read sites:** `src/manifest/package.rs` (`imported_type_def`, `imported_type_field`,
  `package_export_signature`, `imported_type_names`, `imported_global_defs_from_files`), resource
  and `STATE` type names, `src/monomorph/helpers.rs` (`collect_imported_overloads` parameter
  types, `collect_imported_records`), `src/cli/build/mod.rs`'s imported resource-type set.
- **Consumer tables:** `src/ir/lower.rs:TypeIndex::new`, `src/monomorph/lower.rs:normalize_type`,
  `src/ir/verify/mod.rs` imported seeding, `src/ir/shape.rs`.
- **Merge:** `src/ir/package.rs` (`prefix_package_symbols`, `merge_package`). There is no existing
  "visit every type in an `IrProject`" helper. Every type-bearing IR position must be covered:
  `IrType.name`/`includes`/`fields`/`variants`, function params/returns, bindings,
  `IrOp::{Bind,For,ForEach}.type_`, and every typed `IrValue` (`src/ir/value.rs:174-199`
  `annotated_parameter_type` lists them), plus LINK signatures.
- **Compatibility:** `src/ir/verify/compat.rs:compatible` last-segment equality.
- **Name-derived output:** NIR symbols escape `.` (`src/target/shared/nir/symbols.rs:23-40`), so
  qualified type names are link-safe. `NirType.name` copies `IrType.name`. Expect `.nir`/`.ncode`
  golden shifts only in fixtures whose packages export types; each is inspected.
- **Tests and corpus that pin the bare convention, disproved by spec §13:**
  - `tests/runtime/rt_imported_type_qualified_name.rs:a_bare_imported_record_still_reads_its_fields`
    (written in `8f0ebfeb8`) — becomes "a bare imported record is refused".
  - `tests/rt-behavior/native/native-resource-state-import-rt/src/main.mfb`
    (`RES h AS Db STATE DbInfo`) — respelled `db::Db STATE db::DbInfo`.
  - `resolver::packages::tests` asserting bare `DbInfo` resolves.
  - `rt-behavior/resources/native-resource-import-valid` golden lines `"type": "Db"` (moved from
    `demo.Db` by `8f0ebfeb8`) — move back to the qualified identity.
  - Every other `.mfb` in `tests/` and `examples/`, and every man example, that writes a bare
    imported user-package type. Found by the full suite and `scripts/man-examples-gate.sh`, then
    each respelled with its import prefix.

## Fix Design

Extend bug-480's design to user packages: **the package lives in the type's name.** An imported
user-package type's identity is `<package>.<Name>`, spelled with the package name (never the
binding), just as `net.PingStatus` is.

1. **Parser.** In a type position, `binding::A` becomes `<package>.A` (binding resolved to its
   package), not bare `A`. A package's own source is unchanged: its types stay bare.
2. **Resolver.** Install imported type names as `<package>.<Name>`, and refuse a bare imported
   type with the built-in path's `SYMBOL_UNKNOWN_TYPE` diagnostic.
3. **One qualification helper applied at every `.mfp` read.** A `ParameterType` map that rewrites
   each nominal the package *owns* (its exported and internal records, unions, variants, enums and
   resources) from `A` to `<package>.A`, through every nested position (`ListOf`, `MapOf`,
   `ResultOf`, `Res`, `Stateful`, `Func`, `ThreadHandle`, `UserOf`). Applied to
   `ImportedTypeDef`, export signatures, overload parameter types, imported globals, and resource
   and `STATE` names. Every consumer-side table then keys `<package>.A` with no special casing,
   and "local wins" disappears because the two names differ.
4. **Merge.** `prefix_package_symbols` runs the same helper over the package's whole IR, so its
   own `A` becomes `<package>.A` in declarations and every reference. `merge_package` then dedups
   by that name: two packages' `A`s differ, and a diamond's single package collapses as it does
   today. A package IR that names *another* package's type bare (possible only in a prebuilt
   `.mfp` compiled under the old convention) is qualified through the package's recorded
   `dependencies`. If that is ambiguous, merge reports a located error; it never guesses.
5. **Compatibility.** Once every spelling is canonical, `compatible` compares full names. Delete
   the last-segment fallback, or, if an audited site still needs it, restrict it to a built-in
   identity. Justify the result by measurement, not by expectation.
6. **Monomorph.** `normalize_type` stops stripping user-package qualifiers. Imported overload
   parameter types arrive qualified (step 3), so the argument and candidate spellings match
   directly.

Why package name and not `<id>.<package>`: consumer source, signatures and diagnostics all speak
`pkg::A`, and the id exists to separate two different builds of the *same* package name, which
the import rules already forbid in one program. Functions keep their `<id>` prefix unchanged.

Rejected:

- **Key tables by `(package, name)`.** bug-480 rejected it: every table needs the new key, where a
  qualified name needs none.
- **Qualify only on collision.** Gives a type two spellings depending on what else is imported,
  and leaves the qualified path exercised only by collisions.
- **Qualify when the package is compiled** (write `pkg.A` into the `.mfp`). Forces a rebuild of
  every existing package and changes package contents; qualifying on read gives the same identity
  with neither.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] Add `tests/runtime/rt_imported_type_name_collision.rs`: Cases 1–4 and the diamond guard. RED
      at `330c5d81f`: Cases 1, 2, 4 with `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` at merge verify;
      Case 3 with `TYPE_UNKNOWN_FIELD` + `TYPE_CALL_ARGUMENT_MISMATCH` in the consumer's IR verify;
      diamond passes.
- [x] Add RED cases for the rule itself: a bare imported user-package type is refused with
      `SYMBOL_UNKNOWN_TYPE`; `IMPORT ov AS o` / `o::A` names the same type as `ov::A`; an imported
      union variant, enum member and resource type follow the same rule. RED at `330c5d81f`: the
      bare type BUILT; the alias and union/enum cases failed with `TYPE_UNKNOWN_FIELD` +
      `TYPE_CALL_ARGUMENT_MISMATCH`.
- [x] Add the cross-identity checker cases. `an_imported_value_is_refused_by_a_local_type_of_the_same_name`
      (`LET mine AS A = ov::make()` beside a local `A`) is RED on `bf630e0ae`: it builds.
      `one_packages_type_is_refused_where_another_packages_same_named_type_is_expected` already
      passes there (the shape pass's bug-41 declaration-identity check) and is kept as a guard.

Acceptance: every new case fails for its documented reason; the diamond guard passes.
Commit: 39b4576ef

### Phase 2 — the qualification helper and the `.mfp` read sites

- [x] `ParameterType::map_nominals` in `src/types.rs`.
- [x] Apply at every `.mfp` read site: `manifest::package` (`package_owned_type_names`,
      `qualify_package_type`, type defs, export signatures, globals, resource closers — bare
      resource row dropped), monomorph overload candidates (and `package_qualifiers` narrowed to
      built-in imports), `ir/shape.rs` package-interface validation, and codegen's
      `TypeModel::from_module_and_packages` / `add_package_type_export` (found in Phase 2: it
      re-read the `.mfp` and registered bare names, overwriting the consumer's own `A` —
      `native code record 'A' has no field 'z'`).
- [x] Parser (`normalize_qualified_type_name` → `<package>.Name`), resolver (qualified install,
      file-import-gated resolution), `ir/lower.rs:qualified_imported_enum`; the bare imported
      spelling is refused.

Acceptance: Case 3, the bare-refusal case and the alias case pass.
Commit: bf630e0ae

### Phase 3 — merge and compatibility

- [x] `ir::package::qualify_package_types` (called from `prefix_package_symbols`) renames the
      package's own types, variants, enums and native resources across its whole IR, including
      the two positions that name a type by STRING (found in Phase 2): a `CASE Variant(x)` pattern
      (`Local("Variant")`) and an enum member read's target (`MemberAccess { Local("Kind") }`).
      `merge_package` dedups by qualified name; the diamond guard passes. The old-`.mfp` bare
      cross-package reference fallback is not needed by any fixture so far; decided by the full
      suite.
- [x] `compat.rs:compatible` and `ir/shape.rs:compatible` stop equating a user-package-qualified
      nominal with a bare leaf; the bare fallback stays only for built-in qualifiers
      (`codegen::builtins::builtin_qualified_bare_leaf`) because a `.mfp` may record a built-in
      resource bare (`File` for `fs.File`, plan-97).
- [x] A package owns only the types it DECLARES. `package_owned_type_names` skips a re-exported
      foreign type (`foreign_owner`, bug-390), a builtin-backed resource (`tls.Listener`) and any
      already-qualified spelling (`crypto.Certificate`). Found by the full suite: the qualifier was
      being applied twice (`signer.crypto.Certificate`,
      `xfer_tls_listener_worker.tls.Listener` → `PACKAGE_INVALID` / `TYPE_CALL_ARGUMENT_MISMATCH`).

- [x] Ownership is a name → DECLARING package map, not a flat set
      (`manifest::package::package_type_owners`). Three things the full suite proved a set cannot
      express:
      - a **re-exported foreign type** is recorded bare plus its `foreign_owner` (bug-390), so
        `dom391`'s `Node` seen through `worker391` must stay `dom391.Node`; qualifying it with the
        reading package minted `worker391.Node`, a type nothing declares
        (`PACKAGE_INVALID … references unknown type 'Node'`);
      - a **native `RESOURCE`** never reaches the merge through the decoded IR (`ir/binary.rs`
        drops `native_resources` by contract), so the map is read from the `.mfp` in
        `merge_packages` and passed to `prefix_package_symbols`. Without it `sqlite3.Db` stayed
        bare in the package while every consumer-side table qualified it
        (`annotated as returning sqlite3.Db, but … returns Db`);
      - a **built-in** type a package merely references (`tls.Listener`, `crypto.Certificate`) is
        owned by nobody here and must not be qualified at all.
- [x] `MATCH` diagnostics spell types the way source writes them
      (`ir/verify/matching.rs`, bug-605's `display()` rule): "MATCH on UNION `shapes::Item` does
      not cover shapes::Tally", "CASE `shapes::Colour` is not a member of UNION `shapes::Item`".

Acceptance: Cases 1, 2 and 4 pass; the diamond guard passes; bug-631's regression test passes;
both cross-identity cases pass. **Met** — `rt_imported_type_name_collision` 10/10,
`rt_imported_overload_imported_field_argument` 11/11, `rt_imported_type_qualified_name` 5/5.
Commit: bf630e0ae (merge), d7c2f6b46 (compatibility + ownership)

**Two holes the fix closed, found by fixtures that were relying on them** (neither is migration —
each was accepting a program the checker should always have refused):

- `tests/byte-identity/tls` bound `LET peer AS net::Address = net::lookup(…)`. `net::lookup`
  answers a LIST of addresses; the old `compatible` compared the last `.` segment of the two
  RENDERED spellings, so `List OF net.Address` and `net.Address` both ended in `Address` and a
  list bound to a single-address slot. Respelled with `collections::get(…, 0)`, the spelling
  `net::ping`'s own example uses.
- `tests/rt-behavior/resources/p121d-state-reach-rt` declared its own `TYPE Accum` beside the
  `state_reach_worker` package's exported `Accum` and relied on the two agreeing **by name**
  across the boundary (its comment said so). Under spec §13 those are two types; the fixture now
  names `state_reach_worker::Accum` and declares no twin, which is what it was always testing —
  one STATE layout shared by both ends.

Also fixed in the merge rename: a `CSTRUCT`'s `maps_to` names one of the package's own RECORDS and
was left behind (`CSTRUCT 'SfFormatInfo' maps to 'AudioFormat', which is not a record type`).

**A third over-qualification, found by `examples/browser`** (`rt_debug_soak`'s paint-loop case builds
it): an export whose name is ALREADY qualified carries its identity itself. A built-in value type a
package re-exports (`regex.Group`, `http.Response`) has no `.mfp` of its own for the foreign-owner
resolution to read, so the qualifier is all there is; prefixing it again minted `regex.regex.Group`
and every consumer of such a package was refused with `PACKAGE_INVALID … exported type
'regex.Group' references unknown type 'regex.Group'`. `imported_type_def` now leaves a dotted
export name (and variant name) alone, the same rule `package_type_owners` already applied.

**The corpus migration extends to `examples/`**: `examples/browser` is a four-package example
(`dom`, `display`, `fetch`, `app`) whose `display`/`fetch`/`app` sources name `dom`'s types bare —
`Node`, `ElementNode`, `TextNode`, `HeaderNode`, `StyleNode`, `TextSpan`, `Layout`, `Justify`,
`FieldSpec`, `Style` — plus `display::{Link, PaintResult, Rect, Target}` and `fetch::LoadResult` in
`app`. Migrated in every type position, including the two the first pass missed: a constructor head
(`RETURN FieldSpec[…]`) and the type after `TO` in a thread handle
(`Thread OF String TO LoadResult`).

**Integrating `main`** (it advanced to `be9eec9a6` during this fix; merged as `6f1d4e19f`). One real
interaction, in the one function both touched:

- `main` added `scope_private_types`, which gives a package's PRIVATE types an identity-bearing
  name, and a `Target::Type`/`Target::TypeName` arm to the shared IR walk (bug-624) — the same two
  string positions this bug had to rename by hand. The two passes compose in a required ORDER:
  private types are scoped first, and `qualify_package_types` leaves an already-qualified spelling
  alone, so a private type is never qualified twice.
- `qualify_package_types` now rides `visit_project_targets_mut` instead of the walkers this bug
  wrote, and those (`TypeRenames`, `qualify_value_types`, `qualify_op_types`, …) are deleted. A type
  position added to that walk is now qualified here for free, rather than silently missed by a
  second, parallel walk.
- The five `tls` `.ncodesum` goldens conflicted (main moved tls codegen; this branch changed the
  fixture's source). Neither side's hash describes the merged compiler, so all five were rebuilt
  from it.

### Phase 4 — corpus, goldens, spec, full validation

- [x] Respell every bare imported user-package type in `tests/`. Found by running the suite, not by
      grepping — a bare name is only wrong when the package exports it, which the compiler knows and
      a text search does not. The full list:
      `rt_imported_union_enum_members` (two cases became refusals; the qualified enum `CASE` arms
      took the prefix), `rt_foreign_type_reexport` (its packages name `pa390::A`; the app infers,
      since imports are not transitive), `rt_imported_record_map_field_keys`,
      `rt_reexport_union_transitive_field_types`, `rt_recursive_thread_transfer`,
      `rt_imported_resource_scope_drop`, `rt_imported_type_qualified_name`,
      `native-resource-state-import-rt`, `native-link-import-sqlite-rt` (`sqlite3::Db`,
      `sqlite3::SqlValue`, `CASE sqlite3::SqlInt`/`SqlText`), `resource-state-import-rt`
      (`resource_state_export_valid::Cursor`), `thread-return-union`
      (`thread_runtime_workers::ReturnOk`/`ReturnMissing`), `p121d-state-reach-rt` and
      `byte-identity/tls`.
      Checked and NOT migrated: `thread-transfer-union-state-rt` and
      `thread-transfer-union-stateless-rt` declare their own `Stream`/`Cursor` and still build.
- [ ] Artifact gate: inspect every golden diff fixture by fixture.
- [ ] `cargo test --no-fail-fast`; `scripts/man-examples-gate.sh target/release/mfb`.
- [ ] Spec §13 and `resolver::packages` comments stop describing bare imported types as a
      convention.

Acceptance: full suite green; every golden delta explained; every reproduction case prints its
expected output.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_imported_type_name_collision.rs`, the corrected
  `tests/runtime/rt_imported_type_qualified_name.rs`, bug-631's
  `rt_imported_overload_imported_field_argument`.
- Full suite: `cargo test --no-fail-fast` (includes `artifact_gate_all`).
- Man examples: `scripts/man-examples-gate.sh target/release/mfb`.

## Summary

bug-480 made "an imported name requires its prefix" true for built-in packages by putting the
package in the type's name. User packages were never converted: their types stay bare from the
parser to the merge, so two `A`s from different places are one `A`. The fix finishes bug-480 for
user packages. Qualify on read, keep `.mfp` unchanged, refuse the bare imported spelling as the
spec already says.
