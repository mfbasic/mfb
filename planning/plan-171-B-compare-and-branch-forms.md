# plan-171-B: Compare-and-branch forms (`cbz`/`cbnz`/`tbz`/`tbnz`, x86 `test`, rv64 bit tests)

Last updated: 2026-10-04
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
| plan-171-A complete | `ls planning/plan-171-A-* 2>/dev/null` → no match (archived to `planning/completed/`) | NOT MET |

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
- No change to riscv64 branch *size* (8-byte long form stays; sub-plan G).

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
`"rhs": "0"` directly followed by `b.eq`/`b.ne`):

| `.ncode` golden | instrs | `cmp #0; b.eq/ne` |
|---|---|---|
| `func_map_getor_hash_probe` | 13918 | 355 |
| `macos_app_mode_io` | 16110 | 361 |
| `macos_app_mode_plumbing` | 7095 | 203 |
| `list_ops_codegen_rt` | 4857 | 152 |
| `control_flow_match` | 2676 | 111 |
| `control_flow_if` | 2598 | 109 |
| `parser_hello_world` | 2049 | 85 |

(`/tmp/ncode_census.py` prototype, reproduced as `census.py` by plan-171-A
Phase 1.) How many of those have dead flags is UNMEASURED; Phase 1 measures it.

### 2.2 x86-64

`cmp_imm` with any value, and `cmp` with a zero rhs, go through `enc_alu_imm32(7, …)`
— always `REX.W 81 /7 imm32` (7 bytes). Its doc comment says the imm32 form is
used "for a value-independent size". Why a value-independent size is needed is
UNVERIFIED — `sizing.rs` computes sizes by emitting into a throwaway encoder
(plan-171-A §2.3), so the value is known when the size is taken; Phase 1 finds
the reason (`git log -S 'value-independent size' -- src/arch/x86_64`) before
anything depends on it. No `test`, no `bt` (`rg -c 'test' emitter.rs` hits only
a comment). Conditional branches are always `0F 8x rel32`.

### 2.3 riscv64

`select_riscv64::expand_fused` turns `br_cc_imm r, 0` into `rv.br` with the
`zero` register — a native `beq r, zero`/`bne r, zero` (encoded as the inverted
8-byte long form, `emit_rv_br`). So `cbz` has no riscv64 gap. A single-bit test
is `li m, 2^k` (1–2 instructions), `and t, r, m`, `rv.br ne t, zero`.

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
- *Whether every builder stream reaches `finalize_frame`* — UNVERIFIED; Phase 5's sweep lists the emitters that do not.

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

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — measure

- [ ] Run `tools/ncode-census` on the corpus for `macos-aarch64`,
      `linux-x86_64`, `linux-riscv64`; record `cmp#0+b.eq/ne` and single-bit-test
      counts in §2 (replacing UNMEASURED).
- [ ] Find why `enc_alu_imm32` insists on a value-independent size
      (`git log -S 'value-independent size' -- src/arch/x86_64`, read the commit
      and any bug doc it cites); record the answer here and decide §4.5's branch.

Acceptance: §2 carries measured counts and §4.5 names its branch with evidence.
  Check: `grep -c UNMEASURED planning/plan-171-B-*.md` → 0 (est. 10 min of census runs).
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
      `windows-x86_64` and run on box 2230 (`scp` + `ssh -p 2230 … .\fixture.exe`),
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

<Filled in during execution.>

## Summary

Risk is the flags-liveness proof and `tbz`'s 32 KiB reach; both are isolated
behind unit tests and a relaxation test that forces the trampoline. The
register allocator never sees the new terminators.
