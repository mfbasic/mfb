# plan-111-C: key the type tables by type — TypeModel and the registry

Last updated: 2026-08-29
Effort: large (3h–1d)
Depends on: plan-111-B (the front end is typed, so this letter's registry
signature changes have no `&str` callers left upstream of codegen). Requires
`ParameterType: Hash` from plan-111-A Phase 2.

**This letter is the string bottleneck.** plan-106-E named it precisely and
deferred it: "closing it means retyping the name-keyed tables themselves, not
the emitters." That is this letter, and it is the reason codegen's 109
`ParameterType::parse` calls exist — an emitter deep in the tree is handed a
spelling because the table above it is keyed by one.

Two things get retyped: `TypeModel`'s seven `String`-keyed maps, and the
registry's duplicated string API. Once a codegen emitter can ask a typed
question of a typed table, letters D–F have somewhere to put their converted
call sites; until then they would have nothing to call.

See plan-111-A for the shared prerequisites, the five sanctioned boundaries, the
byte-identity gate policy, and the rejected alternatives (in particular: no
`TypeId`, no interning — `ParameterType: Hash` is the whole mechanism).

References:

- `src/codegen/engine/builder/mod.rs:597-643` — `TypeModel`: nine fields, seven
  of them keyed or valued by a rendered type name.
- `src/codegen/registry/mod.rs:2189` `resolve_call` / `:2211` `resolve_call_typed`;
  `:1759` `call_return_type` / `:1766` `call_return_type_typed`; `:2469`
  `argument_types` / `:2480` `argument_types_typed` — the dual API, string half
  dominant.
- `planning/completed/plan-106-E-consolidation-no-strings-census.md` §"The one
  honest gap" — the deferral this letter closes, and its Correction 4 (the
  `refined_list_literal_type` `format!("List OF {element}")` that belongs to the
  same web).
- `src/codegen/collection/layout/builder_collection_layout.rs:2459` — that
  `format!`, the last production type-string construction outside the renderer
  and the wire codec.

## Prerequisites

See plan-111-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-111-A complete | `cargo test --test no_type_strings` passes; `ParameterType` derives `Hash` | NOT MET until A lands |
| plan-111-B complete | `rg 'ParameterType::parse\(' src/ir src/monomorph src/resolver` → hits only `src/ir/binary.rs` | NOT MET until B lands |

## 1. Goal

- Every `TypeModel` map is keyed by `ParameterType` (or by a nominal `Symbol`
  where the key is genuinely a *name*, not a type — decided per field in Phase 1).
- The registry has **one** API per query. `resolve_call`, `call_return_type`,
  `argument_types`, `resolve_call_return_type`, `constant_type_name` and
  `general_override_target` either take and return `ParameterType`, or are
  deleted in favour of their existing `_typed` twin.
- `src/codegen/collection/layout/builder_collection_layout.rs:2459` no longer
  builds a type by `format!`.
- The gate's `string_keyed_type_maps` budget is 0 tree-wide.

### Non-goals (explicit constraints)

- See plan-111-A §1 Non-goals — all apply.
- **No lookup-result change.** Every table lookup and every overload resolution
  must return exactly what it returns today for every input. Overload resolution
  is instantiation-dependent (`.ai/codegen-invariants.md`) — a key change that
  merges two previously-distinct keys, or splits one, changes which overload
  wins. That is the single failure mode this letter must not have.
- **No registry descriptor change.** The `RegistryFunction`/`Parameter`
  descriptors and their `mfb man` output stay exactly as they are; only the
  *query functions* over them change signature.
- Do not convert codegen emitters here. They are D's and E's work; this letter
  changes the tables and the queries they call.
- Do not add a caching layer, an interner, or a type-id side table.

## 2. Current State

`TypeModel` (`src/codegen/engine/builder/mod.rs:597`) is built once per module
and consulted by every emitter. Its keys are rendered type names, which is why an
emitter holding a `ParameterType` must render it to look anything up — and why,
having rendered it, the cheapest way to ask a second question is to parse it back.

The registry (`src/codegen/registry/mod.rs`) already grew typed twins during
plan-104/106 but the string originals were never removed, so the string half is
what almost everything calls.

### Measured populations

At HEAD (`fd09ea809`), 2026-08-29, tests excluded.

`TypeModel`'s fields and their call-site counts (`rg -n '\.<field>\b' src/ --glob '!**/tests*' | wc -l`):

