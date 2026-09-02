# plan-117: Monomorph FunctionContext sharing — root-cause fix for the §5.2 monomorphize row

Last updated: 2026-09-01
Effort: large (3h–1d, dominated by full-suite gate wall time; the code change itself is small)

`monomorphize` is 20.9 % of a release build (14,353.4 ms of 68.8 s —
`planning/speed.md` §4, measured by `mfb test -vv` over `tests/acceptance`) to
convert 1,618 generic HIR functions into 1,609 concrete ones, and it speeds up
only 2.1× from debug to release where every other row gains 3.4–10.9×
(`planning/speed.md` §3). The report called its shape "defect rather than
volume" and left it "Not yet root-caused" (§5.2). This plan records the root
cause — found and **validated by a working prototype** in the `research`
worktree (spike, 2026-09-01) — and lands the fix.

**Root cause (measured, not inferred).** `monomorph::FunctionContext`
(`src/monomorph/mod.rs:114`) couples per-scope state (`locals`,
`enclosing_return`) with four program-wide tables that never vary per scope:
`function_returns` (1,609 entries), `function_types` (1,609),
`record_fields`, and `globals`. Every scope boundary in body lowering — both
arms of every `IF`, every `MATCH` case, every loop body, every lambda, every
trap handler (11 `context.clone()` sites, census below) — deep-clones the
whole thing. Instrumented counters over the `tests/acceptance` build:
**25,987 clones copying 85,282,698 map entries, 11,312 ms inside `Clone`
alone** (plus the matching un-counted `drop` cost of freeing them), out of an
18,699 ms monomorphize span on the measurement box. A
`/usr/bin/sample` capture of the same window shows the top-of-stack is
almost entirely `tiny_malloc`/`tiny_free`, `ParameterType::clone`,
`drop_in_place<ParameterType>`, hashbrown `RawTable::{clone,drop}`, and
`String::clone` — pure allocator traffic, which is exactly why optimized Rust
barely helps (the §3 2.1× anomaly). A second, smaller defect:
`function_context()` (`src/monomorph/lower.rs:1940`) rebuilds all four
tables from scratch for every lowered function — 1,609 builds × ~3,220
inserts, 1,179 ms (O(F²) in the function count).

**Prototype result** (identical corpus and box, this worktree): sharing the
four tables behind one `Rc` and cloning only the per-scope state took
monomorphize from **18,698.8 ms to 1,229.8 ms (15.2×)** — resolve phase
19,002 ms → 1,529 ms — with the same 25,987 clone count (identical control
flow), 732/732 acceptance tests passing, and 63/63 `monomorph` unit tests
passing (`cargo test --release monomorph`). On the report's release numbers
this turns 20.9 % of the build into ~2 %.

References:

- `planning/speed.md` §3, §4, §5.2, recommendation 2 — the measurements this plan answers.
- `src/monomorph/mod.rs` (`FunctionContext`, its manual `Clone`), `src/monomorph/lower.rs` (`function_context`, `add_function_to_context`, `expression_type`, the 11 clone sites).
- `src/docs/spec/architecture/12_monomorphization.md:171-189` — spec text describing `FunctionContext` seeding and `add_function_to_context`; obligated to stay current (`.ai/specifications.md`).
- `.ai/testing-gates.md` — artifact-gate / acceptance-harness mechanics used as this plan's gates.

## Prerequisites

Everything below is written against current `main` (`f4be5ea25` at plan time).
Re-measured at execution time against `main` = `00dbc5102` (2026-09-01).

| Must be true | Command | Status |
|---|---|---|
| The `-vv` compile profiler exists (baseline instrument) | `grep -n 'fn span' src/trace.rs` → `209:pub(crate) fn span(name: &'static str) -> Span` | MET (re-run 2026-09-01) |
| No other in-flight plan owns `src/monomorph/` | `grep -ln 'src/monomorph' planning/plan-11[0-9]*.md planning/plan-1[2-9]*.md` → only `planning/plan-117-monomorph-context-sharing.md` (the plan-118/119 families added since plan time do not touch it) | MET (re-run 2026-09-01) |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop.

