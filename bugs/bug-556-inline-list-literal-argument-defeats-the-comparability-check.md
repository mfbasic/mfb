# bug-556: a list literal written INLINE at a call argument lowers as `List OF Unknown`, so the comparability check never runs

Last updated: 2026-09-06
Effort: small-to-medium (the element inference already exists for a binding; it is the ARGUMENT position that does not use it — but the change moves overload resolution, so it needs a full acceptance cycle)
Severity: MEDIUM (a specified type rule is silently skipped; the same program is accepted or rejected purely on whether the list was given a name)
Class: Type inference gap / silently-skipped rule

Status: Open

## The finding

`collections::contains`/`find`/`replace` compare elements for equality, so the
list's element type must be comparable — `ir::verify::compat::check_builtin_comparability`
emits `TYPE_REQUIRES_COMPARABLE` when it is not. That check reads the argument's
type and skips an `Unknown` element on purpose (never a false rejection):

    let Some(ParameterType::ListOf(element)) = arg_types.first() else { return };
    if !matches!(**element, ParameterType::Unknown) && !self.is_comparable(element) { … }

A list literal written **inline at the call** lowers as `List OF Unknown`
whatever its elements are, so the guard swallows it and the rule never runs. The
identical literal given a NAME first infers `List OF Bag` and is refused. The
verdict therefore depends on whether the author bound the list, not on the types.

## Reproduction

    $ mkdir -p /tmp/shapes/src
    $ cat > /tmp/shapes/project.json <<'JSON'
    {"name":"shapes","version":"0.1.0","mfb":"1.0","kind":"executable",
     "sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],
     "entry":"main","targets":["native"]}
    JSON
    $ cat > /tmp/shapes/src/main.mfb <<'MFB'
    IMPORT collections

    TYPE Bag
      items AS List OF Integer
    END TYPE

    FUNC main AS Integer
      LET one AS Bag = Bag[[1, 2]]
      LET a AS Integer = collections::find([one], one)
      LET inferred = [one]
      LET b AS Integer = collections::find(inferred, one)
      LET c AS Boolean = collections::contains([one], one)
      LET annotated AS List OF Bag = [one]
      LET d AS Integer = collections::find(annotated, one)
      RETURN a + b + d
    END FUNC
    MFB
    $ mfb build /tmp/shapes

`Bag` holds a `List OF Integer`, so it is not comparable. Four call sites, one
type, two verdicts:

| line | form | verdict |
|------|------|---------|
| 9  | `find([one], one)` — inline literal | **accepted** (wrong) |
| 11 | `find(inferred, one)` — inferred binding | refused ✓ |
| 12 | `contains([one], one)` — inline literal | **accepted** (wrong) |
| 14 | `find(annotated, one)` — annotated binding | refused ✓ |

The IR shows the cause directly (`mfb build -ir`): the inline argument is

    { "kind": "list", "type": "List OF Unknown", "values": [ { "kind": "local", "name": "one" } ] }

while `LET inferred = [one]` binds `List OF Bag`. The inference exists; the
argument position does not reach it.

## Why it is filed rather than fixed alongside the comparability boundary fix

Found while fixing the imported-record half of this rule (`is_comparable` had no
field table for an imported record, so `Map OF pkg::Box TO V` was accepted). That
defect is at the PACKAGE boundary and is fixed by seeding `ir::verify`'s
`field_types`. This one is import-independent — it reproduces with a purely local
record, on main, with no package involved — and its fix is in list-literal
element inference, a different subsystem with a different blast radius:
inferring the element type at an argument position can change **overload
resolution**, which is instantiation-dependent. Bundling an inference change into
a diagnostics fix would make neither reviewable.

Measured churn if the spelling changes: `List OF Unknown` appears in exactly one
`.ir` golden (`tests/rt-behavior/collections/flatten-inline-rt/`) and two
`build.log` goldens (`syntax/control-flow/comparisons-invalid`,
`syntax/control-flow/logic-invalid`) —

    $ grep -rl 'List OF Unknown' tests/

so the golden cost is small. The overload-resolution risk is the real cost and
needs a full `scripts/test-accept.sh` cycle plus `cargo test`.

## Scope to check when fixing

- Every `check_builtin_comparability` member: `contains`, `find`, `replace`.
- Whether the same `Unknown` element defeats other argument-typed rules —
  `check_collection_element_thread_free` and the collection-ownership rules read
  the same argument types.
- A `Map`/`Set` literal written inline at a call argument, which was not measured
  here and may share the gap.
- Whether inferring the element type changes any overload pick
  (`.ai/codegen-invariants.md`; monomorph selects by CONCRETE types).

References: `src/ir/verify/compat.rs:check_builtin_comparability`;
`src/ir/verify/values.rs:is_comparable_seen`;
`tests/syntax/types/types-map-key-comparable-invalid` (the local map-key case
that IS enforced); `tests/syntax/packages/package-comparable-import-invalid`
(the imported-record case, fixed separately).
