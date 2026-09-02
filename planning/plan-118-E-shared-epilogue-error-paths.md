# plan-118-E: Shared per-function return epilogue and error-construction blocks

Last updated: 2026-09-01
Effort: large (3h–1d)
Depends on: plan-118-D (family order; also C/D shrink the categories this letter would otherwise double-treat — its census must run AFTER them)

Stop re-emitting function-exit cleanup and error-result construction at every
site. `op:Return` is **2,007,382 builder-emitted instructions over 11,432
return sites = 176/site** (plan-118-A §2): each `RETURN` inlines the live
scope's cleanup (pending-temp frees, owned-slot frees, resource closes,
return-value ownership handling) and the frame exit. Separately, every
fallible inline op emits its own ~40–56-instruction error-construction block
(stage code/line/column/message registers, `bl _mfb_make_error_result`, store
3 result registers — visible verbatim in the `RETURN a & b` micro-fixture);
`op:Fail` alone is 255,872 instrs / 3,609 sites, and the same shape recurs
inside `op:Bind` (551,939), `op:Assign` (319,358), `binop:+` (245,967, the
checked-overflow branch) and every conversion. This letter shares both within
each function: one cleanup epilogue chain jumped to from every exit, and one
error-staging block per (error-kind) per function.

**This is the family's largest blast radius** — it rewrites the exit path of
every function — which is why it is the last letter.

References:

- plan-118-A §2 (attribution).
- Emitters: `src/codegen/engine/control/builder_exits.rs:293`
  (`emit_return_exit` / `emit_return_exit_inner` — the per-site cleanup);
  the error-construction staging around `_mfb_make_error_result` /
  `_mfb_build_error_loc` call sites (grep `make_error_result` under
  `src/codegen/` for the emit helpers; census task).
- Cleanup semantics: `active_cleanups` / pending-temp machinery
  (`builder_values.rs:146-190`, `function_lowering.rs` prologue scan), the
  owned-slot zeroing invariant (`function_lowering.rs:1060-1085` — slots are
  zero-initialized precisely so a shared cleanup can null-check them, which is
  the property that makes a SHARED epilogue safe for paths that skipped an
  initializer).
- Lore: "A scope cleanup needs a null guard + BOTH zero-inits" and "Owned-value
  drops must free-and-null the cleanup slot" (`.ai/codegen-invariants.md:72`).

## Prerequisites

Family gate in plan-118-A, plus:

| Must be true | Command | Status |
|---|---|---|
| plan-118-C and -D landed | their docs archived | NOT MET |
| Post-C/D attribution re-measured | `-vv` "costliest expansion" over tests/acceptance, AFTER D | UNMEASURED — Phase 1 re-census; C/D remove error blocks from their sites, so THIS letter's remaining value must be re-read, not assumed from the pre-C numbers |

## 1. Goal

- Over `tests/acceptance` (post-D baseline): `op:Return` attribution drops
  ≥ 60 %, the residual per-site error-staging cost across `op:Fail` /
  `op:Bind` / `op:Assign` / checked binops drops ≥ 50 %, with zero behavior
  change (same values, same error codes/locs, same frees — no leak, no
  double-free).

### Non-goals (explicit constraints)

- Cleanup ORDER and SET per exit path are exactly today's (frees in the same
  order, same conditional null-guards); only the emission site moves.
- Error values are bit-identical: same code, message symbol, and per-site
  line/column (loc is per-site data staged before the jump, never shared).
- No new runtime calls on the happy path (epilogue is a local jump target, not
  a function).
- The `trap` machinery's control flow (`TrapState`, `builder_exits.rs`) keeps
  its semantics untouched.

## 2. Current State

- `emit_return_exit` (builder_exits.rs:293) emits, at every `RETURN`:
  return-value handling (ownership copy/borrow decision), then the full live
  cleanup list, then frame exit. 11,432 sites × 176 avg.
- Every fallible op's failure branch stages the error inline then either
  returns it or routes to the trap label. The staging (~40+ instrs) differs
  per site only in the loc immediates and error constants.
- The owned-slot zero-init invariant already exists module-wide
  (`function_lowering.rs:1060` block) — the precondition for a shared
  epilogue visiting slots that a given path never wrote.

### Measured populations (PRE-C/D numbers — Phase 1 re-measures)

| What | Count | Command |
|---|---|---|
| `op:Return` | 2,007,382 / 11,432 sites | plan-118-A §2 |
| `op:Fail` | 255,872 / 3,609 | ditto |
| `op:Bind` / `op:Assign` / `binop:+` | 551,939 / 319,358 / 245,967 | ditto |
| Returns per function distribution | UNMEASURED | Phase 1 (attribution key + function) |
| Error-staging emit helpers census | UNMEASURED | Phase 1 grep `make_error_result` emitters |

### Verified properties

- Owned slots are zero-initialized at entry so cleanup null-guards are sound on
  paths that skipped an initializer — read `function_lowering.rs:1060-1085`
  (this is what makes one shared cleanup block correct for all paths).
