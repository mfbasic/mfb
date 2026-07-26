# bug-388: `*_codegen_cover_rt` / `crypto-ec-valid` byte-identity goldens reported as run-to-run flaky in the artifact gate

Last updated: 2026-07-25
Effort: large (3h–1d) — Phase 1 (confirmation) is small; Phase 2 (root-cause + fix) is unbounded until Phase 1 measures how many sources remain.
Severity: MEDIUM
Class: Correctness (build reproducibility) / Footgun (a permanently-noisy gate masks real regressions)
Status: Open
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

<!-- When the fix fully lands:
       ## STATUS: FIXED (<commit hash>)
     then archive to bugs/completed-bugs/. -->

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

## Root Cause

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

- [ ] Build a determinism harness: for each in-scope fixture, for N=50
      iterations, invoke a **fresh** `mfb build -ncode` (host target;
      `macos-aarch64` locally) and collect the sha256 of the emitted `.ncode`.
      Record `|unique hashes|` per fixture.
      - `|unique| > 1` ⇒ **flaky** (residual nondeterminism — hypothesis 1/3).
      - `|unique| == 1` but ≠ golden ⇒ **stable, stale golden** (hypothesis 2 —
        NOT flaky; different fix).
      - `|unique| == 1` and == golden ⇒ **already clean** (memory stale; possibly
        already fixed by bug-01 and never re-baselined).
- [ ] Run `scripts/artifact-gate.sh target/debug/mfb` ≥5 times on a clean tree;
      record the diffing-fixture set each run. A shifting set corroborates
      "flaky"; a stable set corroborates "stale golden."
- [ ] For the three `linux-*` targets, run the same harness on box 2229 (the
      only box with a toolchain) — determinism can be arch-specific.
- [ ] Complete the blast-radius audit (grep of Hash* iteration under
      `src/target/**` reaching `emit_*`); write a verdict per site into this doc.
- [ ] Write the per-fixture classification table (flaky / stale-golden / clean)
      into this doc. **This table decides Phase 2's shape.**

Acceptance: every in-scope fixture has a measured classification with the N=50
hash-count behind it; the blast-radius audit is complete with a verdict per site.

Commit: —

### Phase 2 — resolve + regenerate goldens + full validation

Branch on Phase 1's classification, per fixture:

- [ ] **Flaky fixtures:** fix the residual nondeterminism source(s) found in the
      audit by pinning a canonical order at the source (bug-01 pattern). Apply the
      same fix to every in-scope sibling site.
- [ ] **Stale-golden fixtures (stable but ≠ golden):** regenerate the golden from
      the now-proven-deterministic build and confirm the delta is only the
      intended change. Do this ONLY after the harness shows `|unique| == 1`.
- [ ] **Already-clean fixtures:** no code change; they simply need the gate's
      "known set" carve-out removed.
- [ ] Regenerate any shifted `.ncodesum` goldens for all four targets (macOS
      locally; the three `linux-*` on box 2229 with the release binary + `JOBS=10`
      per `linux-boxes-have-no-rust-toolchain`). Diff and confirm the delta is
      exactly the intended normalization.
- [ ] Delete the "known flaky set" carve-out from `scripts/artifact-gate.sh` so
      its contract is literal `diffs=0`.
- [ ] `scripts/artifact-gate.sh target/debug/mfb` → **`diffs=0`** on a clean tree.
- [ ] Run the full `scripts/test-accept.sh` once at the end (runtime-affecting?
      no — but confirm the byte-identity + rt-behavior suites are green).

Acceptance: gate reports `diffs=0`; golden deltas are exactly the intended
normalization; the "known noise" contract is deleted from the gate; nothing in
Non-goals changed.

Commit: —

### Phase 3 — finish gate: re-run Phase 1's determinism harness

The proof the fix actually removed the nondeterminism (not just froze one seed's
output). This re-runs Phase 1's measurement *after* the fix; it is the closing
gate, and it must be green with **zero further code or golden changes** — if any
fixture still shows `|unique| > 1`, Phase 2 is not done, return to it.

- [ ] Re-run the N=50 per-process determinism harness on **every** in-scope
      fixture (not just the ones classified flaky in Phase 1) — a fix can perturb
      emission order elsewhere. Confirm `|unique| == 1` per fixture per target.
- [ ] Confirm each fixture's single hash **equals its (possibly regenerated)
      golden**, on every target (`macos-aarch64` locally; the three `linux-*` on
      box 2229).
- [ ] Run `scripts/artifact-gate.sh target/debug/mfb` ≥5× on a clean tree and
      confirm `diffs=0` on **every** run (the run-to-run "set shift" symptom is
      gone).

Acceptance: N=50 harness shows exactly one hash per fixture per target, matching
the golden, and the gate is `diffs=0` across ≥5 repeats — with no code/golden
change made during this phase.

Commit: —

### Phase 4 — prune stale memory

Only after Phase 3 is green: the memory notes asserting a "known flaky set"
baseline are now false and must be corrected or removed so they stop justifying a
noisy gate.

- [ ] Re-read each note and decide, from the Phase 1/3 evidence, whether it is
      now wholly invalid (delete the file + its `MEMORY.md` line) or partly stale
      (edit to the new `diffs=0` reality):
      - `union-drop-codegen-nondeterminism` — its root cause (bug-01) is already
        fixed; if Phase 1 found no residual source, this note is **wholly stale**
        (delete). If a *second* source was found and fixed here, rewrite it to
        point at bug-388's cause instead.
      - `known-red-test-baseline` — remove the "artifact-gate is NOT `diffs=0` …
        7–9 flaky `audio`/`net`/`tls` `codegen_cover_rt` diffs" paragraph; the
        gate is now `diffs=0`.
      - `fast-codegen-gate` — remove the "17 pre-existing flaky
        `*_codegen_cover_rt`/`crypto-ec-valid` `.ncode` sha256s — baseline is NOT
        `diffs=0`" claim (near its counter-parity note).
      - Sweep `MEMORY.md` and the `memory/` dir for any other note repeating the
        "known flaky noise" / "baseline is not `diffs=0`" framing.
- [ ] Update each note's `MEMORY.md` index line and `modified` frontmatter; delete
      the index line for any file removed.
- [ ] Add a short `reference`/`project` note (or fold into `bug-01`'s lineage)
      recording that bug-388 closed the residual codegen determinism and the gate
      baseline is now `diffs=0`, so a future diff is a real signal.

Acceptance: no memory note still asserts a flaky `codegen_cover_rt` /
`crypto-ec-valid` baseline; `MEMORY.md` matches the files on disk; the new state
(`diffs=0`) is recorded once.

Commit: —

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
