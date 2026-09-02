# plan-121-A: In-place fast-path audit and the shared destination-slot abstraction

Last updated: 2026-09-02
Overall Effort: huge (> 3d)
Effort: large (3h–1d)
Depends on: nothing

The `try_inplace_*` family in `src/codegen/collection/assign/builder_inplace_assign.rs`
turns a value-semantics collection update (`x = OP(x, …)`) into a mutation of the
live buffer whenever it can prove nothing else observes that buffer. Where an arm
exists the benchmark row is at or near C; where one is missing the row is up to
17742× slower than C. The arms were added one at a time, so each re-derives its
own destination-slot resolution and its own aliasing guards, and the matrix has
holes (§2).

This sub-plan adds **no new fast path and changes no behavior**. It extracts the
part every arm repeats — resolve the destination collection slot, prove unique
ownership, run the aliasing gates — into one reusable seam, and re-expresses the
three existing `append` arms (plain local, record field, STATE field) through it.
The behavioral outcome: **`cargo test` is green and every `.ncode` golden is
byte-identical**, proving the seam is a pure refactor before any of B–G builds on
it.

References:

- `benchmark/RANKING.md` — the grade/cluster system that selected this work.
- `.ai/collections.md` — List/Map/Set codegen invariants; in particular the
  soundness argument for in-place append (value semantics + copy-insertion +
  `FOR EACH` count snapshotting) that every new arm must preserve.
- `.ai/codegen-invariants.md` — arch-neutral codegen/regalloc invariants.
- `.ai/testing-gates.md` — artifact gate, byte-identity, acceptance harness.
- `mfb spec` §14 (value/copy semantics), §4.2 (records are immutable; `WITH` is
  the only update form).
- `tests/codegen_inplace_append_call_result.rs` — the precedent for testing that
  a fast path is *taken*, not merely that the result is correct.

## The plan-121 feature at a glance

Seven sub-plans, letter order = implementation order. The scope is the ten worst
clusters in `./benchmark/rank.py`'s work queue — 60 benchmark rows that are
grade C-or-worse, 48 of them grade F and 32 losing outright to CPython. Those 60
rows take 4613.7 ms in mfb where `c -O0` takes 9.5 ms.

| | sub-plan | rows | effort | what it fixes |
|-|---|---|---|---|
| A | in-place slot abstraction | 0 (refactor) | large | the seam B–D build on; byte-identical |
| B | plain-local container | 6 + 5 | large | `insert`/`removeAt`/Set `remove` have no arm anywhere |
| C | record-field container | 16 | large | only `append` has a record arm |
| D | `RES … STATE` container | 16 | large | only `append` has a STATE arm; holds the worst row (17742×) |
| E | set-algebra work asymmetry | 9 | medium | **not a compiler fix** — the rows measure iteration-order luck |
| F | String element writes | 5 | large | a length-changing `set` is O(N^1.6) |
| G | String accumulator folds | 6 | medium | `reduce` is O(N²) where the hand loop is O(N) |

Rows overlap between C/D and F (a `Record-Dynamic` `set` needs both), which is
why F depends on D: the container copy must go first or F's win is unmeasurable.

**Sub-plan E is a warning about this plan's own evidence.** It was originally
scoped as "rewrite the interpreted set-algebra bodies natively" and a spike
refuted that before any code was written — the bodies are already native-backed,
and the 410× is C early-exiting sooner than mfb on a predicate that is always
FALSE. Every other sub-plan's root cause was proven by a scaling spike for the
same reason; see each one's "Verified properties".

## Prerequisites

These are a precondition on the whole plan-121 feature. Sub-plans B–G point here.

