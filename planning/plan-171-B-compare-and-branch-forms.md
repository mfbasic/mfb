# plan-171-B: Compare-and-branch forms (`cbz`/`cbnz`/`tbz`/`tbnz`, x86 `test`, rv64 bit tests)

Last updated: 2026-10-26
Effort: large (3h–1d)
Depends on: plan-171-A (the census tool, `tests/runtime/rt_codegen_forms.rs`,
and the end-of-`finalize_frame` machine hook it creates)

Every `IF`, every result-tag check after a call, and most runtime guards lower
to "compare against zero, branch on equal/not-equal": `rg -U -P --count-matches`
finds 1802 source sites where `abi::compare_immediate(x, "0")` is followed on the
next line by `abi::branch_eq`/`branch_ne` (856 in `src/codegen/builtins`, 112 in
`src/codegen/runtime`, 332 in `src/target`). On AArch64 each is `cmp x, #0;
b.eq L` — two instructions where `cbz x, L` is one; a single-bit test is
`mov_imm m, 2^k; and t, x, m; cmp t, #0; b.ne L` — four where `tbnz x, #k, L` is
one. This sub-plan adds `cbz`/`cbnz`/`tbz`/`tbnz` to AArch64 at every layer
(op, encoder, decoder, relaxation, a flags-liveness analysis and a machine row
that forms them), gives x86-64 the `test` encodings and riscv64 a
shift-to-sign-bit bit test through the same neutral ops, and converts the code
generators that bypass the machine row.

Outcome: on every target, a compare-with-zero or single-bit test whose flags are
dead after the branch is emitted in the target's shortest compare-and-branch
form, and every runtime fixture's output is unchanged.

References: plan-171-A's list, plus
- `src/arch/aarch64/encode/relax.rs:relax_conditional_branches`, `is_conditional_branch`, `IMM19_LIMIT` (bug-445 veneers).
- `src/arch/aarch64/encode/emitter.rs:patch_labels`, `emit_label_branch`; `sizing.rs:branch_imm19`.
- `src/codegen/engine/regalloc/analysis.rs:is_block_terminator`, `is_unconditional_terminator`, `build_cfg`.
- `src/optimizer/opt2/flags.rs` (the forward flags-availability walk; its notes on which ops write EFLAGS).
- `src/codegen/engine/mir/mir.rs:is_flag_reading_branch`, `fused_variant`.
- `src/arch/x86_64/encode/emitter.rs:enc_alu_imm32`, `jmp_label`, `patch_labels`.
- `src/arch/riscv64/select.rs:expand_fused`, `int_branch`; `src/arch/riscv64/encode/emitter.rs:emit_rv_br`; `src/arch/riscv64/encode/relax.rs:relax_rv64_branches`.
- `.ai/arch-abi.md` "riscv64 branch relaxation", "riscv64 lowering scratch vs. the allocator's spill code" (bug-749/746).

## Prerequisites

See plan-171-A §Prerequisites (stated once for the family). In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-171-A complete | `ls planning/plan-171-A-* 2>/dev/null` → no match (archived to `planning/completed/`) | MET in substance (2026-10-26, `ba1964e81`): the file is still in `planning/` — it is archived when its branch is merged — but `grep -c -- '- \[ \]'` → 0 unticked boxes, `grep -c '^Commit: —'` → 0, and its Summary records every phase landed on main |
| (plan-171-A family row) the decoder has no uncommitted work | `git -C <tree> status --short src/arch/aarch64/decode` → no output | MET (2026-10-26, `/tmp/171b-base` detached at `ba1964e81`: no output) |
| (plan-171-A family row) `src/docs/**` has no uncommitted work | `git -C <tree> status --short src/docs` → no output | MET (2026-10-26, same tree: no output) |
| (plan-171-A family row) the bin unit tests pass at the starting commit | `cargo test --bin mfb > /tmp/171b-pre.log 2>&1; echo EXIT=$?` | see Correction 2 |
| (plan-171-A family row) the artifact gate is clean at the starting commit | `bash scripts/artifact-gate.sh <release mfb of ba1964e81> all` from `/tmp/171b-base` | MET (2026-10-26: `1723 tests, 1903 build(s), 2389 golden(s) checked, 0 diff(s)`) |

