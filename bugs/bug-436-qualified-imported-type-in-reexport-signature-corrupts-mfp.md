<!-- Bug document. See .claude/skills/write-bug/template.md -->

# bug-436: a package function that names an imported type in package-qualified form (`dep::Type`) in an exported signature writes a corrupt `.mfp` (`truncated binary representation`)

Last updated: 2026-08-08
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (silent package corruption — a build reports success-then-failure and emits no usable artifact)

Status: Open
Regression Test: tests/rt_reexport_union_transitive_field_types.rs (add a qualified-form case) — none yet

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

### Phase 1 — failing test + audit
- Add a qualified-form case (the repro above) asserting `mid435` builds; confirm
  it is RED with `truncated binary representation`.

### Phase 2 — the fix
- Canonicalize the qualified imported-type reference so the writer emits the
  bare foreign-type marker (or reject at syntaxcheck).

### Phase 3 — validation
- Qualified and unqualified spellings both build valid, equivalent `.mfp`s.
- Full `cargo test`, package acceptance, and artifact-gate green.

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
