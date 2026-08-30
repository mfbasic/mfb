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
| plan-111-A complete | `cargo test --test no_type_strings` passes; `ParameterType` derives `Hash` | MET (2026-08-29): gate `4 passed`; `src/types.rs:31` derives `Hash`. |
| plan-111-B complete | `rg 'ParameterType::parse\(' src/ir src/monomorph src/resolver` → hits only `src/ir/binary.rs` | MET (2026-08-29). The literal `rg` also lists 6 other files; every hit in them is inside a `#[cfg(test)]` module, confirmed by re-running the gate's own `test_free_lines` stripper over each — 0 live hits in all six. The gate agrees: it prints no `parse_sites` row for `ir`, `monomorph` or `resolver` at all. |

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
### Phase 1 decision table (VERIFIED)

Every key in all nine fields is inserted from `NirType.name` or a variant's
`name` in `TypeModel::from_module` (`src/codegen/engine/validation/validation.rs:240-300`)
and in the imported-package pass (`:452-466`) — i.e. every key is a **declared
type name**, and none is a routing symbol. The VALUES differ, and that is where
the three UNVERIFIED entries land:

| Field | Key | Value | Verdict |
|---|---|---|---|
| `record_fields` | record type | `Vec<(field name, ParameterType)>` | key → `ParameterType`. The `String` in the value is a FIELD name; it stays. |
| `enum_members` | `(enum type, member name)` | `usize` | first element → `ParameterType`; the member name stays a `String`. |
| `union_names` | union type | — | → `ParameterType` |
| `union_variants` | variant type | **union type** | **both** → `ParameterType`. Not a symbol table: the value is looked up as a type (`builder_value_semantics.rs:1165`). |
| `union_variant_unions` | variant type | set of union types | **both** → `ParameterType` |
| `union_variant_tags` | variant type | tag `usize` | key → `ParameterType` |
| `union_variant_fields` | variant type | `Vec<(field name, ParameterType)>` | key → `ParameterType` |
| `resource_names` | resource type | — | → `ParameterType` |
| `resource_closers` | resource type | **routing name** | key → `ParameterType`; **value stays `String`** |

**`resource_closers` — the Open Decision's "recommended: no" is half right, and
the half it protects is the half that matters.** Read
`validation.rs:381-384` (key `resource.name`, value `resource.close_function`),
`:452-466` (key `resource.type_name`, value
`format!("{identity}.{package_name}.{close_function}")`) and its only two
consumers: `builder_resource_cleanup.rs:33` looks it up **by a resource type**
and hands the value straight to `resolve_closer_symbol`, and
`builder/mod.rs:970` iterates `.values()` into the same function.
`resolve_closer_symbol` (`builder/mod.rs:2409`) resolves that value through
`function_symbols`, peeling a 16-hex-digit identity prefix — a *routing* name in
exactly the sense bug-374 and bug-377 established. So the value is untouchable
and stays a `String`; the KEY is a type name like every other, and the gate
counts it as one.

**`union_variants` and `enum_members` ARE type-keyed.** `union_variants`' value
is the union a variant belongs to, consumed as a type
(`builder_value_semantics.rs:1165-1168` looks a variant up and then reads its
tag). `enum_members`' key is a pair whose first element is the enum type and
whose second is a member name — only the first is a type.