## 1. Goal

- AArch64: `cmp_imm x, 0` + `b.eq`/`b.ne` whose flags are dead on both edges
  becomes `cbz`/`cbnz`; the single-bit-test shapes of §4.3 become `tbz`/`tbnz`;
  out-of-range forms are relaxed (`cbz` ±1 MiB, `tbz` ±32 KiB) instead of failing.
- x86-64: `cmp_imm r, 0` encodes as `test r, r` (identical flags, 3 bytes instead
  of 7); single-bit tests become `test r, imm32` or `bt r, k` + `jc`/`jnc`.
- riscv64: single-bit tests become `andi`+`bne` (bit ≤ 10) or `slli`+`blt`/`bge`
  (any bit); compare-with-zero is already native (§2.3), so it is unchanged.

### Non-goals

- No change to `MATCH`/`IF` semantics, trap order, or any value.
- `cbz`/`tbz` never appear in the register allocator's input (they name a
  register at a terminator — the bug-746 hazard class; §3).
- No change to riscv64 branch *size* (8-byte long form stays; sub-plan K).

## 2. Current State

### 2.1 AArch64

No `cbz`/`cbnz`/`tbz`/`tbnz` anywhere (`rg -l 'cbnz|tbz|tbnz' src` → 0 files
outside the decoder tests). Conditional branches are the 12 `b.<cond>` CodeOps.
Labels are patched in `patch_labels` (`imm26` for `b`, `imm19<<5` for `b.cond`;
out of range is an error), and `relax_conditional_branches` (bug-445) rewrites
an out-of-range `b.cond far` into `b.cond Ltramp; b Lcont; Ltramp: b far; Lcont:`
— keeping the condition, so no inverse opcode is needed. That rewrite works
unchanged for `cbz`/`tbz` (§4.4).

The emitted census (committed goldens, macos-aarch64; `cmp_imm` with
`"rhs": "0"` directly followed by `b.eq`/`b.ne`) — re-measured at `ba1964e81`
(Correction 1; the 2026-10-04 table was stale):

| `.ncode` golden | instrs | `cmp #0; b.eq/ne` | `cmp #0; b.lt/ge` | bit tests |
|---|---|---|---|---|
| `func_map_getor_hash_probe` | 16382 | 460 | 19 | 12 |
| `macos_app_mode_io` (`--app`) | 19096 | 468 | 66 | 28 |
| `macos_app_mode_plumbing` (`--app`) | 10229 | 308 | 40 | 25 |
| `list_ops_codegen_rt` | 8100 | 265 | 13 | 12 |
| `control_flow_match` | 5688 | 215 | 10 | 12 |
| `control_flow_if` | 5648 | 214 | 11 | 12 |
| `parser_hello_world` | 5185 | 190 | 10 | 12 |

(`python3 -I tools/ncode-census/census.py <golden>`, metrics `instructions`,
`cmp0_eq_branch`, `cmp0_sign_branch`, `single_bit_test`.)