## 1. Goal

- The `monomorphize` row of `cd tests/acceptance && ../../target/release/mfb test -vv`
  drops from ~21 % of the build to under 3 %, with `scripts/artifact-gate.sh all`
  at 0 diffs and every existing suite green. (Prototype-proven attainable:
  1,229.8 ms vs 18,698.8 ms on the same box.)

### Non-goals (explicit constraints)

- **No change to monomorphization semantics**: which templates instantiate, which
  overloads resolve, what concrete names are mangled, and the emitted HIR must be
  identical. Phase 1 is a pure representation change and is gated on byte-identity.
- **No change to `ParameterType`** or any type-grammar surface (that is plan-111/113
  territory).
- No new crate dependencies (no `im`-style persistent maps).
- Scope-locality of type bindings is preserved: a local bound inside an `IF`
  branch, `MATCH` case, loop, lambda, or trap must still be invisible outside it
  (that is what the per-scope clone exists for; only its *cost* changes).
- The diagnostic behavior (`report`, file attribution via `current_file`,
  instantiation budgets) is untouched.

## 2. Current State

`Monomorphizer` (`src/monomorph/mod.rs:30`) walks every concrete function
(`run()`, `src/monomorph/lower.rs:397`) and, per function
(`lower_function_inner`, `src/monomorph/lower.rs:612`), builds a fresh
`FunctionContext` via `function_context()` (`src/monomorph/lower.rs:1940`):
one `HashMap` entry per concrete function for `function_returns` and again
for `function_types` (via `function_signature_types`,
`src/monomorph/helpers.rs:586` — allocates a `ParameterType::Func` per
function), plus `record_fields` (a `Vec<HirTypeField>` **clone** per record
type) and `globals`. The context then flows down statement/expression
lowering, and every construct that introduces a scope clones it wholesale
(sites below). `expression_type` (`src/monomorph/lower.rs:2009`) is the only
reader; `add_function_to_context` (`src/monomorph/lower.rs:1968`) patches a
newly-mangled callee's signature into the *current* context so later
inference in that scope can type calls to it (per the spec,
`12_monomorphization.md:188-189`).

`FunctionContext` has a hand-written `Clone` (`src/monomorph/mod.rs:131`)
that clones all six fields. Nothing outside `src/monomorph/` names the type
(`grep -rn "FunctionContext" src/ --include="*.rs" | grep -v monomorph` → 0 hits).

### Measured populations

All measurements from the spike (2026-09-01, macOS arm64 measurement box,
release compiler built with `CARGO_PROFILE_RELEASE_DEBUG=true`, corpus
`tests/acceptance` via `cd tests/acceptance && ../../target/release/mfb test -vv`).
Wall times on this box run ~1.3× the `planning/speed.md` numbers (background
load); ratios are what matter.

| What | Count | Command |
|---|---|---|
| `context.clone()` sites in body lowering | 11 | `git show HEAD:src/monomorph/lower.rs \| grep -c "context.clone()"` → 11 (lines 644, 1187, 1188, 1211, 1284, 1319, 1350, 1358, 1683, 1716, 2083) |
| Context clones per acceptance build | 25,987 | spike counter in `FunctionContext::clone` |
| Map entries copied by those clones | 85,282,698 | spike counter (avg 3,282/clone = the full tables) |
| Time inside `Clone` alone | 11,312 ms | spike counter (`Instant` around the clone body); excludes drop cost |
| `function_context()` builds | 1,609 | spike counter (one per lowered function/binding) |
| Time inside `function_context()` | 1,179 ms | spike counter |
| `monomorphize` span, baseline | 18,698.8 ms | `-vv` span tree, unmodified HEAD binary |
| `monomorphize` span, prototype | 1,229.8 ms | `-vv` span tree, prototype binary (same box, same corpus) |
| Entries copied by clones, prototype | 40,769 | spike counter (locals + overlay only; same 25,987 clone count) |
| Concrete functions / table size | 1,609 / ~3,220 | spike counter `concrete_functions=1609`; tables = 2×1,609 + record_fields + globals |
| Reader/writer sites to migrate | 6 reads + 2 writers | `grep -n "\.function_returns\|\.function_types\|\.record_fields\|\.globals" src/monomorph/*.rs` at HEAD |
| Out-of-module users of `FunctionContext` | 0 | `grep -rn "FunctionContext" src/ --include="*.rs" \| grep -v monomorph` → 0 |

