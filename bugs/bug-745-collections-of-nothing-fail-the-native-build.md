# bug-745: `List`/`Map`/`Set OF Nothing` fail the native build with "collection type … is not supported"

Last updated: 2026-09-26
Effort: large (3h–1d)
Severity: MEDIUM
Class: Correctness

Status: Open
Regression Test: (planned) tests/runtime/rt_nothing_collections.rs

Found during bug-744's sibling sweep (a `FUNC … AS Nothing` call as a collection
literal element). Any collection whose element, key or value type is `Nothing` fails
`mfb build` with an internal codegen error — no source location, no error code:

```
error: native code collection type 'List OF Nothing' is not supported while lowering bind xs AS List OF Nothing
```

It is independent of calls: `[NOTHING, NOTHING]`, `MUT xs AS List OF Nothing = []`
and `Map OF String TO Nothing { "a" := NOTHING }` all fail the same way, at HEAD
(5c4ea72e3) and before bug-721 (eee5df136).

**The single correct behavior:** a collection of `Nothing` is an ordinary collection.
`mfb spec language types` §4.6 makes `Nothing` a normal concrete type with one value;
§4 lists it as comparable, copyable and defaultable; `mfb spec language collections`
§12 names `Nothing` among the comparable types a `Set` element / `Map` key may have.
Nothing in the spec restricts `List OF Nothing`. The program must build and run:
`len([NOTHING, NOTHING])` is `2`, `FOR EACH` yields `NOTHING` per element, a
`Set OF Nothing` holds at most one element, a `Map OF Nothing TO V` at most one key.
(If the owner decides instead that such collections are illegal, the correct behavior
is a located front-end diagnostic with an error code — never this internal error.)

References:

- bugs/completed/bug-744-nothing-func-call-in-value-position-panics-codegen.md
  (Blast Radius) — where it was found.
- `.ai/collections.md` — collection codegen invariants.

## Failing Reproduction

```
mfb init nc && cd nc
cat > src/main.mfb <<'EOF'
IMPORT io
SUB main()
  LET xs = [NOTHING, NOTHING]
  io::print(toString(len(xs)))
END SUB
EOF
mfb build .
```

- Observed: `error: native code collection type 'List OF Nothing' is not supported while lowering bind xs AS List OF Nothing`, exit 1.
- Expected: builds; prints `2`.

Same failure for `Map OF String TO Nothing { "a" := NOTHING }` and
`MUT xs AS List OF Nothing = []`.

## Root Cause

`src/codegen/engine/types/type_utils.rs:collection_type_code` maps `Nothing` to
`None`, so `CollectionTypeLayout::from_type`
(`src/codegen/engine/validation/validation.rs`) returns `None` and every collection
emitter reports "native code collection type '…' is not supported". Native collection
codegen has no representation for a `Nothing` payload at all.

Measured by a throwaway experiment (not committed): mapping `Nothing` to the 8-byte
`COLLECTION_TYPE_INTEGER` code, fixed width 8 (`list_element_is_fixed_width`) and
alignment 8 (`collection_payload_alignment`) makes `MUT xs AS List OF Nothing = []`
build and print `0`, but a non-empty literal then fails at the next layer: "native
collection packed payload does not support type 'Nothing'" — the six packed-payload
dispatches in `src/codegen/collection/layout/builder_collection_layout.rs` and
`src/codegen/collection/compare/builder_collection_compare.rs` have no `Nothing` arm,
and `src/codegen/collection/**` has 43 `ParameterType::Integer` arms to audit.

## Goal

- `List OF Nothing`, `Map OF K TO Nothing`, `Map OF Nothing TO V`, `Set OF Nothing`
  build and behave per the collection spec for every `collections::` member and
  `FOR EACH`, `len`, literals, and `MUT` in-place updates.
- Every stored `Nothing` payload is `0` (bug-744 guarantees a materialized `Nothing`
  value is `0`; hashing/equality of `Nothing` keys must not depend on anything else).

### Non-goals (must NOT change)

- The layout of every other element type (all `.ncodesum` goldens that do not
  involve `Nothing` must stay byte-identical).
- bug-443's strict-unify rule (`src/codegen/registry/mod.rs:unify`) that never binds a
  template variable to `Nothing` — so `collections::contains(List OF Nothing, NOTHING)`
  is rejected in the front end with `TYPE_CALL_ARGUMENT_MISMATCH` today. Whether that
  rule is too broad for element-typed (not callback-result) positions is a separate
  owner decision; record it, do not change it silently here.

## Blast Radius

- `type_utils.rs:collection_type_code` — root.
- `builder_collection_layout.rs` packed-payload store/load/copy arms (3 sites) and
  `builder_collection_compare.rs` equality/hash arms (3 sites) — need a `Nothing` arm.
- `list_element_is_fixed_width` / `collection_payload_alignment` /
  `default_payload_alignment` — must agree (`fixed_width_agrees_with_payload_alignment`).
- Runtime helpers that dispatch on the header type code — covered if `Nothing` reuses
  an existing 8-byte scalar code; a NEW code would be a memory-layout spec change
  (`mfb spec memory`).

## Fix Design

Recommended: represent a `Nothing` payload as an 8-byte zero scalar reusing
`COLLECTION_TYPE_INTEGER` (a fixed-width list kind), and add `Nothing` arms beside
`Integer` in the packed-payload store/load/compare/hash dispatches. Rejected: a
zero-width payload (every stride/offset path assumes width ≥ 1); a new type code
(runtime-helper ABI change for no observable gain).

## Phases

### Phase 1 — failing tests

- [ ] `tests/runtime/rt_nothing_collections.rs`: list literal, `MUT` append, map
      literal value, map key, set literal, `FOR EACH`, `len`, and a `FUNC … AS
      Nothing` call as an element (bug-744's removed shapes:
      `LET xs = [f(1), f(2)]` → `f1 f2 2 eq eq`;
      `Map OF String TO Nothing { "a" := f(1) }` → `f1 1 a eq`).

Commit: —

### Phase 2 — the fix

- [ ] Type code, width/alignment, packed-payload arms; spec (`mfb spec memory`
      collection payloads) if the layout table lists per-type widths.

Commit: —

### Phase 3 — validation

- [ ] Full `cargo test --release --no-fail-fast`; `scripts/artifact-gate.sh
      target/release/mfb all` (expect 0 diffs — no existing fixture has a `Nothing`
      collection).

Commit: —

## Open Decisions

- Support vs. reject collections of `Nothing` — **support** (spec §4.6/§12 treat
  `Nothing` as an ordinary comparable type).
