# plan-171-J: x86-64 frame layout and memory operands

Last updated: 2026-10-21
Effort: large (3h–1d)
Depends on: plan-171-I (the register pool changes every spill this letter
orders and folds; its measurements are taken after I)

Once plan-171-I has grown the pool, x86-64 still pays in four ways that other
backends do not:

1. **Hot stack slots sit far from `rsp`.** The builder hands out stack objects
   with a bump allocator and never reuses them
   (`src/codegen/engine/regalloc/builder_registers.rs:allocate_stack_object`).
   The allocator's spill slots, the most-used slots in a function, go after
   all of them (`spill_base = align(self.stack_size, 16)`). So 197,330 of
   306,886 `rsp` accesses in `sorts` are more than 1,023 bytes from `rsp`, and
   plan-171-K's 1-byte displacement can reach only the 45,323 within 127.
2. **A reloaded operand is a separate load.** x86 arithmetic can read memory
   directly: `ldr t, [rsp+d]; add d, d, t` is one `add d, [rsp+d]`.
3. **Address arithmetic is separate instructions.** x86 addressing has a base,
   a scaled index and a displacement; `lea` computes `a + b*{1,2,4,8}` without
   touching flags.
4. **Three-operand arithmetic costs a `mov`.** `alu3` emits `mov dst, lhs;
   op dst, rhs` whenever `dst` is neither input
   (`src/arch/x86_64/encode/emitter/integer.rs:alu3`).

This sub-plan orders x86 frames by use, folds loads and address arithmetic into
x86 memory operands, encodes three-operand `add` as `lea`, and has the
allocator prefer two-address colorings on x86.

Outcome: on `linux-x86_64` and `windows-x86_64` at `-O1` and above, every
function whose stack addresses can be renumbered (§4.1) lays out its stack
objects by use density, and the §4.2 rules fire wherever their conditions
hold. At every level, a three-operand `add` encodes as one `lea`. AArch64 and
riscv64 output are byte-identical, and every program's output is unchanged.

References: plan-171-A's list, plus
- `src/codegen/engine/util/vreg_frame.rs:finalize_frame`,
  `adjust_stack_instruction_offsets` (the existing uniform remap of every
  `sp` reference).
- `src/codegen/engine/regalloc/builder_registers.rs:allocate_stack_object`,
  the spill-slot placement after it, `CodeStackSlot`
  (`src/codegen/engine/types/types.rs`).
- `src/arch/x86_64/encode/emitter/integer.rs:alu3`, `mem_load`, `mem_store`.
- `src/optimizer/opt2/combine.rs` (plan-171-G's rule engine, which this letter
  extends to x86).
- `src/codegen/engine/regalloc/linear_scan.rs` (coloring order, the existing
  `mov` coalescing).
- plan-169-C (Win64 encoder scratch slots in the frame, which stay fixed).

## Prerequisites

See plan-171-A §Prerequisites. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-171-I complete | `ls planning/plan-171-I-* 2>/dev/null` → no match | NOT MET |

plan-169 is complete (a prerequisite of plan-171-H, before this letter), so
its Win64 frame-slot reservations exist and are named in
`planning/completed/plan-169-C-*.md`.

## 1. Goal

- **Frame order** (x86 only, Level 1): in `finalize_frame`, before the uniform
  shift, the movable stack objects are re-laid out in order of descending use
  density. Movable means the `stack_slots` entries, including the allocator's
  spills, other than plan-169-C's reserved scratch slots. Gated by
  `level_enabled(1)` and `Backend::orders_frame_by_use()` (x86 `true`).
- **x86 combine rules** (Level 1, in plan-171-G's `combine.rs`, from the
  machine hook): R1 offset fold, R2 SIB index, R3 `lea` shift-add, R7 load
  folding (§4.2). `Backend::supports_combined_forms()` becomes a per-backend
  rule set: `combine_rules()`, where AArch64 = G's R1–R6 and x86 = R1, R2,
  R3 (`lsl` 1–3 into `add`), R7.