**Key type: `ParameterType`, not `Symbol`** (§Open Decisions). Every key is a
nominal today, so `Symbol` would fit — but it would diverge from the tables
letter B already keyed by `ParameterType` (`ir::verify`'s nine, `ir::lower`'s
`TypeIndex`, `ir::shape`'s two), and it would force each caller to destructure
`Named(sym) => sym` and decide what a composite means. `ParameterType` lets an
emitter pass what it holds.

**Merge/split: measured, and it FAILED — see Correction C1.** The round-trip
property this task named (`parse(key).name() == key`) holds, but it is the wrong
question. A declared type may shadow a built-in spelling (`TYPE Integer`
compiles), so the string tables MERGED the record with the scalar, and building
the key with `ParameterType::named` SPLITS them. Keys are therefore built with
`ParameterType::declared` (= `parse`), which is what an `AS Integer` annotation
elaborates to. Verified against a pre-plan-111 binary over seven shadowing
programs: 7/7 identical after the fix, 5/7 before.

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

- [x] For each of the nine fields, read every construction site and every read
      site, and record in this file: is the key a **type** (convert to
      `ParameterType`), a **nominal name** (convert to `Symbol`), or a
      **routing symbol** (leave `String`)? Cite the code that decides it.
      → §2 "Phase 1 decision table (VERIFIED)". All nine keys are declared type
      names; `ParameterType` for every one (§Open Decisions resolved there).
- [x] Specifically resolve `union_variants`, `resource_closers` and
      `enum_members`, whose doc comments suggest they are symbol tables, not type
      maps. `resource_closers`' value is a routing name (bug-374, bug-377) —
      confirm from `resolve_closer_symbol` and do not convert it on a guess.
      → Confirmed from `resolve_closer_symbol` (`builder/mod.rs:2409`) and both
      consumers: its VALUE stays a `String`, its KEY is a type like the rest.
      `union_variants` and `enum_members` are type-keyed.
- [x] ~~Measure the merge/split hazard: for every key inserted into each map to be
      converted, assert across the whole corpus that
      `ParameterType::parse(key).name() == key`~~ — a key that does not round-trip
      would change identity under re-keying. Record the result.
      → **The named property holds and is the wrong question.** Measured the
      right one instead, against a pre-plan-111 binary: **Correction C1**. A
      declared type may shadow a built-in spelling, so the string tables merged
      the record with the scalar and `named` would split them. Keys use
      `ParameterType::declared`. 7/7 probes match the baseline; a bug letter B
      had already shipped is fixed and pinned by
      `tests/rt_shadowing_type_name_diagnostics.rs`.
- [x] Write the decision table into §2 of this file, replacing the UNVERIFIED
      entries.

Acceptance: **MET** — every one of the nine fields has a recorded verdict with
its citation, and the merge/split measurement is recorded with the command that
produced it (`git worktree add --detach f79f6212a` + a seven-program probe
battery) and its counts (5/7 matching before the fix, 7/7 after).
~~No code changed.~~ **Correction**: this phase was specified as read-only, but
its measurement found a live bug in already-landed work. Per `AGENTS.md`
("never leave a bug you found — fix it now, outranking scope") it is fixed here
rather than recorded for later.
Commit: 219de05cd

### Phase 2 — re-key the type maps

- [x] Change each field Phase 1 marked "type" to a `ParameterType` key.
      All nine; keys built with `ParameterType::declared` (Correction C1).
- [x] Add a temporary `#[cfg(debug_assertions)]` equivalence assertion ~~at each
      converted lookup: the typed lookup's result equals the string lookup's
      result~~ **at construction: the key set is a bijection with its spellings**.
      Then compile the whole fixture corpus with the **debug** binary and
      confirm no assertion trips — CI is linux + DEBUG and the local gates are
      mac + RELEASE, so a release-only run proves neither axis (the CI-jobs
      memory). This is a compile sweep only; no goldens are diffed.
      **Correction C2** explains the change of form.
- [x] ~~Remove the equivalence assertions and the shadow string tables once the
      corpus is clean, in a separate commit that names the run that proved them.~~
      — moot: the assertion needs no shadow table, so there is nothing to remove
      and keeping it makes the invariant permanent (Correction C2).
- [x] Convert the emitters' *lookups* only — do not convert emitter signatures
      (D and E).
- [~] Lower the gate's `string_keyed_type_maps` budget to 0 in the clearing commit.
      `codegen` 11 → 2; `binary_repr` 1 and `target` 1 untouched. The remaining
      4 are `link_thunk`'s `record_native_resources`, `validation`'s
      `native_resources`, `usage.rs`'s `resource_union_closes` and
      `binary_repr`'s `foreign_types` — none is a `TypeModel` field. Cleared in
      Phase 3, which is where §1's "0 tree-wide" is actually met.
- [x] Tests: add a `TypeModel` unit test building a model with a nested container
      key (`List OF Map OF String TO Integer`) and a stateful resource key
      (`File STATE Cursor`) and asserting both resolve — the two shapes most
      likely to differ between spelling-keyed and type-keyed lookup.
      `a_type_model_resolves_nested_container_and_stateful_resource_keys`.

Acceptance: **MET.** `scripts/typemodel-debug-sweep.sh target/debug/mfb` →
`1288 project(s) compiled with the debug binary — 0 assertion trip(s),
511 expected-reject build(s)`, exit 0.
`cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0, 0 failures.
Plus, beyond the plan: the Correction C1 probe battery still matches the
pre-plan-111 baseline 7/7, and
`scripts/artifact-gate.sh {collections,general}` → 0 diff(s), 14 goldens.
Commit: 20c0b68b6

### Phase 3 — collapse the registry's dual API (largest blast radius)

- [x] ~~Add `resolve_call_return_type_typed` beside
      `builtins::resolve_call_return_type` (7 callers, no twin today).~~ — moot:
      it already existed (plan-104-C). What this phase did instead was DELETE the
      string half and remove the twin's own render-in/parse-out pocket.
- [x] Repoint all ~~37~~ `resolve_call` callers to `resolve_call_typed`, then delete
      `resolve_call`. Same for `call_return_type` (~~32~~) and `argument_types` (~~8~~).
      **Correction C3**: the production populations were 7 / 2 / 0, not 37 / 32 / 8.
      One `#[cfg(test)]`-gated spelling shim survives for ~140 registration
      assertions — see Correction C4.
- [x] Convert `constant_type_name` and `general_override_target`
      (`src/codegen/registry/mod.rs:1665`, `:1744`) to typed signatures.
      `constant_type_name` absorbs the parse of its descriptor literal (the
      redundant `constant_type` wrapper is deleted); `general_override_target`
      takes a type and renders once against the descriptor's `&'static str` row,
      which §Non-goals forbids changing.
- [x] Convert `src/ir/shape.rs`'s `arg_types: &[String]` (`:292`) and its
      `type_name.as_str() == "Integer"` (`:302`) ~~now that the typed registry
      query exists~~ — this is the item letter B deferred here. **Correction**:
      letter B did it, on §2's own stated condition (the typed twin already
      existed). What this phase converted in `ir/shape.rs` were its four
      remaining `resolve_call_return_type` calls.
- [x] Repoint `src/ir/lower.rs`'s ~~4~~ registry string calls (2 live:
      `call_return_type`, `constant_type_name`).
- [x] ~~Rename the surviving `_typed` functions back to the plain names~~ — moot,
      and deliberately so: `resolve_call` is taken by the `#[cfg(test)]` shim, and
      a rename that makes the production entry and a test-only shim differ by a
      suffix is worse than one where the suffix marks which is which. The `_typed`
      suffix now reads as "the typed one, as opposed to the test spelling shim",
      which is exactly what a reader needs. See Correction C4.
- [x] Lower the gate budgets for `codegen/registry` and `codegen/builtins/*/mod.rs`
      by what this phase removed.
- [x] Tests: add a registry resolution test covering the resource-vs-value-union
      strict-matcher distinction — a resource param must still reject a
      union→concrete-resource widening, and a value-union param must still accept
      variant→union (the strict-matcher memory). This is the overload-resolution
      regression guard.

Acceptance: **MET.** No production caller of any string-form registry query
remains — measured with the gate's own `test_free_lines` stripper rather than
`rg`, which counts inline test modules (Correction C3): `resolve_call` 0,
`call_return_type` 0, `argument_types` 0, `resolve_call_return_type` 0,
`constant_type` 0. `cargo test --no-fail-fast -- --skip artifact_gate_all` →
exit 0, 0 failures.
Commit: b0c516cac, c950ddf8b

### Phase 4 — the last `format!` type construction

- [x] Rewrite `refined_list_literal_type`
      (`src/codegen/collection/layout/builder_collection_layout.rs:2459`) to build
      `ParameterType::ListOf(Box::new(element))` structurally instead of
      `format!("List OF {element}")`. This is plan-106-E Correction 4's site.
      Its `List OF Unknown` test is a variant match now, and the caller no longer
      parses the refined spelling back.
- [x] Confirm `:2890` is `#[cfg(test)]` (plan-106-E Correction 4 recorded it as
      such) and convert or exempt it accordingly. Confirmed: it is
      `alloc_size_matches_free_size`, inside the test module — the gate excludes
      it, and it is converted anyway so the fixtures read in the same currency.
- [x] Lower the gate's `format!` budget for `codegen` to 0.

Acceptance: **MET.** The `format_type_construction` class reads **0** for
`codegen` (its row is deleted from the budget table);
`cargo test --no-fail-fast -- --skip artifact_gate_all` → exit 0, 0 failures.
Commit: 34f300996

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

**Result: 0 diffs on all four, MET.** Run against a release binary rebuilt at
the letter's tip (the on-disk one predated the last commit — the stale-release
trap):

