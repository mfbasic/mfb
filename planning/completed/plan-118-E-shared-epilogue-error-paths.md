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
| plan-118-C and -D landed | their docs archived | MET (`planning/completed/plan-118-{C,D}-*`) |
| Post-C/D attribution re-measured | `-vv` "costliest expansion" over tests/acceptance, AFTER D | MET — Phase 1 below. **The re-read changed this letter completely**: `op:Return` fell from 2,007,382 to 134,581 before phase 2 even started, and the error block turned out to be 194 instructions rather than ~40. |

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

### Measured populations — **RE-CENSUSED post-D (Phase 1)**

| What | plan-118-A (pre-B) | post-D (this letter's baseline) |
|---|---|---|
| `op:Return` | 2,007,382 / 11,432 sites | **134,581 / 1,523 sites** |
| `op:Fail` | 255,872 / 3,609 | 255,872 / 3,609 |
| `op:Bind` | 551,939 / 12,407 | 552,683 / 12,407 |
| `op:Assign` | 319,358 / 4,540 | 319,358 / 4,540 |
| `binop:Add` | 245,967 / 1,828 | 245,967 / 1,828 |
| module total | 17,079,160 | 11,880,468 |

**`op:Return` collapsed by 93 % before this letter ran a line of code** — 9,909
of its 11,432 sites were the `RETURN` arms of the three generated Unicode
IF-chains plan-118-B deleted. §3's phase 3, the chained cleanup epilogue, was
sized against the old number; against the real one it is 1.1 % of the module.
This is exactly the re-read the Prerequisites row demanded.

The **error-staging census** (Phase 1's third task): 183 call sites reach the
error emitters, 171 of them through `raise_error_bare`, and they all funnel into
`emit_error_register_return`. Its cost is not the staging — the staging is ~8
instructions of immediates — but what follows: `emit_park_error_block_from_registers`,
which builds the owned `Error` block and parks it. Measured on
`FUNC cat2(a, b) RETURN a & b` (`--ncode`), a fallible site is:

```
  8  stage code / line / column / message / filename
  1  bl _mfb_make_error_result
174  build the owned Error block and park it        <-- the category
 11  route (return, or branch to the trap label)
```

Per-function distinct-error-kind distribution: not needed, and Open Decisions
records why — the shape that repeats is not keyed by error kind at all.

### Verified properties

- Owned slots are zero-initialized at entry so cleanup null-guards are sound on
  paths that skipped an initializer — read `function_lowering.rs:1060-1085`
  (this is what makes one shared cleanup block correct for all paths).
- **ANSWERED (Phase 1): the cleanup sets DIVERGE, so a per-depth chained
  epilogue is not sound.** `active_cleanups` is a stack, so the *scope* part
  nests as the design assumed — but a `RETURN` does not emit the stack. It emits
  the stack MINUS a set of per-return deactivations, each keyed on what is being
  returned:
  * `plan_returned_move` (`emit_return_exit:299`) removes the returned local's
    own drop for that path only, then restores it for sibling returns;
  * `deactivate_thread_cleanup`, `deactivate_resource_cleanup` (twice — plain
    resource and resource union, bug-141) and `deactivate_owned_list` each
    remove a cleanup when the returned value transfers that ownership;
  * and `escaping_value_slot` adds a *runtime* skip inside the sequence for the
    block that is escaping.

  Two `RETURN`s at the same depth therefore emit different cleanup sets whenever
  they return different locals — which is the normal case. A block keyed on
  depth alone would free a block the caller now owns (a use-after-free) or leak
  one it does not. Keying on (depth × deactivation-set) degenerates to roughly
  one block per return, i.e. no sharing.

  §3's design is corrected accordingly (Corrections 2); what phase 3 shares
  instead needs no scope reasoning at all.

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

- [x] Re-run the attribution over `tests/acceptance` post-D; update §2's table.
      — and it moved the letter: `op:Return` 2,007,382 → **134,581**.
- [x] Read `emit_return_exit_inner` + scope machinery; answer the
      nested-prefix question (§2 UNVERIFIED); record the answer and, if scopes
      can diverge, the revised block-per-scope-set design here. — **they
      diverge**; see §2 Verified properties and Corrections 2 for the revised
      design.
- [x] Census the error-staging emit helpers (all `make_error_result` emitters)
      and the per-function distinct-error-kind distribution. — 183 sites, 171 via
      `raise_error_bare`, all funnelling into `emit_error_register_return`; the
      cost is the 174-instruction park that follows the staging, not the staging
      itself. The error-kind distribution is moot (Open Decisions): the
      repeating shape is not keyed by kind.

Acceptance: this doc's §2 updated with post-D numbers and the verified scope
model; no source change beyond `-vv` keys (artifact-gate 0 diffs).

MET — §2 rewritten with the post-D numbers and the answered scope question; this
phase changed no source at all.
Commit: 0e5bd7cac (all three phases landed together: phase 1 is measurement, and
phases 2 and 3 share one mechanism, one golden regeneration and one test file)

### Phase 2 — shared error-staging blocks (smaller blast radius first)

- [x] ~~Emit per-(kind) staging blocks at function end; rewrite fallible-op
      failure branches to stage loc + jump.~~ — **corrected** (Corrections 1):
      what repeats is not the staging and is not keyed by kind. The staging is
      ~8 instructions of per-site immediates and must stay per-site (the
      `ErrorLoc` is the point). What repeats is the **174-instruction park**
      after it, which closes over nothing — its inputs are three fixed
      registers, its scratch is its own frame, and its one side effect is a
      store through the per-thread, callee-saved `ARENA_STATE_REGISTER`. So it
      is ONE `_mfb_rt_park_error` per module, not a block per function: smaller
      blast radius, smaller module, and no scope reasoning.
- [x] Regenerate goldens; codegen-inspection test pinning one staged error
      path's register discipline. — `tests/codegen_shared_cleanup_helpers.rs`
      asserts nothing is emitted between `bl _mfb_make_error_result` and
      `bl _mfb_rt_park_error`, which is precisely the register discipline: the
      three loose error registers the first call lands must reach the second
      untouched.

Acceptance: residual error-staging categories drop ≥ 50 %; full suites green;
error-message/loc rt fixtures byte-identical output.

MET. Against Phase 1's post-D re-census:

| category | post-D | after E | Δ |
|---|---|---|---|
| `op:Bind` | 552,683 | 197,677 | −64.2 % |
| `op:Assign` | 319,358 | 134,777 | −57.8 % |
| `binop:Add` | 245,967 | 28,988 | −88.2 % |
| `op:Fail` | 255,872 | 183,516 | −28.3 % |
| **combined** | **1,373,880** | **544,958** | **−60.3 %** |

`op:Fail` moves least, and correctly so: an explicit `FAIL` *is* the per-site
staging (its code, message and loc are the program's own data), so what is left
under that row is the part that cannot be shared.

Error text, codes and locations are unchanged everywhere:
`scripts/test-accept.sh`'s only mismatches across the whole change were three
`.ncode` dumps — **no `.run`, no `build.log`** — and the
`toString_invalid_encoding` fixture still reports `Error: 7-702-0004` /
"Text encoding or decoding failed." / exit 255, on the host and on both remote
boxes.
Commit: 0e5bd7cac

### Phase 3 — chained cleanup epilogue

- [x] ~~Restructure `emit_return_exit` to stage-and-jump; emit the per-depth
      chain + single frame exit at function end; `Fail`/trap exits routed
      through the same chain.~~ — **corrected** (Corrections 2): Phase 1 proved
      a per-depth chain unsound, because a `RETURN`'s cleanup set is the scope
      stack MINUS per-return ownership-transfer deactivations. What is shared
      instead is the individual drop, which closes over no scope at all:
      `_mfb_rt_drop_owned_string` and `_mfb_rt_drop_owned_collection` take the
      slot ADDRESS and do the null-guard, the size computation, the
      `arena_free` and the free-and-null themselves. Eleven instructions become
      two (String) and twelve become four (collection), at every exit of every
      scope holding one — not only at `RETURN`.
- [x] Codegen-inspection test: staged return value survives the cleanup chain
      (callee-saved or spilled) on a function whose cleanup frees temps. —
      `tests/codegen_shared_cleanup_helpers.rs`. With no chain there is no
      staged value to survive; what the test pins instead is the pair of
      guarantees that DID move into the helpers — free-and-null (bug-440) and
      the null guard — plus the error registers' flow across the park call.
      Without it the two commonest cleanup shapes would have lost their only
      check, since `codegen_owned_drop_free_and_null.rs` can no longer see them
      (they emit no `owned_value_free_skip` label at the site).
- [x] Regenerate goldens; benchmark run (expected neutral: same dynamic
      instruction count on any single path).
- [x] Doc sync: `planning/speed.md` closing note for recommendation 3 with the
      family's final numbers; spec architecture page on function lowering if
      it describes per-return cleanup. — no spec page describes per-return
      cleanup (same census plan-118-C ran: the internal `bl` helper family is
      not a documented surface).

Acceptance: `op:Return` attribution −≥ 60 % vs Phase 1's re-census; full
`cargo test --no-fail-fast`, `test-accept.sh` (full count), regenerated
`artifact-gate.sh all`; leak-sensitive and resource-close rt fixtures pass;
remote-box runtime proof (x86-64 + Windows) per `.ai/remote_systems.md` —
exit-path code is exactly where per-arch ABI differences bite
(`.ai/arch-abi.md` read before Phase 3).

MET. `op:Return` 134,581 → **49,226**, **−63.4 %** vs Phase 1's re-census.
Module 11,880,468 → **3,348,186**.

Benchmark: a 400 k-iteration loop owning a `String` and a `List` per iteration
plus 400 k trapped `FAIL`s — i.e. maximally scope-drop- and error-heavy —
compiled by the pre-E compiler (`f9ced6a1f`) and by this one, interleaved 7×:
min 0.227 s → 0.228 s (+0.5 %), median 0.231 s → 0.229 s (−1.0 %). Neutral, as
predicted: the same work, reached by a call.

Remote-box proof, output compared byte-for-byte against the host goldens —
`arena/flat-record-string` (String + record scope drops) and
`rt-error/general/toString_invalid_encoding` (the error path end to end, code
`7-702-0004`, message, exit 255): **identical on 2228 (x86-64 glibc) and 2229
(riscv64 musl)**. 2230 (Win11) is down — `Connection refused`, retried across
the whole letter — so Windows is covered by cross-compilation and its
regenerated `.ncodesum` goldens only.
Commit: 0e5bd7cac

## Validation Plan

- Tests: codegen-inspection tests for the two new shapes (note the
  "codegen-inspection tests hardcode drifting constants" lesson — assert
  structure, not absolute offsets); rt fixtures for multi-return functions
  with per-scope temps and resources; existing trap/error suites.
- Runtime proof: benchmark suite (neutral expected); the leak fixtures.
- Acceptance: family-standard gate set (plan-118-C Validation), plus
  `.ai/arch-abi.md` review before touching exit sequences per arch.

## Open Decisions

- ~~Error-staging blocks per (kind) vs per (kind × trap-target)~~ — **the
  question dissolved.** It presumes the repeating shape is the staging and that
  it must be keyed by error kind and exit destination. Neither holds
  (Corrections 1): the staging is per-site data that stays per-site, and the
  174-instruction park after it is keyed by *nothing* — so it is one function
  per module, and there is no kind or trap-target key to choose between.

## Corrections

1. **The repeating error shape is the PARK, not the staging, and it is not keyed
   by error kind.** §3's phase 2 designs "one error-staging block per
   (error-kind) per function", sites staging only their loc and jumping. The
   measurement says otherwise: staging is ~8 instructions of per-site
   immediates — and per-site is where it must stay, since the `ErrorLoc` is the
   whole point — while what follows it is **174 instructions** that vary by
   nothing at all: build the owned `Error` block, park it in the arena's
   current-error slot, restore the three loose registers, stamp the tag.

   Because it varies by nothing, it does not need to be per-function either. It
   closes over no scope: three fixed input registers, its own frame for scratch,
   and one store through `ARENA_STATE_REGISTER`, which is per-thread and
   callee-saved. So it is **one `_mfb_rt_park_error` per module**, and a
   fallible site is `bl _mfb_make_error_result` + `bl _mfb_rt_park_error`.

   §2's "~40–56-instruction error-construction block" was low by ~4×; the
   original attribution charged most of it to the enclosing category
   (`binop:Concat`, `val:Constructor`, …) rather than to `op:Fail`, which is why
   the earlier letters kept bottoming out on it.

2. **A per-scope-depth chained cleanup epilogue is unsound; the sharable unit is
   the individual drop.** §2 flagged as UNVERIFIED whether cleanup sets at
   different `RETURN`s nest. Phase 1 answered: **they do not.** `active_cleanups`
   is a stack, but a `RETURN` emits that stack MINUS deactivations chosen by
   what is being returned — `plan_returned_move`, `deactivate_thread_cleanup`,
   `deactivate_resource_cleanup` (plain and union, bug-141), `deactivate_owned_list`,
   and a runtime `escaping_value_slot` skip. Two returns at the same depth
   returning different locals emit different sets, so a depth-keyed block would
   free a block the caller now owns or leak one it does not. Keying on
   (depth × deactivation-set) is one block per return — no sharing.

   The corrected unit is the drop itself, which depends only on the slot address
   and the value's type: `_mfb_rt_drop_owned_string(slot)` and
   `_mfb_rt_drop_owned_collection(slot, stride, hasBuckets)`. Eleven and twelve
   instructions become two and four, at **every** scope exit rather than only at
   `RETURN` — which is why the win shows up under `op:Fail`, `op:Assign` and
   `op:If` as well as `op:Return`.

3. **`op:Return` was 93 % smaller than §2 said before this letter began.**
   9,909 of its 11,432 sites were the `RETURN` arms of the three generated
   Unicode IF-chains plan-118-B deleted, so the post-D baseline is 134,581 over
   1,523 sites, not 2,007,382 over 11,432. This is the re-census the
   Prerequisites row existed to force, and it is why phase 3 became a
   small, sound sharing rather than the "largest blast radius in the family"
   rewrite of every function's exit path.

4. **The `.ai/codegen-invariants.md` lore this letter cites is now enforced in a
   second place.** "Owned-value drops must free-and-null the cleanup slot"
   (bug-440) was pinned by `codegen_owned_drop_free_and_null.rs`, which finds
   `owned_value_free_skip` labels. The `String` and collection drops no longer
   emit that label at the site, so that test can no longer see the two
   commonest cleanup shapes; `tests/codegen_shared_cleanup_helpers.rs` pins the
   same guarantee inside the helpers.

## Merge-back gate (2026-09-02)

`main` advanced during the family (bug-467, bug-477, plan-119). Merged before
landing; the merge conflicted on **116 files, every one a `.ncodesum`** — both
sides changed backend codegen, so neither side's value is right and the correct
one is a regeneration from the merged tree (`.ncodesum` is a drift sentinel, per
`AGENTS.md`). No source file conflicted.

Re-gated on the merged tree:

- `scripts/artifact-gate.sh all` — 1327 tests, 1828 goldens, **0 diffs**
- `scripts/test-accept.sh` — **1348 test(s) ran**, passed
- `rustup run 1.96.0 cargo test --no-fail-fast` — **95 suites, 0 failures**
- acceptance suite — 732 / 732
- `mfb test -vv` — 3,348,186 machine instructions, 32,733 recursive NIR ops

## Summary

The deepest cut and the only letter that touches every function's exit path —
scheduled last, split so the error-block half (smaller radius) lands and
soaks before the epilogue rewrite, with codegen-inspection tests carrying the
correctness burden that byte-diffs can't.