Corpus-wide (`tools/ncode-census/ncode-census.sh <release mfb of ba1964e81>
<target> …`, 56 projects; riscv64 builds 43 — twelve app-mode refusals and
bug-1112's panic; committed as `tools/ncode-census/baseline-<target>.tsv`):

| metric | macos-aarch64 | linux-aarch64 | linux-x86_64 | linux-riscv64 |
|---|---|---|---|---|
| `instructions` | 27,057,789 | 27,024,835 | 30,519,671 | 4,506,516 |
| `cmp0_eq_branch` (`cmp_imm r, 0; b.eq/ne`) | 1,052,180 | 1,051,245 | 1,051,830 | — (`rv.br`) |
| … with the flags dead after the branch | 1,052,063 | 1,051,175 | 1,051,760 | — |
| `cmp0_sign_branch` (`cmp_imm r, 0; b.lt/ge`) | 25,061 | 24,863 | 24,863 | — |
| … with the flags dead | 25,009 | 24,790 | 24,790 | — |
| `cmpz_eq_branch` (`mov_imm z, 0; cmp r, z; b.eq/ne`) | 2,486 | 2,486 | 2,486 | — |
| `single_bit_test` (`mov_imm m, 2^k; and t, r, m; cmp_imm t, 0; b.eq/ne`) | 5,618 | 5,512 | 4,816 | — |
| `single_bit_test_reg` (the zero in a register) | 30 | 30 | 30 | — |
| `rv_single_bit_test` (`…; and; [mov_imm z, 0;] rv.br eq/ne t, zero/z`) | — | — | — | 2,171 |
| `rv_zero_reg_branch` (`mov_imm z, 0; rv.br eq/ne r, z`) | — | — | — | 505 |

The flags are dead after **99.99 %** of the compare-with-zero branches
(`cmp0_eq_branch_flags_dead`, the same backward rule as §4.2 run over the
dumps by `census.py:flags_dead_after`); the live remainder is the `share`
chains of §2.4. User `IF x = 0` / `IF x <> 0` does not produce `cmp_imm`: the
zero is a materialized register (`mov_imm x10, 0; cmp x9, x10; b.ne`, probe
`/tmp/171b-probe`, every target), the `cmpz_eq_branch` shape — so the row
also canonicalizes a compare against a register known to hold zero (§4.3,
Correction 4).

### 2.2 x86-64

`cmp_imm` with any value, and `cmp` with a zero rhs, go through `enc_alu_imm32(7, …)`
— always `REX.W 81 /7 imm32` (7 bytes). Its doc comment says the imm32 form is
used "for a value-independent size". **Measured (Phase 1): no constraint
requires it.** `git log -S 'value-independent size' -- src/arch/x86_64` names
`26564a12e` (plan-00-H Phase 1, the first x86 encoder) and `d70bef28c` (the
plan-176-O file split, motion only). `26564a12e`'s message states the actual
invariant — "Single encode path feeds both instruction_size and emit (no
size-table drift); every encoding fixed-size so label offsets never shift" —
and its module doc says "fixed-size and *distance*-independent (rel32,
imm32/imm64, disp32), so a size never depends on a label that hasn't been
placed yet". An immediate is part of the instruction, so `sizing.rs`
(`encode_instruction(instruction)?.bytes_len()`) and the emitter see the same
value: a value-dependent size cannot drift. Only a size that depends on a
*label* would. No bug doc cites it (`grep -rn "value-independent" bugs
planning` → only this plan). No `test`, no `bt` (`rg -c 'test' emitter.rs` hits only
a comment). Conditional branches are always `0F 8x rel32`.

### 2.3 riscv64

`select_riscv64::expand_fused` turns `br_cc_imm r, 0` into `rv.br` with the
`zero` register — a native `beq r, zero`/`bne r, zero` (encoded as the inverted
8-byte long form, `emit_rv_br`). So `cbz` has no riscv64 gap. A single-bit test
is `li m, 2^k` (1–2 instructions), `and t, r, m`, `rv.br ne t, zero`. A user
`IF x = 0` keeps its zero in a register (`mov_imm t5, 0; rv.br ne t4, t5`,
`rv_zero_reg_branch` 505 in the corpus), which `rv.br` could name `zero` for.

### 2.4 Flags knowledge

Nothing models flag *liveness*. `opt2/flags.rs` is a forward *availability* walk
(which comparison the flags hold); `flag_preserving`
(`opt2/plans/mark.rs`) lists ops that leave flags intact. The MIR is flagless by
construction (fused ops), but the post-selection stream is not: a fused op with
the `share` field expands to `cmp; b.lo A; b.hi B`, where the second branch
reads the first branch's flags. Folding `cmp #0; b.eq` into `cbz` there would
leave `b.hi` reading stale flags — so a liveness proof is required, not optional.