| Must be true | Command | Status |
|---|---|---|
| Release `mfb` binary builds | `cargo build --release` → exit 0 | MET (built 2026-09-02) |
| Baseline benchmark logs present | `ls benchmark/baseline/*.log` → 6 files | MET |
| Ranking reproduces the scoped rows | `./benchmark/rank.py --csv \| awk -F, '$3=="F"' \| wc -l` → 56 | MET |
| Full suite green at HEAD | `cargo test --no-fail-fast` → 0 failures | UNMEASURED — run before starting |
| Acceptance goldens green at HEAD | `./scripts/test-accept.sh` → 0 mismatches | UNMEASURED — run before starting |

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. The two UNMEASURED rows are this sub-plan's first
> act: a pre-existing red must be characterized before any change lands, or
> every later gate inherits an unexplained failure.

Everything below is written against the world where these hold.

## 1. Goal

- One seam resolves "where does this collection live and may I mutate it in
  place?" for all three containers (plain MUT local, record field via
  `WITH`, `RES … STATE` field), and the three existing `append` arms are
  re-expressed through it.
- `cargo test --no-fail-fast` green and **every `.ncode`/`.ncodesum` golden
  byte-identical to HEAD** — this sub-plan is provably neutral (§3).

### Non-goals (explicit constraints)

These are the guardrails for **every** sub-plan of plan-121, not just A:

- **No language-semantics change.** `x = OP(x, …)` keeps exact value semantics:
  the result is indistinguishable from allocating a fresh collection and
  assigning it. In-place is an implementation strategy the program cannot
  observe. If a case cannot be proven unobservable, the arm must decline and
  fall through to the copying path — **declining is always correct**.
- **No new surface.** No new builtin, no new keyword, no signature change, no
  `mfb man` page change. `collections::set`/`insert`/`removeAt`/`remove` keep
  their current signatures and documented behavior.
- **No test or golden weakened to pass.** AGENTS.md's four-question gate applies
  in full. `.ncode`/`.ncodesum` are drift sentinels, not behavior — but in *this*
  sub-plan a drift is a bug (§3), not a golden to regenerate.
- **No layout/ABI change.** Collection header layout, entry stride, and the
  data-region base rule (`capacity`, never `count` — `.ai/collections.md`) are
  untouched here.
- **Correctness outranks the benchmark.** A row that cannot be made fast without
  risking a wrong answer stays slow, and the plan records why.

## 2. Current State

`src/codegen/collection/assign/builder_inplace_assign.rs` (907 lines,
`wc -l` → 907) holds the family; the dispatch chain is at
`src/codegen/engine/control/builder_control.rs:879-909`, tried in order and
falling through to the general copying reassignment when every arm declines.

Each arm independently: rejects `by_ref`; pattern-matches the `NirValue` shape;
checks the call target via `crate::codegen::builtins::native_builtin_target`;
requires the mutated collection to be the *same* binding being assigned; and runs
container-specific aliasing gates (e.g. `for_each_iterable_record_fields` at
`builder_inplace_assign.rs:133-139`).

### Measured populations

| What | Count | Command |
|---|---|---|
| `try_inplace_*` arms today | 10 | `grep -rhoE "fn try_inplace_[a-z_]*" src/codegen/ \| sort -u \| wc -l` → 10 |
| Lines in the in-place module | 907 | `wc -l src/codegen/collection/assign/builder_inplace_assign.rs` |
| Benchmark rows C-or-worse | 149 | `./benchmark/rank.py --csv \| awk -F, 'NR>1 && ($3=="C"\|\|$3=="D"\|\|$3=="F")' \| wc -l` |
| …of those, in the top-10 clusters | 60 | script in §2.1 below |
| …grade F among the 60 | 48 | same |
| …that lose to CPython (RED) | 32 | same |
| rt-behavior collection fixtures | 41 | `ls tests/rt-behavior/collections/ \| wc -l` |
| rt fixtures naming set/insert/removeAt | 30 | `grep -rl "collections::set\|collections::insert\|collections::removeAt" tests/rt-behavior/ \| wc -l` |

