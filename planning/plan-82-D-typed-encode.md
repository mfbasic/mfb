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

## Prerequisites

See plan-82-A §Prerequisites. Additionally: **if plan-82-C is not complete,
plan-82-D cannot start, full stop** — the encoder must be guaranteed typed
operands, or a `.position()` fallback would have to stay (a dual-mode design this
plan exists to remove).

| Must be true | Command | Status |
|---|---|---|
| plan-82-C merged: producers return typed handles; operands typed end to end | `rg -n 'fn allocate_register.*-> .*VReg\|fn allocate_register.*Register' src/target/shared/code/builder_registers.rs` | NOT MET (C pending) |

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

### Phase 1 — Typed read in the aarch64 encoder

- [ ] `instruction_size` + `emit_instruction` (and per-form helpers) read
      `Phys.index` directly; measure the encode-substage allocation drop.
- [ ] Tests: keep the aarch64 encode tests green; add one asserting a typed
      `Phys` operand sizes/encodes to the same bytes as the `Raw` string did.

Acceptance: `artifact-gate … aarch64` byte-identical; encode-substage alloc count
(plan-82-A counter) sharply lower than 215 M (record it).
Commit: —

### Phase 2 — Typed read in x86-64 and riscv64 encoders; delete the scans

- [ ] Apply the same typed read to the x86-64 and riscv64 encode paths.
- [ ] Delete `int_concrete_physical_index`/`fp_physical_index` `REG_ARRAY.position`
      scans (or reduce to the compound-operand fallback if one survives), with a
      comment citing plan-82-A's round-trip guarantee.

Acceptance: `artifact-gate … all` byte-identical (all four targets);
`cargo test --bin mfb` green.
Commit: —

### Phase 3 — Headline perf target (plan-82 capstone)

- [ ] Re-measure the plan-82-A baseline table in full (debug + release acceptance
      wall, total allocation count, per-substage counts) and record final numbers
      here and in plan-82-A's Corrections/Summary.
- [ ] Confirm **debug `mfb test tests/acceptance` ≤ 60 s** (stretch ≤ 30 s). If
      the target is not met, the remaining allocation hotspots are named here with
      their `sample`/counter evidence and become a follow-on letter (E) — the
      alphabet is append-only; do not weaken this criterion.

Acceptance: debug acceptance wall ≤ 60 s, measured with the command in the
baseline; acceptance suite exits 0; `artifact-gate … all` byte-identical.
Commit: —

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