### Verified properties

- *`relax_conditional_branches` keeps the condition and rewrites only `target`* — read its module doc and `relax_function`.
- *The CFG's successor edge comes only from the `target` field of an op in `is_block_terminator`* — read `build_cfg`; a new branch op missing from that list loses its edge (the x86 float-branch comment in `is_block_terminator` records the miscompile that caused).
- *Flags never need to survive a call* — stated in `.ai/arch-abi.md` "riscv64 flag-emulation reserved slots"; the analysis treats calls as transparent anyway (conservative), so it does not rely on this.
- *Whether every builder stream reaches `finalize_frame`* — not every one does (plan-171-A Correction 11: `_mfb_macapp_*`, `_main`, the thread trampoline); Phase 5's sweep lists the emitters that do not, and Phase 4 places the row where they all pass (Correction 5).

## 3. Design Overview

The forms are produced **after register allocation**, by a machine row run in
the end-of-`finalize_frame` hook plan-171-A created. Reasons: (1) the operands
are physical, so the allocator never sees a terminator that names a register
(the bug-746 class: an eviction restore around such a branch must land on both
edges); (2) the fold needs flags liveness over the final stream, which is
stable only after allocation; (3) one choke point covers user code and every
vreg-allocated runtime helper.

Four neutral `CodeOp`s — `cbz`, `cbnz`, `tbz`, `tbnz` — each backend encodes in
its own way (AArch64 natively; x86 as `test`/`bt` + `jcc`; rv64 as
`andi`/`slli` + branch). They are flag-*clobbering* on x86 and flagless on
AArch64; since the row only forms them where flags are dead, both are correct.

**Correctness risk** concentrates in the flags-liveness analysis (a missed
reader = a silently wrong branch) and in relaxation of the ±32 KiB `tbz`
(functions of ordinary size exceed 32 KiB). **Design uncertainty**: the x86
"value-independent size" rationale (Phase 1 decides whether x86 `test` is an
encoder change or goes through the row).

**Gate class:** behavior-changing. Expected `.ncode` diffs: every aarch64 golden
(cbz everywhere), every x86 golden (`test`), and riscv64 goldens only where a
single-bit test exists. Phases 2–3 (primitive, analysis) are byte-identical.

**Rejected alternatives.**
- *Select `cbz` in `select_aarch64` from `br_cc_imm` with imm 0* — pre-allocation, so the allocator would meet a register-naming terminator (bug-746), and `share` chains are not visible per instruction.
- *Builder constructors that emit `cbz` directly* — same allocator hazard; allowed only in streams that are never allocated (Phase 5).
- *Invert the condition when relaxing* — not needed; bug-445's keep-the-condition trampoline already handles it.

## 4. Detailed Design

### 4.1 Ops and encodings

| op | fields | AArch64 | x86-64 | riscv64 |
|---|---|---|---|---|
| `cbz` / `cbnz` | `register`, `target` | `CBZ/CBNZ Xt, imm19` | `test r,r; je/jne rel32` | not produced (§2.3) |
| `tbz` / `tbnz` | `register`, `bit`, `scratch`, `target` | `TBZ/TBNZ Xt, #b, imm14` | `bit ≤ 30`: `test r, imm32; je/jne`; else `bt r, imm8; jnc/jc` | `bit ≤ 10`: `andi s, r, 1<<b; beq/bne s, zero`; else `slli s, r, 63-b; bge/blt s, zero` (long form as `rv.br`) |

`scratch` is the dead register the folded pattern freed (§4.3); AArch64 and x86
ignore it. `register` is already a USE field; `scratch` is added to `DEF_FIELDS`
(it is written on rv64), `bit` is an immediate. All four join
`is_block_terminator` (conditional), `is_flag_reading_branch` stays unchanged
(they read no flags), and `flag_preserving` lists them as **not** preserving on
x86 (they clobber EFLAGS there) — on AArch64 they preserve.

### 4.2 Flags liveness

