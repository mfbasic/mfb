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
| plan-118-C complete with its benchmark gate PASSED | C's doc archived with recorded benchmark tables | MET — `planning/completed/plan-118-C-…`, gate tables in both phases (worst row +1.1 %, and a −2.5 %/−9.3 % A/B) |
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
| Distinct record types constructed in the acceptance corpus | **59** | new `-vv` `costliest constructor type` tally |
| Sites per type distribution (is a per-type function amortized?) | extremely top-heavy — see below | ditto |
| Record types clearing the ≥ 3 threshold | **35** | `-vv` counter `synthesized construct.T` |

Per-type census, `-vv` over `tests/acceptance` (inclusive of the arguments' own
lowering, so it reads as "what a construction site costs in total"; the
`val:Constructor` row in the expansion tally is the exclusive aggregate):

```
--- trace: costliest constructor type (40 of 59 keys, 4147094 total) ---
     3104179      3609x  record Error          860/site
      833679      3609x  record ErrorLoc       231/site
       33646        35x  record vector.Fixed4
       30095        43x  record vector.Fixed3
       22092       116x  record vector.Float2
       ...            (the remaining 54 types are 4 % between them)
```

**Two types are 95 % of it, and both are the compiler's own error plumbing**:
every `FAIL` builds an `Error` whose third field is a fresh `ErrorLoc`, which is
why their site counts are equal to each other and to `op:Fail`'s 3,609. The
distribution is so top-heavy that the threshold's exact value moves almost
nothing — 34 of 59 types have ≥ 3 sites and are 99 % of the cost.

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

- [x] ~~Extend the plan-118-A attribution key for Constructor with the type name~~
      — landed as a SEPARATE `-vv` tally (`costliest constructor type`) instead
      of extending the key. Extending it would have split `val:Constructor` into
      59 small rows and dropped the aggregate off the top-40 leaderboard, which
      is the row plan-118-E's re-census reads. Both survive this way, and the
      per-type one is inclusive (what a site costs in total) where the aggregate
      is exclusive.
- [x] From it, set the synthesis threshold (recommended: ≥ 3 sites). — **≥ 3
      confirmed**, and the census shows the choice barely matters: 34 of the 59
      types clear it and account for 99 % of the cost, while the top two alone
      are 95 %.

Acceptance: census table in this doc; no artifact change (`artifact-gate` 0
diffs — the key change is `-vv`-only).

MET: census table in §2; the tally is `-vv`-only, and `artifact-gate.sh all` was
0 diffs at this point.
Commit: —

### Phase 2 — synthesis + site rewrite

- [x] `builder/mod.rs`: synthesize `construct.T` per qualifying type
      (mirroring the recursive-copy synthesis block); unused-function
      validation wired like the runtime helpers. — gated on the relocation
      scan, like every other internal `bl` target: a type can clear the site
      threshold and still have every site fold away.
- [x] `builder_values.rs`: rewrite the general Constructor arm to
      marshal + call for qualifying types; inline path retained below
      threshold.
- [x] Regenerate churned goldens; benchmark before/after recorded here.
- [x] Doc sync: `planning/speed.md` note; spec architecture page if it
      describes constructor lowering (census in-phase). — no spec page describes
      it; the internal `bl` helper family is not a documented surface at all
      (the same census plan-118-C ran).
- [x] Added: the fixture the Validation Plan asks for —
      `tests/rt-behavior/arena/construct-helper-loop`, 20,000 iterations
      constructing a String-field record and a **10-field** record (two
      arguments past the eight register slots, so the bug-08 stack tail is
      exercised through the helper), plus the empty-String boundary and a `WITH`
      copy proving the blocks are independent. Four goldens, hand-created
      (`sync-goldens.sh` creates none).
- [x] Added: **`tests/cli_build_determinism.rs`** — see Corrections 2.

Acceptance: `val:Constructor` ≤ 400 k; module total −≥ 1.5 M vs C's baseline;
full `cargo test --no-fail-fast` + `test-accept.sh` + regenerated
`artifact-gate.sh all` green; benchmark gate ≤ 5 %; leak-sensitive rt fixtures
(construct-in-loop) pass; remote-box runtime proof per `.ai/remote_systems.md`.

Measured: `val:Constructor` 2,173,050 → **1,421,067** (−34.6 %); module
12,872,114 → **11,880,468** (−991,646). 35 types synthesized. As in C, the
≤ 400 k figure is D+E's joint target (Corrections 1): a `construct.T` call site
is marshal + `bl` + check, and what remains under the row is the ~194-instruction
inline allocation-failure block plan-118-E Phase 2 shares.

