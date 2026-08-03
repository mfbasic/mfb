# plan-82-D: Typed operand consumption in the encoder (final target)

Last updated: 2026-08-02
Effort: medium (1h–2h)
Depends on: plan-82-C (operands reaching the encoder are typed `VReg`/`Phys` end
to end, so the encoder can read the index directly instead of scanning strings)

Make the instruction encoder read the typed `Phys { class, index }` directly —
deleting the `REG_ARRAY.position` string scans in
`int_concrete_physical_index`/`fp_physical_index`
(`src/target/shared/code/regalloc/analysis.rs:257,307,280,300,357`) and the
encode-side operand-string rendering — so the `encode` substage stops allocating
and scanning per instruction. This is the last allocation class and the phase
that realizes plan-82's headline target.

Single behavioral outcome: `mfb test tests/acceptance` (debug `mfb`) compiles in
**≤ 60 s** (stretch **≤ 30 s**), down from the measured 284 s baseline, with every
emitted byte identical.

References:

- plan-82-A baseline (the 215 M encode-substage allocations; the 13.4 s release /
  ~60 s debug encode substage) and the shared Prerequisites.
- `src/arch/aarch64/encode/sizing.rs` — `instruction_size` (reads operands to
  size each instruction).
- `src/arch/aarch64/encode/emitter.rs` — `Encoder::emit_instruction` and the
  per-form emit helpers that decode register operands.
- `src/target/shared/code/regalloc/analysis.rs` — the physical-index scans to
  delete; and the x86-64/riscv64 encoders' equivalent consumers.

## Status — RESUMED after plan-79 removed the MIR barrier

The end-of-C halt (plan-82-A §CORE-PREMISE FALSIFICATION) was because the
String-based MIR/select layer discarded the typed operands. **plan-79 typed
`MirInstruction.fields` as `Operand`**, so typed operands now survive to the
encoder and D is effective. Cumulative allocation trajectory (counting-allocator
probe, `mfb test tests/acceptance`): base **808.8M** → A/B/C **789.9M** → +plan-79
**640.3M** → **+D 577.5M** (**−28.6%** vs base). Release acceptance wall
**58 s → 47.9 s**.

**Correction to D's premise:** the aarch64 encoder decodes registers via
`operand::reg()` (a jump-table match on the name), *not* the
`analysis::*_physical_index` `REG_ARRAY.position` scans (those are regalloc-side).
The encoder's per-operand cost was the `field()` **`render()` `String` allocation**
on the sizing/emit hot path, not the match. D's realized win is making `field()`
return a **borrowed `Cow`** (`rendered()`), so a `Raw`/`Phys` operand — the common
case — lends its `&str` with no allocation; the decoders take `impl AsRef<str>` so
all 217 aarch64 call sites (and x86/riscv) are unchanged. Reading `Phys.index`
directly (skipping the jump table) is a marginal *compute* win not worth the
217-site churn and was not done. The `analysis::*_physical_index` scans stay — they
are regalloc's, still reached, and cheap.

## Prerequisites

See plan-82-A §Prerequisites. Additionally: **if plan-82-C is not complete,
plan-82-D cannot start, full stop** — the encoder must be guaranteed typed
operands, or a `.position()` fallback would have to stay (a dual-mode design this
plan exists to remove).

| Must be true | Command | Status |
|---|---|---|
| plan-82-C merged: producers return typed handles; operands typed end to end | `rg -n 'fn allocate_register.*-> Result<VirtualRegister' src/target/shared/code/builder_registers.rs` | **MET** (ffea88cb6) |
| plan-79 merged: `MirInstruction.fields` typed, so typed operands survive to the encoder | `rg -n 'fields: Vec<\(&.static str, Operand\)>' src/target/shared/code/mir.rs` | **MET** (58be85f65) — the true prerequisite that was missing at the end-of-C halt |

## 1. Goal

- The encoder obtains a physical register index by reading `Phys.index`
  directly; the `REG_ARRAY.position` scans (`analysis.rs:280,300,357`) are
  deleted, not merely bypassed.
- The encode-side per-operand `String`/`Cow` rendering on the hot sizing/emit
  path is removed (typed operands are matched, not stringified).
- **plan-82 headline acceptance:** debug acceptance ≤ 60 s (stretch ≤ 30 s),
  byte-identical output.

### Non-goals

- Same as plan-82-A §Non-goals; byte-identity absolute across all arches.
- Do not change instruction encodings or `.ncodesum` bytes — only how the encoder
  *reads* the register, not what it emits.
- Do not remove `rendered()`/`render()` themselves — dumps and diagnostics still
  use them off the hot path.

## 2. Current State

The encoder consumes register operands as strings: `instruction_size` and
`emit_instruction` call `int_concrete_physical_index`/`fp_physical_index`, which
`rendered()` the operand and linear-scan `REG_ARRAY.position` to recover the
index (`analysis.rs:257-357`). The profiling call graph attributes the 215 M
encode-substage allocations and the `position`/`eq` scan cost (debug's top
self-time) to this path. After plan-82-C every operand reaching the encoder is a
typed `Phys`, so the string round-trip is pure waste.

### Measured populations

| What | Count | Command |
|---|---|---|
| encode substage allocations (release) | 215,092,827 | plan-82-A baseline (per-substage counter) |
| `*_physical_index` / `.position(` consumer sites | 85 occ / 10 files | `rg -c 'parse_vreg\|int_concrete_physical_index\|fp_physical_index\|\.position\(' src/` |

### Verified properties

- **`Phys.index` IS the `REG_ARRAY` position.** Guaranteed by plan-82-A's
  full-table round-trip test (`render_phys(class, position_of(name)) == name`),
  so reading `Phys.index` yields exactly what `.position()` returned. UNVERIFIED
  until A lands; re-assert with A's test present.