```
artifact-gate [collections]: 1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [math]:        1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [strings]:     1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
artifact-gate [general]:     1 tests, 6 build(s), 7 golden(s) checked, 0 diff(s)
```

Nothing was regenerated. Cross-arch goldens are included in those 7 each, so the
`TypeModel` re-keying and the registry API collapse are byte-neutral on every
target this letter can see from a macOS host.

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
- Doc sync: **done.**
  * `.ai/resources-packages.md` — new "Registry query surface is typed" section:
    the one-typed-query-per-question rule, the `#[cfg(test)]` shim and why a
    production caller of it is what the ratchet gate catches,
    `call_return_type_name`'s remaining render, the strict/lenient asymmetry,
    and the `ParameterType::declared` shadowing rule (Correction C1).
  * `.ai/collections.md` — new "Layout/model tables are keyed by
    `ParameterType`" section: `declared` not `named`, STATE stripped before a
    union lookup, the `other => named(&other.name())` re-wrap that destroyed
    `Stateful` structure in BOTH `validate_package_type` and `is_comparable_seen`
    (the second silently reported a stateful resource comparable), and
    `refined_list_literal_type`'s structural build.
  * `src/docs/spec/architecture/21_type-name-encoding.md` — the Round-trip
    section said the grammar had to be added "in lockstep to **all** of
    `parse_type_name`, `resolve_type_name`, `concrete_type_name` … there is no
    shared parser to change in one place." That is the exact claim plan-111
    inverts, and `concrete_type_name` no longer exists. Rewritten to state the
    one-place rule *and* its replacement hazard (every consumer has a `_` arm,
    so an unwired variant is silent, not a compile error). Also fixed **7
    dangling citations** — `element_accepts_item`, `func_type_parts`,
    `user_template_parts`, `split_top_level_commas`, `split_top_level_to`,
    `concrete_type_name`, `resolve_type_name` — repointed to the surviving
    symbols; all 20 citations in the file now resolve (`/tmp/check_cites.sh`,
    a `fn|struct|enum|const|static|type <sym>` grep per `[[file:sym]]`). Four of
    the seven predate this plan (plan-105-B retired the monomorph helpers), so
    this was accumulated drift, not only C's.
