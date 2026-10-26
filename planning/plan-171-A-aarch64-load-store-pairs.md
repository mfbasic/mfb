# plan-171-A: Instruction-form census tool and AArch64 `stp`/`ldp`

Last updated: 2026-10-25
Overall Effort: huge (>3d across A–K)
Effort: large (3h–1d)
Depends on: nothing (see Prerequisites)

**plan-171 overall.** The native backends emit only part of the instruction set
their targets offer. On AArch64 the encoder (`match instruction.op.mnemonic()` in
`src/arch/aarch64/encode/emitter.rs:Encoder::emit_instruction`) has single
loads and stores only (`ldr_u64`/`str_u64`/…), compares only as `cmp`/`cmp_imm`
followed by `b.<cond>`, logical operations only in register form (a constant
operand is `mov_imm` + `and`), Booleans built from branch diamonds, and calls
only through `bl`/`blr`. x86-64 and riscv64 share that AArch64-shaped op set
(`src/arch/ops.rs:CodeOp`), so they inherit the same gaps. plan-171 adds the
missing forms at every layer — op vocabulary, MIR, selection, encoder, decoder,
frame code, peepholes, branch relaxation — and converts the code generators to
use them, so the compiled programs contain them. The sub-plans run in payoff
order:

| Letter | Form family | Effort |
|---|---|---|
| **A** | census tool; AArch64 `stp`/`ldp` (frames and body pairing) | large |
| B | compare-and-branch: `cbz`/`cbnz`/`tbz`/`tbnz`; x86 `test`; rv64 bit tests | large |
| C | logical immediates (`and`/`orr`/`eor #imm`, x86 imm32, rv64 `andi`/`ori`/`xori`); `mov_imm` via `orr`/`movn` | large |
| D | conditional set and select: `cset`/`csel`, x86 `setcc`/`cmov`, rv64 `slt` family | large |
| E | if-conversion (MIR row that forms selects from diamonds) | medium |
| F | jump tables for dense `MATCH`; `adr` and PAC/BTI decisions | large |
| G | AArch64 addressing and arithmetic forms: register-offset and folded-offset loads/stores, shifted-operand `add`/`sub`, `madd`/`msub`, `ubfx`/`sbfx`/`ubfiz`/`sbfiz`, `csinc`/`csneg` | large |
| H | shared function exits (AArch64): identical exit sequences in a function become one | large |
| I | x86-64 register pool: `rbp`, `r13` and the argument registers become allocatable (calls declare the argument registers they read) | large |
| J | x86-64 frame layout and memory operands: hot stack objects first, load folding, `lea`/SIB/offset folding, two-address hints | large |
| K | short branch and immediate encodings (rv64 B-type, x86 rel8/imm8); cross-form review; final validation | large |

**This sub-plan** builds the measuring instrument every later letter uses (an
`-ncode` instruction-pattern census) and then teaches AArch64 the
load/store-pair forms: callee-saved save/restore in every prologue and epilogue
becomes `stp`/`ldp`, and a machine pass merges adjacent same-base 8-byte
accesses in the body. The outcome: on `macos-aarch64` and `linux-aarch64` every
function that saves two or more adjacent callee-saved registers does so with
`stp`, and restores them with `ldp`; programs run with unchanged output.

References:

- `src/codegen/engine/util/vreg_frame.rs:finalize_frame`, `push_frame_allocation`,
  `save_callee_saved`, `restore_callee_saved` (the only prologue/epilogue builder).
- `src/arch/ops.rs:CodeOp`, `CodeOp::mnemonic`, `CodeOp::from_mnemonic`, the
  `ALL` list in `mnemonic_round_trips_for_all_ops`.
- `src/codegen/engine/mir/mir.rs:MirOp` (`mir_ops!` macro), `from_code`, `to_code`.
- `src/arch/aarch64/encode/emitter.rs:Encoder::emit_instruction`, `emit_ldr_u64`;
  `src/arch/aarch64/encode/sizing.rs:instruction_size`.
