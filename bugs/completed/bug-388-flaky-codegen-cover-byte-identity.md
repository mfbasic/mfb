# bug-388: `*_codegen_cover_rt` / `crypto-ec-valid` byte-identity goldens reported as run-to-run flaky in the artifact gate

Last updated: 2026-07-25
Effort: large (3h–1d) — Phase 1 (confirmation) is small; Phase 2 (root-cause + fix) is unbounded until Phase 1 measures how many sources remain.
Severity: MEDIUM
Class: Correctness (build reproducibility) / Footgun (a permanently-noisy gate masks real regressions)
Status: FIXED (fc5bcb1ec regen; eb7eac5d0 fix) — see STATUS block below
Regression Test: `tests/byte-identity/{audio,crypto,fs,net,os,tls}` + `tests/rt-behavior/crypto/crypto-ec-valid` (existing `.ncodesum` goldens); plus a new determinism harness (Phase 1).

`scripts/artifact-gate.sh` on a **clean tree** is believed to report a shifting set
of `.ncode`/`.ncodesum` diffs — reported historically as 7–17 diffs — all confined to
the byte-identity codegen-cover fixtures for the runtime-heavy backends
(`audio`, `crypto`, `fs`, `net`, `os`, `tls` `*_codegen_cover_rt`) plus
`crypto-ec-valid`. The distinguishing symptom of *flakiness* (as opposed to a
stale golden) is that **the exact subset that diffs varies run-to-run** for the
same source and the same compiler binary. This has been treated as "known noise"
and used to justify a gate pass criterion of "no diff appears OUTSIDE this known
set" rather than literal `diffs=0`.

**The single correct behavior a fix produces:** compiling any of these fixtures
twice from identical source with identical flags yields **byte-identical**
`.ncode`/`.ncodesum` every time, across independent `mfb` processes and on every
target. `scripts/artifact-gate.sh` then reports **`diffs=0` on a clean tree**,
so any future diff is a genuine signal.

**Why this is dangerous even though each diff is semantically valid:** a gate
whose baseline is "N flaky diffs, set shifts run-to-run" cannot cheaply
distinguish a real codegen regression in those seven backends from the accepted
noise. Reproducibility is also a correctness property in its own right (a build
should be a pure function of its input).

> **Read before starting — this is not a rubber-stamp.** The root cause
> historically blamed for this noise, `TypeModel::variants_for_union` iterating a
> `HashMap`, **was already fixed and archived as bug-01 on 2026-07-01**
> (commit `51fccea7a`, "fix(codegen): deterministic union variant order"). The
> memory notes asserting these fixtures are "known flaky noise" are dated
> **2026-07-25/26 — ~3.5 weeks after that fix landed**. So one of two things is
> true and Phase 1 must decide which by measurement, not memory:
>   1. the flakiness is **real and residual**, from a *different* nondeterminism
>      source that bug-01 did not cover; or
>   2. the flakiness is **already gone**, the "known noise" belief is stale, and
>      any remaining golden diffs are **stale goldens**, not run-to-run variance.
>
> These have opposite fixes. Do not assume (1).

## STATUS: FIXED (fc5bcb1ec)

**Phase 1 decided hypothesis 2 (stale goldens) for the seven in-scope fixtures —
AND the blast-radius audit found a genuine, previously-latent SECOND
nondeterminism source that no in-scope fixture triggered.** Both are resolved.

- **Determinism measured, not assumed.** New per-process harnesses
  (`scripts/ncode-determinism.sh` host, `scripts/ncode-determinism-alltargets.sh`
  cross-target; `-ncode` is execution-free so all four targets regenerate on the
  macOS host — box 2229 was NOT needed). Result: **N=50 host + N=20 × 4 targets,
  every in-scope fixture UNIQ==1 — ZERO FLAKY.** The "known flaky noise" belief
  was stale; the run-to-run "set shift" symptom did not reproduce.
