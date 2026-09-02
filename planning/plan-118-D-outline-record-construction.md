# plan-118-D: Per-type construction/copy functions for records

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-118-C (the out-of-lining pattern and its proven perf gate; C's benchmark verdict is a precondition for repeating the pattern here)

Out-of-line record construction (and the record copies it implies) into one
synthesized function per record type. `val:Constructor` is **2,173,050
builder-emitted instructions over 7,876 sites = 276/site** (plan-118-A §2) —
the second-largest attribution category. A constructor site inlines: arena
alloc + inline alloc-failure error block, per-field stores, and — because
records inline their String fields (`.ai/codegen-invariants.md` "Records
inline their String fields") — an inline byte-copy loop per String field. The
micro-fixture (plan-118-A §2): a one-line three-field constructor function is
554 instructions with 4 allocs and 4 inline error paths.

References:

- plan-118-A §2 (attribution), plan-118-C (pattern + perf gate).
- Emitter: `src/codegen/engine/value/builder_values.rs:1187`
  (`NirValue::Constructor` arm).
- **Precedent for per-TYPE synthesized functions**: the recursive-type
  deep-copy functions (bug-391) — `recursive_transfer_types` +
  `thread_copy_symbol(type)` at `src/codegen/engine/builder/mod.rs:1406-1420`:
  the module already synthesizes one copy `CodeFunction` per qualifying type
  and routes value copies to a `bl` instead of inlining the recursion.
- Layout law: record layout invariants in `.ai/codegen-invariants.md`
  ("Records inline their String fields (offset, not pointer)").

## Prerequisites

Family gate in plan-118-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-118-C complete with its benchmark gate PASSED | C's doc archived with recorded benchmark tables | NOT MET |
| Per-type copy precedent understood | read `builder/mod.rs` recursive_copy_types block | MET (read this session) |

If C's Phase-1 benchmark gate had failed and C was re-designed, re-evaluate
this letter's premise before starting — same mechanism, same risk.

## 1. Goal

- `val:Constructor` attribution over `tests/acceptance` drops from 2,173,050
  to ≤ 400,000; module total drops ≥ 1.5 M vs the plan-118-C baseline;
  benchmark suite regresses ≤ 5 % (expected neutral/faster).

### Non-goals (explicit constraints)

- **Record layout is untouched** — field offsets, String inlining, block
  headers all stay exactly as `.ai/codegen-invariants.md` records them.
- Copy-vs-alias semantics (`mfb spec` §14) unchanged; constructor argument
  ownership (which args are deep-copied into the block) unchanged.
- Vector-native types (`Float2/3/4` register promotion, plan-01-vector) keep
  their register path — constructor out-of-lining applies only where a block
  is built today (the vector-inline path declines before the Constructor arm).
- Compile-time-constant constructors that fold today keep folding.

## 2. Current State

- The `Constructor` arm (`builder_values.rs:1187`) lowers each argument, then
  emits alloc + error path + field stores + String-field byte-copy loops
  inline, per site.
- Per-type synthesized copy functions already exist for recursive types
  (bug-391): the same "one function per type, call it" shape this letter
  generalizes. Their synthesis point, symbol naming (`thread_copy_symbol`),
  and cross-reference closure are working precedent.

### Measured populations

| What | Count | Command |
|---|---|---|
| `val:Constructor` | 2,173,050 instrs / 7,876 sites = 276/site | plan-118-A §2 |
| Micro-fixture `makePoint` (String + 2 Integer fields) | 554 instrs, 4 `_mfb_arena_alloc` + 4 `_mfb_make_error_result` | `--ncode` fixture (A §2) |
| Distinct record types constructed in the acceptance corpus | UNMEASURED | Phase 1 census (attribution keyed by type, or NIR walk) |
| Sites per type distribution (is a per-type function amortized?) | UNMEASURED | same census |

The per-type census gates the design: a type constructed once gains nothing
from a shared function (the function body ≈ the inline code). Expectation from
the test corpus (many constructions of the same testing/record types) is heavy
reuse; MEASURE FIRST — it is Phase 1, and its result sets Phase 2's
per-type threshold (e.g. synthesize only for types with ≥ 3 construction
sites; below that, keep inline).

### Verified properties

- The vector-inline and constant-fold paths run before the general arm —
  read `builder_values.rs:723-905` (Call arm dispatch order) and the
  Constructor arm; out-of-lining the general arm cannot capture them.

## 3. Design Overview

For each record type `T` past the site-count threshold, synthesize
`construct.T(args…, loc) → ptr | error` as a `CodeFunction` (naming/symbol via
the `symbol_fragment` conventions; synthesis alongside the recursive-copy
functions in `builder/mod.rs`), body = today's inline sequence emitted ONCE:
alloc, error route, field stores, String-field copies (word-width, matching
plan-118-C's copy style). Constructor sites lower arguments as today, then
marshal + `bl` + check.

ABI note: constructors with more args than register-argument slots need the
stack-arg convention the user-function ABI already defines
(`abi::incoming_stack_arg_load`, `function_lowering.rs:950`) — reuse it, do
not invent a second convention. Wide constructors (> 8 fields) exist in the
builtin packages; the census counts them.

**Correctness risk**: argument ownership. Today each argument's
pending-temp/ownership resolution happens inline against the constructor's
copies; with a call boundary, the same claims must hold on the caller side
(the helper deep-copies; caller temps stay caller-freed). Wrong either way is
leak/double-free — gated by rt fixtures and the same leak-sensitive tests C
used, not by goldens.

Byte-identity NOT the gate; constructor-heavy goldens are EXPECTED to diff.

Rejected: one generic `runtime.construct(layout-descriptor)` interpreter
(runtime cost per field, layout descriptors in rodata — slower and a second
layout authority, violating the one-layout-law rule); outlining only the
String-field copy (leaves alloc + error + stores inline, < half the win).

## Phases

### Phase 1 — census (no behavior change)

- [ ] Extend the plan-118-A attribution key for Constructor with the type name
      (one-line change to the tally key) and run over `tests/acceptance`;
      record the distinct-type count and per-type site distribution here.
- [ ] From it, set the synthesis threshold (recommended: ≥ 3 sites).

Acceptance: census table in this doc; no artifact change (`artifact-gate` 0
diffs — the key change is `-vv`-only).
Commit: —

### Phase 2 — synthesis + site rewrite

- [ ] `builder/mod.rs`: synthesize `construct.T` per qualifying type
      (mirroring the recursive-copy synthesis block); unused-function
      validation wired like the runtime helpers.
- [ ] `builder_values.rs:1187`: rewrite the general Constructor arm to
      marshal + call for qualifying types; inline path retained below
      threshold.
- [ ] Regenerate churned goldens; benchmark before/after recorded here.
- [ ] Doc sync: `planning/speed.md` note; spec architecture page if it
      describes constructor lowering (census in-phase).

Acceptance: `val:Constructor` ≤ 400 k; module total −≥ 1.5 M vs C's baseline;
full `cargo test --no-fail-fast` + `test-accept.sh` + regenerated
`artifact-gate.sh all` green; benchmark gate ≤ 5 %; leak-sensitive rt fixtures
(construct-in-loop) pass; remote-box runtime proof per `.ai/remote_systems.md`.
Commit: —

## Validation Plan

- Tests: add an rt fixture constructing a String-field record in a hot loop
  (pins both the value and the leak behavior); existing record/union suites.
- Runtime proof: benchmark table per phase; the construct-in-loop fixture's
  output.
- Acceptance: family-standard gate set (see plan-118-C Validation).

## Open Decisions

- Threshold value (≥ 3 sites recommended) — finalized by Phase 1's census.
- Whether union wrap (`val:UnionWrap`, 15,352 instrs — small) rides along —
  recommended: no; not worth the seam.

## Corrections

*(fill during execution)*

## Summary

Second-largest category, same proven pattern as C with an in-tree per-type
precedent (bug-391 copy functions); the census-first phase keeps it from
synthesizing functions nothing amortizes, and ownership at the new call
boundary is where the review attention belongs.