- **`lea` for three-operand `add`** (encoder, every level): `add d, a, b` with
  `d ∉ {a, b}` encodes as `lea d, [a + b]`.
- **Two-address preference** (allocator, Level 1, x86 only):
  `RegisterModel::prefers_two_address()`. When coloring the `dst` of `add`,
  `sub`, `and`, `orr`, `eor` or `mul` whose `lhs` vreg's interval ends at that
  instruction, prefer `lhs`'s physical register.

### Non-goals

- No stack-slot sharing between objects with disjoint lifetimes. That is a
  different transformation, with its own lifetime analysis; the catalog row
  stays open.
- No change to the outgoing-argument area, the callee-saved save area, or
  plan-169-C's scratch slots (fixed region).
- No change to any value, trap or flag result: `lea` writes no flags, and the
  neutral `add` it replaces is the non-flag-setting op (`adds` is separate).
- No AArch64/riscv64 change (all four pieces are gated or x86-encoder-only).

## 2. Current State

- **Frame build**: objects are bump-allocated in creation order with no reuse,
  and spills are appended after them at a 16-aligned base with a 16-byte
  stride (`builder_registers.rs`). `finalize_frame` then adds `outgoing_bytes +
  save_size` to every `sp` reference (`adjust_stack_instruction_offsets`).
  Frames in `sorts` (`linux-x86_64`, `sub_sp` immediates): 1,078 framed
  functions, median 312 bytes, 90th percentile 2,312, largest 63,496.
- **Stack objects are listed** per function in the `-ncode` dump
  (`"stackSlots"`: name, type, offset; 974 of 1,112 functions have entries).
  The most common type is `spill` (27,983 entries in `sorts`).
- **References into the frame** (`sorts`, `/tmp/p171x/objslots.py`):
  - 306,886 `rsp`-based loads/stores;
  - address-of (`add_imm t, rsp, k`): 9,091 at an object's first byte, 1
    inside an object, 49 outside the listed objects;
  - 61 functions (3,175 accesses) read `rsp` in some other operand position.
  So all but a handful of references name their object by its start.
- **x86 memory operands**: `mem_load`/`mem_store` encode `[base + disp32]`. No
  index register, and no ALU instruction with a memory operand. `alu3`
  encodes `mov dst, lhs; op dst, rhs` when `dst ∉ {lhs, rhs}`.
- **Allocator**: colors in interval order with a `mov`-coalescing preference
  (a copy's `dst` takes the source's register when the source dies at the
  copy; `linear_scan.rs`). No preference exists for two-address ops.

### Measured populations

`sorts`, `linux-x86_64`, `-O1`, HEAD `0c6456873`, before plan-171-I (Phase 1
re-measures after I). Censuses: `/tmp/p171x/x86census.py`, `objslots.py`, and
`g_census_x86.py` (plan-171-G's prototype with x86 caller-saved and result
registers).

| What | Count |
|---|---|
| `rsp` accesses at displacement ≤ 127 / 128–255 / 256–1023 / > 1023 | 45,323 / 15,046 / 49,187 / 197,330 |
| ≤ 127 after object reordering (simulation: whole objects, use density, fixed low region, 61 functions left as they are) | 113,985 |
| R7 load folding `ldr t,[m]; op d,l,t`, `t` dead (add / mul / sub / eor / and / orr; `cmp` 0) | 8,479 (5,430 / 2,177 / 640 / 168 / 59 / 5) |
| R1 offset fold | 2,288 |
| R2 SIB index | 373 |
| R3 `lsl #1–3; add` (`lea`) | 249 |
| three-operand ALU sites needing a `mov` (add / and / sub / orr / eor) | 5,209 (4,212 / 682 / 228 / 60 / 27) |