- Formatting: `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`.

## Open Decisions

**Both RESOLVED in Phase 1 — see the decision table in §2.**

- ~~**Key type for nominal-only maps**~~ — recommended: `Symbol` for maps whose key
  is genuinely a bare nominal name (`union_variant_tags`, `resource_names`),
  `ParameterType` for maps whose key can be a composite. Using `ParameterType`
  everywhere is simpler but boxes a nominal into a tree node for no gain.
  Phase 1 decides per field. (§Phase 1)
  → **`ParameterType` for all nine.** `Symbol` would fit (every key is a nominal
  today) but would diverge from the tables letter B already keyed by
  `ParameterType`, and would force each caller to destructure `Named(sym) => sym`
  and decide what a composite means. And Correction C1 makes the point sharper
  than "no gain": the key must equal what the LOOKUP passes, and a lookup passes
  what an annotation elaborated to — a `ParameterType`.
- ~~**Whether `resource_closers` is in scope**~~ — recommended: **no**. Its value is
  a routing name and its key is a resource *name*; bug-374 and bug-377 both live
  here and neither is a type-representation bug. Convert only if Phase 1's read
  of `resolve_closer_symbol` contradicts this. (§Phase 1)
  → **Half in scope.** The read of `resolve_closer_symbol` confirms the VALUE is
  a routing name and it stays a `String` — the half bug-374/bug-377 live in. The
  KEY is a declared resource type name like every other key in `TypeModel`, is
  looked up by a resource TYPE, and is converted.