| Field | Type today | Sites |
|---|---|---|
| `record_fields` | `HashMap<String, Vec<(String, ParameterType)>>` | 46 |
| `union_names` | `HashSet<String>` | 13 |
| `union_variant_tags` | `HashMap<String, usize>` | 13 |
| `union_variants` | `HashMap<String, String>` | 11 |
| `union_variant_fields` | `HashMap<String, Vec<(String, ParameterType)>>` | 10 |
| `resource_closers` | `HashMap<String, String>` | 5 |
| `enum_members` | `HashMap<(String, String), usize>` | 4 |
| `resource_names` | `HashSet<String>` | 3 |
| `union_variant_unions` | `HashMap<String, HashSet<String>>` | 2 |
| **Total** | | **107** |

Registry dual API
(`rg -n '\b<fn>\(' src/ --glob '!**/tests*' | grep -v 'pub(crate) fn' | wc -l`):

| Query | string callers | typed callers |
|---|---|---|
| `resolve_call` | 37 | `resolve_call_typed` 4 |
| `call_return_type` | 32 | `call_return_type_typed` 2 |
| `argument_types` | 8 | `argument_types_typed` 5 |
| `builtins::resolve_call_return_type` | 7 | none — needs one |

The string-API callers cluster in the per-package builtin modules
(`rg` by file): `builtins/os/mod.rs` 14, `builtins/mod.rs` 11,
`builtins/process/mod.rs` 10, `builtins/audio/mod.rs` 10,
`builtins/encoding/mod.rs` 9, `builtins/money/mod.rs` 6, `registry/mod.rs` 5,
`builtins/{vector,general,app}/mod.rs` 4 each, plus `src/ir/shape.rs` 8 and
`src/ir/lower.rs` 4 — the last two are the front-end callers letter B
deliberately left here.

### Verified properties

- **`ParameterType` will be `Hash` when this letter starts** — plan-111-A
  Phase 2, verified there against `src/intern.rs:26` (`Symbol` is already `Hash`,
  and every other payload is).
- **The typed twins exist and are `pub(crate)`** — read
  `src/codegen/registry/mod.rs:1766` (`call_return_type_typed`), `:2211`
  (`resolve_call_typed`), `:2480` (`argument_types_typed`). This letter mostly
  deletes the string originals and repoints callers, not writes new queries.
- **UNVERIFIED: whether `union_variants`, `resource_closers` and `enum_members`
  are type-keyed at all.** Their values are call targets and member names —
  `resource_closers`' doc comment (`src/codegen/engine/builder/mod.rs:611-641`)
  says the value is "the op's *name* as the importing module routes it, not a
  resolved symbol." A nominal→nominal symbol table is **not** a type map and must
  not be converted (plan-106-E census line 4 classified three such maps
  correctly). Phase 1 classifies each of the nine fields before any is touched.
- **UNVERIFIED: whether re-keying merges or splits any key.** Two distinct
  spellings that parse to the same `ParameterType`, or one spelling that two
  code paths render differently, would change lookup results. Phase 1 task 3
  measures this directly rather than assuming.

## 3. Design Overview

**Phase 1 is a classification, and it must come first.** Nine `TypeModel` fields,
each either a *type map* (convert) or a *nominal symbol table* (leave). Getting
this wrong in either direction is the letter's main hazard: converting a symbol
table breaks resource cleanup routing (bug-374/bug-377 live in
`resource_closers`), and leaving a type map is the half-completion this plan
exists to end.

**Phase 2 re-keys the type maps.** Mechanical once Phase 1 has decided, and
guarded by an equivalence check: for every key in the old table, the new table
must answer identically. Phase 2 task 1 builds that check as a temporary
debug-only assertion, runs the whole corpus through it, then removes it — proof
rather than hope.

**Phase 3 collapses the registry's dual API.** 84 call sites move from the string
half to the typed half. The risk here is overload resolution: the strict matcher
distinguishes resource params from value-union params
(`.ai/resources-packages.md`; the registry-strict-matcher memory), and a typed
key that compares differently from its spelling changes which overload matches.

**Phase 4 kills the last `format!` type construction** in
`builder_collection_layout.rs:2459`.

Correctness risk concentrates in Phase 2's key equivalence and Phase 3's overload
resolution.

**This letter carries the plan's worst gate blind spot, and compensates for it
deliberately.** Per plan-111-A §3 the full `artifact-gate.sh all` runs only in
letter G, but this letter touches symbol-adjacent tables, and uniform label
renumbering is invisible to `test-accept` — only `.ncodesum` sees it (the
abi-function-migration memory). With the full golden sweep deferred to G, this
letter's only byte-level check is the scoped spot-check below — four builtins,
multi-target, which does see `.ncodesum` for the shapes it covers, but not the
whole corpus.