Benchmark gate **PASSED**: 1.5 M constructions of a String-field record and a
flat record plus 200 k trapped `FAIL`s, compiled by the pre-D compiler
(`52ab6cd99`) and by this one, run interleaved 7×. min 0.268 s → 0.268 s
(**+0.3 %**), median 0.271 s → 0.272 s (+0.5 %) — neutral, which is the expected
shape: out-of-lining trades inline code for a call and leaves the dynamic
instruction count essentially unchanged.

`scripts/test-accept.sh`: **1346 test(s) ran**, passed (1347 with the new
fixture). `artifact-gate.sh all`: 1823 goldens, **0 diffs** after regenerating
the 89 that churned. Acceptance 732/732.

**Remote-box runtime proof: DONE for three ABIs.** The new
`construct-helper-loop` fixture cross-built and run natively, output compared
byte-for-byte against the macOS AArch64 golden:

| box | target | result |
|---|---|---|
| host | macos-aarch64 | golden |
| 2228 Ubuntu x86_64 | linux-x86_64 (glibc) | **identical** |
| 2227 Alpine x86_64 | linux-x86_64 (musl) | **identical** |
| 2229 Alpine riscv64 | linux-riscv64 (musl) | **identical** |

x86-64 SysV is the strongest of these for this change: it passes only SIX
arguments in registers, so the fixture's 10-field record puts **four** on the
stack tail there against AArch64's two — the argument-marshalling seam is
covered harder on the remote box than on the host.

2230 (Win11) is **down** — `ssh -p 2230` is `Connection refused`, retried. The
Windows path is therefore covered only by cross-compilation and its
`.ncodesum` goldens (regenerated, gate 0 diffs), not by a native run. Recorded
rather than assumed; `.ai/remote_systems.md` and `scripts/test-winapp.sh` are
where the run belongs when the box is back.
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

1. **`val:Constructor ≤ 400 k` is D+E's joint target, not D's** — the same
   correction plan-118-C recorded, for the same reason. A `construct.T` call
   site is marshal + `bl` + check; what remains under the row is the
   ~194-instruction inline allocation-failure block that must stay at the site
   (its `ErrorLoc` names the construction that failed) and that plan-118-E Phase
   2 shares per function. D took the category from 2,173,050 to 1,421,067 and
   the module from 12,872,114 to 11,880,468.

   The census also **re-values E upward again**: `record Error` and
   `record ErrorLoc` are 3,609 sites each — the same 3,609 as `op:Fail` — so
   what D out-of-lined here is largely the error block's own construction cost.
   E's remaining half is the staging and propagation around it.

2. **This letter broke codegen determinism, and nothing would have caught it.**
   `synthesized_constructor_types` returned a `HashMap`, and the caller iterated
   it to decide the ORDER `construct.T` functions are emitted in — which is
   observable in the `.ncode`. Three consecutive builds of
   `tests/byte-identity/csv` produced three different `sha256`s:

   ```
   f6b14698cdd69967196f4990c78ed8d3fa659338cfe1277846b704fba314bac0
   f698e36961d222cab45e4e17b054416cbdd5088f07085d13f46931e05e04268d
   55758a39e5fed52385ddc7cd7ae0a2d23421ee7c8fcdb5f9a6ff0ea21732b8b1
   ```

   The whole byte-identity gate — 1,823 goldens, `artifact-gate.sh`, every
   `.ncodesum` — assumes determinism and **nothing tested it**. Worse, the
   failure does not present as a compiler bug: it looks like a flaky golden, and
   the repair one reaches for is to regenerate the golden, which "fixes" it until
   the next run. (It nearly worked that way here: a regeneration left 56 diffs
   and only then did the pattern become visible.)

   Fixed by sorting the qualifying types by name before emission, and pinned by
   a new **`tests/cli_build_determinism.rs`**, which builds one project three
   times and compares the `-ncode` dumps byte-for-byte. Proven RED against the
   bug: with the sort removed the test fails, with it restored it passes.

3. **The stack-argument tail is reachable and is covered.** §3's ABI note flags
   constructors wider than the register bank as needing
   `abi::incoming_stack_arg_load` / `outgoing_stack_arg_store` rather than a
   second convention — done, and exercised rather than assumed: the new fixture
   builds a **10-field** record, which puts two arguments on the tail on AArch64
   and **four** on x86-64 SysV (six register arguments there), and the x86-64
   run on box 2228 is byte-identical to the host golden.

   A note for the next reader, since it cost a false start here: a record is
   constructed with `T[…]`, not `T(…)`. The parenthesised form is a function
   call and fails with `TYPE_UNKNOWN_VALUE` ("value type could not be
   determined"), which reads like an arity limit and is not one — 10-field
   records compile and run.

## Summary

Second-largest category, same proven pattern as C with an in-tree per-type
precedent (bug-391 copy functions); the census-first phase keeps it from
synthesizing functions nothing amortizes, and ownership at the new call
boundary is where the review attention belongs.
