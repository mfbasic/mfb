# plan-171-F: Jump tables for dense `MATCH`; `adr` and PAC/BTI decisions

Last updated: 2026-10-04
Effort: large (3h–1d)
Depends on: plan-171-E

MFBASIC has no `SELECT CASE`; `MATCH` replaces it
(`mfb spec language pattern-matching`). Every `MATCH` lowers to a linear chain:
the `NirOp::Match` arm in `src/codegen/engine/control/builder_control.rs` stores
the scrutinee to a stack slot and, per case, reloads it and calls
`lower_match_compare` (`src/codegen/memory/value/builder_value_semantics.rs`) —
`compare_immediate` + `branch_eq` for enum members and union tags,
`compare_registers` + `branch_eq` for scalars — then `b next`. An `n`-case
match costs up to `n` loads, compares and branches; the validation code says so
outright: "MATCH lowers to `cmp tag, <value>` compares (not a dense jump table)"
(`src/codegen/engine/validation/validation.rs:module_tables`). The `interp`
benchmark (an enum-opcode VM dispatching through `MATCH`) and `dispatch`
(union `MATCH`) are dominated by that chain.

This sub-plan adds a multiway-branch op (`br_table`) at every layer — op, MIR,
CFG and every optimizer row that reads branch targets, register allocation,
three encoders, relaxation, decoder, debugger — and lowers dense integer-keyed
`MATCH`es through it. It also records the two decisions the plan owes on
AArch64 `adr` and pointer authentication / BTI.

Outcome: a `MATCH` on an Integer, enum, or union scrutinee with at least
`MIN_CASES` unguarded literal arms whose keys are dense compiles to a bounds
check plus one indirect jump through a branch table on all three targets, and
every output is unchanged.

References: plan-171-A's list, plus
- `src/codegen/engine/control/builder_control.rs` (`NirOp::Match` arm),
  `src/codegen/memory/value/builder_value_semantics.rs:lower_match_compare`,
  `src/ir/lower.rs:lower_match_case`.
- `src/codegen/engine/regalloc/analysis.rs:build_cfg`, `is_block_terminator`,
  `is_unconditional_terminator`; `src/codegen/engine/regalloc/linear_scan.rs`.
- Every opt2 file that reads a `"target"` field — 43 files under `src/optimizer`,
  `src/codegen/engine/{regalloc,mir}`, `src/arch`, `src/codegen/compiler`
  (`rg -l '"target"' src/optimizer src/codegen/engine/regalloc src/codegen/engine/mir src/arch src/codegen/compiler | wc -l` → 43; 408 occurrences).
- `src/arch/aarch64/encode/relax.rs`, `src/arch/riscv64/encode/relax.rs`.
- `src/debugger/backtrace.rs:Effect::of` (`BR` → `Leave`), `src/debugger/disassemble.rs`.
- `planning/optimizations.md` rows "Jump table generation", "Switch lowering strategies".

## Prerequisites

See plan-171-A §Prerequisites. In addition: plan-171-E complete
(`ls planning/plan-171-E-* 2>/dev/null` → no match).

## 1. Goal

- `CodeOp` `br_table` with fields `src` (index, USE), `dst`, `dst2` (two
  def-only scratch vregs, see §4.2), and one `case` label field per entry. Its
  semantics: jump to `case[src]`; the producer guarantees `0 <= src < count`.
- Encodings (each a fixed-size unit whose table is part of the instruction, so
  no data object, no relocation and no unlabeled code exists):
  - AArch64: `adr d, T; add d, d, src, lsl #2; br d; T: b case0; b case1; …`
  - x86-64: `lea d2, [src + src*4]; lea d, [rip + T]; add d, d2; jmp d; T: jmp rel32 case0; …` (5-byte entries)
  - riscv64: `auipc d, 0; slli d2, src, 2; add d, d, d2; jalr zero, 16(d); T: jal zero, case0; …`
- `MATCH` lowering emits, for a dense eligible match, `key` computation once,
  `sub_imm idx, key, min` (or `mov`), `cmp_imm idx, span-1; b.hi default`,
  `br_table idx, …` with holes pointing at `default` (`CASE ELSE` or the match end).
- Eligibility: scrutinee Integer/Byte/enum/union; every non-`ELSE` arm unguarded
  with literal integer, enum-member, or union-member patterns; at least
  `MIN_CASES` distinct keys; `count / span >= DENSITY` (Open Decision 1).

### Non-goals

- No change to `MATCH` semantics, arm order for guards (guarded matches are not
  eligible), or exhaustiveness checking.
- String, Float, Fixed, Money and record scrutinees keep the chain.
- No data-section jump tables (no new relocation kinds; §3 rejected alternative).
- No binary-search lowering for sparse matches (Open Decision 2).

