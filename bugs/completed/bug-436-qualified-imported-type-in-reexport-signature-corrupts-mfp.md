<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-436: a package function that names an imported type in package-qualified form (`dep::Type`) in an exported signature writes a corrupt `.mfp` (`truncated binary representation`)

Last updated: 2026-08-08
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (silent package corruption — a build reports success-then-failure and emits no usable artifact)

Status: FIXED (d1347fafb)
Regression Test: tests/rt_reexport_union_transitive_field_types.rs::reexported_union_qualified_type_reference_builds_equivalent_package

STATUS: FIXED (d1347fafb)

Root cause was exactly as documented: the qualified reference `leaf435::Node`
lowers to the dotted IR type name `leaf435.Node`, and the ABI writer's
`TypeTable::type_id` (`src/binary_repr/sections.rs`) fallback looked the dotted
name up in `foreign_types` — which is keyed by **bare** exported name (`Node`) —
missed, and degraded it to an empty RECORD entry (kind 1) that failed its own
read-back with `truncated binary representation`, writing no `.mfp`.

Fix (chosen resolution: **serialize-equivalently**, the preferred Open Decision):
added a bare-name fallback in the `type_id` `_` arm — a dotted `pkg.Type` whose
bare last segment resolves to a foreign export is encoded via `foreign_type`
interned under the **bare** name, producing the identical type-table entry the
unqualified spelling already emits. Composite-type keys use `#`, never `.`, so
only a genuine qualified `pkg.Type` reference reaches the new arm. The fix is in
the writer, not IR lowering — the IR still records `leaf435.Node` (consumers read
ABI exports / the resolved type table, not the IR string), and the new test
proves full equivalence by building `mid435` and then building **and running** an
`app435` consumer that imports only `mid435`.

Verification:
- New regression test GREEN; existing unqualified re-export test still GREEN.
- `cargo test --bin mfb`: 3783 passed, 0 failed.
- `cargo test --tests`: the only failures are 6 tests proven pre-existing on the
  base commit `03309dd8a` (unrelated: 4× `rt_fs_error_path_hygiene` fs::close
  CLOSED-bit ordering [bug-63 cluster], 2× `rt_gtk_term_utf8_grid`) — none
  touched by this change (diff is `src/binary_repr/sections.rs` only).
- artifact-gate not run: the change is package `.mfp` serialization, not native
  codegen (`src/target/`), so it cannot move any `.ncode`/`.ncodesum` golden;
  another session was actively cycling the gate (no-concurrent-gate rule).

When a package function references an imported type by its **package-qualified**
name in an exported signature — `EXPORT FUNC describe(n AS leaf435::Node)` — the
build passes syntaxcheck but the ABI **writer** cannot serialize the type table:
`mfb build` prints `Building <pkg> …` and then `error: truncated binary
representation`, and no `.mfp` is written. The **unqualified** form of the same
signature — `EXPORT FUNC describe(n AS Node)` (with `IMPORT leaf435` in scope) —
builds a correct package. The IR stores the qualified reference as the dotted
name `leaf435.Node` (`mid435.ir`), and the ABI type-table / foreign-type
serialization does not handle a dotted `dep.Type` type name, so the emitted
container is malformed and fails its own read-back with the generic truncation
error.

The single correct behavior a fix produces: a package-qualified imported type in
an exported signature either (a) serializes to the same foreign-type marker the
unqualified form produces (preferred — the two spellings are equivalent), or
(b) is rejected at syntaxcheck with a clear diagnostic. It must never emit a
corrupt `.mfp` under a "Building …" line that looks like success.

References:

- Found while fixing bug-435 (re-exported union transitive field-type closure);
  the qualified spelling was the first repro attempt and surfaced this distinct
  writer-side defect. See `bugs/completed/bug-435-*` and
  `tests/rt_reexport_union_transitive_field_types.rs`.
- `error: truncated binary representation` is emitted by the cursor reads in
  `src/binary_repr/util.rs` and `src/binary_repr/writer.rs:83` — reached here on
  read-back of the just-written (malformed) container.

## Failing Reproduction

Two packages, `leaf435` (owner of a union `Node`) and `mid435` (re-exporter):

```
# leaf435/src/lib.mfb
EXPORT TYPE Meta
  n AS Integer
END TYPE
EXPORT TYPE Box
  meta AS Meta
END TYPE
EXPORT TYPE Leaf
  text AS String
END TYPE
EXPORT UNION Node
  Box
  Leaf
END UNION

# mid435/src/lib.mfb   (IMPORT leaf435 installed at packages/leaf435.mfp)
IMPORT leaf435
EXPORT FUNC describe(n AS leaf435::Node) AS String   # <-- qualified form
  RETURN "node"
END FUNC
```