The 60 in-scope rows total **4613.7 ms** where `c -O0` takes **9.5 ms**
(`./benchmark/rank.py --csv`, summing `mfb_min_ms`/`c0_min_ms` over rows whose
`row` is one of the ten cluster operations and whose grade is C-or-worse).

### 2.1 The fast-path matrix, measured

Built by reading each arm's guard
(`for fn in …; do awk "/fn try_inplace_$fn\(/,/^    }$/" … | grep -oE 'Some\("[a-z]+"\)|NirValue::WithUpdate'; done`):

| operation | plain MUT local | record field (`WITH`) | `RES … STATE` field |
|---|---|---|---|
| `append` (single) | ✓ `try_inplace_append_assign` | ✓ `try_inplace_record_field_append` | ✓ `try_inplace_state_collection_append` |
| `append` (bulk) | ✓ `try_inplace_bulk_append_assign` | ✗ | ✗ |
| `prepend` | ✓ `try_inplace_prepend_assign` | ✗ | ✗ |
| `set` (List/Map) | ✓ `try_inplace_set_assign` | ✗ | ✗ |
| `add` (Set) | ✓ `try_inplace_set_add_assign` | ✗ | ✗ |
| `removeKey` (Map) | ✓ `try_inplace_remove_key_assign` | ✗ | ✗ |
| `remove` (Set) | ✗ | ✗ | ✗ |
| `insert` (List) | ✗ | ✗ | ✗ |
| `removeAt` (List) | ✗ | ✗ | ✗ |
| `&` self-concat (String) | ✓ `try_inplace_concat_assign` | ✗ | ✗ |

**17 empty cells across the three container columns for the nine collection
operations** (3 plain + 7 record + 7 STATE).

### Verified properties

Each claim below was established by running a program, not by reading alone.
Spike sources are in `spikes/` on branch `worktree-research`.

- **VERIFIED — where an arm exists, the row is already at C parity; where it is
  missing, it is catastrophic.** Same list, same container, same benchmark
  harness: `list (Record-Dynamic) append` is **0.839× of C** (grade S) while
  `list (Record-Fixed) set` is **1630×** (grade F)
  (`./benchmark/rank.py --csv | awk -F, '$2=="append"||$2=="set"'`). The only
  difference is whether an arm matched. This is the plan's central premise and
  it is measured, not assumed.
- **VERIFIED — the plain-local `set` arm is genuinely O(1); the record-field
  path is O(N).** Spike 1 holds the call count at 2000 and varies N:
  plain Integer local is flat at 47–55 ns/set across N = 50…1600; the same
  update through a record field rises 951 → 12844 ns/set over the same range.
- **VERIFIED — `FOR EACH` aliasing gates are load-bearing and must be
  replicated by every new arm.** `builder_inplace_assign.rs:133-139` declines
  the record-field append when a live `FOR EACH` iterates that field, because
  the grow would free the buffer the iterator walks. `.ai/collections.md`
  records the same rule for the plain-local arm (count snapshotted at loop
  entry). Read both before adding an arm.
- **UNVERIFIED — that all three container columns can share one slot-resolution
  seam.** This is the sub-plan's design uncertainty and Phase 2 is the cheapest
  experiment that settles it. If the STATE column's resolution turns out to be
  irreducibly different, the seam covers plain+record and STATE keeps its own —
  a smaller win, not a dead design.

## 3. Design Overview

Three layers, bottom-up:

1. **`InPlaceDest`** — resolves *where the collection lives*: a frame slot for a
   plain local, a field offset inside a record local for the `WITH` form, a
   STATE-block offset for the `RES` form. Returns the base pointer, the slot to
   write the (possibly reallocated) block back into, and the element type.
2. **`InPlaceGate`** — the proof obligations, run once per attempt regardless of
   container: not `by_ref`; the assignment target is the same binding being
   mutated; no live `FOR EACH` aliases this collection; the collection type has a
   `CollectionTypeLayout`. Any gate failing returns `Ok(false)` and the caller
   falls through to the copying path.