## 2. Current State

- Linear chain as above; union tags are globally canonical per variant *name*
  (bug-80, `recompute_canonical_variant_tags`), so a union's tag set may be
  sparse — the density test decides per match.
- No multiway branch exists anywhere (`rg -n 'jump_table|JumpTable|switch_table' src/codegen src/arch` → 0).
- `build_cfg` takes at most one target edge per terminator, from the field
  named `target` (`last.operand("target")`), plus fallthrough unless the op is
  `is_unconditional_terminator`.
- Opt2 rows that rewrite targets: `threading.rs` (rewrites terminator `target`
  fields), `merge.rs` (branch-to-next deletion; unreferenced-label removal —
  "census = `opt2::uce`'s over-approximation, any field of any instruction",
  so `case` fields are already counted as references), `simplifycfg.rs`,
  `tailduped.rs`, `uce.rs`, `branches.rs`, the SSA overlay
  (`plans/ssa.rs`), loop finder (`plans/mirloops.rs`).
- Census: number of `MATCH` chains with ≥ 3 arms, their key density, and how many
  are guarded — UNMEASURED (Phase 1).
- Debugger: `backtrace.rs` treats `BR` as `Leave { target: None }`, which resets
  the walk to the post-prologue state — correct inside a body; the following
  `b` table entries are in-function `Leave`s with known targets (no reset).

### Verified properties

- *UCE/merge count a label referenced from any field as live* — `merge.rs` doc
  (quoted above); still re-verified in Phase 2 by a unit test.
- *A def-only vreg at a terminator gets a register free at that instruction and
  is never stored after it* — UNVERIFIED; Phase 2's first task proves or
  disproves it with a regalloc unit test before anything relies on it.
- *x86 has no reserved free scratch GPR* (`INT_ALLOCATABLE = ["r10","r11","r12","r14"]`,
  `src/arch/x86_64/regmodel.rs`; existing expansions push/pop) — why the op carries
  allocator-provided scratch rather than using an encoder register.

## 3. Design Overview

The table lives **inside the instruction**: `br_table` encodes the dispatch
sequence followed immediately by one unconditional jump per case. So the
stream contains no unreachable, unlabeled instructions for UCE to delete, the
encoder resolves every entry with its existing label patches (`b` imm26 / `jmp`
rel32 / `jal`), no data object or cross-section relocation is needed, and the
debugger and disassembler decode the table as ordinary branches.

Scratch registers come from the allocator (`dst`, `dst2`, def-only) — uniform
on all three targets and free of the x86 scratch problem. Writing the
expansions to tolerate `dst`/`dst2` aliasing `src` (each reads `src` before
writing the register that may alias it) means no extra constraint on the
allocator.

**Correctness risk** concentrates in the CFG and the optimizer rows: a
terminator with N successors where every pass assumed ≤ 2. A row that ignores
the extra edges computes wrong liveness (a value live into case 3 gets its
register reused) — exactly the miscompile `is_block_terminator`'s x86 comment
records. Phase 2 lands the op with **no producer** and audits every reader of
`"target"` first. **Design uncertainty:** the density/size thresholds (measured
in Phase 1 and benchmarked in Phase 4).

**Gate class:** Phases 2 byte-identical; Phase 3 behavior-changing — expected
diffs only in goldens whose programs contain an eligible `MATCH` (the runtime's
own helpers included; Phase 1 lists them).

**Rejected alternatives.**
- *Address or offset table in a data section* — needs a code-label →
  data relocation the image format lacks (`src/arch/image.rs:EncodedRelocation`
  is symbol-based), and on PIE targets absolute entries need dynamic relocations.
- *Table words inline after `br`* — data in the text stream mis-decodes in the
  debugger's linear prologue walk and the disassembler.
- *Encoder-private scratch* — fine on AArch64 (x16/x17) and riscv64 (t1/t2) but
  x86 has none; one uniform design is simpler to verify.

### `adr` decision

`adr` is emitted **only inside `br_table`'s AArch64 expansion** (label-relative,
always within ±1 MiB because it addresses the table that immediately follows).
A general `adr` for symbol addresses (replacing `adrp; add :lo12:` from
`MirOp::AddrOf`) is **out of scope**: it saves one instruction per address
materialization only when the target is provably within ±1 MiB at encode time,
which for cross-function and data symbols is known only at link time; it would
need a new `ADR_PREL_LO21` relocation kind in every linker (Mach-O, ELF) plus a
link-time fallback to the pair. Measure before revisiting:
`rg -c '"adrp"' <corpus .ncode>`.

### PAC / BTI decision