## Corrections

**C1 — Phase 1 task 3's merge/split measurement found a REAL hazard, and a bug
letter B had already shipped.** The task asks: for every key inserted, does
`ParameterType::parse(key).name() == key`? That round trip holds. It is also the
wrong question, and the right one fails.

**A declared type may shadow a built-in spelling.** `TYPE Integer` is legal and
compiles — verified by building it. So the name-keyed tables *merged* the record
`Integer` and the scalar `Integer`, because `records["Integer"]` matched a field
annotated `AS Integer` by string equality.

Re-keying can turn that merge into a **split**, and it depends on which
constructor builds the key. `ParameterType::named("Integer")` mints a `Named`
nominal; an `AS Integer` annotation elaborates through `parse` to the `Integer`
*variant*. They are not equal, so a table keyed with `named` stops answering a
lookup the string table answered — silently, because for every name that is NOT
a built-in spelling the two constructors agree.

**Letter B built its keys with `named`, so letter B shipped that split.**
Measured against a pre-plan-111 binary (`git worktree add --detach f79f6212a`,
`cargo build --release`) over a battery of shadowing-type programs:

```
DIFF     record shadows Integer: arity
  --- baseline ---
    …:1 error[…TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION]…
    …:5 error[…TYPE_CONSTRUCTOR_ARITY_MISMATCH]…
  --- current  ---
    …:5 error[…TYPE_CONSTRUCTOR_ARITY_MISMATCH]…
```

`TYPE Integer { a AS Integer }` stopped reporting its self-cycle, because
`record_field_cycle`'s key (`Named("Integer")`) no longer equalled its field's
type (`Integer`). Two of seven probes differed; five already matched.

**Fixed** by `ParameterType::declared(name)` (`src/types.rs`) — it parses, so a
declaration keys as the type its own annotations denote — and by repointing all
**35 + 22** construction sites letter B had written with `named`. Deliberately
NOT converted, each for a stated reason: fixed built-in nominal literals
(`named("Error")`), `ir/shape`'s `named(&other.name())` re-wraps (which
deliberately FLATTEN a structural type into one opaque nominal — letter B
Correction 14 — so `declared` there would re-decompose it and defeat the
re-wrap), `canonical_import_name`'s rewrite of an already-`Named` symbol, and
`instantiate_type`'s minted mangled symbol (minted, not declared, and never a
built-in spelling).

All seven probes now match the baseline. `tests/rt_shadowing_type_name_diagnostics.rs`
pins six of them permanently, including the positive direction (a correct
constructor must not be spuriously rejected) and one pre-existing quirk it would
otherwise be easy to mistake for re-keying damage (a field read on a shadowing
record is rejected by the primitive-name test, in the baseline too). The
self-cycle case was **RED-checked**: reverting just
`ir/verify/types.rs`'s `declared` back to `named` fails it.

**The lesson for the rest of this letter, and for D–F**: the round-trip property
Phase 1 asked about (`parse(k).name() == k`) is necessary but not sufficient.
The question that decides a re-key is **"does the key I build equal the key the
lookup passes?"** — and the two sides are written in different files, months
apart, by different constructors.

**C2 — the equivalence check is a construction-time invariant, not per-lookup
assertions, and it stays.** Phase 2 specified a temporary
`#[cfg(debug_assertions)]` assertion *at each converted lookup* comparing the
typed result against a shadow string table's, to be deleted once the corpus was
clean. That would need a shadow copy of all nine maps threaded through
~90 call sites, and it checks a weaker property than it looks: a lookup
assertion only fires for the keys a given program happens to reach.