```sh
mfb build leaf435          # OK -> leaf435.mfp
mfb build mid435           # FAILS:
#   Building mid435 (package) for macos-aarch64
#   error: truncated binary representation
```

- Observed: `error: truncated binary representation`; no `mid435.mfp` written.
- Expected: `mid435.mfp` written, identical in effect to the unqualified form.

Contrast case that works today (bounds the bug):

```
EXPORT FUNC describe(n AS Node) AS String   # unqualified -> builds a valid .mfp
```

`mfb build mid435 -ast -ir` **succeeds** (it stops before the ABI write) and the
IR records the param type as the dotted name `leaf435.Node` — direct evidence the
qualified reference reaches serialization as `leaf435.Node`.

## Root Cause

The qualified type reference `leaf435::Node` lowers to the dotted IR type name
`leaf435.Node` (confirmed in `mid435.ir`). The ABI writer's type-table /
foreign-type-reference construction (`src/binary_repr/writer.rs`
`external_type_metadata` and the type-table encode path) recognizes a foreign
type by its bare exported name (`Node`) with an owner marker, but a dotted
`leaf435.Node` name matches no export and no owner rule, so it is encoded as a
malformed/empty entry whose later field read runs off the end of the payload —
surfacing as `truncated binary representation` on the writer's read-back. The
unqualified form resolves to a bare `Node` reference that the foreign-type path
handles, which is why it is immune.

(Hypothesis to confirm during the fix: whether the dotted name should be
canonicalized to the bare foreign reference at IR-lowering time, or handled in
the writer's type-table encode.)

## Goal

- `mfb build` of a package whose exported signature names an imported type in
  `dep::Type` form writes a valid `.mfp` equivalent to the unqualified spelling
  (or rejects it at syntaxcheck with a specific diagnostic — see Open Decisions).
- No build ever emits a corrupt/no `.mfp` under a `Building …` line.

### Non-goals (must NOT change)

- The unqualified re-export path (bug-390 / bug-435) — it is correct; do not
  regress it or its goldens.
- The `.mfp` on-disk format.

## Blast Radius

- `src/binary_repr/writer.rs` `external_type_metadata` / type-table encode — the
  suspected site.
- Any IR lowering that produces a dotted `pkg.Type` type name for a
  package-qualified type reference (the producer of the malformed input).
- Contrast: the unqualified form is unaffected and is the regression guard.

## Fix Design

Determine where a package-qualified imported *type* reference should be
canonicalized to the same foreign-type reference the unqualified form yields —
most likely at IR type-name resolution (strip the `dep.` qualifier for a type
that resolves to an imported foreign type), so the existing writer path handles
it unchanged. Rejected alternative: teaching every ABI encode site to parse
dotted type names (wider surface, more places to miss).

## Phases

### Phase 1 — failing test + audit ✅
- [x] Added the qualified-form case; confirmed RED with `truncated binary
  representation` at `build_ok(mid435)`. Commit: d1347fafb

### Phase 2 — the fix ✅
- [x] Resolved the qualified imported-type reference to the bare foreign-type
  marker in `TypeTable::type_id` (writer side; the existing writer path handles
  it unchanged). Commit: d1347fafb

### Phase 3 — validation ✅
- [x] Qualified and unqualified spellings both build valid, equivalent `.mfp`s
  (the qualified test builds `mid435` and builds+runs an `app435` consumer).
- [x] `cargo test --bin mfb` green (3783 passed); `cargo test --tests` clean
  apart from 6 pre-existing reds proven identical on base `03309dd8a`. See the
  STATUS block for the artifact-gate rationale. Commit: d1347fafb

## Open Decisions

- Serialize-equivalently (preferred: the two spellings mean the same type) vs.
  reject-with-diagnostic (if the language intends types to be named unqualified
  once imported). Confirm against the spec's rule for qualified type references.

## Summary

A package-qualified imported type in an exported signature (`dep::Type`) lowers
to a dotted IR type name the ABI writer cannot serialize, so the package build
fails with `truncated binary representation` and emits no artifact, while the
unqualified spelling builds correctly. Fix by canonicalizing the qualified
reference to the foreign-type marker the unqualified form already produces (or
rejecting it up front). Discovered while fixing bug-435.
