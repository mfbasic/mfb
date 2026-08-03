# plan-78-C: Typed-operand register allocation + `colored_mask_at` sweep

Last updated: 2026-08-02
Effort: medium (1h–2h)
Depends on: plan-78-B (storage must already be `Operand`-typed)

Migrate the register allocator's hot loops to read the typed `Operand` values
directly (integer register ids) instead of re-parsing/hashing/`str::eq`-comparing
operand strings, and replace the O(vregs × interval) `colored_mask_at`
construction with an endpoint sweep. This sub-plan **delivers the perf win**: it
removes the measured `str::eq` (#1/#2 self-time) and SipHash costs from the
analysis and eliminates the spill-path quadratic — all while keeping emitted code
byte-identical.

The single behavioral outcome: the lowering/regalloc pass is dramatically faster
(one `regex::match` const ≤ 3 s debug from 31 s; `mfb test tests/acceptance`
≤ 60 s debug from 4 m 21 s) with `artifact-gate … all` still diff-free.

References:

- plan-78-B (`planning/plan-78-B-flip-storage.md`) — provides `operand(name) ->
  &Operand` typed reads on `CodeInstruction`.
- `src/target/shared/code/regalloc/analysis.rs` — `effect`, `is_tracked`,
  `physical_index`, liveness.
- `src/target/shared/code/regalloc/linear_scan.rs` — the scan, `colored_mask_at`
  (:170-182), the rewrite loop (:201).

## Prerequisites

See plan-78-A's Prerequisites table (feature-wide). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-78-B complete (fields are `Operand`, `operand()` accessor exists) | B's phases all ticked (commits 4eafd3830, 02f9bd2ea) | MET (2026-08-02) |
| `bench-lowering.sh` baselines recorded (from A) | `cat planning/plan-78-baseline.txt` | MET (2026-08-02: one-regex 29.2s debug, acceptance 266s debug, regex fn 860,981 instrs / 135,293 int vregs) |

> If B is not complete, C cannot start — full stop. C reads `operand()`, which B
> introduces; it does not itself flip storage.

## 1. Goal

- `analysis::effect`, `is_tracked`, and the liveness/scan/rewrite loops read
  register class + id from `Operand` (integers), with **no** per-operand
  `parse_vreg`/`physical_index` string work and **no** `Vec<String>` operand
  clones.
- `colored_mask_at` (`linear_scan.rs:170`) is built by an endpoint sweep in
  O(instructions + Σ interval endpoints), producing a bit-identical mask.
- Measured perf goal met (see Goal above) with `artifact-gate … all` diff-free.

### Non-goals (explicit constraints)

- **No emitted-byte change.** Allocation decisions (which vreg → which physical,
  which spills, spill-slot order, the bug-87 `(start, id)` tie-break) are
  identical; `artifact-gate … all` is the guardrail.
- **No `MirInstruction`/selection change** (out of scope, not hot).
- **No `-regalloc bump` change.**

## 2. Current State

Post-B, `CodeInstruction` carries typed `Operand`s but the allocator still reads
them as strings via the rendered `get()` path:

- `effect` (`analysis.rs:315`) iterates `instruction.fields`, tests
  `DEF_FIELDS`/`USE_FIELDS.contains(name)` (`:23,27`) and `is_tracked(value)`
  (`:303`), and clones matching operand **strings** into `Vec<String>`.
- `is_tracked` → `parse_vreg` (`mod.rs:45`) + `physical_index`
  (`int_concrete_physical_index`, `analysis.rs:213`), whose core is a linear
  `REG_ARRAY.position(|&reg| reg == name)` (`:227`) — the measured #1/#2
  self-time `str::eq`.
- `effect` is computed **3×** per instruction (`analysis.rs:520`, `:601`,
  `linear_scan.rs:202`).
- `colored_mask_at` (`linear_scan.rs:170-182`): for each colored vreg, OR a bit
  across every instruction index in its (over-approximated, wide) `[s,e]`
  interval — O(vregs × interval), on the spill path only (regex spills heavily).

### Verified properties

- **The perf cost is register-string handling + the spill quadratic.** Profile
  self-time: `str::eq` #1+#2 (~800 samples), SipHash/`hashbrown`, `memmove`/
  `memcmp`, slice `position`/`any`; call tree `allocate`→`linear_scan::run` ≈ 80%,
  `analyze` ≈ 20%. (`sample <pid>` during the one-regex build.)
- **B exposes register class+id as integers via `operand()`** — so `effect` can
  classify def/use and read register identity with zero parsing.

## 3. Design Overview

Two independent wins, both in `regalloc/`:

1. **Typed reads** — rewrite `effect`/`is_tracked` to match on `Operand`:
   `VReg{class,id}` and `Phys{class,index}` are already the integers the scan
   needs; `Raw`/`Imm` are "not a tracked register of this class". Drop
   `parse_vreg`/`physical_index` from the hot path and the `Vec<String>` clones.
   Compute `effect` **once** per instruction per class and share it across
   liveness and the rewrite loop (the 3→1 dedup).
2. **Sweep `colored_mask_at`** — emit `(s, +bit)`/`(e+1, -bit)` events per colored
   vreg, sort once, fold across instruction indices maintaining a running mask.
   Representation-independent; would be worth doing even without B.

Correctness risk: both must reproduce the *exact* current allocation and masks.
Guarded by `artifact-gate … all` plus a property test asserting the sweep mask ==
the naive double-loop mask.

## 4. Detailed Design

- `Effect` becomes index/id-based (store `(RegClass, u32)` register ids, or
  small bitsets, not `Vec<String>`); `effect(instruction, class)` reads
  `instruction.operand(name)` and matches the `Operand` arm.
- `is_tracked(op: &Operand, class)` is a match, not two string parses.
- Memoize `Vec<Effect>` once per `(function, class)`; pass it to `analyze` and to
  the rewrite loop instead of recomputing (`analysis.rs:520,601`,
  `linear_scan.rs:202`).
- `colored_mask_at`: build from interval endpoints (§3); assert equality with the
  naive computation under test.
- The final rewrite (`mod.rs:359`) that substitutes colored vregs → physicals now
  writes `Operand::Phys{class,index}` directly (no sentinel string round-trip).

## Compatibility / Format Impact

None externally. `.ncode`/`.mir`/executables byte-identical.

## Phases

> **NOTE — keep boxes/`Commit:` current; run `artifact-gate … all` after each.**

### Phase 1 — Sweep-based `colored_mask_at` (representation-independent)

Land the algorithmic win first — it's isolated and doesn't depend on the typed
reads.

- [x] Rewrite `colored_mask_at` as an endpoint sweep (`colored_mask_sweep` in
      `linear_scan.rs`): `+pi` at interval start, `-pi` at `end+1`, folded across
      instruction indices with a per-physical-index reference count so a bit clears
      only when the last vreg on it leaves. O(instructions + Σ endpoints) vs the
      naive O(vregs × interval); **bit-identical** to the naive form.
- [x] Tests: property test — **`sweep_equals_naive_over_randomized_intervals`**
      (500 trials, dense 0..16 indices + heavy overlap) and
      `overlapping_same_index_clears_only_after_last`. **Correction:** placed in a
      `#[cfg(test)] mod tests` in `linear_scan.rs` (co-located) rather than
      `regalloc/tests.rs`, so the test reaches the private `colored_mask_sweep`
      directly. The byte-identity of the spill-heavy fixtures is covered by the
      gate below.
- [x] `artifact-gate … all` — **0 diff(s)** (1144 tests, 1286 builds, 1549
      goldens), verified 2026-08-02.

Acceptance: `artifact-gate … all` byte-identical (0 diffs); `sweep_equals_naive`
property test passes. The sweep is provably O(instructions + Σ endpoints) — the
spill-path mask no longer scales with vregs × interval. **Note:** absolute
one-regex wall-clock is *higher* than the plan-78-A baseline at this point (≈37 s
vs 29 s) because plan-78-B added a `render()` clone per operand read in `effect`;
Phase 2 removes that tax (typed/borrowed reads) and delivers the feature's speed
goal. Phase 1's win is the algorithmic removal of the quadratic, proven by the
property test + complexity, not yet visible in wall-clock under B's render tax.
Commit: 2eaaf92a5

### Phase 2 — Typed `effect`/liveness + compute-once

The main perf win.

- [x] Rewrite `effect` (`analysis.rs`) to **classify each operand once** into a
      `RegRef::{Phys(index), VReg(id)}`, so `analyze` and the rewrite loop consume
      the classification without re-parsing; drop the `Vec<String>` operand clones;
      read operands by borrow (`Operand::rendered()` → `Cow`, no clone for the
      `Raw` case). Removed `ClassModel::is_tracked` (folded into `classify`).
      **Correction — reads the `Raw` `&str`, not an `Operand` arm match.** B stores
      register operands as `Raw` (vreg-source typing is the 1794-site change B
      deferred), so the pre-allocation stream has no `VReg`/`Phys` arms to match;
      the equivalent win is the fast-reject below + classify-once + no-clone.
      Added a `%`-prefix fast-reject to `int_concrete_physical_index` /
      `fp_physical_index` — this is what actually removed the measured #1/#2
      `str::eq` (the `REG_ARRAY.position` scan that every cross-class vreg operand
      used to fall through): profile `str::eq` self-time ~800 → ~57 samples.
- [x] Compute `Vec<Effect>` once in `linear_scan::run` and share it between
      `analyze` (new `effects` param) and the rewrite loop (2 of the 3 recompute
      sites deduped; `integer_live_out` is a separate post-coloring pass, left as
      its own). Also replaced the allocator's `u32`-keyed `HashMap`/`HashSet`
      (SipHash) with a fast multiplicative `U32Hasher` — the liveness fixpoint +
      interning were the top self-time after `str::eq` was removed.
- [x] ~~Have the vreg→physical rewrite write `Operand::Phys` directly.~~ —
      **moot:** post-rewrite no consumer reads a typed `Phys` (peephole/finalize
      read via rendered strings), so a `Phys{class,index,name}` there would carry a
      never-read `index`/`class` (dead field). The rewrite keeps writing the
      physical name as `Raw` (byte-identical). The `Phys` arm stays deferred with
      `VReg` (see the perf correction below).
- [x] Tests: `sweep_equals_naive` (Phase 1) covers determinism/coloring-unchanged
      via the gate; full `cargo test --bin mfb` green (3765); the classify-once
      path is exercised by every codegen test + proven byte-identical by the gate.
- [x] `artifact-gate … all` — **0 diff(s)** (1144 tests, 1286 builds, 1549
      goldens), verified 2026-08-02 — the classify-once, fast-reject, and fast
      hasher are all byte-identical (the hasher perturbs no output-affecting order).

Acceptance (corrected — the numeric targets rested on a false premise; see the
Corrections "perf premise" entry): `artifact-gate … all` byte-identical (0 diffs)
✓; `str::eq` out of the top self-time (~800 → ~57 samples) ✓; SipHash removed from
the liveness pass via `U32Hasher` ✓; the sweep is provably O(instructions + Σ
endpoints) with a property test ✓. **Measured wall-clock:** one-regex 28.7 s debug
(from 29.2 s), acceptance 256 s debug (from 266 s) — a ~4% net gain, NOT the
≤3 s / ≤60 s the plan targeted. The targets are unreachable because they assumed
the `colored_mask_at` quadratic + `str::eq` dominated these workloads; profiling
shows one-regex barely spills (13-sample sweep) and the runtime is dominated by
distributed per-instruction processing of the ~860k-instruction lowered stream in
debug mode. C's mechanisms are correct and byte-identical; the magnitude estimate
was wrong.
Commit: (recorded next commit)

## Validation Plan

- Tests: `cargo test --bin mfb` incl. the sweep property test, the typed-effect
  tests, and the determinism check.
- Byte-identity: `artifact-gate.sh … all` diff-free after every phase (guardrail).
- Runtime proof: `mfb test tests/acceptance` exits 0 with all cases passing —
  proves codegen still *executes* correctly, not just that bytes match.
- Performance: `bench-lowering.sh` before/after each phase; final numbers meet §1
  and are recorded next to the A baselines.
- Coverage: `scripts/coverage-check.sh` — changed regalloc code stays ≥95%.
- Acceptance: `cargo test --workspace` + `artifact-gate … all` green.

## Open Decisions

- **`Effect` storage shape** — `Vec<(RegClass,u32)>` vs. per-class bitsets.
  Recommend bitsets if the id space is dense per function (cheapest liveness
  merge), else id vectors. Decide from the Phase-1 function-size measurement. (§4)

## Corrections

- **The perf premise ("regex spills heavily"; the `str::eq` + spill quadratic
  dominate) is false — a design-gate defect.** The plan set the ≤3 s / ≤60 s
  targets on the assumption that fixing `colored_mask_at` (the quadratic) and the
  `str::eq` scan would recover ~90% of the runtime. Profiling the one-regex build
  (`sample`) after C shows: the sweep is 13 samples (one-regex barely spills, so
  the quadratic was never its bottleneck); `str::eq` fell from ~800 to ~57 samples
  (the fast-reject worked) yet wall-clock moved 29.2 s → 28.7 s; the remaining cost
  is *distributed* — `effect` (285), `substitute`/`render`/`CodeInstruction` drop
  (~490), `analyze` (153), the coloring loop (~596) — over the ~860k-instruction
  lowered stream in debug mode, with no single 10× lever. Acceptance moved 266 s →
  256 s (~4%). **C's mechanisms landed correctly and byte-identically; the
  magnitude estimate did not hold.** Per the user's decision (2026-08-02), C is
  landed and the Phase-2 acceptance criterion is corrected to the checkable,
  achieved outcomes (byte-identity, `str::eq`/SipHash out of the profile, sweep
  proven non-quadratic, measured wall-clock) rather than the falsified numeric
  targets. Recorded here as a **Prerequisites/design-gate defect**: the entry gate
  should have profiled where the time actually goes before committing to the
  numeric targets. Evidence: `/tmp/p78-sample*.txt` (profiles), one-regex 28.7 s,
  acceptance 256 s.
- **`effect` reads the `Raw` `&str`, it does not match `Operand` arms; no
  `Operand::Phys` is written at the rewrite.** B stores register operands as `Raw`
  (vreg-source typing is a 1794-site change out of B/C scope), so there are no
  `VReg`/`Phys` arms in the pre-allocation stream to match. The win is delivered by
  classify-once (`RegRef`) + reading operands by borrow (`rendered()`) + the
  `%`-prefix fast-reject in the physical-index scans. `VReg`/`Phys` remain the
  test-exercised typed surface A introduced; typing register operands at source is
  a well-scoped future follow-up, not required for the (delivered) byte-identity or
  the (measured) speed.
- **Added a fast `U32Hasher` for the allocator's dense-`u32` keys.** The default
  SipHash on the liveness `HashSet<u32>` / interning `HashMap<u32,…>` was the top
  self-time once `str::eq` was removed; a multiplicative hash removes it. Applied
  only where iteration order does not feed emitted bytes (every order-dependent use
  is sorted first — bug-87), proven byte-identical by the gate.

## Summary

C lands the register-allocator improvements: the sweep-based `colored_mask_at`
(kills the spill-path quadratic for genuinely spill-heavy code), classify-once
`effect` reads, the `%`-prefix fast-reject (removes the `str::eq` scan), and a fast
`u32` hasher (removes SipHash from the liveness pass) — **all provably
byte-identical** via `artifact-gate … all` (0 diffs). The plan's ≤3 s / ≤60 s
wall-clock targets were NOT met: they rested on a false premise that these hot
loops dominate, but profiling shows one-regex barely spills and the runtime is
distributed per-instruction work over the ~860k-instruction stream in debug mode
(one-regex 29.2 s → 28.7 s; acceptance 266 s → 256 s). The mechanisms are correct;
the magnitude estimate was the defect (see Corrections). Per the user's decision,
C is landed on the corrected, checkable acceptance criterion.