`TypeModel::assert_type_keys_are_bijective` checks the property that actually
makes a re-key safe, at construction, over every key present:

* every key survives a round trip through its own spelling — rules out a
  **split** (Correction C1's failure mode);
* no two keys share a spelling — rules out a **merge**, where one entry silently
  overwrites another and a lookup returns the wrong record layout, union tag or
  close op.

It needs no shadow table, costs nothing in release, and is checked for every
module the corpus compiles rather than only the reached lookups. So there is
nothing to remove, and it is kept as a permanent invariant rather than deleted —
a strictly better outcome than the specified one.

The sweep is `scripts/typemodel-debug-sweep.sh`, kept in the repo so letters D–F
can re-run it after their conversions.

**A false alarm worth recording, because the shape recurs.** The sweep's first
two runs reported 5 trips. All five were the harness, not the compiler: it
grepped the build output for `assert|panicked`, and the word matches a
*diagnostic* — "a bare binding **asserts** the resource has no state" — and a
fixture path (`tests/syntax/testing/testing-assert-invalid`). Each of the five
was re-run individually and reported **0** panics. Tightened to
`panicked at|assertion .* failed`; the clean run is the third.

That is the diagnostic-harness lesson in a new place: a sweep that classifies by
grepping free text will misclassify, and the failure is silent in the
*optimistic* direction just as easily — a filter this loose would also have
matched nothing at all had the panic message been worded differently.

**C3 — Phase 3's census is 84 call sites; the production population was 21.**
The plan counts 37 `resolve_call`, 32 `call_return_type`, 8 `argument_types`,
7 `builtins::resolve_call_return_type`. Re-measured with the gate's own
`test_free_lines` stripper rather than `rg`:

| Query | plan | production |
|---|---|---|
| `registry::resolve_call` | 37 | 1 |
| `registry::call_return_type` | 32 | 1 |
| `registry::argument_types` | 8 | **0** (deleted in letter B) |
| `builtins::resolve_call_return_type` | 7 | 5 |
| `constant_type_name` | — | 2 |
| `general_override_target` | — | 5 |

The difference is inline `#[cfg(test)]` modules — the same blind spot as
plan-111-A Correction 3, and the third time in this plan an `rg`-based census
has over-counted by counting test code. **The remedy is not another correction
per letter**: letters D–F should re-measure their own populations with
`test_free_lines` before budgeting, because the same over-count is baked into
every one of their §2 tables.

**C4 — one `#[cfg(test)]` spelling shim survives, and the `_typed` rename does
not happen.** Phase 3 says to delete `resolve_call` outright and then rename the
surviving `_typed` functions to the plain names. Both were reconsidered against
what the code actually looks like afterwards.

`registry::resolve_call`'s ~140 remaining call sites are per-package
**registration** assertions — `resolve_call("audio.poll", &s(&["audio.AudioInput"]), true)
== Some("Boolean".to_string())`. A spelling is the right thing for those to
assert: a descriptor's job is to resolve to a particular type, and its name is
how the test says which. Rewriting each as `.map(|t| t.name().into_owned())`
makes 140 tests harder to read and proves nothing new. So the shim stays,
`#[cfg(test)]`-gated with a doc comment saying exactly that — it cannot become a
second production API, which is what §1 actually asks for.

Given that, the rename is worse than the status quo: `resolve_call` is taken by
the shim, so `resolve_call_typed` would have to become something else anyway,
and a scheme where the production entry and a test-only shim differ by a suffix
is confusing in exactly the direction that matters. The `_typed` suffix now
reads as "the typed one, as opposed to the test spelling shim", which is the
distinction a reader needs. Recorded rather than done.

## Summary

This is the letter plan-106 declined to write. The risk is real and it is in two
named places: a re-keyed table that merges or splits a key (Phase 2, caught by
the equivalence assertions run in a debug build over the whole corpus) and a
typed registry query that resolves a different overload (Phase 3, caught by the
strict-matcher regression test and by `artifact-gate all`).

Untouched: codegen's emitter signatures and their 147 spelling match arms
(letters D–F), and the five sanctioned boundaries.