The compensation is Phase 2's equivalence assertions: rather than detecting a
changed lookup downstream in the emitted bytes, this letter proves *at the lookup
itself*, over the whole corpus, that the typed table answers identically to the
string one. That is a strictly stronger check than byte-identity for this
specific risk, and it is why the letter is safe without the cross-target sweep.
If the spot-check or G surfaces a diff, C's tables are the first place to look.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; mark
> moot tasks `- [x] ~~text~~ — moot: <evidence>`; fill `Commit:` when a phase
> lands. **An unticked box means NOT DONE.**

### Phase 1 — classify the nine `TypeModel` fields

No code changes. Produces the decision table Phase 2 executes.

- [ ] For each of the nine fields, read every construction site and every read
      site, and record in this file: is the key a **type** (convert to
      `ParameterType`), a **nominal name** (convert to `Symbol`), or a
      **routing symbol** (leave `String`)? Cite the code that decides it.
- [ ] Specifically resolve `union_variants`, `resource_closers` and
      `enum_members`, whose doc comments suggest they are symbol tables, not type
      maps. `resource_closers`' value is a routing name (bug-374, bug-377) —
      confirm from `resolve_closer_symbol` and do not convert it on a guess.
- [ ] Measure the merge/split hazard: for every key inserted into each map to be
      converted, assert across the whole corpus that
      `ParameterType::parse(key).name() == key` — a key that does not round-trip
      would change identity under re-keying. Record the result.
- [ ] Write the decision table into §2 of this file, replacing the UNVERIFIED
      entries.

Acceptance: every one of the nine fields has a recorded verdict with its
citation, and the round-trip measurement is recorded with its command and count.
No code changed.
Commit: —

### Phase 2 — re-key the type maps

- [ ] Change each field Phase 1 marked "type" to a `ParameterType` key.
- [ ] Add a temporary `#[cfg(debug_assertions)]` equivalence assertion at each
      converted lookup: the typed lookup's result equals the string lookup's
      result. Then compile the whole fixture corpus with the **debug** binary and
      confirm no assertion trips — CI is linux + DEBUG and the local gates are
      mac + RELEASE, so a release-only run proves neither axis (the CI-jobs
      memory). This is a compile sweep only; no goldens are diffed.
- [ ] Remove the equivalence assertions and the shadow string tables once the
      corpus is clean, in a separate commit that names the run that proved them.
- [ ] Convert the emitters' *lookups* only — do not convert emitter signatures
      (D and E).
- [ ] Lower the gate's `string_keyed_type_maps` budget to 0 in the clearing commit.
- [ ] Tests: add a `TypeModel` unit test building a model with a nested container
      key (`List OF Map OF String TO Integer`) and a stateful resource key
      (`File STATE Cursor`) and asserting both resolve — the two shapes most
      likely to differ between spelling-keyed and type-keyed lookup.

Acceptance: the debug-build compile sweep with equivalence assertions active
reports zero mismatches (record the command and the fixture count);
`cargo test --no-fail-fast -- --skip artifact_gate_all` green.
Commit: —

### Phase 3 — collapse the registry's dual API (largest blast radius)

- [ ] Add `resolve_call_return_type_typed` beside
      `builtins::resolve_call_return_type` (7 callers, no twin today).
- [ ] Repoint all 37 `resolve_call` callers to `resolve_call_typed`, then delete
      `resolve_call`. Same for `call_return_type` (32) and `argument_types` (8).
- [ ] Convert `constant_type_name` and `general_override_target`
      (`src/codegen/registry/mod.rs:1665`, `:1744`) to typed signatures.
- [ ] Convert `src/ir/shape.rs`'s `arg_types: &[String]` (`:292`) and its
      `type_name.as_str() == "Integer"` (`:302`) now that the typed registry
      query exists — this is the item letter B deferred here.
- [ ] Repoint `src/ir/lower.rs`'s 4 registry string calls.
- [ ] Rename the surviving `_typed` functions back to the plain names (there is
      no longer an untyped one to disambiguate from), in a mechanical commit of
      its own.
- [ ] Lower the gate budgets for `codegen/registry` and `codegen/builtins/*/mod.rs`
      by what this phase removed.
- [ ] Tests: add a registry resolution test covering the resource-vs-value-union
      strict-matcher distinction — a resource param must still reject a
      union→concrete-resource widening, and a value-union param must still accept
      variant→union (the strict-matcher memory). This is the overload-resolution
      regression guard.

Acceptance: `rg -n '\b(resolve_call|call_return_type|argument_types)\(' src/ --glob '!**/tests*'`
returns 0 hits outside the function definitions themselves; `cargo test --no-fail-fast -- --skip artifact_gate_all`
green.
Commit: —

