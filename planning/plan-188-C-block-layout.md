# plan-188-C: Block layout — cold exits out of line, fall-through inversion

Last updated: 2026-10-26
Effort: large (3h–1d)
Depends on: plan-188-B (the `b.hs` op; letter order)

Two layout defects sit in every function the compiler emits. First, a check's
raise block and a fallible call's error-propagation exit are emitted inline, right
after the branch that skips them, so the path that runs — the passing one — takes
a branch over code that almost never executes: 132,938 such blocks, 2,464,131
instructions (9.1 % of all emitted code) in the macos-aarch64 corpus. bug-774
moved the raise blocks of eight emitters out of line (`emit_cold_block`); the
other 219 `raise_error_bare` callers and every call-propagation exit were never
converted. Second, 113,731 places branch around an unconditional jump
(`b.cc L1; b L2; L1:`), one instruction and one taken branch more than
`b.!cc L2`. This sub-plan adds two Level-1 MIR rows that fix both, generally, on
all five targets, and closes the catalog rows "Hot/cold splitting", "Machine block
reordering + fall-through inversion", "Code layout / block placement" and
"Branch layout / prediction hints". The outcome: at `-O1` no cold exit sits on the
fall-through path of a conditional branch, no conditional branch jumps over a lone
unconditional jump, and every program's output, error codes and error locations
are unchanged.

References:

- `src/codegen/engine/builder/builder_emit_helpers.rs:emit_cold_block`,
  `place_cold_block` (bug-774's builder-level placement, and its rules);
  `.ai/codegen-invariants.md` "Out-of-line raise blocks: only for builders that
  flush them (bug-774)".
- `src/codegen/engine/regalloc/builder_registers.rs:run_register_allocation`
  (cold code then error-return pads are appended before the MIR seam runs).
- `src/codegen/engine/control/builder_exits.rs:error_return_pad_label`,
  `route_current_result_to_trap`; `builder_emit_helpers.rs` (the `call_ok` /
  `call_value_ok` sites); `builtins/collections/gen_memory.rs` (`reduce_call_ok`),
  `builtins/collections/func_transform.rs` (`transform_call_ok`).
- `src/optimizer/opt2/mod.rs:optimize_mir`, `src/optimizer/opt2/merge.rs` (the
  Level-1 branch-to-next elimination this row extends), `simplifycfg.rs`.
- `src/arch/ops.rs:CodeOp`, `src/arch/riscv64/select.rs:int_branch`,
  `float_branch` (`rv.br` conditions), `src/arch/x86_64/select.rs` (`x86.j*`).
- plan-188-A §2.4 (assessment), plan-188-B §4.1 (`b.hs`).

## Prerequisites

The family's prerequisites are in plan-188-A. In addition:

| Must be true | Command | Status |
|---|---|---|
| plan-188-B complete (`b.hs` exists on every backend) | `ls planning/plan-188-B-* → no matches` (archived) | NOT MET (2026-10-26) |
| plan-169 complete (Win64 unwind tables describe every epilogue, including the ones this plan moves to the cold tail) — already implied by plan-188-B's plan-171 prerequisite through plan-171-H | `ls planning/plan-169-* → no matches` (archived) | NOT MET (2026-10-26) |

Everything below is written against the world where it holds.

## 1. Goal

- New `CodeOp` `b.pl` (AArch64 `PL`, `N = 0`) and `CodeOp::inverse_branch`, the
  exact complement of every conditional branch the selected streams carry.
- Row "Hot/cold splitting" (Level 1, stage MIR, `src/optimizer/opt2/layout.rs`):
  every cold exit block (§4.2) that sits on the fall-through path of a conditional
  branch moves to the function's cold tail, and the branch is inverted to reach it.
- Row "Machine block reordering + fall-through inversion" (Level 1, stage MIR, same
  module): `br L1; b L2; label L1` → `br' L2; label L1`.
- Every output, error code and error location is unchanged on all five targets;
  `-O0` is byte-identical to today.

### Non-goals (explicit constraints)

- No profile, no heuristic beyond §4.2's structural cold test: a user's early
  `RETURN` inside an `IF` is not moved (3,733 such blocks, measured below).
- No loop rotation of machine loops: measured, no gain on this host (§2.3).
- No branch-prediction hint encodings (none exist that current cores honour, §2.4
  of plan-188-A).
- No move of a block that leaves by a branch into the body (a `TRAP` handler, a
  raw-capture label) — bug-774's rule (2).
- No change to `emit_cold_block` or to which builders may move code (rule (1): the
  rows run only inside `run_register_allocation`, through `optimize_mir`).

## 2. Current State

### 2.1 Shapes

A raise site emits `b.<pass> ok; <raise block ending in ret or b error_return_pad_N>;
ok:` — the passing path takes the branch. A fallible call emits
`cmp_imm tag, OK; b.eq call_ok_N; <cleanup + ret>; call_ok_N:` — the success path
takes the branch. `route_current_result_to_trap` emits
`b.ne trap_route_error_N; ret; trap_route_error_N:` on an error path.
`emit_cold_block` already moves its own blocks after the body (eight callers,
`rg -n "emit_cold_block(" src | grep -v "fn emit_cold_block\|tests" | wc -l` → 8;
`raise_error_bare` has 219 callers, same command → 219).

`run_register_allocation` appends `cold_instructions`, then the error-return pads
(labels `error_return_pad_N`, `builder_exits.rs:error_return_pad_label`), then runs
`optimize_mir`. So when the MIR seam runs, the stream is body, cold code, pads.

### 2.2 Measured populations

Post-frame `.ncode` dumps of the census corpus, base release `mfb` of `d58b5397a`
(`KEEP=` dumps, `python3 -I /tmp/p188/coldcls.py <dumps>` and
`/tmp/p188/layoutcc.py <dumps>`). A "cold candidate" is a conditional branch to
`L`, then a label-free run ending in `ret` or `b error_return_pad_*`, then
`label L`. The rows run before allocation; the counts are the post-frame
census of the same shapes (allocation renames registers and adds spill code
inside blocks; frame finalization adds the prologue, the epilogues before each
`ret` and the entry stack-limit check, none of which is a candidate), so they are
the measured size of the target, not a prediction of the row's `-v` count.

| Cold class (§4.2) | macos-aarch64 blocks / instrs | linux-x86_64 | linux-riscv64 |
|---|---|---|---|
| (a) contains an error-construction call | 101,008 / 1,974,702 | 100,998 / 1,954,486 | 16,264 / 337,589 |
| (c) `call_ok` continuation | 30,237 / 479,153 | 30,237 / 507,879 | 4,163 / 107,614 |
| (c) `trap_route_error` continuation | 1,693 / 10,276 | 1,693 / 6,772 | 1,227 / 12,190 |
| total moved | 132,938 / 2,464,131 | 132,928 / 2,469,137 | 21,654 / 457,393 |
| not moved: `if_else` (a user early `RETURN`) | 3,733 / 32,947 | 3,733 / 33,755 | 763 / 10,283 |
| all instructions | 27,057,789 | 30,519,671 | 4,506,516 |

(b) — a block ending in `b error_return_pad_*` with no error call — counted 0 on
every target: every pad-bound block also builds the error, so it is in (a). The
smaller `call_value_ok` (22), `transform_call_ok` (43) and `reduce_call_ok` (13)
continuations (macos-aarch64) are in (c) too, and not in the totals above.

| Fall-through inversion `br L1; b L2; label L1` | count | of which need `b.hs` / `b.pl` |
|---|---|---|
| macos-aarch64 | 113,731 | 67,642 `b.lo` / 804 `b.mi` |
| linux-x86_64 | 113,170 | 67,922 `b.lo` / 0 `b.mi` |
| linux-riscv64 (`rv.br`) | 7,792 | none (`rv.br` conditions pair already) |

### 2.3 Loop rotation, measured and declined

The corpus has 268,131 top-tested machine loops (`label W; <straight cond>;
b.cc X; …; b W; label X`, `/tmp/p188/layout.py`, macos-aarch64; 51,628 on
riscv64), mostly the builders' word-copy loops. Rotating one saves a taken
unconditional branch per iteration and adds one entry jump. Probe
(`/tmp/p188/rot/r.c`, the copy loop's exact instruction shape in inline assembly,
this host): 4096-byte copies 0.130 s top-tested vs 0.130 s rotated, 512 bytes
0.130 vs 0.132, 64 bytes 0.184 vs 0.188 — no gain, so it is not built.

### 2.4 Condition complements

AArch64 `B.cond` codes pair by their low bit (`EQ/NE`, `HS/LO`, `MI/PL`,
`VS/VC`, `HI/LS`, `GE/LT`, `GT/LE`): `cond` and `cond ^ 1` are exact complements
for every flag state, including after an unordered `fcmp`. x86-64 `jcc` opcodes
pair the same way (`jb/jae`, `je/jne`, `ja/jbe`, `jp/jnp`, `jo/jno`, `jl/jge`,
`jg/jle`, `js/jns`), and the neutral `b.*` ops encode 1:1 to them after the
selector has turned float branches into `x86.j*`. riscv64 `rv.br` conditions pair
`eq/ne`, `lt/ge`, `ltu/geu`. The neutral set lacks only `b.hs` (plan-188-B adds it)
and `b.pl` (this plan).

## 3. Design Overview

Both rows are structural rewrites over the selected, pre-allocation stream, in
`optimize_mir` after `UCE (MIR)` and before `CFG simplification` (so
`simplifycfg`/`merge` sweep the labels they strand). Neither changes an
instruction that computes anything; they move whole blocks and replace a branch by
its complement. That is why they qualify for Level 1 under the 2026-10-24
default-level exception (`planning/optimizations.md`, "Review of 2026-10-24"):
no value/trap/order change, no dataflow beyond a label census, cost only to
debugger fidelity.

**Correctness risk** concentrates in (1) the complement table — a wrong pair
silently inverts a check, and (2) the insertion point — a moved block whose exit
branch lands before it becomes a backward branch that the live-range analyses
read as a loop (bug-774). Both are closed structurally (§4.1, §4.2) and pinned by
unit tests, and by cross-level runtime equality on five targets.

**Byte-identity is not the gate.** Every target's `-O1` goldens move (every
function with a check). Expected, classified by runtime equality, not bytes:
allocation runs over the new order, so registers and spill code may differ too.