- **The diffs were stale goldens.** 230 commits of legitimate, deterministic
  codegen change landed after the sums were last written (`54e5c62f4`,
  plan-45) — bug-387 macOS tokenization, plan-64 native collections, bug-333/334
  emit refactors — and were dismissed as "flaky noise", so the heavy `.ncodesum`
  files were never refreshed. Regenerated the 21 stale cells (commit
  `fc5bcb1ec`); `artifact-gate.sh` now reports **diffs=0** (3× repeated).
- **A real latent bug, fixed (commit `eb7eac5d0`):** `link_thunk.rs`
  `lower_link_thunk` materialized each CONST pin's immediate by iterating a
  `HashMap` (`const_for`), emitting a `move_immediate`/`store_u64` pair per pin in
  per-process hash order — the bug-01 class, a second site. Latent because no
  fixture had ≥2 CONST pins on one OUT-CBuffer function. Fixed by iterating the
  ordered `function.consts` Vec. New fixture `tests/byte-identity/link-const-pins`
  reproduces it: 11 distinct hashes / 30 processes pre-fix → 1 hash on all four
  targets post-fix.
- **Deviation from the doc's plan:** the doc weighted hypothesis 1 (a residual
  source in the in-scope fixtures). Reality was BOTH: the in-scope fixtures were
  stale-golden (hyp 2), and the residual `HashMap` source existed but on a code
  path (`≥2` CONST pins + CBuffer) that no in-scope fixture exercised — so it was
  fixed AND given its own fixture rather than folded into an existing one.

<!-- archived to bugs/completed-bugs/ on landing -->

References:

- bug-01 (`bugs/completed-bugs/bug-01-resource-union-drop.md`) — the first,
  already-fixed instance of exactly this class: `HashMap` iteration order leaking
  into emitted instruction order. The template for the Phase 2 fix (pin a
  canonical order; tags/layout untouched, only emitted order was ever affected).
- `src/target/shared/code/validation.rs:526` `variants_for_union` — now sorts by
  `union_variant_tags` then name (bug-01 fix, see the doc comment at
  `validation.rs:519`). Confirm no consumer reintroduced an unordered path.
- `scripts/artifact-gate.sh`, `scripts/test-accept.sh`, `scripts/exe-oracle.sh` —
  the three harnesses that consume the byte-identity goldens.
- Memory: `union-drop-codegen-nondeterminism`, `known-red-test-baseline`,
  `fast-codegen-gate` (all assert the "known flaky set" baseline — treat as the
  hypothesis under test, now partly disproven by the bug-01 timeline).
- The conversation in which this was raised: audio/crypto/net/tls
  `codegen_cover_rt` + `crypto-ec-valid` diffs, "not string fixtures."

## Failing Reproduction

Suspected — **Phase 1 exists to confirm or refute this**. The claim is:

```
# Same compiler, same source, two independent processes → different output.
git stash list   # confirm clean tree
scripts/artifact-gate.sh target/debug/mfb   # run once, note the diff set
scripts/artifact-gate.sh target/debug/mfb   # run again, note the diff set
# Claim: both runs report diffs, and the *set* of diffing fixtures differs
#        between the two runs (audio/crypto/fs/net/os/tls *_codegen_cover_rt,
#        crypto-ec-valid), with no source or binary change between them.
```

- Observed (claimed): a non-empty, run-to-run-varying subset of the seven
  fixtures reports an `.ncode`/`.ncodesum` sha256 mismatch on a clean tree.
- Expected: zero diffs, every run.

**Fixtures in scope (verified present at HEAD):**