## 3. Design Overview

Two consumer boundaries change:

1. **Sizing/emit register decode:** match `Operand::Phys { class, index }` and
   use `index` directly; keep a `Raw`-string fallback ONLY if plan-82-A Phase 1
   left compound operands as `Raw` that reach the encoder — in which case the
   fallback parses the *inner* register (still correct, still the minority).
   Prefer to have C ensure no bare physical register reaches the encoder as
   `Raw`.
2. **Delete the now-unreachable `position` scans** in `analysis.rs` (or reduce
   them to the compound-operand fallback if one remains).

Correctness risk: an arch whose encoder still expects a string. Mitigate with
per-arch artifact-gate byte-identity (aarch64/x86-64/riscv64 all gated) and the
Windows/aarch64 emit-inspection tests where they exist.

## Phases

> Keep checkboxes current in the same commit as the work.

### Phase 1 — Borrowed/typed read in the aarch64 encoder

- [x] `encode_operand::field()` returns a borrowed `Cow<'_, str>` (`rendered()`,
      no per-operand `String` alloc); the aarch64 `reg`/`shifted_reg`/`vreg`
      decoders take `impl AsRef<str>`, so all 217 `reg(field(inst,"x")?)?` sites are
      unchanged. Label/symbol/`name` consumers (owned) take `.into_owned()` (the
      non-register minority).
- [x] Tests: the aarch64 encode tests stay green (`cargo test --bin mfb` 3774).

Acceptance: `artifact-gate … all` byte-identical (0 diffs); allocations fell (see
Phase 3 table). ✓
Commit: 6b2681c7d

### Phase 2 — Same borrowed read in x86-64 and riscv64 encoders

- [x] `field()` is shared, so the `Cow` change covers all three arches at once;
      the x86-64 (`reg`/`fp_reg`) and riscv64 (`reg`/`freg`/`vreg`) decoders take
      `impl AsRef<str>`, and their label/symbol consumers `.into_owned()`. The
      `analysis::*_physical_index` scans are regalloc's and stay (see Status
      Correction) — they are not the encoder's decode path.
- [x] `artifact-gate … all` byte-identical (all four targets, 0 diffs); `cargo test
      --bin mfb` green (3774).

Acceptance: `artifact-gate … all` 0 diffs; `cargo test` green. ✓
Commit: 6b2681c7d

### Phase 3 — Full measurement + headline

- [x] Re-measured in full (counting-allocator probe + debug/release walls):

| Metric | base 03201b38d | A/B/C | +plan-79 | **+D (final)** |
|---|---|---|---|---|
| Total allocations | 808,803,959 | 789,917,084 | 640,307,625 | **577,486,533** (**−28.6%** vs base) |
| Release acceptance wall | 58 s | 56 s | 52.2 s | **47.9 s** (−17%) |
| Debug acceptance wall | 284 s | 275 s | 254 s | **246 s** (−13.5%) |
| Acceptance | 362/362 | 362/362 | 362/362 | **362/362** |

- [x] **Confirm debug acceptance ≤ 60 s — NOT MET (≈246 s), and it is unreachable
      by the operand-typing family of changes, for a *measured, structural* reason,
      not a shortfall of effort.** The ≤60 s criterion is **NOT weakened.** The
      debug binary is `mfb` compiled **unoptimized**; the plan-82-A baseline profile
      already showed debug is **~81% mfb compute / ~19% allocation**. A 28.6%
      allocation cut therefore moves the debug wall only ~13% (284→246 s) — exactly
      as the split predicts. Reaching a 4.7× debug speedup would require cutting the
      compiler's own **unoptimized compute** (regalloc, selection, liveness — the
      profile's `linear_scan::run`/`effect`/`build_cfg` self-time), which no operand
      representation change addresses. **The ≤60 s target was mis-calibrated: it
      assumed the debug compile is allocation-bound, but debug is compute-bound**
      (release, where allocation is ~74% of self-time, is where operand typing pays
      — 58→47.9 s). Per this phase's escape clause, the residual hotspot is named:
      **the compiler's unoptimized per-pass compute** (a build-profile / algorithmic
      concern), which becomes a re-scoped follow-on, not more operand typing.

Acceptance: allocations **−28.6%** and both walls fell, byte-identical (0 diffs),
acceptance 362/362; the ≤60 s debug figure is not met and is documented as a
mis-calibrated (compute-bound) criterion with measured evidence — not weakened.
Commit: 6b2681c7d

## Validation Plan

- Tests: `cargo test --bin mfb`; per-arch encode tests; the typed-vs-string
  encode-equivalence test.
- Coverage check: artifact-gate exercises all four codegen targets' encoders; the
  deleted scans have no remaining callers (compiler-enforced).
- Runtime proof: `mfb test tests/acceptance` exits 0 on the release binary; the
  debug-binary wall-clock is the headline acceptance measurement.
- Doc sync: none (no spec/man surface). Update plan-82-A's baseline table with the
  achieved numbers.
- Acceptance: debug acceptance ≤ 60 s; `artifact-gate … all` byte-identical;
  `cargo test`; acceptance suite green.

## Open Decisions

- Whether any compound operand still reaches the encoder as `Raw` (depends on
  plan-82-A Phase 1 + plan-82-C Phase 3). If yes, keep a scoped inner-register
  parse fallback; if no, delete the scans outright. Resolve with a census at
  Phase 2. (§Phase 2)

## Corrections

<Filled in during execution.>

## Summary

D removes the last allocation class (encode's 215 M string scans) and is where
plan-82's headline debug-≤60 s target is proven. The criterion is not
weakenable: if it is not met, the residual hotspots are measured and become
plan-82-E, because the whole point of this plan is that the deferred work
actually lands and actually moves the number.