### Verified properties

- **Only `locals` (+ `enclosing_return`) varies per scope.** Verified by reading
  every writer: `record_fields` and `globals` are written only inside
  `function_context()` (`lower.rs:1959,1969`); `function_returns`/`function_types`
  are written there (`lower.rs:1953-1954`) and in `add_function_to_context`
  (`lower.rs:1982-1983`). No other mutation exists (grep above).
- **The prototype preserves behavior on every suite it was run against**:
  732/732 acceptance tests (`mfb test`), 63/63 `monomorph`-filtered unit tests
  (`cargo test --release monomorph`), identical 25,987 clone count (control
  flow unchanged). UNVERIFIED so far: `scripts/artifact-gate.sh all` (1,780
  goldens) and the full `cargo test` — Phase 1's gates below.
- **Overlay-before-shared lookup order reproduces the old overwrite.** Today
  `add_function_to_context` *overwrites* the snapshot entry in the single map;
  an overlay consulted before the shared table yields the same value for the
  same key. Verified by reading both writers and the three readers
  (`expression_type` Identifier and Call arms).

## 3. Design Overview

Two independent pieces, in landing order:

1. **Phase 1 — share the invariant tables (the win, spike-validated).** Split
   `FunctionContext` into `SharedTables` (`function_returns`, `function_types`,
   `record_fields`, `globals`) held behind a single `Rc`, plus the true
   per-scope state (`locals`, `enclosing_return`) and a small overlay
   (`added_returns`, `added_types`) that carries `add_function_to_context`'s
   mid-body additions with today's exact scope-local semantics. A scope clone
   becomes: clone `locals` (avg ~1.6 entries, measured), clone the two overlay
   maps (usually empty), bump one `Rc`. `Rc` (not `&'a SharedTables`) because
   contexts live across `&mut self` lowering calls — a borrow would conflict
   with `instantiate_function` mutating the monomorphizer while contexts are
   alive; `Rc` is the standard escape and the tables are single-threaded.
   This is a **provably-neutral representation change**, so **byte-identity is
   the correctness gate**: `scripts/artifact-gate.sh all` must report 0 diffs.
   (If it diffs, that is a bug in the migration — root-cause the one fixture
   and fix it; it is never a premise-falsified stop.)

2. **Phase 2 — delete the per-function snapshot (the cleanup).** With Phase 1
   landed, `function_context()` still rebuilds `SharedTables` once per
   function (1,609 × ~3,220 inserts = 648.4 ms measured on the prototype).
   Replace the snapshot with **live lookups**: `expression_type` consults
   `self.concrete_functions` (computing `function_signature_types` on demand
   — one function per query, not 1,609) and `self.concrete_types` directly;
   `globals` is built once in `Monomorphizer::new` (its source is immutable).
   `FunctionContext` shrinks to `locals` + `enclosing_return`, and
   `add_function_to_context` + the overlay **are deleted** — the live map
   already contains every instantiation the overlay used to patch in.
   Risk analysis (read, not guessed): during one function's lowering,
   `self.concrete_functions` gains only *new* keys (nested `instantiate_function`
   inserts, `lower.rs:859`); value *replacement* happens only between top-level
   functions (`run()`, `lower.rs:401,412`). Live lookup therefore sees a
   superset of snapshot + overlay: the same values for every key the old code
   could see, plus mangled instantiation names in sibling scopes the overlay
   didn't reach. Un-lowered HIR never names a mangled callee, so a behavioral
   difference requires a lowered subtree re-typed in a scope that didn't
   witness the instantiation — not constructible from the current call sites
   (all `arg_types` are computed in the same statement that lowered the args).
   Still, this is an inference-visibility change in principle, so Phase 2
   keeps the same artifact-gate expectation of 0 diffs **and treats any diff
   as a real inference delta to root-cause individually** (it would surface in
   `.ast`/`.ir` goldens first, not `.ncode`).

