# plan-171-G: AArch64 addressing and arithmetic forms (register-offset and folded-offset loads/stores, shifted operands, `madd`/`msub`, bitfield extracts, `csinc`/`csneg`)

Last updated: 2026-10-21
Effort: large (3h–1d)
Depends on: plan-171-F (letter order; the rows use plan-171-A's machine hook,
plan-171-C's `and_imm`, and plan-171-D's `csel`/`cset` and `cc` field)

After allocation, the AArch64 stream still spells out, in two instructions, work
the ISA does in one. Examples: an address computed into a temporary register and
then dereferenced (`add t, b, i; ldr x, [t]`); a shift feeding an add
(`lsl t, i, #3; add d, b, t`); a multiply feeding an add (`mul t, a, b;
add d, t, c`); a shift then a mask (`lsr t, s, #k; and d, t, #(2^w-1)`); and a
branch around a single increment or negation. The encoder has no
register-offset load/store, no shifted-operand `add`/`sub` (shift is always 0),
no `madd`, no bitfield extract op, and no `csinc`/`csneg`.

This sub-plan adds those forms as AArch64-only `CodeOp`s (encoder, decoder
round-trip rows, register roles, MIR mirror group) and forms them with one
Level-1 machine row, run from plan-171-A's end-of-`finalize_frame` hook ahead
of pairing. It also folds `add_imm t, b, #k; ldr x, [t, #o]` into
`ldr x, [b, #o+k]`, an encoding that already exists.

Outcome: on `macos-aarch64` and `linux-aarch64` at `-O1` and above, every
adjacent pair matching a rule in §4.2 whose temporary register is dead becomes
one instruction. `-O0` output, x86-64 and riscv64 output are byte-identical to
the start of this sub-plan, and every program's output is unchanged.

References: plan-171-A's list, plus
- `src/arch/aarch64/encode/emitter/mod.rs:Encoder::emit_instruction` (`add`/`sub`
  via `shifted_reg`, `msub`), `emitter/memory.rs:emit_ldr_u64` and siblings,
  `encode/operand.rs:shifted_reg` (rejects `sp`).
- `src/arch/aarch64/decode/integer.rs:add_shifted`, `conditional_select`, the
  data-processing-3 decoder (`madd`/`msub`), the bitfield decoder (`ubfx`/`sbfx`/
  `ubfiz`/`bfi`); `decode/memory.rs:register_offset`.
- `src/optimizer/opt2/peephole.rs:remove_fp_shuttles` (the post-allocation
  precedent: physical registers, `regalloc::integer_live_out`, `physical_busy`).