- `src/arch/aarch64/decode/tests.rs:the_table_covers_every_form_the_encoder_emits`,
  `decodes_words_as_lldb_does`; `tools/oracles/aarch64-decode` (the decoder's lldb oracle).
- `src/codegen/engine/regalloc/analysis.rs:DEF_FIELDS`, `USE_FIELDS`, `effect`.
- `src/optimizer/opt2/peephole.rs:forward_stores_to_loads` (the shape a machine
  pass takes), `src/optimizer/opt2/plans/mark.rs:flag_preserving`.
- `src/optimizer/catalog.rs:rows`, `planning/optimizations.md` (row "Aggregate
  load/store combining"), `mfb man optimizations` (renders the catalog).
- `src/debugger/backtrace.rs:state_at`, `Effect::of` (reads prologues by decoding them).
- `.ai/compiler.md`, `.ai/codegen-invariants.md`, `.ai/arch-abi.md`,
  `.ai/testing-gates.md`, `.ai/specifications.md`.
- `mfb spec architecture aarch64-instruction-set`, `mfb spec architecture
  mir-instruction-set`, `mfb spec tooling debugger`.

## Prerequisites

Stated once for the whole plan-171 family; B–K point here.

| Must be true | Command | Status |
|---|---|---|
| The concurrent AArch64 decoder widening has landed (nothing uncommitted under the decoder) | `git -C <tree> status --short src/arch/aarch64/decode` → no output | MET (2026-10-25, worktree `opts` at `89a4dce2c`: no output) |
| The concurrent `src/docs/**` edits have landed (spec/man tasks below edit those files) | `git -C <tree> status --short src/docs` → no output | MET (2026-10-25, `89a4dce2c`: no output) |
| The tree builds and the bin unit tests pass at the starting commit | `cargo test --bin mfb > /tmp/171-pre.log 2>&1; echo EXIT=$?` → `EXIT=0` | MET (2026-10-25, `89a4dce2c`: `EXIT=0`, 6166 passed, 0 failed, 5 ignored) |
| The artifact gate is clean at the starting commit (so every later diff is ours) | from a detached worktree of the starting commit: `bash scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` | MET (2026-10-25, `/tmp/171a-base` detached at `89a4dce2c`, its release `mfb`: `1722 tests, 1902 build(s), 2387 golden(s) checked, 0 diff(s)`) |

Everything below is written against the world where these hold. The decoder
prerequisite exists because every new AArch64 encoder form must gain a decoder
table row (`the_table_covers_every_form_the_encoder_emits` reads the encoder's
dispatch and fails on an undecoded mnemonic). The widened decoder (`f62f27418`)
already decodes every form this family adds — `stp`/`ldp` in all index modes,
`cbz`/`cbnz`/`tbz`/`tbnz`, logical immediates and the `mov` bitmask alias,
`movn`, `csel`/`csinc`/`cset`, `br`, `adr` — checked against lldb; each letter
adds only its encoder round-trip rows, never a decode arm.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- `tools/ncode-census/` exists and, given an `mfb` binary and a target, prints
  per-pattern counts over the `-ncode` dumps of a fixed corpus; every later
  sub-plan quotes its before/after numbers from it.
- On AArch64, `finalize_frame` emits `stp`/`ldp` for every pair of adjacent
  8-byte callee-saved slots (and for adjacent 16-byte `q` slots), folding the
  frame allocation into a pre-index `stp` and the deallocation into a
  post-index `ldp` when the save area sits at the bottom of a frame of at most
  504 bytes.
- A Level-1 machine pass merges two adjacent same-base `str_u64`/`ldr_u64`
  accesses 8 bytes apart into one `stp`/`ldp` when legal.
- Every runtime fixture produces the same output as before on macos-aarch64 and
  linux-aarch64 (box 2223), and the debugger's backtrace still walks through the
  new prologues.

### Non-goals (explicit constraints)

- No change to frame **layout**: slot offsets, save-area order, alignment,
  `CodeFrame.stack_size` and the stack-slot metadata stay exactly as today. Only
  the instructions that read/write the save area change.
- No x86-64 or riscv64 code change (see §2.4 — neither has a pair form worth
  emitting).
- No reordering of body instructions: the pairing pass merges only two
  instructions that are already adjacent.
- No pairing of `ldar`/`stlr`, of accesses whose two halves come from different
  source lines when `--debug` line info is requested (Open Decision 2), or of
  accesses through a base the first instruction writes.
- No frame-pointer chain is introduced (x29 stays unused; `rg -n x29 src/codegen
  src/target` → 2 unrelated hits).

## 2. Current State

### 2.1 Frames

`finalize_frame` (`src/codegen/engine/util/vreg_frame.rs`) is the single
prologue/epilogue builder; `rg -n "finalize_frame\(" src | wc -l` → 13 lines
(definition, `finalize_vreg_body_with_locals`, `function_lowering.rs` ×3,
`park_error_helper.rs` ×3, `fail_status.rs`, `value_functions.rs`, …). It:

1. appends `lr` to `callee_saved` when the body contains `bl`/`blr`;
2. lays out the save area: 8-byte slots at `index * 8`, except a 16-aligned
   16-byte slot for an AArch64 `d8`–`d15` (saved with `str q`, bug-124.2);
3. sizes the frame `outgoing_bytes + align(save_size + locals, 16) + padding`
   and shifts every body `sp` access by `outgoing_bytes + save_size`
   (`adjust_stack_instruction_offsets`);
4. prologue: `push_frame_allocation` (`sub sp, total`, or page probes on Win64)
   then one `save_callee_saved` store per register;
5. epilogue: before every `ret`, one `restore_callee_saved` load per register in
   reverse, then `add sp, total`.

So the save area is a run of adjacent 8-byte slots at `[sp + outgoing_bytes + 8k]`
— exactly the `stp` shape — and today it is one `str` per register. The
`-ncode` dumps show it: `control_flow_if.macos-aarch64.ncode` begins
`label entry; sub_sp 3792; …`.

### 2.2 Body accesses

Spill slots are 16 bytes apart (`Aarch64RegisterModel::spill_slot_bytes` = 16,
`src/arch/aarch64/regmodel.rs`), so two *spills* are never 8 bytes apart and
never pair. The pairable body accesses are builder-emitted 8-byte stack objects
and heap header/field accesses (`abi::store_u64`/`load_u64`). Source-level
census (lower bound — runtime-computed offsets are invisible):
`store_u64` adjacent same-base +8 with literal offsets: 56; `load_u64`: 47
(`rg -U -P --count-matches` over consecutive `abi::store_u64(`/`abi::load_u64(`
lines, summed; total uses `store_u64` 5364, `load_u64` 8205).

Emitted census over the seven committed `.ncode` goldens (all macos-aarch64),
counting directly adjacent same-base 8-byte accesses 8 bytes apart (loads only
when the first destination is neither the base nor the second destination) —
re-measured at `89a4dce2c` (Correction 1; the 2026-10-04 numbers were stale):

| `.ncode` golden | instrs | `str` pairs | `ldr` pairs |
|---|---|---|---|
| `func_map_getor_hash_probe` | 17098 | 676 | 574 |
| `list_ops_codegen_rt` | 8262 | 92 | 116 |
| `control_flow_if` | 5800 | 67 | 89 |
| `control_flow_match` | 5837 | 69 | 143 |
| `parser_hello_world` | 5289 | 63 | 67 |
| `macos_app_mode_io` (`--app`) | 19371 | 785 | 336 |
| `macos_app_mode_plumbing` (`--app`) | 10335 | 178 | 198 |

(`python3 -I tools/ncode-census/census.py --pairs $(find tests -name '*.ncode' | sort)`,
identical to the prototype reproduced in Phase 1; these counts include the
prologue/epilogue saves.) Corpus-wide (`tools/ncode-census/ncode-census.sh
<mfb of 89a4dce2c> macos-aarch64 …`, 56 projects, `baseline-macos-aarch64.tsv`):
27,311,174 instructions in 16,044 functions; 495,927 adjacent `str` candidates
(431,379 legal under §4.4) and 686,275 `ldr` candidates (649,431 legal); the
rejections are 101,267 `range` (lower offset outside −512…504), 706 `same_dst`
and 11 `dst_is_base`. Candidates overlap (three stores at 0/8/16 are two), so the
pairs a greedy left-to-right merge forms are fewer.

### 2.3 Encoder, decoder, MIR, analyses

- No `stp`/`ldp` anywhere: `rg -n "^\s*(Ldp|Stp)\b" src/arch/ops.rs` → 0. The
  encoder's loads/stores are unsigned-scaled `imm12` only (`emit_ldr_u64`).
- The decoder guard `the_table_covers_every_form_the_encoder_emits`
  (`src/arch/aarch64/decode/tests.rs`) slices the encoder dispatch from
  `match instruction.op.mnemonic()` to `other => Err(` and requires each
  mnemonic to be in `table()` or the `by_word` allowlist. The decoder already
  decodes `stp`/`ldp` in every index mode as lldb prints them (`f62f27418`,
  `decodes_words_as_lldb_does`); only the round-trip rows are new.
- `MirOp` is generated by `mir_ops!`; `from_code`/`to_code` are exhaustive, so a
  new `CodeOp` does not compile until it is placed in a group
  (`mfb spec architecture mir-instruction-set`, "Design contract").
- Register roles are by field name: `DEF_FIELDS = ["dst", "carry_out",
  "borrow_out"]`, `USE_FIELDS = ["src", "lhs", "rhs", "minuend", "base",
  "register", "addend", "carry_in", "borrow_in"]`
  (`src/codegen/engine/regalloc/analysis.rs`). A pair needs a second data
  register field (`dst2`/`src2`) added to these lists.
- `flag_preserving` (`src/optimizer/opt2/plans/mark.rs`) already treats any
  mnemonic starting `ldp`/`stp` as flag-preserving.
- `forward_stores_to_loads` (`src/optimizer/opt2/peephole.rs`) clears its state
  on any unmodelled op, so an `stp` in its input is safe but blocks forwarding;
  it runs *before* `finalize_frame` at every call site (e.g.
  `function_lowering.rs` lines 1526/1538), so a pairing step at the end of
  `finalize_frame` runs after it and never feeds it.
- `CodeInstruction.source` carries a source position per instruction (used for
  debug line info, plan-166).

### 2.4 x86-64 and riscv64 equivalents

- **x86-64: none worth emitting.** There is no two-register store. `push`/`pop`
  only address `[rsp]` with an implicit adjust, so they cannot replace
  `mov [rsp+disp], r` saves without changing the frame layout, and the Win64
  `UNWIND_INFO` that plan-169 writes describes the prologue shape — a push-based
  prologue is that plan's business, not this one's. Today the saves are
  `mov [rsp+disp32], r` (`save_callee_saved` → `str_u64` →
  `mem_disp32`); a shorter `disp8` form is sub-plan K.
- **riscv64: none.** RV64GC has no pair load/store (Zilsd is RV32-only), and
  the encoder emits no compressed instructions (`c.sdsp` would be a size, not a
  pairing, change). Saves are one `sd`/`fsd` per register.

### 2.5 Debugger

`src/debugger/backtrace.rs` unwinds by decoding a function from its start. It
already understands `stp`/`ldp` in all three addressing modes (`Effect::of`:
"LDP/STP, 64-bit integer or SIMD", `pair_effect`), because system libraries use
them. It has no test over an MFBASIC-compiled `stp` prologue; its module doc
names the compiler's frame as `sub sp, sp, #N` then `str x30, [sp, #off]`.

### Verified properties

- *The save area is contiguous 8-byte slots in `callee_saved` order, except
  16-byte q slots* — read `finalize_frame` (§2.1).
- *Pairing at the end of `finalize_frame` sees final offsets* — read
  `finalize_frame`: `adjust_stack_instruction_offsets` and
  `resolve_stack_arg_sentinels` run before the prologue is spliced in.
- *`finalize_frame` returns early with no prologue when `total_stack_size == 0`*
  — read; the body-pairing step must also run on that path.
- *Whether requiring equal `source` on both halves loses many candidates* —
  moot (Correction 2): `CodeInstruction.source` is the compiler's own emitting
  call site, not a source position, and the `.ncode` dump does not carry it.
- *Whether any consumer after `finalize_frame` pattern-matches `str_u64`/`ldr_u64`
  on `sp`* — UNVERIFIED; Phase 2 task greps for it.

## 3. Design Overview

Three independent pieces, layered bottom-up:

1. **Census tool** (`tools/ncode-census/`): a reusable measuring instrument.
2. **The pair forms** as `CodeOp`s with encoder, decoder, MIR placement and
   field roles — no producer yet, so landing it changes no emitted byte.
3. **Two producers**: the frame (Level 0 — lowering, all `-O` levels) and the
   body pairing pass (Level 1 — a catalog row, off at `-O0`).

**Where correctness risk concentrates:** the body pairing pass (load pairs that
read a register the first load writes; base registers; trap order — a pair
faults as a unit where two loads faulted one at a time, which is harmless here
because both halves of a merged pair address the same 16 bytes and an MFBASIC
program has no fault handler that can observe partial completion) and the
epilogue fold of `add sp` into a post-index `ldp` (an off-by-one in the frame
size corrupts `sp` for the caller). Both are scheduled after the inert
primitive, behind encoder round-trip tests and runtime fixtures.

**Where design uncertainty concentrates:** how many body candidates survive the
legality filters (Open Decision 2). Phase 1 measures it before Phase 4 is built.

**Byte-identity is not this plan's gate.** Phases 1–2 are provably neutral
(new op, no producer) and are gated by byte-identity. Phases 3–4 are expected to
change `.ncode`/`.ncodesum` for **every aarch64 golden whose functions save two
or more registers** — i.e. essentially every `macos-aarch64`, `linux-aarch64`
and `macos-aarch64.app`/`linux-aarch64.app` golden — and **no** `linux-x86_64`,
`windows-x86_64` or `linux-riscv64` golden. A diff on an x86/rv64 golden is a
bug to root-cause (`-ncode` ONE fixture), never a design verdict.

**Rejected alternatives.**
- *Pair inside `forward_stores_to_loads`* — that pass runs before
  `finalize_frame`, so offsets are not final and the save area does not exist yet.
- *Pair in the MIR (`optimize_mir`) before register allocation* — the pair
  constraints are on physical registers (`ldp` destinations distinct, not the
  base) and on final offsets (imm7), neither known there.
- *Reorder to create adjacency* — needs alias analysis the tree does not have
  (`planning/optimizations.md` "Store-to-load forwarding": "Arbitrary bases /
  memory-SSA remain Plan2 alias infrastructure").
- *Change spill slots to 8 bytes so spills pair* — spill slots are 16 bytes to
  hold `v128` values (`spill_slot_bytes`); changing the layout is a non-goal.

## 4. Detailed Design

### 4.1 Census tool

`tools/ncode-census/ncode-census.sh <mfb> <target> <out.tsv>` builds each corpus
project with `mfb build -q -ncode -target <target>` into a scratch copy (never
in-tree — testing-gates "A corpus sweep that builds IN-TREE clobbers a concurrent
one") and runs `tools/ncode-census/census.py` over the dumps. The corpus is
fixed and listed in the README: every `examples/*` project, `benchmark/language/mfb`, and
every `tests/byte-identity/*` fixture. `census.py` reads one JSON instruction
per line (`{ "op": …, … }`, the `-ncode` format) and counts, per function and in
total: instruction count; adjacent same-base `str_u64`/`ldr_u64` 8 apart (with
and without the legality filters of §4.4, and with/without equal `source`);
`cmp_imm …, 0` + `b.eq`/`b.ne`; `mov_imm` feeding `and`/`orr`/`eor`;
single-bit test shapes (`mov_imm 2^k; and; cmp_imm 0; b.eq/ne`); 0/1 branch
diamonds; `MATCH`-style compare chains (≥ 3 consecutive `cmp_imm r,k; b.eq` on
one register); and the count of each plan-171 form (`stp`, `ldp`, `cbz`, …).
A `--diff a.tsv b.tsv` mode prints the per-pattern delta — the before/after
table each sub-plan's sweep phase quotes. (The `-ncode` dump is post-frame:
`control_flow_if.macos-aarch64.ncode` opens with `sub_sp`.)

### 4.2 The ops

Two `CodeOp`s, `Stp` (`"stp"`) and `Ldp` (`"ldp"`), each with an `addressing`
field `offset` | `pre` | `post`, and a `width` of 8 (X) or 16 (Q):

| op | fields |
|---|---|
| `stp` | `src`, `src2`, `base`, `offset`, `addressing`, `width` |
| `ldp` | `dst`, `dst2`, `base`, `offset`, `addressing`, `width` |

`offset` is the byte offset (signed for pre/post-index); the encoder checks
`offset % width == 0` and `-64*width <= offset <= 63*width` (X: −512…504; Q:
−1024…1008) and returns an encode error otherwise — producers check first (§4.3,
§4.4), so the error is a bug detector, not a path. `ldp` additionally rejects
`dst == dst2` and, for writeback, `dst`/`dst2 == base`.

Placement in the MIR: the **mirror** group (1:1, ISA-neutral spelling). x86-64
and riscv64 never receive them: the producers are gated on
`Backend::supports_pair_access()` (new, default `false`, `true` in
`src/arch/aarch64/backend.rs:Aarch64Backend`), and the x86/rv64 encoders return
an explicit "not emitted on this target" error if one ever arrives, which a unit
test pins (`select_x86`/`select_riscv64` never produce it).

Field roles: add `dst2` to `DEF_FIELDS` and `src2` to `USE_FIELDS`
(`regalloc/analysis.rs`), so `effect`, `is_register_field` and the bug-242
sentinel assert see both registers. `forward_stores_to_loads`'s `classify`
models `stp` to `sp` as two slot stores and `ldp` as two register writes, so a
pair never turns it conservative where it ran before.

### 4.3 Frame pairing (Level 0)

In `finalize_frame`, after offsets are known, build the save list as before and
then, when `supports_pair_access()`:

- Walk `callee_saved` in order; two consecutive entries form a pair when both
  are 8-byte GPR slots (or both 16-byte q slots) at offsets `o` and `o + width`,
  and `o + outgoing_bytes` is within the imm7 range. Emit `stp` for the pair,
  otherwise the existing single store. An odd tail keeps its single `str`.
- **Pre-index fold:** when `outgoing_bytes == 0`, the frame needs no Win64 probe
  (`stack_probe_page_bytes() == 0` on AArch64), `total_stack_size <= 504`, and
  the first pair is at offset 0, replace `sub sp, total` + `stp a,b,[sp,#0]` with
  `stp a,b,[sp,#-total]!`.
- Epilogue mirrors it: `ldp` per pair in reverse; the post-index fold replaces
  `ldp a,b,[sp,#0]` + `add sp,total` with `ldp a,b,[sp],#total` under the same
  condition.

Restore order changes from strictly reverse-per-register to reverse-per-pair;
the restored values are identical (each register reloads its own slot).

### 4.4 Body pairing (Level 1 machine row "Load/store pairing")

`optimizer::opt2::pairing::pair_adjacent_accesses(instructions)`, gated by
`level_enabled(1)` and `supports_pair_access()`. It is the first client of a new
**machine finishing hook**, `optimizer::opt2::machine::finish(instructions)`,
called once at the end of `finalize_frame` on both the framed and the frameless
(`total_stack_size == 0`) return path. Every later post-allocation row of
plan-171 (B's compare-and-branch formation, C's matcher extension) runs from this
hook, as do G's combining rows (ahead of pairing) and H's exit sharing (last), so
all allocated code — user functions and vreg-allocated runtime helpers
— passes through one choke point. For each adjacent `(i, i+1)`
in one basic block (neither is a label or terminator):

- both `str_u64` or both `ldr_u64`, same `base` operand, offsets `o` and `o ± 8`,
  lower offset within `−512…504`, multiple of 8;
- for loads: first `dst` ≠ `base`, `dst` ≠ `dst2` (a load into the base would
  change the second address);
- neither half is `ldar`/`stlr` (different op, never matched);
- `source` positions equal (Open Decision 2).

The merged instruction takes the lower offset, orders registers by offset, and
keeps the first instruction's `source`. Stats counter in `src/optimizer/stats.rs`
like every row.

## Phases

> **NOTE — keep the checkboxes current as you go.** Tick `- [x]` in the same
> commit as the work; `- [~]` for partial with what remains; moot tasks are
> struck through with evidence, never deleted; fill `Commit:` when a phase lands.
> **An unticked box means NOT DONE.**

### Phase 1 — `tools/ncode-census` (measure first)

Delivers the instrument and the baseline numbers; touches no compiler code.

- [x] Write `tools/ncode-census/census.py` (patterns of §4.1) and
      `tools/ncode-census/ncode-census.sh` (scratch-copy builds, one target per
      run), plus `tools/ncode-census/README.md`. Prototype for the pair counts
      (what produced §2.2's table):
      ```python
      import json,sys
      def load(p):
          return [json.loads(l.strip().rstrip(',')) for l in open(p) if l.strip().startswith('{ "op"')]
      for p in sys.argv[1:]:
          ins=load(p); st=ld=0
          for a,b in zip(ins,ins[1:]):
              for op in ('str_u64','ldr_u64'):
                  if a['op']==op==b['op'] and a.get('base')==b.get('base'):
                      try: d=abs(int(a['offset'])-int(b['offset']))
                      except (KeyError,ValueError): continue
                      if d==8:
                          if op=='str_u64': st+=1
                          elif a.get('dst') not in (a.get('base'),b.get('dst')): ld+=1
          print(p, len(ins), st, ld)
      ```
- [x] Record the baseline for `macos-aarch64`, `linux-x86_64`, `linux-riscv64`
      into `tools/ncode-census/baseline-<target>.tsv` with the producing commit
      in the README. (Also `linux-aarch64`, the other target this letter
      changes. riscv64 builds 43 of 56: 12 app-mode refusals and one compiler
      panic on `benchmark/language/mfb`, Correction 3.)
- [x] ~~Measure Open Decision 2: the pair-candidate count with and without the
      equal-`source` filter; write both numbers into this plan's §2.2.~~ Moot:
      there is no per-instruction source position to compare (Correction 2);
      §2.2 carries the corpus-wide candidate counts instead.

Acceptance: the tool reproduces §2.2's table for the seven committed goldens.
  Check: `python3 tools/ncode-census/census.py --pairs $(find tests -name '*.ncode' | sort)` → the seven rows of §2.2 exactly (est. 5 s).
  Result (2026-10-25): the seven rows of the re-measured §2.2 table, and
  `diff <(python3 -I /tmp/171a-proto.py …) <(python3 -I tools/ncode-census/census.py --pairs …)`
  (the prototype above, verbatim) → no output.
Commit: 4dcdd0ef5

### Phase 2 — `stp`/`ldp` primitive (no producer; byte-identical)

- [x] `src/arch/ops.rs`: add `Stp`, `Ldp` (variant, `mnemonic`, `from_mnemonic`,
      `ALL`). (Plus their required fields in `code_impl.rs:CodeInstruction::validate`.)
- [x] `src/codegen/engine/mir/mir.rs`: place both in the mirror group of `mir_ops!`.
- [x] `src/codegen/engine/regalloc/analysis.rs`: `dst2` → `DEF_FIELDS`, `src2` →
      `USE_FIELDS`. Also: a pre-/post-indexed pair's `base` is a definition
      (`effect`), tested by `a_pair_reads_and_writes_both_registers_…`.
- [x] `src/target/shared/abi.rs`: constructors `store_pair_u64`, `load_pair_u64`,
      `store_pair_q`, `load_pair_q` taking `(r1, r2, base, offset, addressing)`.
      Landed with their first caller in Phase 3 (Correction 4), with
      `abi::PairAddressing`.
- [x] `src/arch/aarch64/encode/emitter.rs` + `sizing.rs`: encode STP/LDP (64-bit
      GPR and 128-bit SIMD; signed-offset, pre-, post-index) with the range,
      alignment and register checks of §4.2. (`emitter/memory.rs:emit_pair`;
      `sizing.rs:instruction_size` derives the size from the emitter, so it
      needed no change.)
- [x] `src/arch/x86_64/encode/emitter.rs`, `src/arch/riscv64/encode/emitter.rs`:
      explicit "AArch64-only form" error arm; a unit test per backend
      (`the_aarch64_pair_forms_are_refused` in `x86_64/encode/tests/integer_edges.rs`
      and `riscv64/encode/tests/encoder.rs`).
- [x] `src/arch/aarch64/backend.rs`: `supports_pair_access() -> true`; trait
      default `false` in `mir.rs:Backend`. Landed with its first reader,
      `finalize_frame`, in Phase 3 (Correction 4); the x86-64/Win64/riscv64
      backend tests assert `false`.
- [x] `src/optimizer/opt2/peephole.rs:classify`: model `stp`/`ldp` (§4.2). A
      non-writeback X `stp` to `sp` is two slot stores, a non-writeback X `ldp`
      defines both destinations; write-back and Q pairs flush, as their single
      forms do. The inline tests moved to `peephole/tests/mod.rs` first
      (`2306e97d4`, pure motion: they would pass the 250-line inline budget).
- [x] Grep for post-`finalize_frame` consumers that match `str_u64`/`ldr_u64` on
      `sp` (`rg -n 'StrU64|LdrU64|"str_u64"|"ldr_u64"' src --type rust`), read each
      that runs after the frame, and teach it the pair or record why it cannot
      meet one (list in this task). Result: 366 hits; no non-test `src/` code
      reads a finalized stream for them — the matches are the encoders, the
      allocator and opt2 passes (all before `finalize_frame`: `dse`, `sink`,
      `promote`, `plans/memory`, `stldfwd`, `linear_scan`, `peephole`), and
      `#[cfg(test)]` modules (`linux_common/code.rs`, `macos_aarch64/app/{mod,
      mouse_view,metal}.rs`, `linux_gtk/mouse.rs`, `marshal/record.rs`,
      `canvas/vulkan_handoff.rs`, `arena/arena/mod.rs`), which Phase 3's
      suite run checks. The `-ncode` readers under `tests/` that do meet a
      finalized stream were taught the pair: `tests/common/ncode_emu.rs` (runs
      `stp`/`ldp` X pairs in all three modes), `rt_foreign_entry_callee_saved.rs`
      (both halves, `sp` moved by pre/post index, `dst2` clobbers) and
      `rt_runtime_helper_callee_saved.rs` (`dst2` is a write). The debugger
      already decodes all three modes (§2.5).
- [x] Tests: `src/arch/aarch64/encode/tests.rs` — one case per width × mode ×
      load/store, boundary offsets (−512, 504, −1024, 1008), and each rejected
      form (misaligned, out of range, `ldp x1,x1`, writeback onto base).
      `src/arch/aarch64/decode/tests.rs` — `table()` rows for `stp`/`ldp` (each
      addressing mode) so the guard passes. (`encode/tests/pairs.rs`: 19 forms
      against Apple clang 17 words, 15 refusals; 10 decoder rows.)

Acceptance: encoder↔decoder round trip holds for every new form, and no emitted
byte changes (provably neutral — nothing produces the ops yet).
  Check: `cargo test --bin mfb aarch64::encode aarch64::decode ops:: regalloc::analysis` → all pass (est. 3 min);
  `bash scripts/artifact-gate.sh target/release/mfb strings` → `0 diff(s)` (est. 1 min).
  Result (2026-10-25): `cargo test --bin mfb -- aarch64::encode aarch64::decode ops:: regalloc::analysis opt2::peephole x86_64::encode riscv64::encode`
  → 344 passed, 0 failed; `artifact-gate.sh … strings` → `7 golden(s) checked, 0 diff(s)`;
  `cargo test --release --test rt_runtime_helper_callee_saved --test rt_foreign_entry_callee_saved --test app_special_keys --test codegen_canvas_metal_off_frame_path --test codegen_canvas_vulkan_off_frame_path` → all pass.
Commit: 2b7058db8 (the riscv64 round-trip guard it reddened is fixed in ad873009b, Correction 6)

### Phase 3 — frame `stp`/`ldp` (Level 0)

- [x] `src/codegen/engine/util/vreg_frame.rs:finalize_frame`: pair the saves and
      restores (§4.3), including the pre-/post-index folds; keep the single-store
      path for unpaired slots and for backends without pair access.
      (`save_accesses` builds the access list once; `SaveAccess` emits each
      prologue/epilogue access. The fold also requires a non-probing frame.)
- [x] Unit tests in `vreg_frame.rs`: 0, 1, 2, 3 GPR saves; GPR + `d8` (q slot
      breaks the run); `outgoing_bytes > 0` (no pre-index fold); total 504 vs
      512 (fold boundary); save offset 504 vs 512 (imm7 boundary); multiple `ret`s.
      (`vreg_frame/tests/pairs.rs`, after moving the inline tests out in
      `5276db06b`; also Q+Q pairing and x86-64/riscv64 keeping singles. The
      boundaries are 496/512 and 496/512 — Correction 5.)
- [x] `src/debugger/backtrace.rs`: add a `state_at` test over the exact words the
      new prologue/epilogue emit (pre-index `stp` with `x30` second, plain `stp`,
      post-index `ldp`), asserting CFA and `lr` slot; update the module doc's
      description of the compiler frame.
      (`a_paired_compiled_frame_saves_lr_at_its_offset_from_the_cfa`, words from
      Apple clang 17.)
- [x] rt fixture `tests/rt-behavior/functions/frame-pair-saves` (valid): a
      recursive function that keeps ≥ 6 values live across calls (forces x21–x27
      saves) plus a leaf, printing a checksum; a `--debug`-free build. No
      invalid twin: there is no user-facing surface to misuse (a compiler
      lowering change), which `.ai/compiler.md`'s per-function fixture rule
      does not cover.
- [x] Codegen-inspection test `tests/runtime/rt_codegen_forms.rs::frame_saves_use_stp`
      (new file; extended by every later letter): builds that fixture with
      `-ncode` for `macos-aarch64` and `linux-aarch64`, asserts `stp`/`ldp`
      present and no `str_u64 x30` adjacent to another save; asserts no `stp`
      in the `linux-x86_64`/`linux-riscv64` dumps. (Stronger: no function in the
      dump — helpers included — saves two adjacent integer slots of its frame
      with two `str`; `windows-x86_64` also checked. Red on the `89a4dce2c`
      compiler: `MFB_TEST_EXE=<base mfb> … frame_saves_use_stp_on_macos` →
      `mix prologue` assertion fails.)
- [x] Golden re-baseline (this form's own): run the AGENTS.md four questions for
      the `.ncodesum` family once and record them in the commit (written for
      byte-identity of emitted code; dependents are `artifact-gate.sh` only;
      proof = `-ncode` diff of ONE fixture per aarch64 target showing only
      save/restore instructions changed, via `census.py --diff`). Full suite
      first (`cargo test --no-fail-fast -- --skip artifact_gate_all`), then
      `bash scripts/regen-native-goldens.sh target/release/mfb`, then
      `artifact-gate.sh … all` → 0. Compare `git status --porcelain | wc -l`
      against the predicted set (aarch64 goldens only).
      Result: `artifact-gate.sh <phase-3 mfb> all` → `2389 golden(s) checked, 73
      diff(s)`, every one aarch64 (29 linux-aarch64 + 3 linux-aarch64.app +
      29 macos-aarch64 `.ncodesum`, 3 macos `.app.ncodesum`, 5 macos `.ncode`,
      2 macos `.app.ncode`, 2 macos `.mir` — Correction 7), none x86-64,
      Windows or riscv64. Proof for **all 73** rather than one fixture: each was
      rebuilt with the `89a4dce2c` and the phase-3 compiler and compared by a
      `/tmp` probe that expands every `stp`/`ldp` back into its two accesses
      (and a pre/post fold into its `sub_sp`/`add_sp`) — 73 of 73 equal, 96,130
      pairs, every other instruction, function, data object and import
      unchanged. Full `test-accept.sh` first: 1758 ran, 9 mismatches, exactly
      the raw `.ncode`/`.mir` goldens among the 73; every `build.log` run
      output matched. `regen-native-goldens.sh <phase-3 mfb>` → 231 rewritten,
      `git status --porcelain` 73 changed = the 73 diffs; gate re-run →
      `2389 golden(s) checked, 0 diff(s)`.
      Four questions (per AGENTS.md): (1) the `.ncode`/`.ncodesum`/`.mir`
      goldens are the byte-identity sentinels of emitted code, regenerated by
      every codegen change since `regen-native-goldens.sh` (plan-131-B);
      (2) they protect the exact emitted instruction stream per target;
      (3) `scripts/artifact-gate.sh` (and `cargo test --test golden`), and
      `test-accept.sh` for the raw macOS `.ncode`/`.mir`; (4) wrong because
      the new frame is the intended stream: the expansion probe shows each
      diff is only the planned save/restore pairing.
- [x] Runtime proof: `bash scripts/test-accept.sh target/release/mfb /tmp/171a-accept`
      on macOS and `bash scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64`.
      macOS: the acceptance run above (every run output matched). Linux
      aarch64: **box 2223 refused ssh** (`ssh -p 2223 test@127.0.0.1` →
      `Connection refused`, 2026-10-25), so the proof ran on **2226** (Debian 12
      aarch64 glibc): phase-3 compiler `899 passed, 11 failed, 1 not run`; the
      `89a4dce2c` compiler on the same box `899 passed, 11 failed, 1 not run`
      with the same failures (libsnd vendor library absent, ping, worker thread
      names, fs thread errors — environmental). Correction 8.
- [x] Benchmark: `./benchmark/language/run.sh 10` on the macOS host; compare with
      `benchmark/language/baseline/mfb-O1.log` via `./benchmark/language/rank.py`; record the
      `recurse` and `tree` rows (call-heavy) in Corrections/Summary. No baseline
      regeneration. Done with the final (phase-4) compiler and, same session, the
      `89a4dce2c` one (`MFB=… ./benchmark/language/run.sh 10`: checksums 515 keys,
      0 mismatched), then the three `-O1` benchmark binaries re-run alternately
      (min of runs). Recorded in Correction 13 — including one real regression.

Acceptance: every aarch64 prologue with ≥ 2 adjacent saves uses `stp`; output of
every runtime fixture is unchanged.
  Check: `cargo test --release --test rt_codegen_forms frame_saves_use_stp` → pass (est. 2 min);
  `linux-runtime-proof.sh … 2223 linux-aarch64` → 0 mismatches (est. 25 min — the only check that executes Linux aarch64 code; nothing smaller runs a Linux prologue).
  Result (2026-10-25): `bash scripts/test-fast.sh --test rt_codegen_forms --test rt_foreign_entry_callee_saved --test rt_runtime_helper_callee_saved --test app_special_keys --test codegen_canvas_metal_off_frame_path --test codegen_canvas_vulkan_off_frame_path`
  → all steps ok (`rt_codegen_forms`: 3 passed); Linux aarch64 on 2226 (2223
  refused ssh) → verdicts identical to the starting compiler (Correction 8).
Commit: dd383c356 (preceded by `5276db06b`, the vreg_frame test motion, and `ad873009b`, the fixes of Correction 6)

### Phase 4 — body pairing row (Level 1)

- [x] `src/optimizer/opt2/pairing.rs`: `pair_adjacent_accesses` (§4.4) with unit
      tests: every legality rule as a positive/negative pair, reversed offset
      order, cross-label adjacency (not merged), different `source` (per Open
      Decision 2). (`pairing/tests/mod.rs`, 8 tests; `source` is not a
      legality rule — Correction 2 — so the test pins that the pair keeps the
      first half's `source`. Also refused: a sentinel base, an `sp` data
      register, an instruction carrying any other field.)
- [x] `src/optimizer/opt2/machine.rs:finish` (the hook of §4.4), called at the
      end of `finalize_frame` on both return paths; it runs
      `pair_adjacent_accesses` gated `level_enabled(1)` and `supports_pair_access()`.
      Unit test: a frameless leaf function still reaches the hook.
      (`a_frameless_leaf_reaches_the_machine_hook_and_level_zero_skips_it`.)
- [x] `src/optimizer/catalog.rs`: row "Load/store pairing", level 1, stage
      `machine`, summary; counter in `src/optimizer/stats.rs`. (Named "Load/store
      pair formation (ldp/stp)" to match its `planning/optimizations.md` row,
      as every catalog name does; counter `LOAD_STORE_PAIRS`.)
- [x] `planning/optimizations.md`: mark "Aggregate load/store combining" `Y`
      with the landing note (scope: adjacent only, AArch64 only). ("Load/store
      pair formation (ldp/stp)" is the row this lands, marked `Y` with the
      measured note; "Aggregate load/store combining" is `~` — its vector and
      non-adjacent halves are not done; "Prologue / epilogue optimization"
      notes the paired saves.)
- [x] `tests/runtime/rt_regalloc_levels_agree.rs`-style agreement: add the
      Phase 3 fixture to the `-O0` vs `-O1` agreement list (or a sibling test if
      that file's list is not extensible). (The file has no list: sibling test
      `frame_and_body_pairs_agree_at_every_level`, -O0..-O3.)
- [x] Golden re-baseline as Phase 3 (aarch64 goldens only), with the census diff
      quoted: candidates before, pairs formed after.
      Result: gate with the phase-4 compiler over the phase-3 goldens → 73
      diffs, the same 73 aarch64 goldens (none x86-64/Windows/riscv64); each
      rebuilt with the phase-3 and phase-4 compilers and compared by the
      expansion probe → 73/73 equal apart from the new pairs (49,142 body
      pairs). Full test-accept first: 1758 ran, 9 mismatches = the raw
      goldens; every run output matched. Regenerated (73 changed), gate re-run →
      `2389 golden(s) checked, 0 diff(s)`. Census (`ncode-census.sh`,
      macos-aarch64, 56 projects): phase 3 → phase 4 instructions 26,938,356 →
      26,594,781, `stp` 24,131 → 284,906, `ldp` 321,582 → 404,382, `str`
      candidates 453,548 → 67,427, `ldr` 139,547 → 40,248. From the
      `89a4dce2c` baseline: 27,311,174 → 26,594,781 instructions (−2.6 %);
      linux-aarch64 27,279,152 → 26,561,935. The four questions are Phase 3's.
      Linux aarch64 on 2226 with the phase-4 compiler: `899 passed, 11 failed,
      1 not run`, verdict list (`VERDICTS=`) identical to the `89a4dce2c`
      compiler's, 911 lines.
- [x] Sweep (code review, this form): run `census.py` on the corpus; for the 20
      functions with the most remaining unpaired adjacent-candidate rejections,
      read the producing builder (`-ncode` function name → emitter) and decide per
      site: convert to `abi::store_pair_u64`/`load_pair_u64` at the builder when
      the pair is structural (header word pairs written together, e.g. a string
      header length+capacity), leave otherwise. "Makes sense" = the two
      accesses are always adjacent in that builder's output and the pair is
      legal for every register the allocator can give them. Record the list of
      converted and declined sites in Corrections. (Correction 11: no site
      converted.)

Acceptance: `-O1` forms pairs, `-O0` forms none, and both run identically.
  Check: `cargo test --bin mfb opt2::pairing` → pass (est. 2 min);
  `mfb build -q -ncode -O0` vs `-O1` of the Phase 3 fixture → `ldp`/`stp` count 0 body pairs at `-O0`, > 0 at `-O1` (est. 1 min);
  `cargo test --release --test rt_regalloc_levels_agree` → pass (est. 6 min).
  Result (2026-10-25): `opt2::pairing` 8 passed; the fixture's dump at `-O0` has
  25 `stp` / 38 `ldp`, every one a frame save of a `calleeSaved` register
  (0 body pairs), at `-O1` 42 / 163 with at least 17 body pairs (pairs whose
  registers are not all frame saves); `bash scripts/test-fast.sh --test rt_codegen_forms --test rt_regalloc_levels_agree --test rt_foreign_entry_callee_saved --test rt_runtime_helper_callee_saved --test app_special_keys --test codegen_canvas_metal_off_frame_path --test codegen_canvas_vulkan_off_frame_path`
  → one red, `zero_fill_tests::the_arena_fill_stores_only_zero_words`
  (Correction 12), fixed, then that test and fmt green; every listed binary
  passed (`rt_regalloc_levels_agree` 2 passed); optimizer sweep
  `cargo test --bin mfb builtins_codegen::optimizer::` → 3 passed.
Commit: 2e315b138 (with `451f8f49b`, the test correction of Correction 12)

### Phase 5 — spec and docs

- [x] `src/docs/spec/architecture/14_aarch64-instruction-set.md`: `stp`/`ldp` in
      the op catalog with a worked encoding; frame section notes pair saves.
      (In `dd383c356`, Correction 9; the frame itself is owned by
      `memory native-calling-convention`, which the ISA topic points to.)
- [x] `src/docs/spec/architecture/17_mir-instruction-set.md`: the two mirror ops
      and the `dst2`/`src2` field roles. (`dd383c356`.)
- [x] `src/docs/spec/memory/06_native-calling-convention.md` (or whichever topic
      `mfb spec memory --all | grep -n 'callee-saved'` names as owner): save/restore
      instructions. (`dd383c356`, "Stack Frame, Prologue, and Epilogue".)
- [x] `src/docs/spec/tooling/10_debugger.md`: the compiler frame shape the
      backtrace reads. (`dd383c356`.) The pairing row itself: `architecture
      native` and `tooling cli-reference` in `2e315b138`.
- [x] `.ai/codegen-invariants.md`: one durable entry — pairing runs after
      `finalize_frame` on final offsets; never pair across a label or into a base.

Acceptance: specs render with no leaked `[[` and every new citation resolves.
  Check: `cargo test --bin mfb spec` → pass; `target/release/mfb spec architecture --all | grep -c '\[\['` → 0 (est. 2 min).
  Result (2026-10-25): the spec tests ran green in both landing-tier runs
  (`spec_citations_resolve` included); `mfb spec architecture --all` and
  `memory --all` → 0 `[[`; `tooling --all` → 1, the debugger's own
  `[[sp+0x10]+8]` location syntax, identical with the `89a4dce2c` compiler;
  `mfb man optimizations` renders the row "Load/store pair formation (ldp/stp)".
Commit: this commit

## Validation Plan

- Tests: encoder/decoder round trip (`src/arch/aarch64/encode/tests.rs`,
  `decode/tests.rs`), frame unit tests (`vreg_frame.rs`), pairing unit tests,
  backtrace `state_at` tests, `tests/runtime/rt_codegen_forms.rs`,
  `tests/rt-behavior/functions/frame-pair-saves`.
- Coverage check: the frame change is reached by every compiled function; the
  pairing row by any `-O1` build — confirm with `mfb build -vv` that the row's
  counter fires on the fixture.
- Runtime proof: acceptance on macOS; `linux-runtime-proof.sh` on 2223
  (`linux-aarch64`). No x86/rv64 execution needed — no x86/rv64 byte changes
  (verified by the golden classification).
- Doc sync: Phase 5 topics; `mfb man optimizations` renders the new row
  (`target/release/mfb man optimizations | grep -c 'Load/store pairing'` → 1).
- Final gate (once, after Phase 5): `cargo test --no-fail-fast > /tmp/171a.log 2>&1; echo EXIT=$?` → `EXIT=0`, `grep -c '^failures:' /tmp/171a.log` → 0 (est. 60–90 min; it includes `artifact_gate_all`).

## Open Decisions

1. Frame pre-/post-index fold — **recommend doing it** (saves one instruction
   per prologue and epilogue on small frames) vs. offset-form only (simpler,
   one less boundary). (§4.3)
2. Equal-`source` requirement for body pairs — **recommend requiring it at every
   level** (keeps `--debug` line stepping exact; Phase 1 measures the loss; if
   the loss is > 50 % of candidates, revisit by requiring it only for `--debug`
   builds). (§4.4)

## Corrections

1. **§2.2's golden table was stale.** The seven `.ncode` goldens were regenerated
   many times between 2026-10-04 and `89a4dce2c` (plans 182/183, call-graph DCE),
   so the prototype now prints different numbers (e.g. `control_flow_if` 5800
   instructions, 67/89 pairs, was 2598, 35/43). The census tool reproduces the
   prototype exactly on the current goldens; §2.2 carries the re-measured table.
   The plan's own *design* does not depend on those numbers.
2. **There is no per-instruction source position; Open Decision 2 is moot.**
   `CodeInstruction.source` is `core::panic::Location::caller()` — the
   *compiler's* emitting call site (plan-71-C audit metadata, "never
   serialized", `src/codegen/engine/types/types.rs:CodeInstruction`) — not an
   MFBASIC line, and plan-166 debug info is symbols only ("No DWARF, no line
   tables", `planning/completed/plan-166-A-…`). Nothing steps by line, so equal
   `source` protects no observable and would only compare Rust emitter lines.
   The pairing row requires no source equality and keeps the first half's
   `source`.
3. **A pre-existing riscv64 compiler panic surfaced in the census.**
   `mfb build -ncode -target linux-riscv64 benchmark/language/mfb` (with its
   `bench_workers` package installed) panics at
   `src/arch/riscv64/select.rs:select_riscv64`: `rv64 v128: function needs 172
   concurrent vector slots, exceeding the 126-slot region` — with the release
   `mfb` of `89a4dce2c`, before any plan-171 change. Unrelated to pairing
   (riscv64 never pairs); tracked as its own bug (see the Summary).
4. **The constructors and `supports_pair_access` land with Phase 3, not Phase 2.**
   Phase 2 has no producer, so `abi::store_pair_u64`/… and
   `Backend::supports_pair_access` would have no reader there, and the tree is
   kept free of dead code (AGENTS.md: no "consumed by a later phase"
   suppressions). Phase 2 lands the op, its encoding, decoding, roles and the
   peephole model; Phase 3 lands the constructors and the backend flag with
   `finalize_frame`, their first caller.
5. **The fold boundary is 496/512, not 504/512.** `total_stack_size` is always a
   multiple of 16 on AArch64 (`outgoing_bytes` and the aligned save+locals are,
   and `call_padding` is 0), so the largest frame the post-index `ldp` (+504)
   can release is 496; the tests pin 496 (folded) vs 512 (not). The same holds
   for the imm7 offset boundary: integer pairs start at 16-aligned offsets, so
   the tests pin a pair at 496 (paired) vs 512 (two singles).
6. **Phase 2 reddened the riscv64 decoder guard.** The explicit refusal arm in
   the riscv64 encoder's dispatch read as an emitted form to
   `riscv64::decode::round_trip::decodes_every_form_the_encoder_emits`, and
   Phase 1's script was missing from `tests/gate/gate_lock_covers_every_writer.rs`'s
   classification; the landing tier caught both before Phase 3 landed; fixed in
   `ad873009b` (refusal moved ahead of the dispatch; script classified exempt).
7. **The frame change also moves `.mir` goldens.** The `-mir` dump shows the
   hand-written runtime helpers over their finalized stream (spec
   `mir-instruction-set`), so the two macOS `.mir` goldens moved with the
   `.ncode`s. Expected target, classified like the rest.
8. **Box 2223 was unreachable; 2226 stood in.** `ssh -p 2223` → connection
   refused (2222, 2225, 2231 too). Linux aarch64 execution ran on 2226 (Debian 12
   aarch64, glibc 2.36), verdict-for-verdict against the starting compiler.
9. **Spec topics land with the phase that makes them observable.** Rather than
   a separate Phase 5, the ISA/MIR/calling-convention/debugger topics land with
   Phase 3 (the first producer) and the pairing row's with Phase 4
   (`.ai/specifications.md`: same change). Phase 5 keeps the `.ai` entry and
   the render checks.
10. **The full cargo suite runs once, as the Final gate.** Each re-baseline is
   gated per AGENTS.md by `artifact-gate.sh … all` and a full `test-accept.sh`,
   plus `scripts/test-fast.sh`; the 60–90 min `cargo test --no-fail-fast` runs
   at the end over all phases.
11. **Phase 4 sweep: no builder site converted.** After the row, the corpus's
   remaining adjacent candidates (macos-aarch64, `census.py` over the
   `KEEP=` dumps) are of two kinds, neither a builder pairing would fix:
   (a) 100,706 rejected for `range` — the lower offset is past +504 in a large
   frame (`_mfb_fn_main` of the examples: up to 3,116 per function,
   `_mfb_fn_drawBug` 1,447, `_mfb_macapp_program` 933, …); a builder `stp`
   cannot reach them either, it needs a materialized base (plan-171-G's
   folded-offset forms) or hot-slots-first layout (plan-171-J); (b) 3,269 `str` /
   3,575 `ldr` legal candidates, all in 71 hand-written platform bodies that are
   not finalized by `finalize_frame` and so never reach the hook — the top 20 by
   count: `_mfb_macapp_metal_draw` 25, `_mfb_macapp_term_drawRect` 23,
   `_mfb_macapp_term_setFrameSize` 23, `_mfb_macapp_metal_builder` 22,
   `_mfb_macapp_term_writeString` 20, `_mfb_macapp_term_drawText` 20,
   `_mfb_macapp_term_drawLine` 19, `_mfb_macapp_term_fillRect` 18,
   `_mfb_macapp_term_drawBox` 17, `_mfb_macapp_key_down` 16,
   `_mfb_macapp_term_keyDown` 16, `_mfb_macapp_term_drawGlyph` 15,
   `_mfb_macapp_metal_init` 14, `_main` 12, `_mfb_macapp_term_scroll` 12,
   `_mfb_macapp_canvas_blit` 12, `_mfb_rt_thread_trampoline` 12,
   `_mfb_macapp_key_inject` 10, `_mfb_macapp_term_init` 10,
   `_mfb_macapp_reconcile_canvas` 10. Declined: these are the macOS app
   backend's Objective-C IMPs, the program entry and the thread trampoline —
   once-per-event or once-per-thread code whose frames and callee-saved
   brackets are hand-built (bug-982's foreign-entry rules); their stores are not
   structural header pairs, and routing them through `machine::finish` is a
   change to the hand-written backends, not to a builder. Every
   `finalize_frame`d function has no legal candidate left (the row merges them
   all), so a builder-level `abi::store_pair_u64` would emit the same bytes the
   row already does.

12. **One unit test counted `str_u64` ops where it meant stored words.**
   `codegen::memory::arena::arena::zero_fill_tests::the_arena_fill_stores_only_zero_words`
   (plan-183-A, `260efd0d7`) required ≥ 9 `str_u64` in the AArch64 zero fill to
   pin "the eight-word loop and the one-word tail", and that every store writes
   the zero register. The pairing row turns the loop's eight adjacent `str xzr`
   into four `stp xzr, xzr`, so the op count fell while the fill still stores
   nine zero words. Four questions: (1) written by plan-183-A for the fill's
   shape and its zero-only stores; (2) it protects "the fill writes only zero,
   eight words a turn plus a tail"; (3) nothing else reads it; (4) the count was
   of an op, not of words — a pair is two words. Corrected only that count: it
   now counts words (two per `stp`) and checks both of a pair's sources are the
   zero register, which is stricter than before.
13. **Benchmark: −2.4 % overall, one row +16 %, root-caused to `stp` on Apple M2.**
   Apple M2 Max, `-O1` benchmark binaries of `89a4dce2c` (base), phase 3 and
   phase 4 (final) re-run alternately, min of runs, 452 rows: phase 3/base
   geomean 0.9865, final/phase 3 0.9891, final/base **0.9757**. `recurse`: fib
   46.96 → 47.14 ms (noise), ackermann 2.736 → 2.610; `tree`: binarytrees
   25.51 → 24.70, treewalk 14.46 → 13.22. (The committed
   `baseline/mfb-O1.log` binarytrees 13.15 ms is stale against today's base
   compiler, 25.5 ms — pre-existing, not this change.) **`dispatch/union`
   regresses 13.04 → 15.15 ms (+16 %),** caused by phase 3. Isolated in a
   one-function program (`evalNode` alone: 13.1 → 16.6 ms) and proven
   layout-independent: the base binary patched in place — `str;str` →
   `stp;nop`, same addresses — reproduces it (13.4 → 17.3 ms), while patching
   only the `x21/x22` pair costs nothing and only the `x23/x24` pair costs it
   all; padding the program's layout eight ways never moves base above 13.5 ms
   or final below 16.3. `x23` holds the left operand's result across the
   second recursive call, so the callee's save/restore of it is on the
   critical path; a `str`/`ldr` round trip of it is fast on this core and the
   same round trip through `stp`/`ldp` (or `stp` + `ldr`, or `str` + `ldp`) is
   not — consistent with M2's store-to-load bypass applying only to matching
   single-register accesses. A same-shape assembly loop with no value on the
   critical path shows no difference (8.2 ms all three forms). Kept as
   designed (the plan's goal, net −2.4 %), and recorded here and in the report
   as an open trade-off for the owner: a selective rule ("don't pair a save
   whose register carries a value across a call") would need the
   allocator's liveness at frame time.
14. **Pre-existing doc drift fixed on the way:** `bugs/status.md` had no row for
   `bug-1109` (its own check printed `['1109']` at `89a4dce2c`); added beside
   the new `bug-1112` row (Correction 3's panic).
## Summary

Risk sits in the epilogue `sp` fold and in load pairing's register rules; both
land behind round-trip tests, frame unit tests and Linux execution.

**Landed (2026-10-25):** the census tool (`4dcdd0ef5`), the op (`2b7058db8`,
`ad873009b`), paired frames (`dd383c356`), the Level-1 row (`2e315b138`,
`451f8f49b`), docs (`56fad9ca2`). macos-aarch64 corpus: 27,311,174 → 26,594,781
instructions (−2.6 %), 284,906 `stp` and 404,382 `ldp`; x86-64, Windows and
riscv64 byte-identical. Linux aarch64 proven on box 2226 (2223 unreachable).
Open: the `dispatch/union` regression (Correction 13) and `bug-1112`
(Correction 3, pre-existing). x86-64 and
riscv64 are untouched by design. B–K reuse the census tool and the
`rt_codegen_forms` test file this letter creates.