**Where risk concentrates:** Phase 2's live-lookup equivalence argument. It is
scheduled last, gated identically, and severable — Phase 1 alone delivers the
plan's goal. Design uncertainty in Phase 1 is ~zero: the prototype is this
design, already run against the acceptance corpus.

**Rejected alternatives:**

- *`Rc<RefCell<SharedTables>>` with in-place mutation* — would make
  `add_function_to_context` additions visible outside their scope, changing
  overlay semantics Phase 1 promises to preserve; also spreads interior
  mutability for no measured need.
- *Cache `SharedTables` on the `Monomorphizer`, invalidate on mutation* —
  degenerates to a rebuild per function because `run()` re-inserts every
  lowered function (`lower.rs:412`), invalidating between every pair of
  functions. Incremental single-entry maintenance via `Rc::make_mut` COWs the
  whole table whenever a context is alive (every nested instantiation). Both
  shapes are more machinery than Phase 2's deletion for strictly less win.
- *Persistent (HAMT) maps* — new dependency for a problem the split removes.
- *Reducing `expression_type` recomputation* (e.g. `builtin_call_return_type`
  re-typing arguments) — after Phase 1+2 the whole remaining span is ~580 ms
  (1,229.8 − 648.4); no further row is worth the churn until re-measured.

## 4. Detailed Design — Phase 1

New module-private struct in `src/monomorph/mod.rs`:

```rust
/// The program-wide, per-function-immutable half of a FunctionContext: built
/// once per lowered function, shared (never cloned) by every nested scope.
#[derive(Default)]
struct SharedTables {
    function_returns: HashMap<String, ParameterType>,
    function_types: HashMap<String, ParameterType>,
    record_fields: HashMap<ParameterType, Vec<HirTypeField>>,   // plan-111-B: keyed by TYPE
    globals: HashMap<String, ParameterType>,
}

#[derive(Default, Clone)]
struct FunctionContext {
    locals: HashMap<String, ParameterType>,
    shared: std::rc::Rc<SharedTables>,
    /// Mid-body additions (`add_function_to_context`); consulted BEFORE
    /// `shared`, matching the overwrite the old single-map insert performed.
    added_returns: HashMap<String, ParameterType>,
    added_types: HashMap<String, ParameterType>,
    enclosing_return: Option<ParameterType>,
}

impl FunctionContext {
    fn function_return(&self, name: &str) -> Option<&ParameterType> { /* added → shared */ }
    fn function_type(&self, name: &str) -> Option<&ParameterType> { /* added → shared */ }
}
```

The hand-written `Clone` impl (`mod.rs:131`) is replaced by the derive — every
field is `Clone` and the derive is exactly the old body minus the cost.

