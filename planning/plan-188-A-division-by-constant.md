# plan-188-A: Division and remainder by a constant

Last updated: 2026-10-26
Overall Effort: x-large (1d–3d across A–D)
Effort: large (3h–1d)
Depends on: nothing (see Prerequisites)

**plan-188 overall.** The unimplemented Level 1–2 rows of `planning/optimizations.md`
were assessed against the tree at `d58b5397a` (2026-10-26). Four of them apply and
are planned here, in implementation order; the rest are recorded in the catalog with
their evidence (the assessment table is in this letter's §2.4).

| Letter | Rows | Effort |
|---|---|---|
| **A** | Division-by-constant lowering; Reciprocal / remainder lowering (exact) | large |
| B | Checked-operation fusion (the unsigned-wrap check idiom) | medium |
| C | Hot/cold splitting; Machine block reordering + fall-through inversion (also closes Branch layout and Code layout / block placement) | large |
| D | Identical code folding | medium |

**This sub-plan** replaces integer division by a compile-time constant with a
multiply-high and shifts (Granlund–Montgomery / Hacker's Delight §10), in two
places: a Level-1 MIR row that rewrites any `sdiv`/`udiv` whose divisor register
provably holds one constant (runtime helpers, inline lowerings, package code), and
the user-operator lowering, which for `x / c` and `x MOD c` with a literal `c ∉ {0,
-1}` also stops emitting the zero-divisor and `MIN / -1` checks that can never fire.
The outcome: at `-O1` and above no `sdiv`/`udiv` with a constant divisor other than
`0`, `-1`, `i64::MIN` (signed) or `≥ 2^63` (unsigned) reaches the encoder, every
program prints exactly what it printed before on all five targets, and `-O0` is
unchanged.

References:

- `src/codegen/engine/operators/builder_numeric/binary_emit.rs:emit_integer_binary_checked`
  (the `Divide | IntDiv` and `Mod` arms), `operator_lowering.rs` (the caller that
  computes `elide_overflow` from the NIR operands — the precedent for reading a
  literal operand), `numeric_kernels.rs:emit_nonzero_or_invalid`,
  `emit_integer_division_overflow_check`.
- `src/optimizer/opt2/mod.rs:optimize_mir` (pipeline order),
  `src/optimizer/opt2/constant_folding.rs` (shape of a Level-1 MIR row),
  `src/optimizer/opt2/gvn.rs` (the single-def rule this row relies on),
  `src/optimizer/opt2/plans/mark.rs:removable_op`.
- `src/codegen/engine/util/vreg_frame.rs:finalize_vreg_body_with_locals` (the vreg
  helper seam), `src/codegen/string/format/int_format.rs:lower_int_to_string_helper`.
- `src/optimizer/catalog.rs:rows`, `src/optimizer/stats.rs`.
- `src/arch/x86_64/encode/emitter/integer.rs:div_seq`,
  `src/arch/x86_64/encode/emitter/instruction.rs` (`umulh`/`smulh` expansion).
- `mfb spec language types` §4.1 "Checked arithmetic" (`/` truncates toward zero;
  `MOD` has the sign of the dividend; `x / 0` and `MOD 0` raise `ErrInvalidArgument`;
  `MIN MOD -1` and `MIN / -1` raise `ErrOverflow`).
- `.ai/compiler.md`, `.ai/codegen-invariants.md` ("A pass belongs on the `-O` dial
  only if it can't change a value or a trap"), `.ai/testing-gates.md`,
  `.ai/arch-abi.md`, `.ai/remote_systems.md`.
- H. S. Warren, *Hacker's Delight*, 2nd ed., §10-1…10-10 (signed and unsigned
  magic numbers, figures 10-1 and 10-2, the "add" indicator); T. Granlund and
  P. Montgomery, "Division by Invariant Integers using Multiplication", PLDI 1994.

## Prerequisites

Stated once for the whole plan-188 family; B–D add their own rows and point here.

| Must be true | Command | Status |
|---|---|---|
| The tree builds and the bin unit tests pass at the starting commit | `cargo test --bin mfb > /tmp/188-pre.log 2>&1; echo EXIT=$?` → `EXIT=0` | MET (2026-10-26, `/tmp/p188-tree` detached at `d58b5397a`: `EXIT=0`, 6200 passed, 0 failed, 5 ignored) |
| The artifact gate is clean at the starting commit, so every later diff is this plan's | from a detached worktree of the starting commit: `bash scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)` | MET (2026-10-26, `/tmp/p188-tree` detached at `d58b5397a`, its release `mfb`: `1723 tests, 1903 build(s), 2389 golden(s) checked, 0 diff(s)`) |

Everything below is written against the world where these hold. plan-171 is **not**
a prerequisite of this letter: the row uses only ops every backend already encodes
(`smulh`, `umulh`, `asr_imm`, `lsr_imm`, `add`, `sub`, `mov_imm`, `msub`), and the
files it edits are not the encoder/frame files the plan-171 letters change.

> **NOTE — the Status column is a snapshot; the Command column is the truth.**
> Re-run every command and update every status before you continue, and again
> before you decide to stop. Never act on a status you did not just verify.
>
> **If you stop, report the current status of *all* prerequisites.**

## 1. Goal

- A Level-1 MIR row "Division-by-constant lowering" rewrites `sdiv d, a, v` /
  `udiv d, a, v`, where `v` provably holds the constant `c`, into the
  multiply-high sequence of §4.2. It runs in `optimize_mir` and on every vreg helper
  stream finalized by `finalize_vreg_body_with_locals`.
- The user-operator lowering, at `level_enabled(1)`, lowers Integer/Byte `x / c`
  and `x MOD c` with a literal `c ∉ {0, -1}` to that same sequence (plus `msub` for
  `MOD`) and emits neither `emit_nonzero_or_invalid` nor
  `emit_integer_division_overflow_check` for it.
- Every value and every trap is unchanged at every level on all five targets; the
  `-O0` output is byte-identical to today's.

### Non-goals (explicit constraints)

- No change to any value, trap, error code or error location. `x / 0`, `x MOD 0`,
  `x / -1` and `x MOD -1` keep today's lowering exactly (checks included).
- No Float/`DIV` change, and no Fixed or Money operator kernel change (their scale
  divisions reach the MIR row only when the divisor is a single-def constant vreg;
  their checks stay).
- No divisibility-test form (`x MOD c = 0` via a modular inverse): 21 source sites
  (`rg -c -g '*.mfb' -e 'MOD +[0-9]+\)? *(=|<>) *0\b' examples benchmark packages src`
  summed → 21), not worth a second matcher.
- No post-register-allocation form (after allocation there is no free scratch
  register for the multiply-high temporaries).
- No new op and no encoder change.

## 2. Current State

### 2.1 How a constant division is emitted today

User `/` and `MOD` on Integer/Byte go through `emit_integer_binary_checked`: the
`Divide | IntDiv` arm emits `emit_nonzero_or_invalid(right)` (a `cmp_imm 0; b.ne`
plus an inline `ErrInvalidArgument` raise), `emit_integer_division_overflow_check`
(`mov_imm MIN; cmp left; b.ne; mov_imm -1; cmp right; b.ne` plus an inline
`ErrOverflow` raise) and `sdiv`; `Mod` emits the same and then `msub`. Nothing looks
at whether `right` is a literal.

Probe (`/tmp/p188/div`, base release `mfb` of `d58b5397a`, `mfb build -q -ncode`,
macos-aarch64, `-O1`): `FUNC f(x AS Integer) AS Integer: RETURN x / 7`, inlined into
`main` by leaf inlining, is 23 stream entries — `mov_imm 7`, a dead `cmp_imm x11, 0`
(the zero check's branch was folded, its compare left), the six-instruction
`MIN / -1` check, a 13-instruction inline raise block the passing path branches
over, a label, and `sdiv`. `x MOD 10` is the same plus `msub`.

Runtime and helper lowerings divide by constants held in vregs, e.g.
`lower_int_to_string_helper` (`int_format.rs`, behind every `toString(Integer)`)
runs `udiv quotient, value, divisor` once per digit with `divisor = mov_imm 10`, and
is finalized by `finalize_vreg_body_with_locals`, which never runs `optimize_mir`.

No pass rewrites a division: `opt1/strength.rs` lists `x / 2^k` as out of scope
("belongs to Division-by-constant lowering"), and `rg -n -i magic src/optimizer`
→ no matches. bug-774 B3 (`bugs/completed/bug-774-…md`) hand-rewrote `MOD 16` /
`/ 16` into `bits::band` / `bits::sr` in a package source to get speed the compiler
did not provide.

### 2.2 What the encoders do with `sdiv`/`udiv`

AArch64 encodes `sdiv`/`udiv` natively (no trap: `x / 0 = 0`, `MIN / -1 = MIN`).
x86-64 expands both through `div_seq` (`idiv`/`div` with `rax`/`rdx` staging),
which raises `#DE` on a zero divisor and on `MIN / -1`. riscv64 encodes
`div`/`divu`. `smulh`/`umulh` are native on AArch64 and riscv64 (`mulh`/`mulhu`);
x86-64 expands them to `push`/`mov rax`/`imul|mul`/`mov dst, rdx`/`pop`.

### 2.3 Measured populations

All corpus numbers are over the `ncode-census` corpus (56 projects; riscv64 43),
built by the base release `mfb` of `d58b5397a` with
`KEEP=/tmp/p188/keep-<t> tools/ncode-census/ncode-census.sh <mfb> <target> <out>`,
then `python3 -I /tmp/p188/probe.py <arch> <dumps>`. "Constant divisor" means the
nearest earlier write of the divisor register in the function is a `mov_imm`
(a backward scan that skips exit blocks and stops at a call): an approximation
over physical registers, recorded as such.

| What | macos-aarch64 | linux-x86_64 | Command |
|---|---|---|---|
| instructions (all functions) | 27,057,789 | 30,519,671 | `probe.py` `instructions` |
| `sdiv` | 4,213 | 4,213 | `probe.py` `sdiv` |
| … constant divisor | 3,717 | 3,666 | `sdiv_const_divisor` |
| … power of two | 2,168 | 2,128 | `sdiv_const_pow2` |
| `udiv` | 2,290 | 2,130 | `udiv` |
| … constant divisor | 1,299 | 582 | `udiv_const_divisor` |
| … power of two | 101 | 13 | `udiv_const_pow2` |

Where the macos-aarch64 constant divisions live (`python3 -I /tmp/p188/divwhere.py
<dumps>`): `_mfb_ifn_*` (package functions) 3,179 `sdiv`; `_mfb_rt_*` 924 `udiv`,
168 `sdiv`; `_mfb_fn_*` (user) 370 `sdiv`, 38 `udiv`; `_mfb_macapp_*` 296 `udiv`
(hand-written physical-register bodies — out of reach, §3).

Source: `MOD <literal>` 191 sites, `/ <literal>` 464 sites (the latter includes
Float `/`) — `rg -c -g '*.mfb' -e 'MOD +[0-9]+' examples benchmark packages src`
and `-e ' / +[0-9]+'`, summed. The benchmark's hot rows use them:
`dispatch.mfb` `MOD 1000000007` in `evalNode` (the `dispatch/union` row),
`list.mfb`/`elem.mfb` hash loops `MOD 1000000007`, `bigint.mfb`, `vector.mfb`
`i / 1000`, `i / 10`, `i / 90`.

Rust producer sites whose divisor is a literal `move_immediate` within six lines
before the divide: 41 (`rg -n -B6 "(signed|unsigned)_divide_registers\(" src/codegen
src/target | rg 'move_immediate\([^)]*"[0-9]+"' | grep -v '"0"' | wc -l` → 41).

Cost probe (this host, Apple Silicon; `/tmp/p188/cdiv/d.c`, `cc -O2
-fno-vectorize -fno-unroll-loops`): the latency-bound loop
`s = (s*31 + i) % d`, 200M iterations, takes 0.920 s with `d` opaque (hardware
`sdiv`) and 0.796 s with `d = 1000000007` known to clang (multiply-high) —
−14 %. On x86-64 `idiv r64` is slower still (guess, not measured here: no x86-64
hardware on this host).

### 2.4 Assessment of every row plan-188 was asked to cover

| Row | Verdict | Evidence |
|---|---|---|
| Identical code folding | planned, D | 550 removable functions / 51,921 instructions (aarch64, 0.19 %), 466 / 57,444 (x86-64), 75 / 38,770 (riscv64, 0.86 %) — `/tmp/p188/icf.py` |
| Division-by-constant lowering | planned, A | §2.3 |
| Reciprocal / remainder lowering (exact) | planned, A | `MOD c` = constant quotient + `msub`; a shared quotient is numbered by GVN/LVN once it is pure ALU; the divisibility form is declined (21 sites) |
| Bit-tricks / idiom recognition | subsumed | the idioms are builtins that already lower to native forms (`bits::popCount` → `cnt`/`addv` on AArch64, `bits::bswap64` → `rev_x`, `bits::rl64` → `rorv`); MFBASIC has no shift operators, and the corpus has 0 hand-rolled rotate/popcount idioms (`rg -g '*.mfb' -e 'bits::(bor|bxor)\(\s*bits::(sl|sr)'` → 5 hits, all limb packing; `-e 'band\([a-z]+, *[a-z]+ *- *1\)'` → 0) |
| Carry / borrow chain formation | wide part declined; checked-op part → B | 877 chained `add_carry` pairs / 1,371 (aarch64), each saving `cset`+`cmp` = 1,754 instructions (0.006 %), against the flagless-MIR contract (plan-00-G §4); the checked-op idiom (`add; cmp; b.lo` → `adds` + one carry branch) is 52,058 sites and is B |
| Constant-island / literal-pool placement | N/A | no target has literal pools (AArch64 `movz`/`movk`, x86-64 `movabs`, riscv64 `li` chains); a pool would serve only `mov_imm`s of ≥ 3 AArch64 words (7,610 sites) or ≥ 4 riscv64 words (2,403 sites, 13,675 words of 4,506,516) — `/tmp/p188/probe.py`, `/tmp/p188/rvconst.py`; branch islands are already `aarch64/encode/relax.rs` (Level 0) |
| Constant hoisting | declined | repeated multi-word constants outside exit blocks, excluding error codes: 12,986 extra instructions (0.048 %, `/tmp/p188/consts.py aarch64`), most of them (`0x7fffffff`, `0xffffffff`, `0xffffffff00000000`, `0x8080…80`) single-instruction under plan-171-C's bitmask immediates; the repeated constants that dominate the raw count are error codes in cold raise blocks |
| Frame pointer omission | already done | no target builds a frame-pointer chain (`mfb spec memory native-calling-convention` "There is no `x29` frame-pointer chain"); x86-64 `rbp` joins the pool in plan-171-I; riscv64 `s0` is already used (it realizes scratch `x13`, `riscv64/select.rs:map_scratch_register`); AArch64 `x29` must address a frame record on macOS (Apple ABI) and, measured on linux-aarch64, making it allocatable costs +11,249 instructions (+0.04 %: `ncode-census --diff` of base vs a `x29`-allocatable build) |
| Branch layout / prediction hints | planned, C (layout); hints N/A | no target has a static branch-hint encoding the backends could use (AArch64 none; x86 `2E`/`3E` prefixes are ignored by current cores; RV64GC none) |
| Code layout / block placement | planned, C | cold exit placement |
| Machine block reordering + fall-through inversion | planned, C | 113,731 `b.cc L1; b L2; L1:` (aarch64) |
| Hot/cold splitting | planned, C | 132,938 inline cold exit blocks, 2,464,131 instructions (aarch64) |
| Store merging | declined | 1,207 adjacent contiguous narrow store pairs in 27,057,789 instructions, 82 of them both-zero (`probe.py` `st_narrow_*`); 8-byte pairs are already `stp` (plan-171-A) |
| Load widening | declined | 686 adjacent contiguous narrow load pairs (`probe.py` `ld_narrow_*`) |
| Aggregate load/store combining (remainder) | declined | narrow merging = store merging above; a `q` access saves nothing over `ldp`/`stp` for 16 bytes; non-adjacent needs alias analysis (Plan2) |
| Checked-operation fusion | planned, B | 52,058 `add|add_imm; cmp; b.lo` unsigned-wrap checks (aarch64), 23,422 (x86-64); the `adds; b.vs; cmp d, 0` form is 526 / 477 sites and is declined |
| Source-location metadata compression | folded into plan-171-H | its mechanism (a per-site id passed to a shared stub) is the Error-path deduplication row's, which is plan-171-H's; measured: 234,630 raise sites, 68,829 distinct (file, line, column), 937,181 location instructions (3.5 %), `/tmp/p188/loc.py` |

## 3. Design Overview

Two producers share one expansion:

1. `optimizer::opt2::divconst` — the magic-number computation and `expand`, which
   returns the instruction sequence for one division by one constant (or `None`
   for a divisor it does not handle).
2. The MIR row `divconst::lower` — finds `sdiv`/`udiv` whose divisor vreg is a
   single-definition constant and splices in `expand`. Called from `optimize_mir`
   and from `finalize_vreg_body_with_locals`.
3. The operator producer — `emit_integer_binary_checked` with a literal divisor
   calls `expand` directly and omits the two checks.

Why two producers: the MIR row cannot remove the checks. The zero check and the
`MIN / -1` check are separate compare-and-branches with raise blocks; at `-O1` the
block-local folder only removes the zero check's branch (§2.1 probe), and the
`MIN / -1` check compares the *dividend*, which is unknown. The operator lowering
knows the divisor is the literal `c` and that both checks need `c = 0` or
`c = -1` to fire, so it is the one place that can omit them at Level 1. The MIR
row reaches every division the operator lowering does not see (helpers, package
code, scale divisions).

**Correctness risk** concentrates in the magic numbers and the sign fix-ups. It is
closed by exhaustive unit tests of `expand`'s arithmetic (§4.4) and by a
cross-level runtime fixture on all five targets.

**Byte-identity is not this plan's gate.** Every target is expected to diff: the
int-to-string helper is in almost every binary, so nearly every `.ncodesum` moves
on all five targets. `-O0` must not move (a rerun of the gate at `-O0` is not
available; the `-O0` check is `rt_regalloc_levels_agree` output equality plus a
direct `-O0` `-ncode` diff of the probe, Phase 1).

Rejected alternatives:

- **A NIR (Opt1) rewrite.** NIR has no multiply-high node; adding one for this is a
  NIR-wide change for a backend concern.
- **Seeding the block-local constant folder with single-def constants so it folds
  the checks itself.** Moves every Level-1 constant fold in the tree (goldens on
  every target) to get what the operator lowering gets locally.
- **Converting the 41 Rust producer sites one by one.** The helper seam covers
  every vreg helper at once; the physical-register bodies left over (§2.3: the
  app-mode `_mfb_macapp_*` / bootstrap code) run once per event or once per run.

## 4. Detailed Design

### 4.1 Which divisors

`expand(signed, dst, a, c, mint)`:

| Case | Handled | Sequence |
|---|---|---|
| `c = 0` | no (`None`) | — |
| signed `c = -1` | no | — (x86 `idiv` traps on `MIN / -1`; the MFB check must stay) |
| signed `c = i64::MIN` | no | — |
| unsigned `c ≥ 2^63` | no | — |
| `c = 1` | yes | `mov dst, a` |
| signed `|c| = 2^k`, `k ≥ 1` | yes | `asr_imm t, a, 63; lsr_imm t, t, 64-k; add t, a, t; asr_imm dst, t, k`; for `c < 0` then `sub dst, zero, dst` |
| signed other | yes | `mov_imm m, M; smulh q, a, m`; `add q, q, a` when `c > 0 ∧ M < 0`, `sub q, q, a` when `c < 0 ∧ M > 0`; `asr_imm q, q, s` when `s > 0`; `lsr_imm t, q, 63; add dst, q, t` (Hacker's Delight fig. 10-1, 64-bit) |
| unsigned `c = 2^k` | yes | `lsr_imm dst, a, k` |
| unsigned other, no add indicator | yes | `mov_imm m, M; umulh q, a, m; lsr_imm dst, q, s` |
| unsigned other, add indicator | yes | `mov_imm m, M; umulh q, a, m; sub t, a, q; lsr_imm t, t, 1; add t, t, q; lsr_imm dst, t, s-1` (fig. 10-2 / §10-8) |

`zero` is `abi::ZERO` (as `func_rl64.rs` uses it). `t`, `q`, `m` are fresh vregs
from `mint` (`regalloc::mint_vreg(RegClass::Int)` in the MIR row,
`allocate_register` in the builder). Every op in every sequence is in
`plans::mark::removable_op` (pure, flag-free on AArch64, single `dst`), so LVN/GVN
can number a shared quotient and LICM can hoist `mov_imm m` out of loops at
Level 3 — neither could touch `sdiv` (it is not in `removable_op`: x86 traps).

### 4.2 The MIR row

`divconst::lower(instructions: &mut Vec<CodeInstruction>) -> u64`, gated
`level_enabled(1)`:

1. Census: for every vreg, the number of instructions that define it (the
   allocator's `DEF_FIELDS` roles over `regalloc::class_models`) and, when exactly
   one, that instruction's index.
2. Resolve a divisor `v`: if its single definition is `mov_imm c`, the constant is
   `c`; if it is `mov v, w` with `w` itself single-def, resolve `w` (at most 8
   hops). This is the shape a user literal reaches the division in: the operand
   lowering parks it in its own frame slot (the probe's `-mir`, captured before
   promotion, shows the divisor as `ldr_u64 %v41, [sp, #72]`), and stack-slot
   promotion replaces that slot's one store by `mov %vO, src` and its load by
   `mov dst, %vO` (`opt2/promote.rs` module doc). Anything else: not a constant
   (a function promotion declines keeps the `ldr`, and the row skips it).
3. Each `sdiv`/`udiv` (fields `dst`, `lhs`, `rhs`) whose `rhs` resolves to `c` and
   for which `expand` returns `Some` is replaced in place by the sequence. `msub`
   remainders that read the quotient are left alone (they read `dst`, which the
   sequence still defines).

Soundness of step 2: a vreg with one definition holds that definition's value at
every use it reaches, and no lowering reads a vreg on a path its definition does
not reach — the premise GVN's "a single-def dst is read directly" rule already
rests on (`opt2/gvn.rs` module doc). Physical registers and ABI tokens are never
resolved.

Placement: in `optimize_mir`, immediately after `constant_folding::fold_constants`
(so the value-numbering and LICM rows below see the pure sequence), and in
`finalize_vreg_body_with_locals` before `regalloc::allocate`. Catalog row
"Division-by-constant lowering", level 1, stage `MIR`, counter
`stats::DIVISIONS_BY_CONSTANT`.

### 4.3 The operator producer

`operator_lowering.rs` computes `constant_divisor: Option<i64>` beside
`elide_overflow`: `Some(c)` when `op` is `Divide`, `IntDiv` or `Mod`, the right NIR
operand is `NirValue::Const` of type `Integer` or `Byte` whose value parses as
`i64`, and `crate::optimizer::level_enabled(1)`. It passes it to
`emit_integer_binary_checked` (new parameter; the `emit_integer_binary` wrapper
and the Money caller pass `None`).

In the `Divide | IntDiv` arm, when `constant_divisor` is `Some(c)` with
`c ∉ {0, -1}` and `expand(true, dst, left, c, …)` is `Some`: emit the sequence and
nothing else. In the `Mod` arm likewise into a fresh quotient vreg, then
`mov_imm k, c` and `msub dst, q, k, left`. Otherwise both arms emit exactly what
they emit today.

Why removing the two checks cannot change a trap: `emit_nonzero_or_invalid(right)`
raises only when `right = 0`, and `emit_integer_division_overflow_check` raises only
when `left = MIN ∧ right = -1`; with `right` the literal `c ∉ {0, -1}` neither
condition can hold, on any input. This is the same class of rewrite as the
Level-1 branch folding that already deletes a constant-decided check.

### 4.4 Tests that pin exactness

`src/optimizer/opt2/divconst/tests/mod.rs` evaluates `expand`'s output with a
small interpreter over the ops it emits (`mov`, `mov_imm`, `smulh`, `umulh`,
`asr_imm`, `lsr_imm`, `add`, `sub`; i128 for the high halves) and compares with
Rust's `i64::wrapping_div` / `u64 /`:

- every divisor in `-4096..=4096` (minus the excluded) × 512 dividends (`0`, `±1`,
  `MIN`, `MAX`, `MIN+1`, `MAX-1`, every `±2^k` and `±2^k±1`, and a fixed-seed
  64-bit LCG);
- every `±2^k` divisor, every `2^k ± 1`, `10^k` for `k ≤ 18`, `1000000007`, and
  20,000 fixed-seed random 64-bit divisors (signed and unsigned);
- `None` for exactly the excluded divisors.

## Phases

> **NOTE — keep the checkboxes current as you go** (same rules as the template:
> tick in the same commit as the work; `- [~]` for partial; moot tasks struck,
> never deleted; fill `Commit:` the moment a phase lands).

### Phase 1 — `divconst` and the MIR row

Delivers the expansion and its first caller; safe alone because it only replaces a
division by the same quotient computed another way.

- [ ] `src/optimizer/opt2/divconst.rs`: `SignedMagic`/`UnsignedMagic` computation
      (§4.1), `expand`, and `lower` (§4.2); `pub(crate) mod divconst;` in
      `opt2/mod.rs`; call `divconst::lower` right after constant folding in
      `optimize_mir`, inside a `timed("division by constant", …)` span.
- [ ] `src/codegen/engine/util/vreg_frame.rs:finalize_vreg_body_with_locals`: call
      `divconst::lower` on `instructions` before `regalloc::allocate`.
- [ ] `src/optimizer/stats.rs`: `DIVISIONS_BY_CONSTANT` + `count_divisions_by_constant`;
      `src/optimizer/catalog.rs`: row "Division-by-constant lowering" (level 1,
      stage `MIR`) in pipeline order.
- [ ] Tests: `src/optimizer/opt2/divconst/tests/mod.rs` — §4.4's exhaustive
      arithmetic; the census (a two-def divisor is not resolved; a `mov` chain of
      three resolves; a chain through a two-def link does not; a physical divisor
      is not resolved); `-O0` leaves the stream untouched.
- [ ] `tests/common/ncode_emu.rs`: interpret `smulh`, `umulh` and `asr_imm` (the
      emulator panics on an unknown op, and an emulated helper whose constant
      division is now lowered would hit it). Run every test binary that uses the
      emulator (`rg -l ncode_emu tests` lists them).
- [ ] Golden re-baseline (every target): from a detached worktree of the phase's
      parent, `scripts/artifact-gate.sh <base> all` → `0 diff(s)`; on the branch,
      `scripts/artifact-gate.sh target/release/mfb all`, classify every diff
      (in one fixture per target, the functions whose code changed are exactly the
      functions that held a constant-divisor `sdiv`/`udiv` — compare the two
      `-ncode` dumps function by function; allocation inside those functions may
      differ too), then `scripts/regen-native-goldens.sh target/release/mfb` and
      the gate again → 0.
- [ ] Runtime: `bash scripts/test-accept.sh target/release/mfb /tmp/188a1-accept`
      (every runnable fixture prints integers through the lowered helper).

Acceptance: at `-O1` the int-to-string helper divides by 10 with `umulh` and
shifts, and the arithmetic is exact.
  Check: `cargo test --bin mfb opt2::divconst` → pass (est. 3 min);
  `mfb build -q -ncode` of any program that prints an Integer: function
  `_mfb_rt_int_to_string` has 0 `udiv` and ≥ 1 `umulh` (base compiler: 1 `udiv`,
  measured on `/tmp/p188/div`), est. 1 min; the same build with `-O0` is
  byte-identical to the base compiler's `-O0` dump (`cmp`, est. 1 min);
  `test-accept.sh` → no new red against the base run (est. 30 min — the only
  execution of every helper that now divides by multiplication).
Commit: —

### Phase 2 — the operator producer

User `x / c` and `x MOD c` lose their impossible checks.

- [ ] `operator_lowering.rs`: compute `constant_divisor` (§4.3); thread it through
      `emit_integer_binary_checked` (new parameter, `None` from
      `emit_integer_binary` and from `gen_money_math.rs`).
- [ ] `binary_emit.rs`: the `Divide | IntDiv` and `Mod` arms take the §4.3 path.
- [ ] Fixtures: `tests/rt-behavior/arithmetic/integer-div-by-constant` — the
      literal divisors `1, 2, 3, 7, 10, 16, 64, 100, 1000000007,
      9223372036854775807, -2, -3, -7, -16, -1000000007` against the dividends
      `0, ±1, ±6, ±7, ±8, MIN, MAX, MIN+1, MAX-1` (held in a list so they are not
      folded), printing every `/` and `MOD` result, plus a Byte loop printing
      `b / 3` and `b MOD 7` for `b` in `0..255`. `tests/rt-error/arithmetic/integer-div-by-literal-zero` and
      `integer-min-div-literal-minus-one` (both still raise, same code and
      location as today — their goldens are generated with the base compiler and
      must not change).
- [ ] `tests/runtime/rt_regalloc_levels_agree.rs`: add the rt-behavior fixture
      (`-O0` is the oracle; `-O1`..`-O3` must print the same).
- [ ] `tests/runtime/rt_codegen_forms.rs::division_by_constant`: in the fixture's
      `-O1` aarch64 dump the user functions contain no `sdiv` and no `div_not_min`
      label; at `-O0` they contain both.
- [ ] Golden re-baseline as in Phase 1 (expected movers: every fixture that
      divides by a literal).
- [ ] Runtime proof: `bash scripts/test-accept.sh target/release/mfb /tmp/188a-accept`
      (macOS); `bash scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64`;
      `… 2227 linux-x86_64`; `… 2229 linux-riscv64`; the fixture built
      `-target windows-x86_64`, copied to `test@192.168.1.206` and run, output
      equal to the golden.

Acceptance: literal divisions lower without checks and every target prints the
golden.
  Check: `cargo test --release --test rt_regalloc_levels_agree` → pass (est. 4 min);
  `cargo test --release --test rt_codegen_forms division_by_constant` → pass
  (est. 2 min); the four remote runs → 0 mismatches (est. 25 min aarch64,
  60–120 min x86-64 emulated, 60 min riscv64 — each is the only execution of that
  target's emitted division code; 5 min Windows).
Commit: —

### Phase 3 — docs, benchmark, catalog

- [ ] `mfb spec architecture native` (the MIR-optimization paragraph): division by
      a constant is lowered at `-O1`+; `mfb spec architecture mir-instruction-set`
      if it lists the rows. Follow `.ai/specifications.md`.
- [ ] `planning/optimizations.md`: the two rows' `Imp` → `Y` with the landing note
      (row text in this plan's report), measured fire counts from
      `mfb build -O1 -v examples/brogue` and `benchmark/language/mfb`.
- [ ] `.ai/codegen-invariants.md`: one entry — "a constant divisor's checks are
      decided at the operator, the quotient at MIR; `-1`, `0`, `MIN` stay `sdiv`
      because x86 `idiv` traps".
- [ ] Benchmark (macOS host): `./benchmark/language/run.sh 10` before/after with
      `rank.py`; record `dispatch/union`, the `list`/`elem` hash rows and
      `math`/`vector` rows in Corrections. No baseline regeneration.

Acceptance: the man page renders the row and the spec cites it.
  Check: `target/release/mfb man optimizations | grep -c 'Division-by-constant'` → 1
  (est. 1 min).
Commit: —

## Validation Plan

- Tests: §4.4 unit tests; census unit tests; the rt-behavior and two rt-error
  fixtures; `rt_regalloc_levels_agree`; `rt_codegen_forms::division_by_constant`.
- Coverage check: `mfb build -O1 -v` on the rt-behavior fixture prints a non-zero
  "Division-by-constant lowering" count; the int-to-string helper is in every
  fixture that prints an Integer, so the artifact gate covers the helper seam.
- Runtime proof: the fixture's output on macOS, linux-aarch64 (2223), linux-x86_64
  (2227), linux-riscv64 (2229), Windows (192.168.1.206).
- Doc sync: Phase 3.
- Final gate (once, after Phase 3): `bash scripts/test-fast.sh`;
  `scripts/artifact-gate.sh target/release/mfb all` → `0 diff(s)`;
  `bash scripts/test-accept.sh target/release/mfb /tmp/188a-accept` → no new red;
  `cargo test --workspace --no-fail-fast > /tmp/188a.log 2>&1; echo EXIT=$?` →
  `EXIT=0` (est. 60–90 min; includes `artifact_gate_all`).

## Decisions

- The two checks are omitted only by the operator lowering, never by the MIR row —
  author's decision, 2026-10-26 (§3). The MIR row has no way to know a check's
  operand is the literal; the operator does.
- Level 1 for both producers — the catalog's level for both rows; neither changes
  a value or a trap (§4.3).
- Excluded divisors stay `sdiv`/`udiv` — author's decision (§4.1): `-1` and `MIN`
  keep x86's `#DE` path behind the MFB check, `≥ 2^63` unsigned is a compare, not a
  division, and not worth a form.

## Corrections

(Filled in during execution.)

## Summary

Risk is in the magic numbers and sign fix-ups, closed by exhaustive unit tests
against Rust's division and by cross-level runtime equality on five targets. The
user-visible change is speed: constant division in loops (`MOD 1000000007`
hashing, `toString` digit loops, `/ 10` and `/ 1000` index math) loses its
hardware divide and, for user operators, its two impossible checks.
