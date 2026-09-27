# bug-745: `List`/`Map`/`Set OF Nothing` fail the native build with "collection type … is not supported"

Last updated: 2026-09-27
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness

Status: FIXED
Regression Test: tests/runtime/rt_nothing_collections.rs

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

## STATUS: FIXED (15f5aa83e)

`List OF Nothing`, `Map OF K TO Nothing`, `Map OF Nothing TO V` and `Set OF Nothing`
build and work across the `collections::` families, literals, `FOR EACH`, `len` and
`MUT` in-place updates. A `Nothing` payload is a 1-byte lane that always holds `0`
(identifier 1). Strict template unification now refuses a `Nothing` binding only in a
callback's result and in thread slots.

Deviations:
- The owner asked for every collection operation to work. `collections::transform`,
  `mapValues` and `groupBy` still reject a callback that *returns* `Nothing`: spec §7
  and plan-159-A record that as intended, and the narrowed rule keeps it.
  `transform` *from* a `List OF Nothing` works.
- `collections::sort` over `Nothing` is a type error, because `Nothing` is not
  orderable. Its diagnostic points at a line inside the injected sort body, not the
  call. That is the pre-existing bug-642 class, not this change.
- Effort came in at medium, not the large first estimated.

Verified on the merged tip:
- `cargo test --release --no-fail-fast`: exit 0, 250 binaries, 6249 passed,
  0 failed.
- `scripts/artifact-gate.sh target/release/mfb all`: 0 diffs over 2166 goldens.
- `[NOTHING, NOTHING]` prints `2`, and the map literal repro prints `1`.

References:

- bugs/completed/bug-744-nothing-func-call-in-value-position-panics-codegen.md
  (Blast Radius): where it was found.
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

A second, front-end layer blocks every generic member even once codegen can
represent the type. Strict validation in `src/codegen/registry/mod.rs:unify` refused
to bind any template variable to `Nothing` (bug-443). So `collections::contains(List
OF Nothing, NOTHING)`, `get`, `append` and the rest were `TYPE_CALL_ARGUMENT_MISMATCH`
at 5c4ea72e3, before codegen was reached. bug-443 only needed the refusal in two
places: a callback's result (`transform`'s `FUNC(T) AS U` must not accept a
`Nothing`-returning callback, spec §7) and a thread-handle slot (a `Nothing` resource
plane means "none").

## Goal

- `List OF Nothing`, `Map OF K TO Nothing`, `Map OF Nothing TO V`, `Set OF Nothing`
  build and behave per the collection spec for every `collections::` member and
  `FOR EACH`, `len`, literals, and `MUT` in-place updates.
- Every stored `Nothing` payload is `0` (bug-744 guarantees a materialized `Nothing`
  value is `0`; hashing/equality of `Nothing` keys must not depend on anything else).

### Non-goals (must NOT change)

- The layout of every other element type (all `.ncodesum` goldens that do not
  involve `Nothing` must stay byte-identical).
- bug-443's purpose: `collections::transform` (and any callback-result position)
  still rejects a `Nothing`-returning callback (spec §7). The refusal is narrowed to
  callback results and thread slots, not dropped.
- `Nothing` stays non-orderable: `collections::sort` over it is a type error, as for
  `Boolean`.

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

A `Nothing` payload is a **1-byte lane that always holds `0`**, with its own runtime
identifier `COLLECTION_TYPE_NOTHING = 1` (free, below `Boolean` = 2). A `List OF
Nothing` is a fixed-width (kind 2) list of stride 1. Per dispatch:

- `collection_type_code`, `list_element_is_fixed_width` (1),
  `collection_payload_alignment` (1): `Nothing` beside `Boolean`/`Byte`.
- Payload length and load: beside `Boolean`/`Byte` (`ldrb`).
- Payload **store** writes the constant `0`, not the incoming register. The unit type
  has one value, so the stored byte never depends on where the value came from.
- The three packed-payload **compares** (`builder_collection_compare.rs`) branch
  straight to *equal* for `Nothing`, which the scalar-value compare already did.
  Every two `NOTHING`s are equal without reading memory, so a `Set OF Nothing` and a
  `Map OF Nothing TO V` collapse to one entry.
- The map probe treats a `Nothing` key as a 1-byte probe-eligible key
  (`gen_map.rs`).
- `unify` → `unify_at` with a `NothingBinding` position flag. Refused only in a
  `Func` pattern's return and in thread slots; allowed everywhere else.

Rejected alternatives:
- A zero-width payload: every stride, offset and copy path assumes width ≥ 1, and a
  zero-size allocation is treated as one byte anyway (`mfb spec memory heap-values`).
- Reusing the `Integer` identifier at 8 bytes: it costs 8× the space and mislabels
  the header.
- Dropping bug-443's refusal entirely: that would reopen `transform` with a
  `Nothing` callback, against spec §7 and plan-159-A's recorded property.

## Phases

### Phase 1 — failing tests

- [x] `tests/runtime/rt_nothing_collections.rs`, 10 tests: literal/`get`/`getOr`;
      builders and slices (`append` both overloads, `prepend`, `insert`, `set`,
      `removeAt`, `mid`, `take`, `drop`, `repeat`, `replace`, `distinct`,
      `flatten`); searches and predicates (`contains`, `find`, `findIndex`,
      `findLastIndex`, `all`, `any`, `filter`, `partition`); callbacks and regrouping
      (`transform` from `Nothing`, `reduce`, `forEach`, `chunks`, `window`, `zip`,
      `toSet`); a `MUT` list grown and edited in place; `Map OF String TO Nothing`
      (`get`, `getOr`, `hasKey`, `set`, `removeKey`, `keys`, `values`, `merge`,
      `mapValues`, `FOR EACH`); `Map OF Nothing TO Integer`; `Set OF Nothing`
      (`add`, `contains`, `remove`, `union`, `intersection`, `difference`,
      `symmetricDifference`, `isSubset`, `isSuperset`, `isDisjoint`, `toList`);
      `FUNC … AS Nothing` calls as elements (bug-744's two deferred shapes); nesting,
      record fields, function boundaries and copies. RED on the pre-fix compiler:
      10/10 fail (`MFB_TEST_EXE=<old mfb> cargo test --test rt_nothing_collections`).

Commit: ee728a099

### Phase 2 — the fix

- [x] Payload representation and dispatch arms (Fix Design).
- [x] `unify_at` position flag.
- [x] Spec: `mfb spec memory collections` (identifier table, fixed-width table, the
      constant-byte note); `mfb spec language templates` (when a template variable
      may bind `Nothing`). Man: `mfb man types` states that `Nothing` values bind,
      pass and collect. Example verified with
      `scripts/man-run-examples.sh types --topic --run`.

Acceptance: 10/10 green.
Commit: 15f5aa83e (fix), 39e4c8a82 (spec + man)

### Phase 3 — validation

- [x] Full `cargo test --release --no-fail-fast`; `scripts/artifact-gate.sh
      target/release/mfb all` (0 diffs).

Commit: — (validation only)

## Open Decisions

- Support vs. reject collections of `Nothing` — **support** (spec §4.6/§12 treat
  `Nothing` as an ordinary comparable type).