`src/monomorph/lower.rs` changes (all sites enumerated in §2's population table):

- `function_context()` (`lower.rs:1940`): build a `SharedTables`, wrap in
  `Rc::new`, return `FunctionContext { shared, ..Default::default() }`. Loop
  bodies unchanged.
- `add_function_to_context` (`lower.rs:1968`): insert into
  `added_returns`/`added_types` instead.
- Read sites: `lower.rs:1523` and the `expression_type` Constructor
  (`:2047`) / MemberAccess (`:2074`) arms → `context.shared.record_fields`;
  Identifier arm (`:2038-2039`) → `context.function_type(..)` then
  `context.shared.globals`; Call arm (`:2083`) → `context.function_return(..)`.

No signature outside the module changes; nothing else names the type (§2).

## 5. Detailed Design — Phase 2

- `Monomorphizer` gains `globals: HashMap<String, ParameterType>` built in
  `new()` from `source` (the exact loop now in `function_context()`,
  `lower.rs:1956-1964`).
- `expression_type` replaces its three shared-table reads with:
  - functions: `self.concrete_functions.get(name).map(function_signature_types)`
    (returns/signature picked per arm);
  - records: `self.concrete_types.get(type_)` filtered to
    `TypeDeclKind::Type`, fields borrowed (the `.cloned()` at `lower.rs:1523`
    keeps its clone of one `Vec`, not of the table);
  - globals: `self.globals`.
- Delete `SharedTables`, the overlay fields, `FunctionContext::function_return/
  function_type`, `function_context()`'s table build (the function reduces to
  `FunctionContext::default()` + `enclosing_return` handling at its callers),
  and `add_function_to_context` with its call site (`lower.rs:1487-1489`).
- `builtin_call_return_type` and every `expression_type` recursion keep their
  signatures (`&self` methods reading `self` maps borrow-check as today).

## Compatibility / Format Impact

None externally. No CLI, `.mfp`, golden format, ABI, or spec-visible language
behavior changes. The spec's *architecture* page describing `FunctionContext`
internals is updated to match (task below).

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with one line on what remains; fill
> `Commit:` the moment a phase lands. An unticked box means NOT DONE.

### Phase 1 — Share the invariant tables behind one `Rc`

Delivers the measured 15× monomorphize win as a provably-neutral representation
change; safe to land alone (Phase 2 is severable cleanup).

- [x] `src/monomorph/mod.rs`: add `SharedTables`; re-shape `FunctionContext`
      per §4 (locals + `Rc<SharedTables>` + `added_returns`/`added_types` +
      `enclosing_return`); replace the manual `Clone` impl with `#[derive(Clone)]`;
      add the two overlay-first accessors.
- [x] `src/monomorph/lower.rs`: rebuild `function_context()` to fill a
      `SharedTables` and `Rc`-wrap it; re-point `add_function_to_context` at the
      overlay; migrate the 6 read sites (§4 list). No other line changes.
- [x] Spec sync: `src/docs/spec/architecture/12_monomorphization.md:182-189` —
      describe the shared-tables/overlay split where it currently describes the
      six-field context and `add_function_to_context`'s insert.
- [x] Tests: no new tests — the 63 existing `monomorph` unit tests
      (`cargo test monomorph`) pin instantiation/overload/unification behavior,
      and the gate below pins output identity. (A perf assertion would be a
      flaky timing test; the `-vv` measurement in Acceptance is the perf proof.)
- [x] Run `rustup run 1.96.0 cargo check --all-targets` (test-target warnings)
      and the fmt pair (`rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`).

Acceptance: (1) `rustup run 1.96.0 cargo test --no-fail-fast` fully green;
(2) `scripts/test-accept.sh` green with the full fixture count (watch the
`N ran` line — a shrunken count is a silent skip, per
`planning/completed/` test-accept lore); (3) `scripts/artifact-gate.sh all`
reports **0 diffs over all 1,780 goldens** (byte-identity is this phase's
correctness gate; a diff = root-cause that one fixture, fix, re-run — run the
gate uncontended, `exit=98` means refused/contended, re-run); (4)
`cd tests/acceptance && ../../target/release/mfb test -vv` shows the
`monomorphize` span at ≤ 2,000 ms on the measurement box (baseline 18,698.8 ms),
i.e. under 3 % share.

Measured (2026-09-01, this worktree, macOS arm64):
(1) `rustup run 1.96.0 cargo test --no-fail-fast` → 90 suites, 4391 passed, 0
    failed (`grep -a "test result" | grep -av " 0 failed"` → empty);