3. **Per-operation lowerings** — unchanged in behavior, but written once against
   `InPlaceDest` instead of once per container.

**Where correctness risk concentrates:** layer 2. A gate that is *weaker* in the
shared seam than in the arm it replaced silently un-protects an aliasing case in
all three containers at once. Mitigation: Phase 1 writes the gates down as an
explicit inventory (an audit with no code change), and Phase 2 must implement
exactly that inventory — a gate present in any arm today is present in the seam.

**Where design uncertainty concentrates:** whether STATE resolution fits
(§2, UNVERIFIED). Phase 2 does plain+record first and STATE second, so the
uncertainty is isolated to one task.

### Byte-identity is this sub-plan's gate — deliberately

This sub-plan is the provably-neutral class: pure code motion behind an
unchanged dispatch chain, no new arm, no new match. So
**byte-identical `.ncode`/`.ncodesum` across all targets IS the acceptance
check**, and it is the strongest available.

A diff is therefore a bug this refactor introduced — objdump ONE fixture,
localize, fix. **A byte-identity failure here never means the seam is
impossible**; it means a specific emission changed and the change must be found.
Sub-plans B–G are the other class entirely: they change codegen on purpose, so
byte-identity is the wrong gate there and each names the goldens expected to
drift.

### Rejected alternatives

- **Write 17 more standalone arms.** Rejected: the family is already 907 lines of
  near-duplicated guard logic; 17 more triples the surface on which a
  guard can be forgotten, which is exactly the correctness risk the non-goals
  forbid. The measured cost of the seam (this sub-plan) is far less than the
  cost of auditing 27 hand-written guard sets.
- **A generic "mutate any collection in place" pass over NIR.** Rejected: it
  would need whole-function alias analysis to be sound, which is a much larger
  correctness surface than the current syntactic, conservative,
  decline-by-default arms. The existing shape is conservative *by construction*;
  that property is worth keeping.
- **Make records mutable / add `a.field = v`.** Rejected outright: a language
  semantics change, forbidden by the non-goals. §4.2 makes `WITH` the only
  update form and this plan does not touch that.

## Compatibility / Format Impact

None. No API, file format, wire format, or collection layout changes. The
observable contract of every affected builtin is unchanged — that is what the
byte-identity gate proves.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work it describes; fill `Commit:` the moment a phase lands. An
> unticked box means NOT DONE.

### Phase 1 — Characterize HEAD and inventory the gates (no code change)

Establishes the baseline every later gate is measured against, and writes down
the proof obligations before anything is refactored.

- [ ] Run `cargo test --no-fail-fast` at HEAD; record the pass/fail count and
      characterize any pre-existing red (attribute it via
      `git worktree add --detach`, per AGENTS.md) in this plan's Corrections.
- [ ] Run `./scripts/test-accept.sh`; record `N ran` and any mismatches. Watch
      the ran-count — a silently skipped fixture reads as a pass.
- [ ] Read all 10 arms and write `planning/plan-121-gate-inventory.md`: for each
      arm, every condition under which it declines, and which of the 10 arms
      enforce it. This file is the specification Phase 2 implements.
- [ ] Record the `.ncodesum` baseline for the targets the artifact gate covers,
      so Phase 2/3 can diff against it.

Acceptance: `planning/plan-121-gate-inventory.md` exists and lists, for each of
the 10 arms, its decline conditions; the HEAD test and acceptance results are
recorded in this plan with their counts. No source file under `src/` is modified.
Commit: —

### Phase 2 — Introduce `InPlaceDest` + `InPlaceGate` and port the three `append` arms

The refactor itself, gated on producing identical machine code.

- [ ] Add `InPlaceDest` to `src/codegen/collection/assign/` resolving the three
      container forms to (base pointer, write-back slot, element type). Plain
      local and record field first.
- [ ] Add `InPlaceGate` implementing exactly the inventory from Phase 1 — every
      decline condition any arm enforces today, no fewer.