### Phase 4 — the last `format!` type construction

- [ ] Rewrite `refined_list_literal_type`
      (`src/codegen/collection/layout/builder_collection_layout.rs:2459`) to build
      `ParameterType::ListOf(Box::new(element))` structurally instead of
      `format!("List OF {element}")`. This is plan-106-E Correction 4's site.
- [ ] Confirm `:2890` is `#[cfg(test)]` (plan-106-E Correction 4 recorded it as
      such) and convert or exempt it accordingly.
- [ ] Lower the gate's `format!` budget for `codegen` to 0.

Acceptance: the `format!` needle class reads 0 for `codegen`;
`cargo test --no-fail-fast -- --skip artifact_gate_all` green.
Commit: —

### End-of-letter spot-check (scoped, read-only)

Before closing this letter, run the scoped artifact gate on the builtins it
touched — **`collections`, `math`, `strings`, `general`** (`TypeModel` re-keying and the registry API collapse touch every builtin's resolution):

```
scripts/artifact-gate.sh target/release/mfb collections
scripts/artifact-gate.sh target/release/mfb math
scripts/artifact-gate.sh target/release/mfb strings
scripts/artifact-gate.sh target/release/mfb general
```

Measured cost: ~31s per builtin (one builtin = 1 test, 6 builds, 7 goldens).
This is **read-only diffing**: it regenerates nothing and updates no golden. It
is multi-target — per-target goldens (`*.linux-aarch64.ncode` and friends) are
discovered by filename and rebuilt with `-target`, so cross-arch drift is caught
on a macOS host, which no other per-letter check can see.

Expect **0 diffs**. A diff here is this letter's, which is the entire point of
running it now instead of discovering it in G behind six letters of churn —
root-cause it with objdump on one fixture and fix the conversion. **Do not
regenerate a golden here.** All regeneration happens once, in letter G, after
attribution (plan-111-A §3).

## Validation Plan

- Tests: `cargo test --no-fail-fast -- --skip artifact_gate_all`.
- Gate: `cargo test --test no_type_strings` — `string_keyed_type_maps` 0
  tree-wide; `codegen` `format!` 0.
- Coverage check: Phase 2's equivalence assertions need a **debug**-build
  compile sweep over the fixture corpus — `#[cfg(debug_assertions)]` guards do
  not fire in release, and the local gates are RELEASE (the CI-axis memory).
  This is a *compile* sweep, not `test-accept.sh`: build the fixtures with the
  debug binary and check that no assertion trips. It does no golden diffing, so
  it is far cheaper than the acceptance run and does not violate the
  deferred-goldens policy — and it is the one thing in this letter that must
  not be deferred, because it is C's substitute for the byte-level gate.
- Runtime proof: **deferred to letter G.** No `test-accept.sh` run in this
  letter — the acceptance corpus and its goldens are swept once, at the end
  (plan-111-A §3). The per-phase `rt_*` runtime tests are this letter's
  behavioral signal.
- Artifact gate: **scoped spot-check only** — the builtins above, ~31s each,
  read-only. The full `artifact-gate.sh all`, `tests/golden.rs`,
  `test-accept.sh` and every golden regeneration run once, in letter G.
- Diagnostics: **not run in this letter** — this letter touches codegen, which
  emits no source diagnostics (plan-111-A §3). G re-checks it.
- Doc sync: `.ai/resources-packages.md` (registry query surface),
  `.ai/collections.md` (layout table keys), and
  `src/docs/spec/architecture/21_type-name-encoding.md`.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

- **Key type for nominal-only maps** — recommended: `Symbol` for maps whose key
  is genuinely a bare nominal name (`union_variant_tags`, `resource_names`),
  `ParameterType` for maps whose key can be a composite. Using `ParameterType`
  everywhere is simpler but boxes a nominal into a tree node for no gain.
  Phase 1 decides per field. (§Phase 1)
- **Whether `resource_closers` is in scope** — recommended: **no**. Its value is
  a routing name and its key is a resource *name*; bug-374 and bug-377 both live
  here and neither is a type-representation bug. Convert only if Phase 1's read
  of `resolve_closer_symbol` contradicts this. (§Phase 1)

## Corrections

<Filled in DURING execution.>

## Summary

This is the letter plan-106 declined to write. The risk is real and it is in two
named places: a re-keyed table that merges or splits a key (Phase 2, caught by
the equivalence assertions run in a debug build over the whole corpus) and a
typed registry query that resolves a different overload (Phase 3, caught by the
strict-matcher regression test and by `artifact-gate all`).

Untouched: codegen's emitter signatures and their 147 spelling match arms
(letters D–F), and the five sanctioned boundaries.