| Fixture | Golden kind | Location |
| --- | --- | --- |
| `audio_codegen_cover_rt` | `.ncodesum` ×4 targets | `tests/byte-identity/audio/golden/` |
| `crypto_codegen_cover_rt` | `.ncodesum` ×4 | `tests/byte-identity/crypto/golden/` |
| `fs_codegen_cover_rt` | `.ncodesum` ×4 | `tests/byte-identity/fs/golden/` |
| `net_codegen_cover_rt` | `.ncodesum` ×4 | `tests/byte-identity/net/golden/` |
| `os_codegen_cover_rt` | `.ncodesum` ×4 | `tests/byte-identity/os/golden/` |
| `tls_codegen_cover_rt` | `.ncodesum` ×4 | `tests/byte-identity/tls/golden/` |
| `crypto-ec-valid` | `.ncodesum` ×4 | `tests/rt-behavior/crypto/crypto-ec-valid/golden/` |

Targets: `macos-aarch64`, `linux-aarch64`, `linux-x86_64`, `linux-riscv64`.
The macOS host can only re-derive the `macos-aarch64` sums locally; the three
`linux-*` sums regenerate only on box 2229 (see memory
`linux-boxes-have-no-rust-toolchain`).

## Root Cause — RESOLVED

**Two independent facts, decided by measurement (see STATUS block):**

1. **For the seven in-scope fixtures: stale goldens (hypothesis 2).** N=50 host +
   N=20 cross-target showed every fixture UNIQ==1 on every target — deterministic.
   The committed sums simply predated 230 commits of legitimate codegen change.
   No code fix; regenerate only (21 cells; `os` and `tls` linux-* were already
   current and untouched).
2. **A separate latent nondeterminism source (hypothesis 1, different site):**
   `src/target/shared/code/link_thunk.rs` `lower_link_thunk` (~line 764) iterated
   the CONST-pin set as a `HashMap` and EMITTED an instruction pair per pin, so
   emission order followed per-process hash order once a single OUT-CBuffer
   function carried ≥2 matching pins. Same class as bug-01; fixed by iterating the
   ordered `function.consts` Vec. Not reachable from any in-scope fixture (they
   have ≤1 CONST pin per CBuffer function), so it did NOT cause the reported
   flaky-gate history — but it was a real reproducibility bug and is fixed here.

### Phase 1 classification table (measured)

| Fixture | macos-aarch64 | linux-aarch64 | linux-x86_64 | linux-riscv64 |
| --- | --- | --- | --- | --- |
| audio_codegen_cover_rt | stale (regen) | stale | stale | stale |
| crypto_codegen_cover_rt | stale | stale | stale | stale |
| fs_codegen_cover_rt | stale | stale | stale | stale |
| net_codegen_cover_rt | stale | stale | stale | stale |
| os_codegen_cover_rt | clean | clean | clean | clean |
| tls_codegen_cover_rt | stale | clean | clean | clean |
| crypto-ec-valid | stale | stale | stale | stale |

Every cell UNIQ==1 (deterministic). "stale" = stable hash ≠ golden → regenerated;
"clean" = already matched → untouched. **No cell was FLAKY.**

### Blast-radius audit verdict (Hash* iteration reaching emission under src/target)

`variants_for_union` (validation.rs:530) — still sorted (bug-01), SAFE.
`emit_resource_union_cleanup_call` — consumes the ordered iterator, SAFE.
All other Hash* iteration (mod.rs string/close-op, function_lowering,
builder_registers, regalloc, mir by_symbol, plan/symbols, crypto imports/EC
tables) — sorts-before-emit, membership-only, or iterates a Vec, SAFE.
**Sole REACHES-EMISSION site besides the fixed `variants_for_union`:**
`link_thunk.rs:764` `const_for` — fixed here.

## Root Cause (original hypotheses at filing)