(2) `./scripts/test-accept.sh target/release/mfb /tmp/p117-accept-actual` →
    `acceptance tests passed (1346 test(s) ran)`, exit 0;
(3) `./scripts/artifact-gate.sh target/release/mfb all` →
    `1325 tests, 1487 build(s), 1823 golden(s) checked, 0 diff(s)`, exit 0;
(4) `cd tests/acceptance && ../../target/release/mfb test -vv` →
    `monomorphize 1207.3ms 1207.3ms 1 2.2%` (resolve 1495.2ms total),
    `Tests: 732  Pass: 732  Fail: 0`.
Commit: c835446f6

### Phase 2 — Delete the per-function snapshot (live lookups)

Removes the remaining O(F²) rebuild (648.4 ms measured) and the overlay/patch
machinery; `FunctionContext` becomes just `locals` + `enclosing_return`.

- [x] `src/monomorph/mod.rs`: add `globals` to `Monomorphizer`, built in
      `new()`; shrink `FunctionContext` to `locals` + `enclosing_return`
      (derive `Clone`); delete `SharedTables` and the accessors.
- [x] `src/monomorph/lower.rs`: re-point `expression_type`'s function /
      record / global reads at `self.concrete_functions` (+ on-demand
      `function_signature_types`), `self.concrete_types`
      (`TypeDeclKind::Type`-filtered), and `self.globals` (§5); delete
      `add_function_to_context` and its call site (`lower_expression`, the
      `target != *callee` block); collapse `function_context()`.
- [x] Spec sync: update `12_monomorphization.md` again — the
      `add_function_to_context` sentence (`:188-189`) and the seeding
      description now describe live lookups.