`optimizer::opt2::plans::flagslive::dead_after(instructions) -> Vec<bool>`: a
backward dataflow over `build_cfg`. Per instruction: a flag **reader** (every
`b.<cond>`, `X86J*`, and — from sub-plan D — `cset`/`csel`) makes flags live
before it; a definite **setter** (`cmp`, `cmp_imm`, `adds`, `subs`, `fcmp_d`,
`fcmp_zero_d`) kills; everything else, calls included, is transparent (keeps
liveness — the conservative direction). Block exit liveness = union over
successors; `ret`/`branch_self` exit dead. The fold at `(cmp, b.cc)` requires
flags dead *after the branch on both edges* — i.e. dead at the fallthrough
successor's entry and at the target's entry.

### 4.3 The machine row "Compare-and-branch formation" (Level 1)

Runs in the end-of-`finalize_frame` hook, after pairing. Per backend pattern:

- **cbz (AArch64 only):** `cmp_imm r, 0; b.eq L` → `cbz r, L`; `b.ne` → `cbnz`.
- **tbz (AArch64, x86):** `mov_imm m, 2^k; and t, r, m` (either operand order),
  `cmp_imm t, 0; b.eq|b.ne L`, with `m` and `t` dead after the branch (physical
  liveness from `regalloc::analysis::Liveness` re-run on the final stream) →
  `tbz|tbnz r, k, L` (scratch `t`). `m` may be defined earlier in the block if no
  instruction in between writes `r`, `m` or `t`.
- **tbz (riscv64):** `mov_imm m, 2^k; and t, r, m; rv.br ne|eq t, zero, L` with
  `m`, `t` dead → `tbnz|tbz r, k, L, scratch t`. No flags on rv64, so only
  register liveness applies.

Sub-plan C extends the matcher to the `and_imm t, r, #2^k` shape it introduces.
Every fold increments the row's counter.

### 4.4 Relaxation

AArch64 `relax.rs`: `is_conditional_branch` → a `reach(op)` returning
`IMM19_LIMIT` for `b.cond`/`cbz`/`cbnz` and `1 << 15` for `tbz`/`tbnz`; the
trampoline rewrite is unchanged (`cbz r, Ltramp; b Lcont; Ltramp: b far; Lcont:`).
`patch_labels` gains an `imm14` patch kind (`tbz`) and range-checks it. x86
needs no relaxation (rel32). riscv64 `relax.rs`: `tbz`/`tbnz` contain a `jal` at
a fixed offset inside their expansion exactly like `rv.br`; teach the hop-island
pass that offset (it rewrites only `target`).

### 4.5 x86 `test` for `cmp_imm r, 0`

`cmp r, 0` and `test r, r` set ZF, SF, PF from `r`, clear CF and OF; only AF
differs and nothing reads AF. So if Phase 1 finds no real value-independence
constraint, the x86 encoder emits `REX.W 85 /r` for `cmp_imm r, 0` — every
condition stays correct, no liveness needed. If the constraint is real, x86
instead gets `cbz`/`cbnz` from the row (§4.3) under the same flags proof.

**Decided (Phase 1): the encoder branch.** §2.2 found no value-independence
constraint, so the x86 encoder emits `test r, r` for `cmp_imm r, 0` and for
`cmp r, <zero token>`, at every level, every condition unchanged; the row forms
no `cbz`/`cbnz` for x86, and the x86 encoder refuses them like `stp`/`ldp`.

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — measure

- [x] Run `tools/ncode-census` on the corpus for `macos-aarch64`,
      `linux-x86_64`, `linux-riscv64`; record `cmp#0+b.eq/ne` and single-bit-test
      counts in §2 (replacing the not-yet-measured markers). (Also
      `linux-aarch64`. `census.py` gained the metrics `cmp0_*_flags_dead`,
      `cmpz_eq_branch`, `single_bit_test_reg`, `rv_single_bit_test`,
      `rv_zero_reg_branch`; the four baselines were re-taken at `ba1964e81`.)