- UNVERIFIED: whether cleanup sets at different RETURNs within one function
  are nested prefixes of one scope stack (required for a chained epilogue) or
  can diverge (parallel scopes with different live temps). Phase 1 reads
  `emit_return_exit_inner` + the scope machinery and answers this; the design
  below assumes chained scopes, which is how `active_cleanups` is maintained
  (a stack), but the answer gates Phase 2.

## 3. Design Overview

1. **Chained cleanup epilogue.** Per function, emit one cleanup block per
   scope depth at function end, each freeing its scope's slots then falling
   through to the enclosing scope's block, ending in the frame-exit sequence.
   A `RETURN` at depth d stages the return value in its ABI register(s) and
   jumps to block d. Because slots are zero-initialized and cleanup is
   null-guarded, one block per depth serves every path through that depth.
   `Fail`/trap-exit paths reuse the same chain with the error registers
   staged instead.
2. **Per-function error-staging blocks.** For each distinct (error constant,
   message symbol) used in a function, one block that loads the constants,
   calls `_mfb_make_error_result`, and routes to the trap label or the
   epilogue; sites stage only their loc (2 immediates) and jump. Sites keep
   per-site loc precision at ~5 instructions instead of ~45.

**Correctness risk (the family's largest):** exit-path rewrite of every
function; failure modes are leaks, double-frees, wrong cleanup order, or a
stale return register clobbered by cleanup code (cleanup calls `arena_free`,
which clobbers caller-saved registers — the staged return value must live in
a callee-saved register or be spilled across the chain; today's inline order
avoids this by construction, the shared version must prove it). This is
regalloc-adjacent: per the "Register/slot/import bugs need codegen-inspection"
lesson, acceptance is codegen-inspection tests plus rt fixtures, not black-box
green.

Byte-identity NOT the gate (every function's tail changes). Gates: full
behavior suites, leak fixtures, codegen-inspection tests, attribution deltas.

Rejected: out-of-line cleanup as a runtime call taking a descriptor (adds a
call + descriptor tables to every return; local jumps are free); sharing
error blocks module-wide (loses trap-label locality and inter-function jumps
don't exist in the code plan).

## Phases

### Phase 1 — re-census + feasibility read (no behavior change)

- [ ] Re-run the attribution over `tests/acceptance` post-D; update §2's table.
- [ ] Read `emit_return_exit_inner` + scope machinery; answer the
      nested-prefix question (§2 UNVERIFIED); record the answer and, if scopes
      can diverge, the revised block-per-scope-set design here.
- [ ] Census the error-staging emit helpers (all `make_error_result` emitters)
      and the per-function distinct-error-kind distribution.

Acceptance: this doc's §2 updated with post-D numbers and the verified scope
model; no source change beyond `-vv` keys (artifact-gate 0 diffs).
Commit: —

### Phase 2 — shared error-staging blocks (smaller blast radius first)

- [ ] Emit per-(kind) staging blocks at function end; rewrite fallible-op
      failure branches to stage loc + jump.
- [ ] Regenerate goldens; codegen-inspection test pinning one staged error
      path's register discipline.

Acceptance: residual error-staging categories drop ≥ 50 %; full suites green;
error-message/loc rt fixtures byte-identical output.
Commit: —

### Phase 3 — chained cleanup epilogue

- [ ] Restructure `emit_return_exit` to stage-and-jump; emit the per-depth
      chain + single frame exit at function end; `Fail`/trap exits routed
      through the same chain.
- [ ] Codegen-inspection test: staged return value survives the cleanup chain
      (callee-saved or spilled) on a function whose cleanup frees temps.
- [ ] Regenerate goldens; benchmark run (expected neutral: same dynamic
      instruction count on any single path).
- [ ] Doc sync: `planning/speed.md` closing note for recommendation 3 with the
      family's final numbers; spec architecture page on function lowering if
      it describes per-return cleanup.

Acceptance: `op:Return` attribution −≥ 60 % vs Phase 1's re-census; full
`cargo test --no-fail-fast`, `test-accept.sh` (full count), regenerated
`artifact-gate.sh all`; leak-sensitive and resource-close rt fixtures pass;
remote-box runtime proof (x86-64 + Windows) per `.ai/remote_systems.md` —
exit-path code is exactly where per-arch ABI differences bite
(`.ai/arch-abi.md` read before Phase 3).
Commit: —

## Validation Plan

- Tests: codegen-inspection tests for the two new shapes (note the
  "codegen-inspection tests hardcode drifting constants" lesson — assert
  structure, not absolute offsets); rt fixtures for multi-return functions
  with per-scope temps and resources; existing trap/error suites.
- Runtime proof: benchmark suite (neutral expected); the leak fixtures.
- Acceptance: family-standard gate set (plan-118-C Validation), plus
  `.ai/arch-abi.md` review before touching exit sequences per arch.

## Open Decisions

- Error-staging blocks per (kind) vs per (kind × trap-target) — decided by
  Phase 1's census of functions mixing trapped and untrapped fallible ops.

## Corrections

*(fill during execution)*

## Summary

The deepest cut and the only letter that touches every function's exit path —
scheduled last, split so the error-block half (smaller radius) lands and
soaks before the epilogue rewrite, with codegen-inspection tests carrying the
correctness burden that byte-diffs can't.