Rejected alternatives:

- **Converting the 219 `raise_error_bare` callers to `emit_cold_block`.** Level 0
  (ungated), 219 edits, and still misses the call-propagation exits. The MIR row
  reaches every site at once and is on the dial.
- **Identifying propagation exits by the tag compare after a `bl`** instead of by
  the continuation labels. After selection the tag register is a physical return
  register that other code also compares with 0; the five label stems are each
  minted only by a fallible-call success test (verified, §4.2), the precedent
  `opt2/checks.rs` set for its division-check labels.
- **Doing the move after allocation.** Pre-allocation is what bug-774 does, and it
  is what lets the allocator stop counting cold code inside hot intervals.

## 4. Detailed Design

### 4.1 `b.pl` and `inverse_branch`

`CodeOp::inverse_branch(self) -> Option<CodeOp>`: `b.eq↔b.ne`, `b.ge↔b.lt`,
`b.gt↔b.le`, `b.vc↔b.vs`, `b.hi↔b.ls`, `b.lo↔b.hs`, `b.mi↔b.pl`,
`x86.jae↔x86.jb`, `x86.ja↔x86.jbe`, `x86.je↔x86.jne`, `x86.jp↔x86.jnp`; `None`
for everything else. `rv.br` is inverted by its `cond` field
(`eq↔ne`, `lt↔ge`, `ltu↔geu`; any other value: not inverted). A unit test asserts
the table is an involution and, for AArch64, that every pair encodes to condition
codes differing only in bit 0; for x86, to `jcc` opcodes differing only in bit 0.