**Out of scope.** `pacibsp`/`autibsp` (return-address signing) and `bti`
(branch-target landing pads) are hardening, not performance — they add
instructions. MFBASIC targets plain arm64 (not arm64e) on macOS, where user
processes do not enforce PAC; on Linux, BTI is enforced only for binaries whose
ELF carries `GNU_PROPERTY_AARCH64_FEATURE_1_BTI`, which `mfb` does not write.
The debugger already strips PAC bits from foreign return addresses
(`backtrace.rs:ADDRESS_MASK`). A future hardening plan would need: the ELF
property note, `bti c` at every function entry (all are `bl`/`blr` targets),
**`bti j` at every `br_table` case label** (indirect `br` targets), and
`paciasp`/`autiasp` around every `lr` save — recorded here so this plan's
`br_table` does not surprise it.

## 4. Detailed Design

### 4.1 CFG and optimizer contract

- `build_cfg`: for `br_table`, successors are every `case` label's block
  (deduplicated); `br_table` is in `is_block_terminator` and
  `is_unconditional_terminator`.
- Every row reading `"target"`: either generalize to "every branch-target field"
  through one helper `branch_targets(&CodeInstruction) -> impl Iterator` in
  `regalloc/analysis.rs`, or explicitly skip `br_table` blocks (must be
  conservative: treat as opaque multiway exit). `threading` may retarget `case`
  fields (hop collapsing is valid per entry); `merge`'s branch-to-next never
  deletes `br_table`; `simplifycfg`'s "both edges same block" rule does not apply.

### 4.2 Register allocation

`dst`/`dst2` are def-only at a terminator. Required: they get registers not live
across the `br_table` other than `src`, and no spill store is placed after the
terminator. Phase 2 task 1 verifies; if the allocator would place a store after,
the fix is in `linear_scan.rs` (a def with no later use is never spilled) —
landed in this plan with its own test, since it is a latent allocator bug for
any dead def at a terminator.

### 4.3 Relaxation

AArch64 entries are `b` (±128 MiB) — never relaxed. riscv64 entries are `jal`
(±1 MiB): `relax_rv64_branches` must route an out-of-range entry through a hop
island exactly as it routes `b` (it rewrites only target fields; the island is
placed outside the table so the table stays contiguous).

### 4.4 Lowering

In the `NirOp::Match` arm, before the chain: classify; if eligible, compute
the key once (the same value `lower_match_compare` compares — enum ordinal or
union tag word), emit the bounds check and `br_table`; each case label then
starts with its existing body (union binds included). Ineligible matches keep
the chain unchanged.

## Phases

> **NOTE — keep the checkboxes current** (same rules as plan-171-A).

### Phase 1 — measure

- [ ] Census: `MATCH` chains (≥ 3 compares of one register against immediates),
      key span, density, guard presence, per target, over the corpus; plus the
      `interp` and `dispatch` benchmark functions specifically. Record in §2 and
      propose `MIN_CASES`/`DENSITY` values from the distribution (Open Decision 1).

Acceptance: §2 has no UNMEASURED. Check: `grep -c UNMEASURED planning/plan-171-F-*.md` → 1 or 0 (only the allocator property, owned by Phase 2) (est. 20 min).
Commit: —

### Phase 2 — `br_table` primitive (no producer; byte-identical)

- [ ] Allocator property (§4.2): unit test in `src/codegen/engine/regalloc/tests.rs`
      with a def-only vreg at a terminator under register pressure; fix
      `linear_scan.rs` if it stores after the terminator.
- [ ] `ops.rs` (`BrTable`), `mir.rs` (mirror), `regalloc/analysis.rs`
      (`branch_targets` helper; `build_cfg`; terminator predicates).
- [ ] Audit and update each of the 43 files that read `"target"` (list them in
      this task as you go, each with "generalized" / "skips br_table because …").
- [ ] Encoders: AArch64 (`adr` label-relative, `add … lsl #2`, `br`, entries),
      x86-64 (§1), riscv64 (§1); sizing = fixed prefix + 4|5|4 bytes × count.
- [ ] AArch64 decoder: `adr`, `add (shifted register)`, `br` rows (the guard
      requires one per new encoder mnemonic); x86/rv64 encoder tests.
- [ ] Relaxation (§4.3) with a riscv64 test whose case target is 1.5 MiB away.
- [ ] Debugger: a `state_at` test over a body containing a `br_table` expansion
      (walk state unchanged after it).
- [ ] Tests: `build_cfg` with N-way successors; liveness of a value live into
      only case 2 (stays allocated across the dispatch); threading/merge/uce/
      simplifycfg units with a `br_table` block; encoder round trips.

Acceptance: every target-reading pass handles `br_table`; round trips pass; no
emitted byte changes.
  Check: `cargo test --bin mfb regalloc opt2:: aarch64:: x86_64::encode riscv64::encode debugger::backtrace` → pass (est. 6 min);
  `bash scripts/artifact-gate.sh target/release/mfb general` → `0 diff(s)` (est. 1 min).