- [x] Find why `enc_alu_imm32` insists on a value-independent size
      (`git log -S 'value-independent size' -- src/arch/x86_64`, read the commit
      and any bug doc it cites); record the answer here and decide §4.5's branch.
      (§2.2: no constraint; §4.5 takes the encoder branch.)

Acceptance: §2 carries measured counts and §4.5 names its branch with evidence.
  Check: `sed -n '/^## 2/,/^## 3/p' planning/plan-171-B-*.md | grep -c -e UNMEASURED -e UNVERIFIED`
  → 0 (Correction 3: the original whole-file grep matched its own spelling) (est. 10 min of census runs).
  Result (2026-10-26): `0`.
Commit: —

### Phase 2 — the four ops (no producer; byte-identical)

- [ ] `src/arch/ops.rs`, `mir.rs` (`mir_ops!` mirror group), `regalloc/analysis.rs`
      (`scratch` → `DEF_FIELDS`; the four ops → `is_block_terminator`),
      `opt2/plans/mark.rs:flag_preserving` (x86: not preserving).
- [ ] AArch64 encoder/sizing (`CBZ`/`CBNZ` imm19, `TBZ`/`TBNZ` b5:b40 + imm14),
      `patch_labels` imm14 kind; decoder table rows.
- [ ] x86 encoder: `test r,r`+jcc; `test r,imm32`/`bt r,imm8`+jcc expansions.
- [ ] riscv64 encoder: `andi`/`slli` + long-form branch expansions; sizing.
- [ ] A debug assertion at the head of `regalloc::allocate`: no `cbz`/`cbnz`/`tbz`/`tbnz`
      in the input (with a `#[should_panic]` unit test).
- [ ] Tests: per-backend encoder unit tests incl. bit 0, 10, 11, 30, 31, 63 and
      registers r8–r15/x0–x30/t-regs; AArch64 decoder round trip;
      `build_cfg` test: a block ending in `cbz` has both successor edges.

Acceptance: round trips pass; no emitted byte changes.
  Check: `cargo test --bin mfb aarch64:: x86_64::encode riscv64::encode regalloc::analysis` → pass (est. 4 min);
  `bash scripts/artifact-gate.sh target/release/mfb strings` → `0 diff(s)` (est. 1 min).
Commit: —

### Phase 3 — flags liveness (analysis only; byte-identical)

- [ ] `src/optimizer/opt2/plans/flagslive.rs` (§4.2) with unit tests: `share`
      chain (`cmp; b.eq; b.lt` → not dead), reader across a label (live),
      setter on one path only (live), call between (transparent), back edge.

Acceptance: the analysis answers every unit case; nothing calls it yet.
  Check: `cargo test --bin mfb flagslive` → pass (est. 2 min).
Commit: —

### Phase 4 — the row, relaxation, x86 `test`

- [ ] `src/optimizer/opt2/cbranch.rs`: the row (§4.3), called from the A hook;
      catalog row "Compare-and-branch formation" (level 1, stage `machine`).
- [ ] Relaxation (§4.4) on AArch64 and riscv64, with tests: a `tbz` to a label
      40 KiB away encodes through the trampoline; a `cbz` 1.5 MiB away likewise;
      a riscv64 `tbz` 1.5 MiB away gets a hop island.
- [ ] x86 per Phase 1's decision (§4.5), with an encoder test asserting the 3-byte form.
- [ ] Fixtures: `tests/rt-behavior/control-flow/branch-zero-and-bit-tests` (valid)
      — `IF x = 0`, `IF x <> 0`, `bits::band(x, 1 << k) <> 0` for k ∈ {0, 10, 11,
      30, 31, 62, 63}, a function large enough (generated straight-line body,
      > 64 KiB of code) that a `tbz` must relax, printing the taken branches.
- [ ] `tests/runtime/rt_codegen_forms.rs`: `cbz` and `tbnz` present in the
      aarch64 dump of that fixture; `test` on x86; `slli`-based test on rv64 is
      checked by op name in the `-ncode` dump.