- [x] Doc sync: append a dated note to `planning/speed.md` §5.2 recording the
      root cause and pointing at this plan (the report currently says "Not yet
      root-caused").
- [x] Run `cargo check --all-targets` + the fmt pair again.
- [x] *(added during execution)* Retire the now-dangling
      `("src/monomorph/mod.rs", "record_fields")` row in
      `tests/no_type_strings.rs`'s curated `TYPE_KEYED_TABLES`, which
      `curated_type_keyed_tables_all_exist` failed on once the map was deleted.
      See Corrections for the four-question evidence and why no budget moves.
- [x] *(added during execution)* Fix the dangling
      `Monomorphizer::function_context` / `add_function_to_context` citation in
      `function_signature_types`'s doc comment (`src/monomorph/helpers.rs:592`),
      orphaned by deleting both functions.

Acceptance: same four gates as Phase 1, same thresholds. For this phase a
non-zero artifact-gate diff is a **real inference-visibility delta** (§3 risk
analysis): root-cause it individually starting from the diffing fixture's
`.ast`/`.ir` golden — do not regenerate first, and do not revert the phase; fix
or consciously accept the specific delta with evidence in the commit.

Measured (2026-09-01, this worktree, macOS arm64):
(1) `rustup run 1.96.0 cargo test --no-fail-fast` → 90 suites, 4390 passed and
    one `artifact_gate_all` **refusal** (`exit 98`, a peer session held the
    machine-wide gate lock — nothing was checked; see Corrections). Re-run
    uncontended: `cargo test --test golden` → `test artifact_gate_all ... ok`,
    `1325 tests, 1487 build(s), 1823 golden(s) checked, 0 diff(s)`. Every one of
    the 90 suites is green, 4391 tests passed, 0 real failures;
(2) `./scripts/test-accept.sh target/release/mfb /tmp/p117-accept2-actual` →
    `acceptance tests passed (1346 test(s) ran)`, exit 0;
(3) `./scripts/artifact-gate.sh target/release/mfb all` →
    `1325 tests, 1487 build(s), 1823 golden(s) checked, 0 diff(s)`, exit 0.
    **The live-lookup equivalence argument (§3) holds byte-exactly** — the
    inference-visibility change this phase's risk analysis flagged produces no
    delta anywhere in the corpus;
(4) `./target/release/mfb test -vv tests/acceptance` →
    `monomorphize 101.0ms 101.0ms 1 0.2%` (resolve 375.9ms total),
    `Tests: 732  Pass: 732  Fail: 0`, same corpus
    (`HIR functions (generic) 1618` → `(concrete) 1609`).
Commit: 81982a91c

## Validation Plan

- Tests: existing `src/monomorph` unit suite (63 tests,
  `rustup run 1.96.0 cargo test monomorph` → `63 passed`) pins semantics; no
  new unit tests (no behavior is added).
- Coverage check: the changed code is in the suite's denominator — the 63
  monomorph tests drive `lower_function`/`expression_type` through
  `monomorphize_files`, and every acceptance fixture exercises
  `function_context` on all 1,609 functions.
- Runtime proof: `cd tests/acceptance && ../../target/release/mfb test -vv` —
  `Tests: 732  Pass: 732`, and the span tree's `monomorphize` row at ≤ 2,000 ms
  (record the number in the Commit message; prototype measured 1,229.8 ms).
- Doc sync: `src/docs/spec/architecture/12_monomorphization.md` (both phases);
  `planning/speed.md` §5.2 note (Phase 2).
- Acceptance: `rustup run 1.96.0 cargo test --no-fail-fast` +
  `scripts/test-accept.sh` + `scripts/artifact-gate.sh all` (0 diffs), plus
  `cargo check --all-targets` and the two-root `cargo fmt`, per `AGENTS.md`.
  CI note: CI runs a DEBUG `mfb` on Linux — this plan also shrinks the debug
  monomorphize row (30.5 s in §3), so CI wall time should visibly drop; no CI
  change is required.

## Open Decisions

- **Land Phase 2 at all?** ~~Recommended: yes~~ — **RESOLVED: landed.** The
  deciding factor named here was whether Phase 2's artifact-gate surfaces an
  inference delta. It surfaced none: `./scripts/artifact-gate.sh
  target/release/mfb all` → `1325 tests, 1487 build(s), 1823 golden(s) checked,
  0 diff(s)`. The snapshot→live-lookup change is byte-identical across the whole
  corpus, so there was nothing to investigate and no delta to file. Phase 2 is a
  net code removal (the snapshot, the overlay, `function_context`,
  `add_function_to_context`) and took the row a further 1,207.3 ms → 101.0 ms.

## Corrections

- **Golden count: 1,780 → 1,823.** The plan's Phase 1/2 acceptance says
  "0 diffs over all 1,780 goldens". The gate now checks 1,823:
  `./scripts/artifact-gate.sh target/release/mfb all` →
  `artifact-gate [all]: 1325 tests, 1487 build(s), 1823 golden(s) checked, 0 diff(s)`.
  The corpus grew between plan authoring (`f4be5ea25`) and execution
  (`00dbc5102`); no scope derived from the old number, and the acceptance
  criterion (0 diffs) is unaffected.
- **Base commit: `f4be5ea25` → `00dbc5102`.** The plan was written against
  `f4be5ea25`; execution forked `worktree-P-117` from `main` at `00dbc5102`.
  Both prerequisite rows were re-measured at that base and are still MET (see
  Prerequisites). The two plan families added since (plan-118, plan-119) do not
  touch `src/monomorph/` — `grep -ln 'src/monomorph' planning/plan-11[0-9]*.md
  planning/plan-1[2-9]*.md` returns only this plan.
- **Spec sync widened beyond `:182-189`.** The two `expression_type` table rows
  at `12_monomorphization.md:171` (identifier) and `:176` (call) named
  `context.function_types` / `function_returns` directly, so they went stale with
  the same edit. Both were corrected in Phase 1; the identifier row had also been
  silently wrong before this plan (it omitted the `globals` fallback that
  `lower.rs`'s Identifier arm has had since bug-103).
- **Baseline not re-measured on this box.** The plan's 18,698.8 ms baseline comes
  from the spike box; building a HEAD-of-`main` release binary purely to restate a
  ratio was not done. Phase 1's acceptance criterion is an *absolute* threshold
  (≤ 2,000 ms / under 3 %), and the measurement satisfies it directly:
  `monomorphize 1207.3ms 1207.3ms 1 2.2%`, with the enclosing `resolve` phase at
  1495.2 ms against the plan's 19,002 ms baseline for the same span.

### Phase 2

- **Phase 2 beat its own prediction by ~6×.** §3 estimated the post-Phase-2 span
  at "~580 ms (1,229.8 − 648.4)", modelling the win as *only* the deleted
  snapshot rebuild. Measured: **101.0 ms** (`./target/release/mfb test -vv
  tests/acceptance` → `monomorphize 101.0ms 101.0ms 1 0.2%`). The estimate
  undercounted because deleting the snapshot also deletes the overlay — so a
  scope clone drops from `locals` + two maps to `locals` alone — and it retires
  every one of the 1,609 × 2 `function_signature_types` calls the seeding
  performed, in favour of one call per actual query. Final: 20.9 % → 0.2 % of the
  build. This is a miscalibrated *prediction*, corrected here; the plan's Goal
  (under 3 %) was already met by Phase 1 alone.
- **`tests/no_type_strings.rs` carried a row that Phase 2 dangles.** The curated
  `TYPE_KEYED_TABLES` inventory listed `("src/monomorph/mod.rs",
  "record_fields")`; deleting the map made
  `curated_type_keyed_tables_all_exist` fail. Not a regression, and not a test
  weakened to pass — the test is doing exactly its job, and its own assert
  message prescribes the remedy for a genuine removal. Four-question evidence
  per `AGENTS.md`: (1) written for plan-111-B's no-type-strings ratchet, to keep
  the inventory honest; (2) it protects against a listed type-keyed table being
  silently re-keyed to `String` or vanishing from the census population;
  (3) the only consumer is `string_keyed_type_maps` (`tests/no_type_strings.rs:762`),
  and `grep -n 'string_keyed_type_maps", "monomorph' tests/no_type_strings.rs`
  returns nothing — no budget row exists to lower, because the map was already
  type-keyed and contributed zero hits; (4) `grep -rn record_fields
  src/monomorph/` shows no field of that name survives, only the new `fn
  record_fields` accessor, and the table it now reads through
  (`concrete_types`) is *already* a listed type-keyed entry, so no population is
  lost. The one row was removed with that reasoning recorded inline;
  `cargo test --test no_type_strings` → 7 passed, including the
  tight-in-both-directions budget assertion, confirming no count moved.
- **A dangling doc citation in `src/monomorph/helpers.rs`.** The plan did not
  list it: `function_signature_types`'s doc comment said "Shared by
  `Monomorphizer::function_context` and `add_function_to_context`", both of which
  Phase 2 deletes. Rewritten to describe the on-demand call it now serves.
- **`artifact_gate_all` failed once on lock contention, not on a diff.** The
  Phase 2 `cargo test --no-fail-fast` reported
  `artifact-gate.sh could not START: another gate run holds the lock. This is NOT
  a golden regression -- nothing was checked` (`tests/golden.rs:39`). Re-run
  uncontended: `cargo test --test golden` → `1325 tests, 1487 build(s), 1823
  golden(s) checked, 0 diff(s)`, `test artifact_gate_all ... ok`. Recorded so the
  transient is not mistaken for a phase-2 inference delta.

## Summary

The engineering risk is almost nil in Phase 1 (the exact change was prototyped
and measured: 18,698.8 ms → 1,229.8 ms monomorphize, all suites that were run
are green) and concentrated in Phase 2's snapshot→live-lookup equivalence,
which is severable and byte-gated. Untouched: `ParameterType`, the type
grammar, instantiation/mangling/overload semantics, diagnostics, and every
consumer downstream of monomorph's emitted HIR.