Commit: —

### Phase 3 — `MATCH` lowering

- [ ] `builder_control.rs` `NirOp::Match`: classification + table path (§4.4).
- [ ] Update the comment in `validation.rs:module_tables` (tags may now be
      table-dispatched; sparse tag spaces simply fail the density test).
- [ ] Fixtures: `tests/rt-behavior/control-flow/match-jump-table` (dense enum 8 members,
      Integer 0..15 with holes, union of 5 members with binds, `CASE ELSE`,
      `OneOf` arms, the first and last key, out-of-range keys to `ELSE`,
      negative `min`); a guarded match and a sparse Integer match in the same
      fixture that must still take the chain.
- [ ] `rt_codegen_forms`: `br_table` present for the dense matches on all three
      targets' dumps and absent for the guarded/sparse ones.
- [ ] `rt_regalloc_levels_agree`: add the fixture.
- [ ] Golden re-baseline (all targets, predicted set from Phase 1).
- [ ] Runtime: macOS acceptance; 2223, 2227, 2229 `linux-runtime-proof.sh`; 2230.
- [ ] Benchmark (macOS host): `interp` (`vm`, `vm_loop`) and `dispatch` rows vs.
      `benchmark/language/baseline/mfb-O1.log`; if a row regresses, tune thresholds (Open
      Decision 1) and record the numbers.

Acceptance: dense matches dispatch through `br_table` on every target; outputs unchanged.
  Check: `cargo test --release --test rt_codegen_forms match` → pass (est. 3 min); runtime proofs → 0 mismatches (est. 25–60 min each).
Commit: —

### Phase 4 — review sweep

- [ ] Census diff; list remaining compare chains ≥ `MIN_CASES` long by function;
      for those in `src/codegen/builtins/**`, `src/codegen/runtime/**`,
      `src/codegen/string/**` (hand-written dispatch on a kind/tag word — e.g.
      error-code or type-tag switches), convert to `br_table` where keys are
      dense constants and every target is a label in the same function. "Makes
      sense" = ≥ `MIN_CASES` arms on a path the benchmark or acceptance suite
      executes; a chain whose first arm takes almost every hit stays a chain
      (record which).
- [ ] Golden re-baseline for sweep diffs.

Acceptance: every remaining long chain is converted or explained in Corrections.
  Check: census `--diff` pasted (est. 20 min).
Commit: —

### Phase 5 — spec and docs

- [ ] `mfb spec architecture` topics: `aarch64-instruction-set`, `x86_64-…`,
      `riscv64-…` (the `br_table` encodings), `mir-instruction-set` (multiway
      terminator, scratch fields), `native-ir` or wherever `MATCH` lowering is
      described (`mfb spec architecture --all | grep -n 'MATCH'`).
- [ ] `mfb spec tooling debugger`: backtrace through a table dispatch.
- [ ] `planning/optimizations.md` rows "Jump table generation" → `Y`, "Switch
      lowering strategies" → note sparse remains open.
- [ ] `.ai/codegen-invariants.md`: "a branch op's successors come from
      `branch_targets`; a new branch op must be added there and to
      `is_block_terminator`".

Acceptance: specs render. Check: `cargo test --bin mfb spec` → pass (est. 2 min).
Commit: —

## Validation Plan

- Tests: allocator property test, CFG/liveness tests, per-pass units, encoder
  round trips, relaxation, debugger, fixture, `rt_codegen_forms`, levels-agree.
- Coverage: `tests/byte-identity/*` fixtures with an eligible match are listed in
  Phase 1; if none, the fixture above plus `rt_codegen_forms` is the coverage —
  say so, and add a dense match to `tests/byte-identity/general` so the gate
  sees `br_table` (its `.ast`/`.ir` goldens move with it).
- Runtime proof: Phase 3's five targets.
- Final gate (once): full `cargo test --no-fail-fast` to a log; `EXIT=0` (est. 60–90 min).

## Open Decisions

1. `MIN_CASES` / `DENSITY` — **recommend 4 cases and ≥ 40 % density** (the
   common LLVM defaults) as the starting point, adjusted by Phase 1's
   distribution and Phase 3's `interp`/`dispatch` measurements.
2. Sparse matches (binary search / bit-test clusters) — **recommend a follow-up
   plan**, only if Phase 1 shows many long sparse chains.

## Corrections

<Filled in during execution.>

## Summary

The work is mostly making every CFG consumer N-way-aware, landed before any
producer exists. `adr` ships only as part of the table dispatch; PAC/BTI are
out of scope with the facts a future hardening plan needs.