- [ ] `rt_regalloc_levels_agree`: add the fixture (`-O0` forms nothing).
- [ ] Golden re-baseline for this form family (aarch64 + x86 + rv64 where
      predicted), same procedure as plan-171-A Phase 3 (four questions, full suite
      first, `regen-native-goldens.sh`, classify by target).
- [ ] Runtime: macOS acceptance; `linux-runtime-proof.sh` on 2223
      (`linux-aarch64`), 2227 (`linux-x86_64`, emulated — needed because x86
      bytes changed), 2229 (`linux-riscv64`); the fixture built for
      `windows-x86_64` and run on box 206 (`scp` + `ssh test@192.168.1.206 … .\fixture.exe`),
      output compared with its golden `build.log` run section.
- [ ] Benchmark on the macOS host (`./benchmark/language/run.sh 10`; `rank.py`); x86/rv64
      are compared by census instruction counts only (their boxes are emulated,
      so timings there measure QEMU).

Acceptance: forms appear where predicted; all runtime outputs unchanged on
five targets.
  Check: `cargo test --release --test rt_codegen_forms` → pass (est. 3 min);
  `linux-runtime-proof.sh` ×3 → 0 mismatches (est. 25–60 min each; the only checks that execute each ISA's new encodings).
Commit: —

### Phase 5 — code-review sweep (this form)

- [ ] Census diff (`census.py --diff` A-baseline vs now) per target: count of
      remaining `cmp #0; b.eq/ne` and bit-test shapes, and why each class
      remained (flags live, not adjacent, emitted outside `finalize_frame`).
- [ ] List emitters that never reach `finalize_frame` (hand-written physical
      streams: `rg -L 'finalize_(frame|vreg_body|vreg_helper)' $(rg -l 'compare_immediate' src/codegen src/target)`
      then read each hit); for each, either route it through the row's function
      directly (it is a pure stream transform) or record why not.
- [ ] Read the 20 functions with the most remaining candidates (census per-function
      output); where a builder computes a bit test in a register-shifting form the
      row cannot see (`lsr; and #1; cmp`), rewrite the builder to the
      `mov_imm 2^k; and` shape or a direct bit-test constructor usable only in
      unallocated streams. "Makes sense" = the rewrite removes ≥ 2 instructions
      on the hot path and keeps the trap order. Record converted/declined sites.

Acceptance: every remaining candidate class is explained in Corrections.
  Check: `python3 tools/ncode-census/census.py --diff …` output pasted in Corrections (est. 15 min).
Commit: —

### Phase 6 — spec and docs

- [ ] `mfb spec architecture aarch64-instruction-set` (ops, imm14/imm19 patches,
      relaxation of the new forms), `x86_64-instruction-set` (`test`/`bt`
      expansions), `riscv64-instruction-set` (bit-test expansion, relaxation),
      `mir-instruction-set` (four mirror ops; never pre-allocation).
- [ ] `.ai/arch-abi.md` "A single function over ~1 MiB of code fails to build"
      — stale since bug-445 added veneers; correct it while editing relaxation.
- [ ] `planning/optimizations.md`: "Instruction selection / combining" note.

Acceptance: specs render cleanly.
  Check: `cargo test --bin mfb spec` → pass; `mfb spec architecture --all | grep -c '\[\['` → 0 (est. 2 min).
Commit: —

## Validation Plan

- Tests: encoder/decoder units, `flagslive` units, row units (`cbranch.rs`), relax
  tests, `rt_codegen_forms`, the new rt-behavior fixture, `rt_regalloc_levels_agree`.
- Coverage: the row runs on every allocated function at `-O1`; `-vv` shows its
  counter on the fixture.
- Runtime proof: Phase 4's five-target runs.
- Doc sync: Phase 6.
- Final gate (once): `cargo test --no-fail-fast > /tmp/171b.log 2>&1; echo EXIT=$?` → 0, `grep -c '^failures:'` → 0 (est. 60–90 min).

## Open Decisions

1. x86 `test`: encoder-level (recommended — covers every compare with zero) vs.
   row-formed `cbz` — decided by Phase 1's finding. (§4.5)
2. Bit-test registers on rv64: reuse the dead `t` (recommended — needs no
   reserved register) vs. lowering scratch `t1` (works, but adds a hidden use).

## Corrections

1. **§2.1's golden table was stale.** The seven `.ncode` goldens were
   regenerated by every codegen change since 2026-10-04 (call-graph DCE, plans
   175/182/183, plan-171-A's pairs), so `control_flow_if` is now 5648
   instructions with 214 `cmp #0; b.eq/ne` (was 2598 / 109). §2.1 carries the
   re-measured table and the corpus totals; the design does not depend on the
   old numbers.
2. **Prerequisite "bin unit tests pass at the starting commit"** — see the
   Prerequisites row; result recorded below when the run finished.
3. **Phase 1's check matched its own spelling.** `grep -c UNMEASURED
   planning/plan-171-B-*.md` counts the task line that says "replacing
   UNMEASURED" and the check line itself, so it could never reach 0. The check
   now greps §2 (where the markers lived) for both markers, `UNMEASURED` and
   `UNVERIFIED`.
4. **The compare-with-zero population is wider than `cmp_imm r, 0`.** User
   `IF x = 0` / `IF x <> 0` / `bits::band(x, 2^k) <> 0` compare against a
   *register* holding zero (`mov_imm z, 0; cmp r, z` on AArch64/x86-64,
   `mov_imm z, 0; rv.br r, z` on riscv64; §2.1/§2.3), which the §4.3 matchers
   as written never see; and `cmp_imm r, 0; b.lt/ge` (25,061 sites, a sign
   test) is a compare with zero whose shortest AArch64 form is `tbnz/tbz r,
   #63`. Both are inside the Outcome ("a compare-with-zero … in the target's
   shortest compare-and-branch form"), so the row also (a) rewrites a `cmp`
   whose operand is known zero (an earlier `mov_imm z, 0` in the block, `z`
   not redefined since) to `cmp_imm r, 0` — and riscv64's `rv.br` operand to
   `zero` — deleting the `mov_imm` when `z` is dead, and (b) forms the sign
   test on AArch64 (x86-64 gets it from the `test` encoding; riscv64's
   `rv.br lt/ge r, zero` is already one instruction).
5. **The row runs at the end of `route_function_through_mir`, not in the
   end-of-`finalize_frame` hook.** At `finalize_frame` a hand-written helper's
   stream on x86-64 and riscv64 is not yet selected: `finalize_vreg_body*`
   allocates and frames it without `Backend::select`, and the selection that
   realizes it runs later, in `mir::route_function_through_mir` (called for
   every function by `lower_module`). Until then it still carries neutral
   `cmp`/`b.cc` pairs (riscv64 turns them into `rv.br` there) and AArch64
   scratch spellings `x9`–`x30` that `select_x86`'s `realize_x86_residual` and
   `select_riscv64`'s `map_scratch_register` move onto other registers
   ("post-allocation, so these are fixed physicals", `riscv64/select.rs`), next
   to registers the allocator colored with the target's own names. A register
   liveness proof over those spellings could treat two names of one register
   as two registers. After `route_function_through_mir`'s `select` the stream
   is final — nothing re-selects, re-allocates or re-frames it before the
   `-ncode` dump and the encoder — so the row runs there (a second machine
   hook, `opt2::machine::finish_selected`). Two more gains: the allocator can
   never meet the new ops (the stream it sees is long gone), and the hook
   covers the hand-written bodies that never reach `finalize_frame`
   (`_mfb_macapp_*`, `_main`, the thread trampoline; plan-171-A Correction 11),
   which Phase 5 would otherwise have to route by hand.

## Summary

Risk is the flags-liveness proof and `tbz`'s 32 KiB reach; both are isolated
behind unit tests and a relaxation test that forces the trampoline. The
register allocator never sees the new terminators.