- `src/codegen/engine/regalloc/analysis.rs:DEF_FIELDS`, `USE_FIELDS`.
- `src/codegen/engine/mir/mir.rs` `mir_ops!` (the "synthesized after MIR
  lowering" entries in the `mirror` group).
- `planning/optimizations.md` rows "Addressing mode optimization",
  "Bit-field extract/insert recognition", "Instruction selection / combining".

## Prerequisites

See plan-171-A §Prerequisites. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-171-F complete | `ls planning/plan-171-F-* 2>/dev/null` → no match | NOT MET |

Everything below assumes A–F have landed: the machine hook
`optimizer::opt2::machine::finish` exists (A), `tools/ncode-census/census.py`
exists (A), `and_imm` exists (C), `cset`/`csel` with a `cc` field exist and
B's flags-liveness reader list includes them (D).

## 1. Goal

- New AArch64-only `CodeOp`s, each one instruction:
  - `ldr_u64_idx`, `ldr_u32_idx`, `ldr_u16_idx`, `ldr_u8_idx`, `ldr_d_idx`,
    `str_u64_idx`, `str_u32_idx`, `str_u16_idx`, `str_u8_idx`, `str_d_idx`:
    `[base, index]` or `[base, index, lsl #log2(size)]` (field `scaled`
    `0`/`1`);
  - `add_shift`, `sub_shift`: `dst = lhs ± (rhs <kind> #shift)`, `kind` ∈
    {`lsl`, `lsr`, `asr`}, `shift` 1–63;
  - `madd`: `dst = addend + lhs*rhs` (fields as `msub`, with `addend` for
    `minuend`);
  - `ubfx`, `sbfx`, `ubfiz`, `sbfiz`: fields `dst`, `src`, `lsb`, `width`;
  - `csinc`, `csneg`: `dst = cc ? lhs : rhs+1` and `dst = cc ? lhs : -rhs`,
    following D's `csel` field convention (`lhs` = value if true).
- One Level-1 machine row, "Instruction combining (machine)", in
  `src/optimizer/opt2/combine.rs`. It runs from `machine::finish` before
  plan-171-A's pairing, gated `level_enabled(1)` and
  `active_backend().supports_combined_forms()` (AArch64 `true`, x86-64 and
  riscv64 `false`). It applies the rules of §4.2.

### Non-goals

- No x86-64 or riscv64 change: those backends never receive the new ops; their
  encoders reject them as producer bugs (C's precedent for non-encodable
  immediates). x86 SIB/`lea` addressing and riscv64 Zba are separate work.
- No change to any value, trap, trap order or flag result. Every rule rewrites
  a pair into one instruction computing the same 64-bit result, and drops only a
  temporary that is dead.
- No `-O0` change (the row is Level 1).
- No `ldur`/`stur` and no `bfi`/`bfxil` (§3, rejected, with their census).
- No shifted-operand `adds`/`subs` (§3, rejected, census 0).
- No tail calls (removed from plan-171 by the owner on 2026-10-21; §3).

## 2. Current State

- **Loads/stores** take an unsigned scaled `imm12` offset only. An unaligned
  offset is an error, and an offset out of range goes through an encoder
  scratch `add` (`emitter/memory.rs:emit_ldr_u64`: `checked_imm12(offset / 8)`,
  else `scratch_excluding` + `emit_add_imm`). There is no register-offset
  form: `rg -n '_idx"' src/arch/ops.rs` → 0.
- **`add`/`sub`** are the shifted-register class with shift 0. `shifted_reg`
  refuses `sp` because register 31 encodes `xzr` there
  (`encode/operand.rs:shifted_reg`).
- **Multiply** has `mul` and `msub` (`dst = minuend - lhs*rhs`, used for
  remainders) but no `madd`: `rg -n '"madd"' src/arch/ops.rs` → 0. On
  riscv64 and x86-64, `msub` expands to multiply + subtract
  (`riscv64/encode/emitter.rs` `"msub"`, `x86_64/encode/emitter/instruction.rs`
  `"msub"`).
- **Bitfields**: `lsl_imm`/`lsr_imm`/`asr_imm` are UBFM/SBFM aliases with a
  fixed width (`emitter/scalar.rs:emit_asr_imm`). No `ubfx`/`sbfx` op exists.
- **Conditional select**: after D, `cset` and `csel` exist; `csinc`/`csneg`
  appear only as the raw `cset` word inside `add_carry`/`sub_borrow`.
- **Decoder**: every form above already decodes as lldb prints it.
  `decode/integer.rs:add_shifted` (`, lsl #3` suffixes), `conditional_select`
  (`csinc`/`csneg` and the `cinc`/`cset`/`cneg` aliases), the data-processing-3
  table (`madd`/`mul`), the bitfield table (`sbfx`/`ubfx`/`ubfiz`/`bfi`/`bfxil`),
  and `decode/memory.rs:register_offset` (`ldr x0, [x1, x2, lsl #3]`). Only the
  round-trip rows in `decode/tests.rs` are new.
- **Register roles**: `USE_FIELDS` has `addend`, `lhs`, `rhs`, `src`, `base`
  but no `index` (`src/codegen/engine/regalloc/analysis.rs`). An `index`
  field must be added so `integer_live_out` (which the hook's later rows use)
  sees the read.
- **Post-allocation precedent**: `remove_fp_shuttles` matches an adjacent pair
  on physical registers, proves the temporary dead with
  `regalloc::integer_live_out` + `physical_busy`, and rewrites. This row takes
  the same shape.

### Measured populations

Corpus: `examples/{brogue,dungeon,life,sorts}` built with
`target/release/mfb build --ncode` (binary `MFBasic Compiler 0.1.0, 2026-10-20
21:12:26 UTC`, tree at `5d6ca88dc`) into scratch copies under `/tmp/p171h/src`,
`macos-aarch64`, default `-O1`. (`yaml-json` did not build: its package is
from an older compiler. `swat` has no `project.json`. `benchmark/language/mfb`
needs its `bench_workers` package built first.) Counted by the prototype in
Phase 1 (`python3 -I g_census.py <dumps>`). Every rule requires the
intermediate register to be dead after the second instruction. The census
proves that with a straight-line scan that treats anything unknown as live,
which is stricter than the row's `integer_live_out`.

| Rule (§4.2) | brogue | dungeon | life | sorts |
|---|---|---|---|---|
| instructions | 548647 | 1087523 | 147756 | 1038182 |
| R1 offset fold `add_imm t,b,#k; M [t]` | 706 | 1187 | 7 | 2057 |
| R2 register offset `add t,b,i; M [t]` | 553 | 862 | 92 | 801 |
| R3 shifted operand `lsl\|lsr\|asr t; add\|sub d,…,t` | 2032 | 963 | 43 | 897 |
| R4 `mul t; add\|sub d,…,t` | 2470 | 1677 | 190 | 1497 |
| R5a `lsr t,s,#k; and d,t,lowmask` | 36 | 401 | 92 | 429 |
| R5b `lsl t; lsr\|asr d,t` | 0 | 122 | 0 | 206 |
| R6a `b.cc L; add_imm d,d,#1; L:` | 0 | 59 | 0 | 58 |
| R6b `b.cc L; sub d,xzr,d; L:` | 0 | 36 | 0 | 36 |
| **sum (instructions saved, 1 each)** | **5797 (1.06%)** | **5307 (0.49%)** | **424 (0.29%)** | **5981 (0.58%)** |
| rejected: `ldur`/`stur`-only offsets | 0 | 1 | 0 | 0 |
| rejected: shifted `adds`/`subs` | 0 | 0 | 0 | 0 |

Chains matched by both R3 and R2 (`shift; add; M [t]`, adjacency only):
0 / 88 / 6 / 77 (same corpus, prototype variant). The sum overcounts by at
most that. These counts are pre-A–F. C turns `mov_imm`+`and` into
`and_imm`, which R5a matches. D and E produce new `csel`/`cset` shapes,
which R6c/R6d match. Phase 1 re-runs the census on the post-F tree; that
changes no rule.

### Verified properties

- *`bfi` has no legal site in the corpus.* A strict matcher (inserted value
  masked to `w` bits, hole in the destination mask exactly at the shifted
  field) finds 0 in all four programs. A loose matcher (any `and`/`lsl`/`orr`
  triple) finds 11 / 10 / 19 / 21, none of them provably a field insert.
  Probed with `/tmp/p171h/tool/bfi_strict.py` and `bitf.py`.
- *Shifted-register `add`/`sub` cannot name `sp`.* `encode/operand.rs:shifted_reg`
  says so. The rules refuse any `sp`/`raw_sp` operand there.
- *A register-offset load may use `sp` as base; the index may not be `sp`.*
  This is architectural (Rn=31 is `sp`, Rm=31 is `xzr`). R2 refuses an `sp`
  index.
- *The new forms read and write no flags.* Architectural (`add`/`sub` without
  `S`, `madd`, UBFM/SBFM). `csinc`/`csneg` read flags exactly like `csel`.
- *A checked multiply's result is read again by its overflow check.*
  `mul t; smulh h` reads `t` afterwards, so R4's deadness test refuses it. Only
  unchecked address and index arithmetic folds.

## 3. Design Overview

One file of rules, `combine.rs`, runs on the final post-frame stream inside
`machine::finish`. Order in the hook: **G combine → A pairing → B
compare-and-branch → (C's matcher, inside B) → H shared exits**. Combine runs
first because R1 turns `[t]` accesses into `[b, #o]` ones, which pairing can
then merge.

Each rule is a local rewrite of two (R2-scaled: three) adjacent instructions in
one basic block. The temporary must be dead after the last instruction it
feeds, by `integer_live_out` on the final stream. For a load it may instead be
that load's `dst`. A rewrite keeps the second instruction's `source`.

**Correctness risk** sits in R1/R2. Folding an address into a memory operand
must keep the effective address bit-identical, and must refuse a store whose
data register is the dropped temporary. R6 removes a branch and a label, so the
label must have no other reference. All of these are unit-tested as
positive/negative pairs. Every new encoding is round-tripped through the
decoder and checked against lldb with `tools/oracles/aarch64-decode`.

**Gate class:** behavior-preserving, golden-changing. Phases 3–4 are expected
to change `.ncode`/`.ncodesum` for **aarch64 goldens only** (`macos-aarch64`,
`linux-aarch64`, and the `.app` variants). Any `linux-x86_64`,
`linux-riscv64` or Windows golden diff is a bug to localize (objdump one
fixture). Phase 2 is inert and gated by byte-identity.

**Rejected alternatives**
- *`ldur`/`stur` (owner decision 2026-10-21):* 0–1 sites per program (table
  above). Replaced by R1, which folds into the existing scaled form.
- *`bfi`/`bfxil`:* 0 strictly legal sites (§2 Verified properties). A loose
  matcher would change values whenever the inserted value has bits above the
  field.
- *Shifted-operand `adds`/`subs`:* 0 sites. The checked-arithmetic paths never
  feed a shift straight into the checked add.
- *Forming the ops in MIR, before allocation:* the register allocator would
  have to learn the `index` role and the new ops' shapes before any benefit,
  and frame offsets are not final until `finalize_frame` (R1 needs them). The
  post-frame hook sees exactly the stream the census measured.
- *Tail calls (owner decision 2026-10-21):* the safe frame-escape rule leaves 148
  tail calls in `sorts` after H's exit merging (`/tmp/p171h/tool/tailfn.py`),
  about 0.01% of instructions, against a memory-corruption failure mode if
  the proof is wrong.
- *x86-64 and riscv64 counterparts:* out of the owner's AArch64 scope. x86 SIB
  addressing and `lea`-as-arithmetic stay catalog rows; riscv64 needs Zba,
  which RV64GC does not have (`src/arch/riscv64/mod.rs`).

## 4. Detailed Design

### 4.1 Ops

| `CodeOp` | Fields | AArch64 encoding class |
|---|---|---|
| `ldr_{u64,u32,u16,u8,d}_idx`, `str_…_idx` | `dst`/`src`, `base`, `index`, `scaled` | load/store register offset, `option=011` (LSL), `S=scaled` |
| `add_shift`, `sub_shift` | `dst`, `lhs`, `rhs`, `kind`, `shift` | add/sub shifted register |
| `madd` | `dst`, `lhs`, `rhs`, `addend` | data-processing 3-source |
| `ubfx`, `sbfx`, `ubfiz`, `sbfiz` | `dst`, `src`, `lsb`, `width` | UBFM/SBFM |
| `csinc`, `csneg` | `dst`, `lhs`, `rhs`, `cc` | conditional select |

- `src/arch/ops.rs`: variants, `mnemonic`, `from_mnemonic`, the `ALL` list.
- `mir.rs` `mirror` group: listed with the other "synthesized after MIR
  lowering (never produced by `from_code`)" entries.
- `analysis.rs:USE_FIELDS` += `index`.
- `opt2/plans/mark.rs:flag_preserving`: every new op except `csinc`/`csneg`
  preserves flags. B's flags-liveness analysis lists `csinc`/`csneg` as flag
  readers, as D did for `csel`/`cset`.
- `Backend::supports_combined_forms()`: AArch64 `true`; x86-64/riscv64 `false`.
  Their encoders return `Err("<op> is AArch64-only; the combine row never
  produces it here")`.

### 4.2 Rules (all in one basic block; `t` dead after the last instruction unless noted)

- **R1 offset fold.** `add_imm t, b, #k ; M [t, #o]` → `M [b, #o+k]`, and
  `sub_imm t, b, #k ; M [t, #o]` → `M [b, #o-k]`, where `M` is an integer or
  `d` load/store of size `s`. Requires the new offset ≥ 0, a multiple of `s`,
  and `< 4096·s`. For a store, its `src` ≠ `t`. For a load, its `dst` may be
  `t` (then `t` is dead by definition). `b` may be `sp`.
- **R2 register offset.** `add t, b, i ; M [t, #0]` → `M_idx [b, i]` (`scaled 0`),
  trying both operand orders. Scaled form: `lsl_imm u, i, #log2(s) ;
  add t, b, u ; M [t, #0]` → `M_idx [b, i, lsl]` (`scaled 1`), with `u` dead too.
  The index is never `sp`/`xzr`. Store `src` ≠ `t`, `u`.
- **R3 shifted operand.** `lsl_imm|lsr_imm|asr_imm t, s, #k ; add d, a, t` (either
  order) or `sub d, a, t` → `add_shift|sub_shift d, a, s, kind, k`. Requires
  `a` ≠ `t` and no `sp` among `d`, `a`, `s`. Not `sub d, t, a`: AArch64 shifts
  only the second operand.
- **R4 multiply-add.** `mul t, x, y ; add d, t, c` (either order) → `madd d, x, y, c`.
  `mul t, x, y ; sub d, c, t` → `msub d, x, y, c` (existing op). Requires
  `c` ≠ `t`.
- **R5 bitfields.**
  - (a) `lsr_imm t, s, #k ; and_imm d, t, #(2^w−1)` → `ubfx d, s, #k, #w`, when
    `k+w ≤ 64`. Same with `asr_imm` when `k+w ≤ 64`, where the mask leaves only
    bits that `asr` and `lsr` agree on.
  - (b) `lsl_imm t, s, #a ; lsr_imm d, t, #b`: `b ≥ a` → `ubfx d, s, #(b−a),
    #(64−b)`; `b < a` → `ubfiz d, s, #(a−b), #(64−a)`. With `asr_imm`:
    `sbfx`/`sbfiz` likewise.
- **R6 conditional increment/negate.** Only for conditions in D's `cc` set
  (`eq ne lt ge gt le lo hs hi ls`, closed under inversion); `b.vs`/`b.vc`/`b.mi`
  triangles are left alone.
  - (a) `b.cc L ; add_imm d, d, #1 ; label L` → `csinc d, d, d, cc`, when `L` is
    referenced by no field of any other instruction. Every label-carrying field
    counts: `target`, and F's `br_table` case fields. The label is deleted.
  - (b) `b.cc L ; sub d, xzr, d ; label L` → `csneg d, d, d, cc`, same condition.
  - (c) `cset t, cc ; add d, x, t` (either order) → `csinc d, x, x, !cc`.
  - (d) `csel d, p, q, cc`, where `p` is `x+1` or `−x` and `q` is `x`, or the
    mirror. `p` (`add_imm p, x, #1` / `sub p, xzr, x`) must be defined earlier
    in the block, with neither `x` nor `p` redefined between, and `p` dead after
    the `csel`. Rewritten to `csinc d, x, x, …` / `csneg d, x, x, …` with the
    condition sense that keeps the selected value. This catches D's `abs` and
    any select over an increment.

### 4.3 Stats and catalog

`src/optimizer/stats.rs`: one counter per rule family (`COMBINE_ADDRESSING`,
`COMBINE_SHIFTED`, `COMBINE_MADD`, `COMBINE_BITFIELD`, `COMBINE_SELECT`).
`src/optimizer/catalog.rs`: row "Instruction combining (machine)", level 1,
stage `machine`. Its summary names the forms and says AArch64 only.

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — census rules and post-F baseline

Adds the measuring patterns and records the baseline this sub-plan's diff is
read against. It changes no compiler code.

- [ ] Add the §4.2 patterns to `tools/ncode-census/census.py` (one counter per
      rule, temporary-dead test as in the prototype below, plus R6c/R6d on
      `cset`/`csel`). Prototype that produced §2's table:
      ```python
      import json, sys
      LD = ('ldr_u64','ldr_u32','ldr_u16','ldr_u8','ldr_d'); ST = ('str_u64','str_u32','str_u16','str_u8','str_d')
      SIZE = {'u64':8,'u32':4,'u16':2,'u8':1,'d':8}
      NONREG = ('op','dst','target','name','offset','imm','shift','value','symbol','cc','type')
      CALLER = {f'x{i}' for i in range(9,19)}
      def reads(x, r): return any(v == r for k, v in x.items() if k not in NONREG)
      def dead_after(I, j, r):   # straight-line scan; anything unknown counts as live
          for x in I[j+1:j+40]:
              if reads(x, r): return False
              if x.get('dst') == r: return True
              o = x['op']
              if o in ('bl','blr'): return r in CALLER
              if o == 'ret': return r not in ('x0','x1')
              if o == 'label' or o == 'b' or o.startswith('b.'): return False
          return False
      def lowmask(v):
          try: v = int(v, 0)
          except ValueError: return False
          return v > 0 and v & (v + 1) == 0
      def dead(I, j, t, d): return d == t or dead_after(I, j, t)
      for path in sys.argv[1:]:
          I = [json.loads(l.strip().rstrip(',')) for l in open(path) if l.strip().startswith('{ "op"')]
          n = len(I); c = dict.fromkeys(['offset-fold','reg-offset','shifted-operand','shifted-adds/subs','madd/msub','ubfx(lsr;and)','bitfield(lsl;lsr|asr)','cinc-triangle','cneg-triangle','ldur/stur-only'], 0)
          for i in range(n - 1):
              a, b = I[i], I[i+1]; op = a['op']
              if op in ('add_imm','sub_imm') and b['op'] in LD+ST and b.get('base') == a['dst'] and b.get('offset') == '0':
                  t = a['dst']; k = int(a['imm']); sz = SIZE[b['op'].split('_')[1]]
                  if dead(I, i+1, t, b['dst'] if b['op'] in LD else None) and not (b['op'] in ST and b.get('src') == t):
                      if op == 'add_imm' and k % sz == 0 and k // sz < 4096: c['offset-fold'] += 1
                      elif (op == 'sub_imm' and k <= 256) or (op == 'add_imm' and k % sz and k <= 255): c['ldur/stur-only'] += 1
              if op == 'add' and b['op'] in LD+ST and b.get('base') == a['dst'] and b.get('offset') == '0':
                  t = a['dst']
                  if dead(I, i+1, t, b['dst'] if b['op'] in LD else None) and not (b['op'] in ST and b.get('src') == t): c['reg-offset'] += 1
              if op in ('lsl_imm','lsr_imm','asr_imm') and b['op'] in ('add','sub','adds','subs'):
                  t = a['dst']
                  if (b.get('rhs') == t or (b['op'] in ('add','adds') and b.get('lhs') == t)) and b.get('lhs') != b.get('rhs') and dead(I, i+1, t, b['dst']):
                      c['shifted-adds/subs' if b['op'] in ('adds','subs') else 'shifted-operand'] += 1
              if op == 'mul' and b['op'] in ('add','sub'):
                  t = a['dst']
                  if (b.get('rhs') == t or (b['op'] == 'add' and b.get('lhs') == t)) and dead(I, i+1, t, b['dst']): c['madd/msub'] += 1
              if op == 'lsr_imm':
                  t = a['dst']
                  for j in (i+1, i+2):
                      if j < n and I[j]['op'] == 'and' and t in (I[j].get('lhs'), I[j].get('rhs')):
                          m = I[j]['rhs'] if I[j].get('lhs') == t else I[j].get('lhs')
                          if any(I[k]['op'] == 'mov_imm' and I[k].get('dst') == m and lowmask(I[k].get('value','x')) for k in range(max(0, i-3), min(n, i+3))) \
                             and dead(I, j, t, I[j]['dst']) and not any(I[q].get('dst') in (t, a['src']) for q in range(i+1, j)):
                              c['ubfx(lsr;and)'] += 1
                          break
              if op == 'lsl_imm' and b['op'] in ('lsr_imm','asr_imm') and b.get('src') == a['dst'] and dead(I, i+1, a['dst'], b['dst']):
                  c['bitfield(lsl;lsr|asr)'] += 1
              if op.startswith('b.') and i + 2 < n and I[i+2]['op'] == 'label' and I[i+2].get('name') == a.get('target'):
                  if b['op'] == 'add_imm' and b.get('dst') == b.get('src') and b.get('imm') == '1': c['cinc-triangle'] += 1
                  if b['op'] == 'sub' and b.get('lhs') == 'xzr' and b.get('rhs') == b.get('dst'): c['cneg-triangle'] += 1
          print(path.split('/')[-1], n, ' '.join(f'{k}={v}' for k, v in c.items()))
      ```
- [ ] Run `tools/ncode-census/ncode-census.sh` for `macos-aarch64` on the post-F
      tree. Write the per-rule counts into §2 as a second table, "post-F".

Acceptance: §2 has the post-F table.
  Check: `grep -c 'post-F' planning/plan-171-G-*.md` → ≥ 2 (est. 20 min — corpus build).
Commit: —

### Phase 2 — ops, encoders, decoder rows, roles (byte-identical)

- [ ] `src/arch/ops.rs`, `mir.rs` mirror group, `analysis.rs:USE_FIELDS += index`,
      `mark.rs:flag_preserving`, B's flags-reader list (`csinc`, `csneg`),
      `Backend::supports_combined_forms` with its three impls.
- [ ] AArch64 encoder arms and sizing for every §4.1 op (`emitter/mod.rs`
      dispatch; memory forms in `emitter/memory.rs`; the rest in
      `emitter/scalar.rs`), each rejecting out-of-range fields with an error.
- [ ] x86-64 and riscv64 encoder arms returning the AArch64-only error (§4.1).
- [ ] `decode/tests.rs`: round-trip rows for every new mnemonic. Extend
      `tools/oracles/aarch64-decode`'s word list with one word per form, and
      run it against lldb.
- [ ] Tests (test-only files, AGENTS.md size rule):
      `src/arch/aarch64/encode/tests/combined_forms.rs`: every op at its field
      boundaries (shift 1/63, `lsb`+`width` = 64, `scaled` 0/1 per size, `sp`
      base accepted / `sp` index rejected, `sp` in a shifted operand rejected).

Acceptance: every new encoding round-trips and matches lldb; no emitted byte changes.
  Check: `cargo test --bin mfb aarch64::encode aarch64::decode` → pass (est. 5 min);
  `bash scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` (est. 15 min — the only check that every target's codegen is unchanged by the new op plumbing).
Commit: —

### Phase 3 — addressing rules R1, R2 (Level 1)

- [ ] `src/optimizer/opt2/combine.rs`: the row entry `combine(instructions)`,
      the shared adjacency/deadness helper (`integer_live_out` from
      `active_backend().register_model()`), and R1/R2. Called first in
      `machine::finish`.
- [ ] `src/optimizer/opt2/combine/tests/mod.rs`: per rule, a positive case and
      one negative per condition (temporary live, store of the temporary,
      offset unaligned/out of range, `sp` index, cross-label).
- [ ] Catalog row, stats counters, and `planning/optimizations.md` row
      "Addressing mode optimization" marked `Y` with the landing note
      (AArch64, adjacent, post-frame).
- [ ] rt fixture `tests/rt-behavior/collections/indexed-access-forms` (valid): it
      indexes Lists of Integer/Byte/Float, record fields at increasing offsets,
      and a nested list, and prints a checksum. No invalid twin: there is no
      user surface, as in plan-171-A Phase 3.
- [ ] `tests/runtime/rt_codegen_forms.rs::indexed_access_forms`: the fixture's
      `macos-aarch64` and `linux-aarch64` dumps contain `ldr_u64_idx` and an
      R1-folded offset at `-O1` and none at `-O0`. `linux-x86_64` and
      `linux-riscv64` dumps contain no `_idx` op.
- [ ] `rt_regalloc_levels_agree`: add the fixture.
- [ ] Golden re-baseline (aarch64 only), following plan-171-A Phase 3's
      procedure. The `census.py --diff` of one fixture per aarch64 target shows
      only R1/R2 rewrites.
- [ ] Runtime: `bash scripts/test-accept.sh target/release/mfb /tmp/171g-accept`
      on macOS; `bash scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64`.

Acceptance: R1/R2 fire at `-O1` on aarch64 only, and outputs are unchanged.
  Check: `cargo test --bin mfb opt2::combine` → pass (est. 2 min);
  `cargo test --release --test rt_codegen_forms indexed_access_forms` → pass (est. 2 min);
  `linux-runtime-proof.sh … 2223 linux-aarch64` → 0 mismatches (est. 25 min — the only check that executes Linux aarch64 code).
Commit: —

### Phase 4 — arithmetic, bitfield and select rules R3–R6 (Level 1)

- [ ] `combine.rs`: R3, R4, R5, R6 (§4.2). If `combine.rs` passes 1000 code
      lines, split per family into `combine/{addressing,arith,select}.rs`
      (AGENTS.md size rule).
- [ ] Unit tests in `combine/tests/`: per rule, positive plus one negative per
      condition. Include `sub d, t, a` (not R3), `k+w > 64` (not R5a),
      R6a with a second branch to `L` (not converted), and R6d with `x`
      redefined between (not converted).
- [ ] `planning/optimizations.md`: "Bit-field extract/insert recognition" `~`
      (extract and insert-zero forms landed; `bfi` declined with §2's census).
      Note R6 under "Multi-way select / CSEL-chain formation" as its
      single-select precursor; that row stays open.
- [ ] rt fixtures (valid): `tests/rt-behavior/bits/bitfield-extract-forms`
      (shift-then-mask and shift-pair extracts, signed and unsigned, at widths
      1, 8, 31, 63) and `tests/rt-behavior/math/multiply-add-forms` (index
      arithmetic `i*stride+base`, `a - b*c`, `abs`, conditional
      increment/negate). Each prints a checksum.
- [ ] `rt_codegen_forms`: `add_shift`, `madd`, `ubfx`, `csinc`, `csneg` present in
      the aarch64 dumps of those fixtures at `-O1`.
- [ ] Golden re-baseline (aarch64 only, same procedure); runtime as Phase 3.
- [ ] Census `--diff` (post-F baseline → now) for the corpus, quoted in
      Corrections per rule.
- [ ] Benchmark (macOS host): `./benchmark/language/run.sh 10` vs.
      `benchmark/language/baseline/mfb-O1.log` with `rank.py`; record
      collection-heavy rows in Corrections. No baseline regeneration.

Acceptance: R3–R6 fire at `-O1` on aarch64 only; outputs unchanged.
  Check: `cargo test --bin mfb opt2::combine` → pass (est. 2 min);
  `cargo test --release --test rt_codegen_forms` → pass (est. 4 min);
  `linux-runtime-proof.sh … 2223 linux-aarch64` → 0 mismatches (est. 25 min).
Commit: —

### Phase 5 — spec and docs

- [ ] `mfb spec architecture aarch64-instruction-set`: the new forms and their
      field ranges. `mir-instruction-set`: the AArch64-only post-lowering ops.
      The optimizer spec topic that lists machine rows: the combine row and its
      hook position.
- [ ] `.ai/codegen-invariants.md`: "a combine rule may drop only a register
      `integer_live_out` proves dead; shifted-register operands never name
      `sp`".

Acceptance: specs render and the catalog renders the row.
  Check: `cargo test --bin mfb spec` → pass (est. 2 min); `target/release/mfb man optimizations | grep -c 'Instruction combining (machine)'` → 1.
Commit: —

## Validation Plan

- Tests: encoder/decoder round trips + lldb oracle, `opt2::combine` units,
  `rt_codegen_forms`, the three fixtures, levels-agree.
- Coverage: the fixtures exercise every rule. Check with `mfb build -vv` that
  each rule's counter is non-zero on its fixture before trusting a green gate.
- Runtime proof: macOS acceptance; `linux-runtime-proof.sh` on 2223
  (`linux-aarch64`). No x86/riscv64 execution: no byte changes there,
  shown by the golden classification.
- Doc sync: Phase 5.
- Final gate (once, after Phase 5): `cargo test --no-fail-fast > /tmp/171g.log 2>&1; echo EXIT=$?` → `EXIT=0`, `grep -c '^failures:' /tmp/171g.log` → 0 (est. 60–90 min; includes `artifact_gate_all`).

## Decisions (owner, 2026-10-21)

1. `ldur`/`stur` are replaced by offset folding (R1): 0–1 sites per program vs.
   7–2057.
2. Tail calls are not in plan-171 (§3, rejected).
3. Scope is AArch64 only.
4. These forms land as their own letter, G; the short-encoding and closing
   letter became I, and later K (x86 letters I and J were inserted before it).

## Corrections

<Filled in during execution.>

## Summary

Five op families, one post-frame machine row, all AArch64. Every rule is a
two- or three-instruction local rewrite gated on a dead temporary, so a wrong
rule shows up as a unit-test failure or a changed program output on 2223.
Measured payoff before A–F: 0.29–1.06% of instructions across four example
programs, led by `madd` and shifted operands.
