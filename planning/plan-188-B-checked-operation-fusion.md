# plan-188-B: Checked-operation fusion (unsigned-wrap checks become `adds` + carry branch)

Last updated: 2026-10-26
Effort: medium (1h–2h)
Depends on: plan-188-A (letter order), and plan-171 complete (see Prerequisites)

Every collection, string and map growth path checks a size addition for unsigned
wrap the same way: `emit_checked_size_add` / `emit_checked_size_add_immediate`
(`src/codegen/error/emission/builder_error_emission.rs`) emit
`add d, l, r; cmp d, r; b.lo overflow` (or `add_imm d, s, k; cmp d, s; b.lo
overflow`). The compare only recomputes the carry the add already produced. This
sub-plan lands the catalog row "Checked-operation fusion" for that idiom: on
AArch64 and x86-64 the triple becomes `adds d, l, r` / `adds_imm d, s, k` plus one
carry branch, when the flags are dead after the branch on both edges. The outcome:
at `-O2` and above every such check is one instruction shorter, the overflow path
still raises the same error from the same place, and riscv64 is unchanged.

References:

- `src/codegen/error/emission/builder_error_emission.rs:emit_checked_size_add`,
  `emit_checked_size_add_immediate` (the producers; "dst may alias lhs but must not
  alias rhs").
- `src/optimizer/opt2/machine.rs:finish` (the post-frame hook),
  `src/optimizer/opt2/plans/flagslive.rs:dead_after` (plan-171-B §4.2),
  `src/optimizer/opt2/plans/mark.rs:flag_preserving`.
- `src/arch/ops.rs:CodeOp` (`Adds`, `BranchLo`), `src/codegen/engine/mir/mir.rs`
  (`mir_ops!` lists), `src/codegen/engine/regalloc/analysis.rs` (`DEF_FIELDS`,
  `USE_FIELDS`, `effect`).
- `src/arch/aarch64/encode/emitter/`, `src/arch/x86_64/encode/emitter/`,
  `src/arch/riscv64/encode/emitter.rs`, the decoders' table tests.
- `tests/common/ncode_emu.rs` (models flags as the last compare's operands).
- plan-188-A §2.4 (the row assessment), plan-171-B §4.2.

## Prerequisites

The family's prerequisites are in plan-188-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-188-A complete | `ls planning/plan-188-A-* → no matches` (archived) | NOT MET (2026-10-26) |
| plan-171 complete — B's flags liveness exists and K's final branch-form tables are in place, so the new ops are added once into the finished tables | `ls planning/plan-171-* → no matches` and `ls src/optimizer/opt2/plans/flagslive.rs` → exists | NOT MET (2026-10-26: plan-171-B is being executed) |

Everything below is written against the world where these hold.

## 1. Goal

- New `CodeOp`s: `adds_imm` (`dst`, `src`, `imm`; flag-setting add of an
  immediate) and `b.hs` (branch if unsigned higher-or-same, AArch64 condition
  `HS`/`CS`, `C = 1`).
- One Level-2 machine row, "Checked-operation fusion", in
  `src/optimizer/opt2/checkfuse.rs`, called from `machine::finish`, gated
  `level_enabled(2)` and `active_backend().has_carry_flag()` (AArch64 and x86-64
  `true`, riscv64 `false`).
- At `-O2`+, on AArch64 and x86-64, every `add`/`add_imm` + `cmp` + `b.lo`
  unsigned-wrap check whose flags are dead after the branch is two instructions.

### Non-goals (explicit constraints)

- No change to any value, trap, error code or error location. The overflow branch
  still goes to the same raise block.
- No riscv64 change: it has no flags; the same check is already `add` + `bltu`
  (two instructions), so there is nothing to fuse.
- No fusion of a checked `adds`/`subs` with a later `cmp_imm d, 0`
  (526 AArch64 / 477 x86-64 sites, `/tmp/p188/probe.py` `checked_then_cmp0_*`) or
  of `subs d, a, b` with a later `cmp a, b` (102 / 115, `subs_then_same_cmp`) —
  declined as too few; the "Compare elimination / flag reuse" row stays `~`.
- No change at `-O0` or `-O1` (Level 2, the catalog's level for this row).

## 2. Current State

### 2.1 The idiom

`emit_checked_size_add(dst, lhs, rhs, overflow)`: `add dst, lhs, rhs; cmp dst, rhs;
b.lo overflow`. `emit_checked_size_add_immediate(dst, src, k, overflow)`:
`add_imm dst, src, k; cmp dst, src; b.lo overflow` ("`dst` must not alias `src`").
Both rely on: for unsigned 64-bit `x + y`, the sum wrapped iff `(x + y) mod 2^64 <
y` (and `< x`). The add itself already computes that carry:

- AArch64: `adds` sets `C = 1` iff the unsigned add carried; `b.hs` (`C = 1`)
  branches on it. (After a *compare*, `C = 1` means no borrow — the condition names
  describe a compare, which is why the fused branch is `b.hs` and not `b.lo`.)
- x86-64: `add` sets `CF = 1` iff it carried; `jb` (`CF = 1`) branches on it. The
  neutral `b.lo` already encodes as `jb` (`x86_64/encode/emitter/instruction.rs`),
  so on x86-64 the branch is unchanged and only the `cmp` goes.

### 2.2 Measured populations

Post-frame `.ncode` over the census corpus (`/tmp/p188/probe.py`
`add_cmp_blo_unsigned_wrap`: `add|add_imm d` directly followed by `cmp d, X` with
`X` the add's other operand, then `b.lo`), base release `mfb` of `d58b5397a`:

| Target | Sites | Instructions in corpus |
|---|---|---|
| macos-aarch64 | 52,058 | 27,057,789 |
| linux-x86_64 | 23,422 | 30,519,671 |

In `examples/ai_chat` (`/tmp/p188/wrap.py`) the 244 sites are half `add`, half
`add_imm`; their targets are the `*_size_overflow` labels of `list_append`,
`set_repack`, `list_insert`, `list_remove_at`, `map_grow`, `strings_split`, …; and
in all 244 the next flag-relevant instruction on the fallthrough path is a flag
writer, call, label or branch, never a flag reader.

### 2.3 Op tables the new ops join

`rg -n "BranchLo\b" src` lists every table a conditional branch appears in:
`arch/ops.rs` (4), `arch/aarch64/encode/relax.rs`, `arch/riscv64/select.rs`,
`codegen/engine/builder/code_impl.rs`, `codegen/engine/mir/mir.rs` (2),
`codegen/engine/regalloc/analysis.rs`, `codegen/engine/validation/validation.rs`,
`optimizer/opt2/branches.rs` (2), `optimizer/opt2/plans/ranges.rs` (5),
`target/win_x86_64/code.rs`; the mnemonic string `"b.lo"` additionally appears in
the AArch64 and x86-64 encoders, `riscv64/select.rs`, `x86_64/select.rs` and their
tests. `b.hs` joins each of these with the same role as `b.lo`.

## 3. Design Overview

One post-frame peephole plus two ops. It runs after register allocation because
that is where plan-171-B's flags liveness (`flagslive::dead_after`) is computed and
where the census above was taken; the shapes are adjacent instructions, so no
earlier stage is needed.

**Correctness risk** is in two places: (1) the alias guard — if `d` equals the
compared operand, the original `cmp d, d` never branches, while the fused carry
branch would; (2) flags liveness — the fused sequence leaves the add's flags, not
the compare's. Both are checked per site; the risk is a missed guard, pinned by
unit tests that assert the row declines.

**Byte-identity is this plan's gate for the committed goldens.** They are built
at the default `-O1`, and nothing in this plan acts below `-O2`: the ops are new
(no `-O1` producer) and the row is Level 2. So `scripts/artifact-gate.sh … all`
must stay at `0 diff(s)` with no regeneration; a diff is a bug (most likely a
missing level guard) to root-cause in one fixture, never a re-baseline. What
*does* change is `-O2`+ code on AArch64 and x86-64, proven by the runtime checks
of Phase 2. riscv64 changes at no level.

Rejected alternatives:

- **Emitting `adds` in `emit_checked_size_add` itself.** Unconditional (Level 0)
  and blind to whether a caller reads the flags after the branch; the 52,058 sites
  come from many callers and the flags fact is a CFG property.
- **A new `b.cs` op for "carry after an add".** On AArch64 it is the `b.hs`
  encoding and on x86-64 the `b.lo` encoding, so it would be an alias with no
  distinct machine meaning. The row picks the right existing condition per backend
  via `Backend::carry_set_branch()` instead.

## 4. Detailed Design

### 4.1 Ops

- `adds_imm` — fields `dst`, `src`, `imm` (unsigned). Roles: `dst` def, `src`
  use; flag setter. AArch64: `ADDS Xd, Xn, #imm12{, LSL #12}`; an immediate that
  is neither `imm12` nor `imm12 << 12` is an encode error (a producer bug — the
  row checks encodability first, `Backend::adds_imm_encodable(imm)`). x86-64:
  `mov d, s` when `d ≠ s`, then `add d, imm` (`83 /0 ib` for `i8`, `81 /0 id` for
  `i32`; beyond `i32` not encodable). riscv64: refused by the encoder ahead of the
  dispatch, as plan-171-A refuses `stp` (no producer can reach it).
- `b.hs` — field `target`. AArch64 `B.HS` (cond `0b0010`), relaxed like every
  conditional branch. x86-64 `jae` (`73 rel8`/`0F 83 rel32`, per plan-171-K's
  forms). riscv64 select already maps the mnemonic after an integer compare
  (`riscv64/select.rs:int_branch`: `"b.hs" | "b.cs"` → `geu`); add the float arm,
  the exact complement of its `"b.mi" | "b.lo"` arm (`rv.fcmp lt` then
  `rv.br eq`), so the op is real on every backend.

Both join every table of §2.3, the `mir_ops!` mirror list, `flag_preserving`
(neither preserves on any backend), `flagslive` (`adds_imm` a definite setter,
`b.hs` a reader), the AArch64 and riscv64 decoders' table tests (the widened
decoder already decodes `ADDS` immediate and every `B.cond`; only round-trip rows
are added), and `tests/common/ncode_emu.rs` (§4.3).

### 4.2 The row

`checkfuse::fuse(instructions: &mut Vec<CodeInstruction>) -> u64`. One
`flagslive::dead_after` over the finished stream, then a forward scan for:

- `add d, l, r` at `i`, `cmp d, X` at `i+1` with `X ∈ {l, r}` and `d ≠ X`,
  `b.lo L` at `i+2`; or
- `add_imm d, s, k` at `i`, `cmp d, s` at `i+1` with `d ≠ s` and
  `Backend::adds_imm_encodable(k)`, `b.lo L` at `i+2`;

and `dead_after[i+2]` (flags dead on both edges of the branch). Rewrite: `i` →
`adds d, l, r` / `adds_imm d, s, k` (same `source`), delete `i+1`, and `i+2` →
`active_backend().carry_set_branch()` with the same `target` (AArch64 `b.hs`,
x86-64 `b.lo`). Count one per rewrite in `stats::CHECKED_OPS_FUSED`.

Placement in `machine::finish`: after plan-171-G's combine row and plan-171-B's
compare-and-branch row, before plan-171-H's shared exits (which compares exit text
and must see the final instructions).

### 4.3 The emulator

`tests/common/ncode_emu.rs` keeps "the last compare's operands" as its flag
model. Add a carry fact: `adds`/`adds_imm` record `(sum, carried)`; `b.hs` after
an add reads `carried`, `b.lo` after an add on an x86 ABI reads `carried`, and
after a compare both keep their current unsigned meaning. An emulated function
that now contains the fused form would otherwise panic or mis-branch.

## Phases

> **NOTE — keep the checkboxes current** (template rules).

### Phase 1 — the ops

- [ ] `adds_imm` and `b.hs` through §2.3's tables, the `mir_ops!` mirror list, the
      three encoders (§4.1), relaxation, `flag_preserving`, `flagslive`, and
      `Backend::adds_imm_encodable` / `Backend::carry_set_branch` /
      `Backend::has_carry_flag` with their three impls.
- [ ] Tests: AArch64 encoder round trip (`adds_imm` at `0`, `4095`, `4096`,
      `0xfff000`, refusal at `4097`; `b.hs` near and relaxed); x86-64 encoder
      bytes (`imm8`, `imm32`, `d ≠ s` move, `jae` short/near); riscv64 encoder
      refusal of `adds_imm`; riscv64 select of `b.hs` (int and float);
      `flagslive` unit (an `adds_imm` kills, a `b.hs` reads); emulator (§4.3).

Acceptance: every backend encodes or refuses the ops exactly as §4.1 says.
  Check: `cargo test --bin mfb -- adds_imm b_hs flagslive` → pass (est. 3 min);
  `cargo test --release --test app_special_keys --test codegen_canvas_metal_off_frame_path --test codegen_canvas_vulkan_off_frame_path`
  (the three binaries that use `ncode_emu`, `rg -l ncode_emu tests`) → pass
  (est. 5 min); `scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`
  (est. 20 min — the only proof that adding the ops moved no `-O1` byte).
Commit: —

### Phase 2 — the row

- [ ] `src/optimizer/opt2/checkfuse.rs` (§4.2), called from `machine::finish`;
      catalog row "Checked-operation fusion" (level 2, stage `machine`);
      `stats::CHECKED_OPS_FUSED`.
- [ ] Unit tests `src/optimizer/opt2/checkfuse/tests/mod.rs`: `add` and `add_imm`
      forms fuse; `cmp d, l` (compare with lhs) fuses; declines when `d` equals the
      compared operand, when the immediate is not encodable, when a flag reader
      follows on the fallthrough edge, when one follows at the target, when the
      branch is not `b.lo`, when an instruction sits between; riscv64 and `-O1`
      untouched.
- [ ] `tests/runtime/rt_codegen_forms.rs::checked_op_fusion`: an `-O2` aarch64
      dump of a program that appends in a loop has `adds` + `b.hs` at the append's
      size check and no `cmp` there; the `-O1` dump keeps `add; cmp; b.lo`; the
      linux-riscv64 `-O2` dump has no `adds_imm`.
- [ ] `rt_regalloc_levels_agree`: add `tests/rt-behavior/security/allocator-02-size-overflow`
      (it drives the checked size add to its overflow edge —
      `strings::repeat("ab", i64::MAX)` raises 77050002 at the per-site check — and
      then churns the heap), compared across `-O0`..`-O3`; and a collection-growth
      loop fixture (`tests/rt-behavior/collections/append-growth-levels`: append
      100,000 Integers and 10,000 Strings, print `len` and a checksum).
- [ ] Gate: `scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` with no
      regeneration (the goldens are `-O1`; Level 2 must not move them).
- [ ] Measure: rerun `/tmp/p188/probe.py`'s pattern (ported into
      `tools/ncode-census/census.py` as metric `wrap_check` in this phase) with
      `-O2` on macos-aarch64 and linux-x86_64; record before/after in Corrections.
- [ ] Runtime: `bash scripts/test-accept.sh target/release/mfb /tmp/188b-accept`;
      `linux-runtime-proof.sh` is `-O1`, so instead build the
      `rt_regalloc_levels_agree` fixtures with `-O2 -target` each of
      linux-aarch64 (run on 2223), linux-x86_64 (2227), windows-x86_64
      (192.168.1.206) and compare with `-O0`.

Acceptance: the idiom is fused at `-O2` on AArch64 and x86-64 and every output is
unchanged.
  Check: `cargo test --bin mfb opt2::checkfuse` → pass (est. 2 min);
  `cargo test --release --test rt_codegen_forms checked_op_fusion` → pass (est. 2 min);
  the three remote `-O2`/`-O0` comparisons → equal (est. 15 min; the only execution
  of fused code on Linux and Windows).
Commit: —

### Phase 3 — docs

- [ ] `mfb spec architecture aarch64-instruction-set`, `x86_64-instruction-set`,
      `riscv64-instruction-set` (refusal), `mir-instruction-set`: the two ops.
      `mfb spec architecture native`: the row in the machine hook paragraph.
- [ ] `.ai/codegen-invariants.md`: "after an `adds` the carry branch is `b.hs` on
      AArch64 and `b.lo` on x86-64 — condition names describe a compare; ask
      `Backend::carry_set_branch()`".
- [ ] `planning/optimizations.md`: row "Checked-operation fusion" `Imp` → `~` with
      the landing note (the unsigned-wrap idiom only), and the checked-op clause of
      "Carry / borrow chain formation" pointed here.

Acceptance: `target/release/mfb man optimizations | grep -c 'Checked-operation fusion'` → 1 (est. 1 min).
Commit: —

## Validation Plan

- Tests: Phase 1/2 units; `rt_codegen_forms::checked_op_fusion`; the rt-error and
  levels-agree fixtures.
- Coverage check: `mfb build -O2 -v` on the levels-agree fixture prints a non-zero
  "Checked-operation fusion" count.
- Runtime proof: Phase 2's remote `-O2` runs.
- Doc sync: Phase 3.
- Final gate (once): `bash scripts/test-fast.sh`; artifact gate `0 diff(s)`;
  `test-accept.sh` full; `cargo test --workspace --no-fail-fast` → `EXIT=0`
  (est. 60–90 min).

## Decisions

- Level 2 — the catalog's level, and the 2026-10-24 default-level exception does not
  apply: the row needs flags liveness, a CFG dataflow (criterion 2).
- No `b.cs` op; the carry condition is chosen per backend (§3).
- riscv64 is out of scope — nothing to fuse (§2.1).

## Corrections

(Filled in during execution.)

## Summary

A small, local rewrite whose risk is two guards (alias, flags liveness), both
unit-pinned. Touches the op tables once, after plan-171 has finished reshaping
them; letter C reuses `b.hs`.