**Unknown at filing — this is the crux, not a formality.** The historically-cited
cause (`variants_for_union` HashMap order) is closed (bug-01, `51fccea7a`,
in HEAD's history). Ordered hypotheses for any *residual* nondeterminism:

1. **A second `HashMap`/`HashSet` iteration in the codegen path** — same class as
   bug-01, different site, reachable only from the resource-heavy backends
   (`audio`/`crypto`/`net`/`tls` all emit native-resource cleanup;
   `crypto-ec-valid` walks EC curve/parameter tables). Rust's `std` `HashMap`
   uses a per-process random seed, so such an order is stable *within* one `mfb`
   process but varies *across* processes — which is exactly the "set shifts
   run-to-run" signature. Confirm by the multi-process determinism harness
   (Phase 1) + `grep` audit of Hash* iteration under `src/target/**`.
2. **The flakiness is already resolved** and the remaining golden mismatches are
   **stale goldens** (output changed deterministically since the sums were last
   regenerated, e.g. across a codegen refactor). Confirm/eliminate: if the
   multi-process harness shows the same fixture produces one **stable** hash that
   simply differs from the golden, it is NOT flaky — it is a stale golden, and
   the fix is a proven regen, not a determinism hunt.
3. **Non-`HashMap` nondeterminism** — iteration over another unordered container
   (e.g. address-keyed maps, pointer-set ordering, or a parallel/`rayon`
   emission stage if any). Lower likelihood; fall back here only if (1)/(2) are
   ruled out.

Note: `crypto-ec-valid` sits in `rt-behavior/`, not `byte-identity/` — if it
flakes it may share cause (1) or have its own; do not assume one fix covers it.

## Goal

- Every in-scope fixture compiles to a **byte-identical** `.ncode`/`.ncodesum`
  across ≥50 independent `mfb` processes, per target.
- `scripts/artifact-gate.sh target/debug/mfb` reports **`diffs=0`** on a clean
  tree — the "known flaky set" carve-out is deleted from the gate's contract and
  from the three memory notes.

### Non-goals (must NOT change)

- **Codegen semantics, instruction selection, register allocation, union tags, or
  memory layout.** Like bug-01, only *emitted order* may change, and only where
  order was never semantically meaningful.
- **Do NOT loosen, delete, or `.gitignore` the byte-identity goldens, and do NOT
  make the gate tolerate a "known set."** Masking the noise instead of removing
  it is the tempting wrong fix and is explicitly forbidden — that path is what
  created the current permanently-yellow baseline.
- **Do NOT re-baseline a golden to whatever the current build emits** to make the
  diff go away. A golden may be regenerated only *after* the source is proven
  deterministic (Phase 1 harness green for that fixture); regenerating a still-
  flaky fixture just freezes one random seed's output.

## Blast Radius

To be completed in Phase 1 by an actual `grep` audit — not from memory. Starting
points (verified present):

- `src/target/shared/code/validation.rs:526` `variants_for_union` — **fixed**
  (bug-01); confirm still ordered and that no caller bypasses it.
- `src/target/shared/code/builder_resource_cleanup.rs:78`
  `emit_resource_union_cleanup_call` — the consumer bug-01 fixed; confirm it only
  reads the now-ordered iterator.
- All other Hash* iteration under `src/target/shared/code/**` reachable from
  native emission — audit `\.(keys|values|iter|into_iter|drain)\(\)` over
  `HashMap`/`HashSet` values whose iteration reaches an `emit_*`. Classify each:
  fixed here / latent-out-of-scope (with reason) / provably order-independent.
- EC-specific tables feeding `crypto-ec-valid` (curve/param lookup in the crypto
  backend) — audit separately; it is not a byte-identity fixture.

## Fix Design

Same shape as bug-01: locate each unordered iteration whose order reaches emitted
bytes and **pin a canonical order** at the source (declaration index, tag, or a
stable name sort), so codegen becomes a pure function of the program without
per-call-site churn. Layout/tags/semantics stay byte-for-byte; only emission
order is normalized. Rejected alternative: sorting the *output* bytes or
post-processing the dump — that hides the nondeterminism rather than removing it
and breaks the moment a real diff needs to be seen. Expected golden shift: the
`.ncodesum` for a fixed target should land on **one** value and stay there; if a
fixture's sum changes as a result, that is the intended and only delta.

## Phases

### Phase 1 — confirm flaky-or-not (measurement, no behavior change)

The whole point: replace the "known noise" belief with a measurement, per
fixture. Determinism is a **per-process** property (std HashMap seeds per
process), so the harness MUST spawn a fresh `mfb` process each iteration — an
in-process loop shares one seed and would falsely show stability.

- [x] Determinism harness built (`scripts/ncode-determinism.sh` host,
      `scripts/ncode-determinism-alltargets.sh` cross-target). N=50 host: every
      in-scope fixture UNIQ==1. Result: 5 stale-golden, `os` clean (see table).
- [x] `scripts/artifact-gate.sh` run 3× — the diffing set was STABLE (same cells
      every run), corroborating "stale golden", not "flaky".
- [x] `linux-*` targets: NO box 2229 needed — `-ncode` is execution-free, so all
      four targets cross-build on the macOS host. N=20 × 4 targets: all UNIQ==1.
- [x] Blast-radius audit complete — verdict per site above. One residual
      REACHES-EMISSION site (`link_thunk.rs:764`) found and fixed.
- [x] Per-fixture classification table written (above).

Acceptance: MET — every fixture has a measured classification (N=50 host + N=20 ×
4 targets); audit complete with a per-site verdict.

Commit: 5a3408e54 (harnesses)

### Phase 2 — resolve + regenerate goldens + full validation

Branch on Phase 1's classification, per fixture:

- [x] **Latent flaky site (not an in-scope fixture):** `link_thunk.rs:764` fixed
      by iterating `function.consts` (Vec) instead of the `HashMap`. New fixture
      `tests/byte-identity/link-const-pins` pins it (RED 11-hashes/30-proc →
      GREEN 1-hash all targets). Commit `eb7eac5d0`.
- [x] **Stale-golden fixtures:** regenerated the 21 stale cells from
      proven-deterministic builds; deltas are the intended hash shifts only.
      Commit `fc5bcb1ec`.
- [x] **Already-clean fixtures:** `os` (all 4) and `tls` linux-* untouched.
- [x] Regenerated `.ncodesum` for all four targets locally (no box 2229; `-ncode`
      is execution-free — each new sum equals the harness-measured deterministic
      hash for that cell).
- [x] Gate carve-out: `scripts/artifact-gate.sh` ALREADY enforced literal
      `diffs=0` (no code carve-out existed — the "known set" lived only in memory
      notes, corrected in Phase 4). No script change needed.
- [x] `scripts/artifact-gate.sh target/debug/mfb` → **`diffs=0`** (3× repeated).
- [x] `cargo test` full: 0 failed. `scripts/test-accept.sh` affected subset
      (`*native* *crypto* *link-const-pins*`): 80 tests pass; full run confirmed.

Acceptance: MET — gate `diffs=0`; deltas are exactly the intended normalization
plus the new fixture; no Non-goal changed (semantics/tags/layout untouched, only
emission order for ≥2-pin CBuffer thunks).

Commit: eb7eac5d0 (fix + fixture), fc5bcb1ec (golden regen)

### Phase 3 — finish gate: re-run Phase 1's determinism harness

The proof the fix actually removed the nondeterminism (not just froze one seed's
output). This re-runs Phase 1's measurement *after* the fix; it is the closing
gate, and it must be green with **zero further code or golden changes** — if any
fixture still shows `|unique| > 1`, Phase 2 is not done, return to it.

- [x] Re-ran the cross-target determinism harness on all seven in-scope fixtures
      post-fix: UNIQ==1 per fixture per target (N=20 × 4). New fixture
      link-const-pins: UNIQ==1 × 4 (N=30). Zero FLAKY.
- [x] Each fixture's single hash equals its (regenerated or already-current)
      golden on every target — verified by `artifact-gate.sh` diffs=0.
- [x] `artifact-gate.sh` `diffs=0` on 3 repeated runs; the per-process harness
      (N=20–50 fresh processes/fixture) is a stronger proof than 5 single-sample
      gate runs and subsumes it.

Acceptance: MET — one hash per fixture per target, matching the golden; gate
`diffs=0` repeated; the fix landed in Phase 2 so no code/golden changed here.

Commit: — (verification only)

### Phase 4 — prune stale memory

Only after Phase 3 is green: the memory notes asserting a "known flaky set"
baseline are now false and must be corrected or removed so they stop justifying a
noisy gate.

- [x] Corrected each note (rewrote, did not delete — the union-drop lineage is
      still useful history and a 2nd source WAS found):
      - `union-drop-codegen-nondeterminism` — rewrote: bug-01 fixed
        `variants_for_union`; bug-388 measured the fixtures deterministic (stale
        goldens) and fixed the 2nd `link_thunk` site. Marked RESOLVED.
      - `known-red-test-baseline` — corrected the "artifact-gate is NOT diffs=0"
        paragraph to "IS diffs=0".
      - `fast-codegen-gate` — corrected the "17 flaky … baseline NOT diffs=0"
        counter-parity claim AND the ".ncodesum goldens are DEAD" section (the
        byte-identity `.ncodesum` ARE read by the gate — verified by corrupting
        one → `DIFF (sha256)`).
      - Swept the rest: also corrected `bugs-333-344-are-a-refactoring-backlog`
        ("9 flaky") and `plan-64-benchmark-perf-progress` ("17 flaky").
- [x] Updated `MEMORY.md` index lines (union-drop, fast-codegen-gate,
      known-red-test-baseline) + `modified` frontmatter on each edited note.
- [x] Added `bug-388-codegen-determinism-resolved` note (+ its `MEMORY.md` line)
      recording the diffs=0 baseline and the load-bearing mechanics.

Acceptance: MET — no memory note asserts a flaky `codegen_cover_rt` /
`crypto-ec-valid` baseline; `MEMORY.md` matches disk; diffs=0 recorded once.

Commit: — (memory lives outside the repo)

## Validation Plan

- Regression test(s): the N=50 per-process determinism harness (new; keep it as a
  script under `scripts/`), plus the existing byte-identity `.ncodesum` goldens
  now enforced at `diffs=0`.
- Runtime proof: not a runtime bug — the proof is reproducibility (identical hash
  across independent processes and across the target matrix).
- Doc sync: update the three memory notes; no `src/docs/spec` change expected
  (determinism, not semantics) — confirm bug-01's "no spec change" still holds.
- Full suite: `scripts/artifact-gate.sh` (`diffs=0`, repeated) then one
  `scripts/test-accept.sh`; `linux-*` sums via box 2229.

## Open Decisions

- N for the determinism harness — **50 fresh processes** (recommended; cheap,
  `-ncode` is execution-free) vs. a larger N if 50 proves too few to surface a
  low-probability seed collision. Raise only if Phase 1 shows borderline behavior.
- Whether `crypto-ec-valid` (an `rt-behavior` fixture, not `byte-identity`) is
  folded into this bug or split out — keep it here unless Phase 1 shows an
  unrelated cause.

## Summary

The real engineering risk is entirely in **Phase 1's measurement**: the root
cause everyone "knows" (`variants_for_union` HashMap order) was already fixed as
bug-01 on 2026-07-01, three-plus weeks before the "known flaky noise" belief was
last written down — so the belief may describe a residual *second* nondeterminism
source, or may be stale and the diffs are merely unrefreshed goldens. Those have
opposite fixes, and the N=50 multi-process harness is what tells them apart per
fixture. Codegen semantics, tags, and layout are untouched throughout; only
emitted order (if anything) is normalized, and no golden is re-baselined until its
fixture is proven deterministic.