`b.pl`: AArch64 `B.PL` (cond `0b0101`), relaxed like every conditional branch;
x86-64 `jns` (`79`/`0F 89`, with plan-171-K's short/near forms); riscv64 select:
the float arm is the exact complement of `"b.mi" | "b.lo"` (`rv.fcmp lt`, then
`rv.br eq`); an integer `b.pl`, like an integer `b.mi`, is produced by no lowering
and stays outside `int_branch`. It joins every branch table `b.hs` joined in
plan-188-B §2.3, `mir_ops!`, `flagslive` (reader), and `tests/common/ncode_emu.rs`
(`N` of the last compare).

### 4.2 Hot/cold splitting (cold exit placement)

`layout::place_cold_exits(instructions) -> u64`, gated `level_enabled(1)`:

1. Insertion point `P`: the index of the first `label` whose name starts with
   `error_return_pad`, else the stream's length.
2. Candidates: index `j` holding a branch with an inverse (or an invertible
   `rv.br`), target `L`, `j < P`; indices `j+1..=k` contain no `label`; `ins[k]` is
   `ret`, or `b X` where `X` is a label at index `≥ P`; `ins[k+1]` is `label L`;
   and the block is **cold**:
   (a) some `bl` in it targets `_mfb_make_error_result`, `_mfb_rt_park_error` or
       `BUILD_ERROR_LOC_SYMBOL` (`_mfb_build_error_loc`) — the error-construction
       entry points; or
   (b) `ins[k]` is `b error_return_pad_*`; or
   (c) `L` is `call_ok_N`, `call_value_ok_N`, `transform_call_ok_N`,
       `reduce_call_ok_N` or `trap_route_error_N` — a fallible call's
       success-continuation label (minted only at `builder_emit_helpers.rs`
       (`call_ok` ×2, `call_value_ok`), `func_transform.rs`, `gen_memory.rs`,
       `builder_exits.rs:route_current_result_to_trap`; verified with
       `rg -n '"call_ok"|"call_value_ok"|"trap_route_error"|"transform_call_ok"|"reduce_call_ok"' src`).
3. Rewrite each candidate, in stream order: mint a fresh label `cold_exit_N`
   (unique in the function — checked against its label set), replace `ins[j]` by
   its inverse targeting `cold_exit_N` (same `source`; `rv.br` keeps its operands
   and flips `cond`), cut `j+1..=k`, and append `label cold_exit_N` + the block to
   a list inserted, in order, at `P`.

Why it is exact: the block had one entry (the fall-through of `ins[j]`; no label
inside it) and one exit (its last instruction, which leaves the function or goes
to a pad at or after `P`), so moving it changes no path; the inverted branch sends
exactly the inputs that used to fall into the block to it, and the rest fall to
`L` as they used to branch there. Every exit stays a forward branch, which is
bug-774's condition for the live-range analyses.

Counter `stats::COLD_EXITS_PLACED`.

### 4.3 Fall-through inversion

`layout::invert_fallthroughs(instructions) -> u64`, gated `level_enabled(1)`, run
after §4.2: `ins[j]` an invertible branch to `L1`, `ins[j+1]` = `b L2`,
`ins[j+2]` = `label L1` → `ins[j]` becomes the inverse targeting `L2`, `ins[j+1]`
is deleted. Exact: the original sends `cond` to the next instruction and `¬cond`
to `L2`; the rewrite sends `¬cond` to `L2` and lets `cond` fall through. Counter
`stats::FALLTHROUGHS_INVERTED`.

## Phases

> **NOTE — keep the checkboxes current** (template rules).

### Phase 1 — `b.pl`, `inverse_branch`, fall-through inversion

- [ ] `tools/ncode-census/census.py`: metrics `fallthrough_over_jump` (§2.2's
      `br L1; b L2; label L1`, `rv.br` included) and `cold_exit_inline:<class>`
      (§2.2's candidate shape, classified (a)/(b)/(c)/`other` exactly as §4.2), and
      their README rows. Re-run on the base compiler: the totals equal §2.2's
      table (macos-aarch64 113,731 and 132,938).
- [ ] `b.pl` through the tables of §4.1, both encoders, relaxation, the riscv64
      float arm, `flagslive`, the emulator; `CodeOp::inverse_branch`.
- [ ] `src/optimizer/opt2/layout.rs` with `invert_fallthroughs`; call it in
      `optimize_mir` after `UCE (MIR)`; catalog row "Machine block reordering +
      fall-through inversion" (level 1, stage `MIR`); stats counter.
- [ ] Unit tests (`src/optimizer/opt2/layout/tests/mod.rs`): every invertible op
      and `rv.br` cond inverts; a label between `br` and `b`, or after `b`,
      declines; a non-invertible branch declines; `-O0` untouched. Encoder round
      trips for `b.pl` (AArch64 near/relaxed, x86 short/near); the involution and
      bit-0 tests of §4.1.
- [ ] Golden re-baseline, every target (procedure of plan-188-A Phase 1; classify
      by `census.py --diff` on one fixture per target: the drop in `b` equals the
      row's `-v` count for it).
- [ ] `rt_regalloc_levels_agree`: add `tests/rt-behavior/control-flow/control-flow-if`
      and `tests/rt-behavior/control-flow/optimizer-loop-shapes-rt`.

Acceptance: the shape is gone at `-O1` and outputs agree across levels.
  Check: `cargo test --bin mfb opt2::layout` → pass (est. 2 min);
  `cargo test --release --test rt_regalloc_levels_agree` → pass (est. 4 min);
  `KEEP=/tmp/188c tools/ncode-census/ncode-census.sh target/release/mfb macos-aarch64 /tmp/188c.tsv`,
  then `census.py --top fallthrough_over_jump 50 <the KEEP dumps>` lists no
  `_mfb_fn_*`/`_mfb_ifn_*` function (those are lowered through
  `run_register_allocation`, so the row sees them; what remains is hand-built
  helper code the MIR seam never sees — record its total in Corrections)
  (est. 25 min — the corpus build is the only check that sees every producer's
  branches).
Commit: —

### Phase 2 — cold exit placement

- [ ] `layout::place_cold_exits` (§4.2), called before `invert_fallthroughs`;
      catalog row "Hot/cold splitting" (level 1, stage `MIR`); stats counter.
- [ ] Unit tests: each cold class moves; an `if_else` early return stays; a block
      ending in a branch to a label before `P` stays; a block with a label inside
      stays; two candidates keep their relative order in the tail; the inverted
      branch targets the moved block; `rv.br` flips; label uniqueness against an
      existing `cold_exit_0`; `-O0` untouched.
- [ ] `tests/runtime/rt_codegen_forms.rs::cold_exits`: in the `-O1` aarch64 dump
      of `tests/rt-behavior/control-flow/control-flow-if`, no `bl
      _mfb_make_error_result` sits between a conditional branch and its target
      label; at `-O0` the inline shape is present.
- [ ] rt-error coverage: every `tests/rt-error/**` golden is unchanged
      (`test-accept.sh` compares their output, which carries code and location).
- [ ] Golden re-baseline, every target.
- [ ] Runtime: `bash scripts/test-accept.sh target/release/mfb /tmp/188c-accept`;
      `bash scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64`;
      `… 2227 linux-x86_64`; `… 2229 linux-riscv64`; the rt-error fixtures and
      `control-flow-if` built `-target windows-x86_64` and run on
      `test@192.168.1.206`, output equal to the goldens; plan-169-C's
      attach-and-backtrace check (box 206, twenty gdb attaches to the `debug 3000
      20000` example; `planning/completed/plan-169-C-win64-encoder-scratch-slots.md`)
      run on an `-O1` build — every backtrace reaches `_main` with exits now in the
      cold tail.
- [ ] Benchmark (macOS host): `./benchmark/language/run.sh 10` before/after,
      `rank.py`; binary sizes of four corpus programs; record in Corrections.

Acceptance: no cold exit on a fall-through path at `-O1`; every target prints the
goldens; Win64 backtraces still unwind.
  Check: `cargo test --bin mfb opt2::layout` → pass (est. 2 min);
  `cargo test --release --test rt_codegen_forms cold_exits` → pass (est. 2 min);
  the same census run: `census.py --top cold_exit_inline:a 50` (and `:b`, `:c`)
  lists no `_mfb_fn_*`/`_mfb_ifn_*` function, the remaining total recorded in
  Corrections (est. 25 min); the remote runs → 0 mismatches (est. 25 min aarch64, 60–120 min
  x86-64 emulated, 60 min riscv64, 10 min Windows — each the only execution of
  moved code on its target).
Commit: —

### Phase 3 — docs and catalog

- [ ] `mfb spec architecture native` (MIR optimization: the two rows, and that
      cold exits follow the body at `-O1`+); `aarch64-instruction-set`,
      `x86_64-instruction-set`, `riscv64-instruction-set`, `mir-instruction-set`
      (`b.pl`).
- [ ] `.ai/codegen-invariants.md`: extend the bug-774 entry — the MIR row is the
      general mover; a new cold-continuation label stem must be added to §4.2's
      list or its exits stay inline.
- [ ] `planning/optimizations.md`: the four rows' text (this plan's report).

Acceptance: `target/release/mfb man optimizations | grep -c -e 'Hot/cold splitting' -e 'fall-through inversion'` → 2 (est. 1 min).
Commit: —

## Validation Plan

- Tests: layout units; `rt_codegen_forms::cold_exits`; `rt_regalloc_levels_agree`
  additions; every rt-error golden through `test-accept.sh`.
- Coverage check: `mfb build -O1 -v examples/brogue` prints non-zero counts for
  both rows.
- Runtime proof: Phase 2's five-target runs and the Win64 backtrace check.
- Doc sync: Phase 3.
- Final gate (once): `bash scripts/test-fast.sh`; artifact gate `0 diff(s)` after
  classified regeneration; `test-accept.sh` full; `cargo test --workspace
  --no-fail-fast` → `EXIT=0` (est. 60–90 min).

## Decisions

- Both rows at Level 1 under the 2026-10-24 default-level exception — author's
  call (§3); "Hot/cold splitting" moves 2 → 1 and "Machine block reordering +
  fall-through inversion" 2 → 1 in the catalog, with that rationale.
- Cold = §4.2's three structural classes; no frequency guess.
- No machine loop rotation (measured, §2.3).

## Corrections

(Filled in during execution.)

## Summary

The largest layout lever the backend has: about 9 % of emitted instructions leave
the fall-through path, and about 0.4 % disappear. Risk is the complement table and
the insertion point, both structural and unit-pinned; outputs are proven equal on
all five targets.