The reordering row is an upper bound under the simulation's model. Uses are
counted statically, not weighted by loop depth. Phase 3 reports the real
count.

## 3. Design Overview

Four independent pieces, landed lowest-risk first: the `lea` encoding
(encoder-local), the two-address preference (allocator-local), the combine
rules (machine hook, rule-by-rule), and frame order (`finalize_frame`, the
widest blast radius).

**Correctness risk**:
- *Frame order.* Every reference must be rewritten consistently. The rule
  refuses a function unless every `rsp` reference is a memory access or an
  address-of whose offset maps to exactly one object (§4.1). A refused
  function keeps today's layout.
- *R7.* The folded memory operand must read the same address at the same
  point. It is adjacent by construction; the temporary must be dead; the
  store-forwarding peephole has already run.

**Gate class:** behavior-preserving, golden-changing; x86 goldens only. An
aarch64/riscv64 golden diff is a bug.

**Rejected alternatives**
- *An `rbp` frame base* (1 byte shorter per access): `rbp` is now an
  allocatable register (plan-171-I, owner decision).
- *Ordering at allocation time* (giving spills low offsets as they are
  created): the builder's objects are already placed by then. Only
  `finalize_frame` sees the whole frame.
- *Folding loads into `cmp`:* 0 sites in the census.
- *`imul`-based `madd` or BMI `bextr` (x86 versions of G's R4/R5):* `bextr`
  needs BMI1, which the baseline x86-64 target does not assume. `imul` has no
  accumulate form.

## 4. Detailed Design

### 4.1 Frame order (`finalize_frame`, x86, Level 1)

1. **Movable set**: the function's `stack_slots`, sorted by offset, minus
   plan-169-C's scratch slots. Each object's size is the gap to the next
   object; the last runs to the locals' end. Its alignment is
   `min(16, largest power of two dividing its original offset)`.
2. **Refusal**: leave the function as is when any instruction reads `sp`
   except as a memory `base`, as the `src` of `add_imm`/`sub_imm`, or in
   `add_sp`/`sub_sp`, or when an `add_imm`/`sub_imm` from `sp` lands inside the
   movable region but not on an object's first byte.
3. **Density**: for each object, the number of `sp`-based memory accesses
   whose offset falls in it, plus address-of references to it, divided by its
   size.
4. **Layout**: place objects in descending density (ties keep original order),
   each at the next offset that meets its alignment, starting at the movable
   region's original base.
5. **Rewrite**: every `sp` reference with offset `o` inside object `X` becomes
   `o − old(X) + new(X)`, and each `stack_slots[i].offset` is updated. The
   frame size is recomputed (alignment can change it by at most 15 bytes per
   object; the total is re-aligned to 16). The existing uniform shift then runs
   unchanged.

### 4.2 x86 combine rules

All in one basic block, temporary dead after (as plan-171-G §4.2):
- **R1 offset fold**: G's R1 with x86 legality, where the new displacement
  fits `i32` (negative allowed).
- **R2 SIB index**: G's R2 shapes, encoded `[base + index*s]`. The scaled
  form needs `s ∈ {1,2,4,8}` to equal the access size. `rsp` is never an
  index.
- **R3 `lea` shift-add**: `lsl_imm t, s, #k` (k ∈ 1–3) `; add d, a, t` →
  `add_shift d, a, s, lsl, k`, encoded `lea d, [a + s*2^k]`. Not `sub`, not
  `lsr`/`asr`.
- **R7 load folding**: `ldr_u64 t, [b, #o] ; op d, l, t` with `op` ∈ {add,
  sub, and, orr, eor, mul}, `t` ≠ `l` → `op_mem d, l, [b, #o]`, new x86-only
  ops `add_mem sub_mem and_mem orr_mem eor_mem mul_mem` (fields `dst`, `lhs`,
  `base`, `offset`). Encoded `mov d, l` (when `d ≠ l`), then `op d, [b+o]`
  (`03`/`2B`/`23`/`0B`/`33` /r, `0F AF` for `imul`).

The x86 encoder implements G's `ldr_*_idx`/`str_*_idx` (SIB) and `add_shift`
(only `lsl` ≤ 3; anything else is an encode error). The AArch64 encoder
rejects the `*_mem` ops, as the x86 encoder rejects AArch64-only forms
(plan-171-G §4.1). `analysis.rs:USE_FIELDS` already has `base` and, after G,
`index`.

### 4.3 `lea` for three-operand `add` (encoder)

In `alu3`, opcode `add` with `dst ∉ {lhs, rhs}`: emit `lea dst, [lhs + rhs]`.
`rsp` cannot be an index, so if `rhs` is `rsp` the operands swap. `rbp`/`r13`
as base need an explicit `disp8 0`. Sizes are measured by the shared
`instruction_size` path, so relaxation and labels follow automatically.

### 4.4 Two-address preference (allocator, x86, Level 1)

In `linear_scan.rs`, when the model `prefers_two_address()` and the vreg being
colored is the `dst` of an `add`/`sub`/`and`/`orr`/`eor`/`mul` whose `lhs`
vreg's interval ends at that instruction, try `lhs`'s physical register first.
Every existing legality check still applies; it is only a preference order,
like the existing `mov` coalescing.

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — post-I baseline

- [ ] Add the §2 measured patterns to `tools/ncode-census/census.py`. Re-run for
      `linux-x86_64` and `windows-x86_64` on the post-I tree, and record a
      "post-I" table in §2.

Acceptance: §2 has the post-I table.
  Check: `grep -c 'post-I' planning/plan-171-J-*.md` → ≥ 2 (est. 20 min).
Commit: —

### Phase 2 — `lea` add and two-address preference

- [ ] `alu3` §4.3, with encoder tests (each register pair including `rsp`
      operand swap, `rbp`/`r13` base, `r8`–`r15` REX).
- [ ] `prefers_two_address` (x86 `true`) and §4.4 in `linear_scan.rs`, with
      allocator unit tests (preference taken; preference refused when `lhs`
      is live after; refused when the register is busy).
- [ ] Golden re-baseline (x86 only, plan-171-A Phase 3's procedure).

Acceptance: three-operand `add` is `lea`; the extra-`mov` census count falls.
  Check: `cargo test --bin mfb x86_64::encode regalloc::` → pass (est. 4 min); census extra-`mov` count < the post-I count.
Commit: —

### Phase 3 — combine rules R1, R2, R3, R7

- [ ] `combine.rs`: `combine_rules()` per backend; x86 R1/R2/R3/R7 (§4.2).
      Tests in `combine/tests/x86.rs`: positive plus one negative per
      condition (live temporary, `t == l`, `rsp` index, scale ≠ size,
      displacement overflow, `sub` for R3).
- [ ] `src/arch/ops.rs` + `mir.rs` mirror group: `add_mem`…`mul_mem`.
      x86 encoder arms for them and for `*_idx`/`add_shift`; AArch64 and
      riscv64 encoder rejections.
- [ ] rt fixture `tests/rt-behavior/collections/x86-memory-operand-forms`
      (valid): list indexing at each element width, struct-field address
      arithmetic, and accumulations whose operands are spilled
      (`sum += a[i] * b[i]` over wide expressions). It prints a checksum.
- [ ] `rt_codegen_forms::x86_memory_operands`: the fixture's `linux-x86_64`
      dump contains `add_mem`, an `_idx` op and `add_shift` at `-O1`, and
      none at `-O0`.
- [ ] Golden re-baseline (x86 only).

Acceptance: the rules fire on x86 only; outputs unchanged.
  Check: `cargo test --bin mfb opt2::combine x86_64::encode` → pass (est. 4 min);
  `cargo test --release --test rt_codegen_forms x86_memory_operands` → pass (est. 2 min).
Commit: —

### Phase 4 — frame order (largest blast radius)

- [ ] `vreg_frame.rs`: §4.1 behind `orders_frame_by_use()` and
      `level_enabled(1)`. Unit tests (test-only file
      `src/codegen/engine/util/tests/frame_order.rs`):
      - hot spill moved below cold objects;
      - an address-of reference moves with its object;
      - an interior address-of refuses the function;
      - a non-remappable `sp` read refuses it;
      - 16-byte spill alignment kept;
      - plan-169-C scratch slots fixed;
      - frame size re-aligned.
- [ ] rt fixture `tests/rt-behavior/functions/frame-order` (valid): a function
      with dozens of cold temporaries and a hot loop over spilled values, plus
      a callee taking the address of a local record. It prints a checksum.
- [ ] `rt_regalloc_levels_agree`: add the Phase 3 and Phase 4 fixtures.
- [ ] Census: disp ≤ 127 share before/after, recorded in Corrections.
- [ ] Golden re-baseline (x86 only).
- [ ] Runtime: `linux-runtime-proof.sh` on 2227 and 2228; box 206 fixtures and
      plan-169-C's backtrace check.

Acceptance: frames are ordered on x86 at `-O1`; outputs and Win64 backtraces
unchanged.
  Check: `cargo test --bin mfb vreg_frame` → pass (est. 2 min);
  `linux-runtime-proof.sh … 2227` and `… 2228` → 0 mismatches (est. 60–120 min each, emulated — the only Linux x86 execution);
  box 206 → pass (est. 30 min).
Commit: —

### Phase 5 — spec and docs

- [ ] `mfb spec architecture x86_64-instruction-set`: memory-operand ALU forms,
      SIB and `lea` uses, frame ordering. The optimizer topic: the x86 combine
      rules and the frame-order row.
- [ ] `src/optimizer/catalog.rs` row "Frame layout by use (x86)" (level 1) and
      the combine row's summary naming x86. `planning/optimizations.md`:
      "Addressing mode optimization" and "x86 LEA-as-arithmetic" landing
      notes.
- [ ] `.ai/codegen-invariants.md`: "on x86, stack objects are renumbered at
      `finalize_frame`; a builder that takes an address inside an object
      (not its start) opts its function out — prefer object-start addresses".

Acceptance: specs and catalog render.
  Check: `cargo test --bin mfb spec` → pass (est. 2 min); `target/release/mfb man optimizations | grep -c 'Frame layout by use'` → 1.
Commit: —

## Validation Plan

- Tests: encoder (`lea`, memory forms, SIB), allocator preference, combine
  x86 rules, frame-order units, `rt_codegen_forms`, both fixtures,
  levels-agree.
- Coverage: each rule's counter is non-zero on its fixture (`mfb build -vv`).
- Runtime proof: 2227, 2228, box 206. AArch64/riscv64 byte-identical
  (`artifact-gate.sh … all` shows diffs only in x86 goldens).
- Final gate (once, after Phase 5): `cargo test --no-fail-fast > /tmp/171j.log 2>&1; echo EXIT=$?` → `EXIT=0`, `grep -c '^failures:' /tmp/171j.log` → 0 (est. 60–90 min).

## Decisions (owner, 2026-10-21)

1. These x86 fixes are plan-171 letter J, after the register pool (I) and
   before short encodings (K).
2. Both x86 targets, with plan-169 complete first.
3. `rbp` is a register (plan-171-I), so frames stay `rsp`-based.

## Corrections

<Filled in during execution.>

## Summary

Frame order is the large item. It moves the hottest slots, mostly spills, into
the range plan-171-K's 1-byte displacements reach (45,323 → at most 113,985
accesses in `sorts`). The memory-operand rules remove 8,479 reloads and 2,910
address instructions. `lea` and the two-address preference remove most of the
5,209 extra `mov`s.
