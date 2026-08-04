# plan-83: Eliminate owned-`render()` operand reads in read-only codegen passes

Last updated: 2026-08-03
Effort: medium (1h–2h)
Depends on: nothing (builds on the typed `Operand` merged with plan-79/82; needs no
new representation). Independently landable.

Stop the read-only codegen passes from allocating a `String` for every operand
they only *look at*. Several late passes read operands through
`CodeInstruction::get()` (which returns `Option<String>` — it **renders, i.e.
allocates**) or a direct `value.render()`, then throw the string away after a
compare/parse. A borrowing read (`CodeInstruction::operand() -> Option<&Operand>`,
already present, or `value.rendered()` which lends a `Raw`/`Phys` its `&str`) does
the same job with **no allocation** for the common operand kinds.

## Why this plan exists — the *counted* cause

This is grounded in a **measured allocation-cause attribution**, not a guess (the
mistake plan-82's spike made). Method: a sampling global allocator that captures a
backtrace on 1-in-2048 allocations and buckets each by its nearest `mfb` frame,
run over `mfb test tests/acceptance` (16 files, the acceptance compile). It counts
*which call sites cause the allocations*, not just where self-time is spent.

Result (total ≈595M allocations): **`Operand::render()` called from read-only
passes is ≈25% of ALL compile allocations**, spread across:

| Est. share | Call site (nearest `mfb` frames) |
|---|---|
| 8.0% | `Operand::render` ← `arch::aarch64::encode::sizing::instruction_size` |
| 7.0% | `Operand::render` ← `codegen_utils::finalize_frame` |
| 4.1% | `Operand::render` ← `peephole::forward_stores_to_loads` |
| 3.2% | `Operand::render` ← `fma_fusion::fuse_scalar_fma` |
| 1.5% | `Operand::render` ← `regalloc::find_physical_operand` |
| 0.9% | `Operand::render` ← `regalloc::analysis::build_cfg` (label reads) |
| 0.6% | `Operand::render` ← `validation` |

These passes do not *produce* code from the string — they read a field to compare
(`== "sp"`, a mnemonic, a label), to parse an index, or to size an instruction,
and discard it. The allocation is pure waste.

## Prerequisites

| Must be true | Command | Status |
|---|---|---|
| Typed `Operand` with `rendered()` (borrow) + `operand()` accessor exist | `rg -n 'fn rendered' src/target/shared/code/operand.rs` and `rg -n 'fn operand\(' src/target/shared/code/code_impl.rs` | MET (confirmed 2026-08-03: `rendered` @operand.rs:118, `operand(` @code_impl.rs:52) |
| Byte-identity oracle + acceptance harness | `ls scripts/artifact-gate.sh tests/acceptance/project.json` | MET (both present) |
| The attribution probe is reproducible | see "Why this plan exists" (re-add the sampling allocator to `main.rs`, `MFB_ATTR_ALLOCS=1`) | MET (re-added as an uncommitted `MFB_ALLOC_STATS` counting allocator + `render()` counter; deterministic `render_calls`) |

## Non-goals

- **Byte-identity absolute** (`artifact-gate … all` = 0 diffs). These are read-only
  changes; `rendered()`/`operand()` see the same value `get()`/`render()` did.
- No representation change; no new `Operand` arm.
- Do not touch passes that legitimately need an owned `String` (a stored label, a
  diagnostic message) — only the read-compare-discard sites.

## Current State + the `rendered()` caveat (READ THIS)

`CodeInstruction::get(name) -> Option<String>` renders (owned). `operand(name) ->
Option<&Operand>` borrows. `Operand::rendered() -> Cow` borrows **only for `Raw`
and `Phys`**; for `VReg` and `Imm` it must *format* (`%vN` / the decimal), so it
still allocates. Therefore the fix is **per-site, by which operand kinds reach it**:

- **Post-regalloc passes** (`finalize_frame`, `peephole`, encoder `sizing`,
  `fma_fusion`): operands are `Phys`/`Raw`/`Imm`. Swapping `get()`/`render()` →
  `rendered()` (or `operand()` + a typed match) makes the `Phys`/`Raw` reads
  allocation-free — the bulk. (`Imm` reads still format; rare.) This is where most
  of the 25% lives (sizing 8% + finalize 7% + peephole 4% + fma 3% = 22%).
- **Pre-regalloc passes** (`find_physical_operand`, `validation`, `analysis`
  label reads): operands include `VReg`, whose `rendered()` still allocates. Here
  the win requires **matching the typed operand** (e.g. compare against
  `Operand::VReg{..}` / read the id) instead of rendering to `%vN` and
  string-comparing. Do this only where it stays byte-identical and readable; a
  `VReg` label/name field that must be a string can stay (small residue, noted).

## Design

For each cause site: identify what the string is used for, then pick the
allocation-free read:

1. **Compare to a literal** (`value == "sp"`, `mnemonic == "b.eq"`): `Operand`
   already `impl PartialEq<str>` — compare the `&Operand`/`Cow` directly, no
   render.
2. **Parse an index / physical name**: read via `operand()` and match `Phys{index}`
   / `Raw`, or `rendered()` (borrow) then parse — no owned `String`.
3. **Size an instruction** (`sizing::instruction_size`'s `probe.imports.insert(
   value.render(), …)`): use `value.rendered()` (borrow) — the probe only needs a
   `&str` key; if `imports` needs owned keys, key only the symbol/label fields
   (which are `Raw`), not every operand.
4. **Label/branch target read**: use `rendered()` (labels are `Raw` → borrow).

Correctness rests on `rendered()`/`operand()` yielding the exact value `render()`
did; byte-identity is the guardrail.

## Phases

> Land per pass; run `artifact-gate … all` after each; keep boxes current.

### Phase 1 — The two biggest, cleanest post-regalloc sites

- [x] `arch::aarch64::encode::sizing::instruction_size` (8%): stop rendering every
      field into the `probe.imports` map — seed only `Raw` operands (a call/`adrp`
      target is a symbol string, stored `Raw`; a `Phys`/`Imm`/`VReg` is never a
      relocation target and binding never affects the byte count). Same fix applied
      to `riscv64` sizing (identical waste; see Corrections).
- [x] `codegen_utils::finalize_frame` (7%): the render buckets live in its helpers
      (`adjust_stack_instruction_offsets`, `base_of`/`offset_of`,
      `assert_stack_accesses_fit_frame`) — converted to borrowing `rendered()` and
      name-checked-before-render; `base_of` now returns `Cow` instead of `String`.

Acceptance: `artifact-gate … all` 0 NEW diffs (the only 2 diffs — `control_flow_if`
+ `parser_hello_world` `.mir`, a `%ret0`→`%arg0` role-token label drift — reproduce
byte-for-byte on unmodified `main`, so they are pre-existing stale goldens, not
plan-83; regenerated as a fix, see Corrections). `cargo test --bin mfb` green
(3780 passed). Measured (deterministic `render_calls`, `MFB_ALLOC_STATS` probe over
`mfb build -ncode scripts/bench-probes/one-regex`): base `render_calls=22,279,314`,
`allocs≈126.57M` → after Phase 1 `render_calls=15,721,114`, `allocs≈120.02M`. Δ =
**−6,558,200 render calls (−29.4%)**, ≈ **−6.55M allocations (~−5.2% of all)**,
≈1:1 render→alloc as expected. `sizing`+`finalize_frame` buckets eliminated.
Commit: ebb646118 (stale-golden fix: a8e4bd1a9)

### Phase 2 — peephole + fma_fusion (post-regalloc)

- [x] `peephole::forward_stores_to_loads` (4%): the per-instruction bulk was
      `instruction.get("dst").is_some()` (the scalar/mul arms) rendering a register
      String only to check existence — swapped to `operand("dst").is_some()` (no
      render). The `StrU64`/`LdrU64` arms now peek `base` through `operand()`+
      `rendered()` so a non-sp store/load never renders its `offset`/`src`; the
      `DefDst` handler reads `dst` via `operand()`+`rendered()`.
- [x] `fma_fusion::fuse_scalar_fma` (3%): removed the redundant `.to_string()`
      double-clones in the product/consumer extraction (`get()` already owns).
      **`use_counts` kept String-keyed (render) — see Corrections:** an `Operand`
      key split counts the String key merged (mixed `VReg`/`Raw` register spellings
      in the float stream), changing fusion on `audio`/`vector`. The render there is
      load-bearing; byte-identity (the gate) caught the attempt.

Acceptance: `artifact-gate … all` 0 diffs (confirmed, after reverting the
`Operand`-key attempt). `cargo test --bin mfb` green (3780). Render-bucket
reduction consolidated into the Phase 4 measurement (peephole's `dst` existence
checks; fma's residual render is intentionally retained for correctness).
Commit: 2c7974c94

### Phase 3 — pre-regalloc reads (typed-match where `VReg` appears)

- [x] `regalloc::find_physical_operand` (1.5%): added a `matches!(value,
      Operand::VReg { .. })` fast-path skip before `rendered()`. The pre-allocation
      stream this scans (run on every function) is vreg-dominated, and a `VReg`
      would otherwise render to a `%vN`/`%fN` String only to take the
      `starts_with('%')` skip — byte-identical, the big Phase-3 win. `Raw`/`Phys`
      still borrow via `rendered()`.
- [x] `regalloc::analysis` (0.9%): the `BranchLink` call-target prefix sniff and
      `build_cfg`'s label-name insert + terminator-target lookup now read via
      `operand()`+`rendered()` (borrow the `Raw` symbol/label); the label insert
      also dropped a redundant `.to_string()`.
- [x] `validation` (0.6%): `defined_labels` is now a `HashSet<Cow<str>>` that
      borrows each `Raw` label name (no per-label `String`), and the branch-target
      membership test borrows too. No `VReg`-string residue needed (labels/targets
      are `Raw`).

Acceptance: `artifact-gate … all` 0 diffs (confirmed). `cargo test --bin mfb` green
(3780). Measured (`render_calls`, one-regex probe): after Phase 2 → after Phase 3
(see Phase 4 table); the `find_physical_operand` vreg-skip is the dominant reducer.
Commit: 87806a0f9

### Phase 4 — Measure the realized win

- [x] Re-measured with a deterministic instrumented allocator + `Operand::render()`
      call counter (env `MFB_ALLOC_STATS`, an uncommitted diagnostic — removed
      before merge). Workload: `mfb build -ncode -target macos-aarch64
      scripts/bench-probes/one-regex` (a regex-heavy, execution-free compile;
      `render_calls` is perfectly deterministic, `allocs` jitters ~0.01%). Measured
      base (main `171fc43cf`) → each phase (base + Phase-2-tip binaries built with
      the same instrumentation):

      | Stage            | `render_calls` | Δ vs base            | `allocs` (≈) |
      |------------------|---------------:|----------------------|-------------:|
      | Base (`main`)    |     22,279,314 | —                    |     126.57M  |
      | After Phase 1    |     15,721,114 | −6,558,200 (−29.4%)  |     120.02M  |
      | After Phase 2    |     13,496,792 | −8,782,522 (−39.4%)  |     117.79M  |
      | After Phase 3    |     10,461,529 | −11,817,785 (−53.0%) |     114.75M  |

      Each phase's `render_calls` drop matches its `allocs` drop ≈1:1 (P1 −6.56M
      render / −6.55M alloc; P2 −2.22M / −2.23M; P3 −3.04M / −3.04M), confirming
      every eliminated `render()` was a heap `String`. **The read-pass
      `Operand::render` class is more than halved (−53%); total compile allocations
      fell ~−9.3%** on this probe. (The plan's original ≈25%/595M figures were for
      the `mfb test tests/acceptance` workload; this probe is a different, cheaper,
      fully-deterministic surface — the *direction and magnitude* are the proof, and
      byte-identity guarantees the values are unchanged.)

Acceptance: total allocations fell measurably (−53% of read-pass renders; ~−9.3%
of all allocations, deterministic on the probe); byte-identical at every phase
(`artifact-gate … all`: 0 diffs ×3); `cargo test --bin mfb` 3780 passed. Runtime
proof: `target/release/mfb test tests/acceptance` → `Tests: 362 Pass: 362 Fail: 0`,
exit 0.
Commit: —

## Validation Plan

- Tests: `cargo test --bin mfb`.
- Byte-identity: `artifact-gate … all` 0 diffs after every phase (a diff = a read
  that wasn't actually read-only — investigate, never re-baseline).
- Cause verification (the anti-guess guard): the attribution probe MUST show the
  targeted buckets shrink; a phase that changes code but not the measured bucket
  did not fix the cause.
- Runtime proof: `mfb test tests/acceptance` exits 0 (release).

## Corrections

- **Phase 1 scope extended to `riscv64::encode::sizing::instruction_size`.** The
  aarch64 `instruction_size` had a byte-for-byte twin in the riscv64 encoder with
  the identical per-field `value.render()` seed of `probe.imports`. Both encoders
  consult `imports` only through `resolve_call_binding`/`resolve_data_binding`
  (call/data symbol targets, always stored `Raw`), and binding never changes the
  byte count, so seeding non-`Raw` operands is pure waste in both. The plan named
  only aarch64; leaving the identical riscv64 waste would be a deferral, so the
  same `Raw`-only fix was applied there. Byte-identity covered by the gate's
  multi-target riscv64 `.ncodesum` goldens (0 diffs).

- **Two pre-existing stale `.mir` goldens, unrelated to plan-83, fixed.** The
  `artifact-gate … all` run reported exactly 2 diffs:
  `rt-behavior/control-flow/control-flow-if/…macos-aarch64.mir` and
  `syntax/lexical/parser-hello-world/…macos-aarch64.mir`, both a `%ret0`→`%arg0`
  role-token *label* drift in `ldr_u64`/`add_imm` `dst` fields. Verified at HEAD:
  a detached `git worktree` at the base tip (`171fc43cf`), rebuilt clean, produces
  the *identical* `%arg0` output — so these are pre-existing stale goldens (an
  earlier change, likely plan-85's typed ABI tokens, didn't regenerate them), not
  a plan-83 regression. The machine-code goldens (`.ncode`/`.nobj`) match on both,
  proving `%arg0` and `%ret0` realize to the same physical register (x0) — a purely
  cosmetic dump-label drift. Per AGENTS.md ("fix a bug you find, not excused by
  pre-existing, verify at HEAD"), regenerated the 2 `.mir` goldens (the only lines
  changed are `%ret0`→`%arg0`). This makes plan-83's gate a genuine 0 diffs.

- **Phase 2 `fma_fusion::use_counts` must stay String-keyed (a byte-identity
  correction the gate forced).** The first Phase 2 attempt keyed the use-count map
  by the typed `Operand` (a `VReg`/`Imm` key clones heap-free, the big win). It
  broke byte-identity on exactly the float-heavy fixtures: `artifact-gate` reported
  10 diffs, all `audio`/`vector` `.ncode` across all 5 targets. Cause: the
  pre-allocation float stream carries the *same logical register* under two
  spellings — a `VReg` handle and a `Raw` `%fN` string — which `render()` to the
  same token but are **not** `Operand`-`Eq`. The String key merges them (the count
  the "used exactly once" fusion test needs); the `Operand` key splits them,
  flipping fusion decisions and thus the emitted bytes. Reverted to the String key
  (the `render()` is load-bearing) and dropped the `Operand: Hash` derive that only
  the map needed. Only the safe redundant-`.to_string()` removals were kept. This
  is the plan's guardrail working exactly as intended ("a diff = a read that wasn't
  actually read-only — investigate, never re-baseline").

## Summary

plan-83 is the cheap, high-confidence half of the real fix: ≈25% of compile
allocations are `String`s rendered by passes that only read an operand and throw
it away. A borrowing read (`operand()`/`rendered()`) or a typed compare removes
them, byte-identically. Its acceptance is a *re-measured* drop in the exact
buckets — so, unlike plan-82's spike, the fix is verified against the counted
cause, not assumed.