- [ ] Re-express `try_inplace_append_assign` and `try_inplace_record_field_append`
      through the seam. Dispatch order in `builder_control.rs:879-909` unchanged.
- [ ] Extend `InPlaceDest` to the STATE container and port
      `try_inplace_state_collection_append`. If STATE resolution does not fit,
      stop extending, leave that arm as-is, and record in Corrections what
      differs — plain+record sharing is still the deliverable.
- [ ] Tests: extend `tests/codegen_inplace_append_call_result.rs`-style coverage
      with a positive case per container proving the fast path is still *taken*
      (a black-box rt fixture cannot see a missed fast path — it just gets slow).

Acceptance: `cargo test --no-fail-fast` green with no new failures vs. Phase 1's
recorded baseline, **and** `.ncode`/`.ncodesum` byte-identical to the Phase 1
baseline on every target the artifact gate covers. A diff on any target is a bug
in this refactor: objdump one fixture, localize it, fix it — then the gate passes.
Commit: —

### Phase 3 — Prove the seam is reusable, without adding a fast path

De-risks B–D by demonstrating the seam accepts a *new* operation before any
sub-plan depends on it.

- [ ] Wire `collections::removeAt` for the plain-local container through the
      seam **behind a compile-time-off constant** (not a user-facing flag), so
      the code path exists and is compiled but never selected.
- [ ] Add a unit test that constructs the `InPlaceDest`/`InPlaceGate` pair for
      `removeAt` and asserts the gate declines in each aliasing case from the
      Phase 1 inventory.
- [ ] Confirm with the constant off that `.ncode` is still byte-identical.

Acceptance: the gate-decline unit test passes for every inventory condition, and
`.ncode` is byte-identical with the path disabled — proving the seam admits a new
operation without disturbing existing emission. Sub-plan B turns the constant on.
Commit: —

## Validation Plan

- **Tests:** codegen-inspection coverage per container in
  `tests/codegen_inplace_*.rs` (positive: the fast path is taken); unit coverage
  for `InPlaceGate` decline conditions. No rt-behavior fixture should need to
  change — if one does, that is a semantics change and the non-goals forbid it.
- **Coverage check:** confirm the new module is in the suite's denominator
  (`cargo test` alone can pass while never executing the seam) — build with
  `--bin mfb` coverage per `.ai/build-tooling.md` and check the new file appears.
- **Runtime proof:** `spikes/s1` re-run before and after must produce the same
  per-set nanosecond profile (this sub-plan changes no performance); the
  benchmark rows must not move.
- **Doc sync:** `.ai/collections.md` gains a short subsection describing the seam
  and pointing at the gate inventory, since it is the doc AGENTS.md sends future
  sessions to before collection codegen work.
- **Acceptance:** `cargo test --no-fail-fast`, `./scripts/test-accept.sh`, and
  the artifact gate, all compared against the Phase 1 recorded baseline.
  `rustup run 1.96.0 cargo fmt --all && (cd repository && rustup run 1.96.0 cargo fmt)`
  at the end, per AGENTS.md.

## Open Decisions

- **STATE container in the seam or left alone** — recommend attempting it in
  Phase 2 and accepting a plain+record-only seam if resolution differs, rather
  than forcing a shared shape. (§3)
- **Where `InPlaceDest` lives** — recommend `src/codegen/collection/assign/`
  beside the arms, rather than promoting it to `engine/`, until sub-plan D shows
  whether STATE needs engine-level access. (§3)

## Corrections

<Filled in during execution.>

## Summary

The engineering risk is entirely in the gate inventory: the seam must not be one
condition weaker than the arms it replaces, because a missing gate becomes an
aliasing bug in three containers simultaneously. Byte-identity plus the
per-container positive tests are what hold that. Untouched here: every empty cell
in the matrix (that is B–D), the String representation issues (F–G), and the
interpreted set-algebra bodies (E).
