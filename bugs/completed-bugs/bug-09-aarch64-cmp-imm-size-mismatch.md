# bug-09: AArch64 `cmp_imm` size pre-pass disagrees with the emitter for rhs > 4095

Last updated: 2026-07-08
Effort: small (<1h)

The AArch64 encoder computes each instruction's byte length twice: once up
front in `instruction_size` (`src/arch/aarch64/encode/sizing.rs`) to place
function symbols and record label offsets, and once for real in
`emit_instruction` (`src/arch/aarch64/encode/emitter.rs`). For `CmpImm` these two
paths use **different algorithms** and produce **different byte counts** whenever
the compared immediate exceeds the 12-bit range (`rhs > 4095`):

- `instruction_size` sizes `CmpImm` with `sized_add_sub_imm(rhs)` — the
  add/sub-immediate 12-bit-chunk decomposition (as if the value were materialized
  with `add`/`sub #imm, lsl 12`).
- `emit_cmp_imm` does **not** use that decomposition. For `rhs > 4095` it emits
  `mov_imm scratch, rhs` (1–4 `movz`/`movk` words) followed by a register
  `cmp` — a completely different word count.

For example `cmp_imm rhs=4096`: `sized_add_sub_imm(4096)` returns **4 bytes**
(one shifted chunk), but `emit_cmp_imm` emits `movz scratch,#4096` + `cmp` =
**8 bytes**. Any exact multiple of 4096, or any value whose `mov_imm` word count
differs from its add/sub chunk count, diverges.

The single correct behavior a fix produces: `instruction_size(CmpImm)` returns
**exactly** the number of bytes `emit_cmp_imm` will emit for the same immediate,
for every `rhs` — so the size pre-pass and the emitter never disagree.

This is currently **latent**: every existing `abi::compare_immediate` call site
passes an immediate ≤ 2048 (verified by an exhaustive search — see Blast
Radius), so the `> 4095` emit fallback is never reached and the sizes always
agree at 4 bytes. It is filed as a real defect because (a) the emit fallback is
deliberately implemented and is documented in the spec
(`src/docs/spec/architecture/14_aarch64-instruction-set.md:82` — "falls back to
`mov_imm`+`cmp`"), so it is intended to be reachable, and (b) if it is ever
reached the failure is silent and catastrophic (see below), with no
compile-time error. Severity: LOW by current reachability; HIGH by blast radius
if triggered.

References:

- `src/arch/aarch64/encode/sizing.rs:13-22` (`CmpImm` sizing branch),
  `:97-113` (`sized_add_sub_imm`).
- `src/arch/aarch64/encode/emitter.rs:862-869` (`emit_cmp_imm`).
- `src/arch/aarch64/encode/mod.rs:102-134` — the pre-pass that consumes
  `instruction_size` to place function symbols (`:102-112`) and label offsets
  (`:118-127`).
- Spec: `src/docs/spec/architecture/14_aarch64-instruction-set.md:82`.
- Found during goal-01 review of `src/arch/aarch64/encode/**`.

## Failing Reproduction

No end-to-end `.mfb` reproduction exists today because no codegen path emits a
`cmp_imm` with `rhs > 4095` (the fallback is unreached). The divergence is
demonstrable directly against the two encoder functions:

```
# In src/arch/aarch64/encode/tests.rs, the two must agree but do not:
instruction_size(cmp_imm{lhs:"x0", rhs:"4096"})  ->  4   (sized_add_sub_imm)
emit_words("cmp_imm", &[("lhs","x0"),("rhs","4096")]).len()  ->  8   (movz + cmp)
```

- Observed: `instruction_size` = 4 bytes, `emit_instruction` writes 8 bytes.
- Expected: both report the same byte count.

Contrast cases that are correct today:

- `rhs <= 4095` (e.g. the real `rhs="2048"` / `"1000"` call sites): sizing → 1
  add/sub chunk = 4 bytes; emit → `checked_imm12` path = 4 bytes. Agree.
- `AddImm`/`SubImm`/`AddSp`/`SubSp`: sizing (`sized_add_sub_imm`) and emit
  (`emit_add_imm`/`emit_sub_imm`) share the **same** chunk-decomposition
  algorithm, so they agree for all values. `CmpImm` is the lone op whose emit
  path (`mov_imm`+`cmp`) does not match its sizing algorithm.

## Root Cause

`src/arch/aarch64/encode/sizing.rs:13` groups `CmpImm` with `AddSp`/`SubSp` and
sizes it via `sized_add_sub_imm`. That is correct for `add_sp`/`sub_sp` (they are
emitted by `emit_add_imm`/`emit_sub_imm`, the same decomposition) but wrong for
`cmp_imm`, which `emit_cmp_imm` realizes as `mov_imm scratch, rhs` + `cmp` for
out-of-range immediates (`emitter.rs:862-869`). The two word counts coincide for
`rhs <= 4095` (both 1 word for the compare) and diverge above it. The grouping
looks like a copy-paste of the add/sub sizing that overlooked `emit_cmp_imm`'s
different lowering.

Why the pre-pass makes this fatal: `mod.rs:102-112` computes every function's
text offset purely from `instruction_size`, and `:118-127` records label
positions the same way before truncating and re-emitting. If a function contains
a `cmp_imm rhs>4095`, its emitted length exceeds its reserved length, so every
later function symbol and every label patch in that function is off by the delta
— branches resolve to the wrong address and calls land mid-function. No error is
raised.

## Goal

- `instruction_size(&CodeInstruction{op: CmpImm, ..})` returns exactly the byte
  count `emit_cmp_imm` emits for the same `rhs`, for all `rhs` in `0..=u64::MAX`
  representable by `immediate(...)`.

### Non-goals (must NOT change)

- Do not change `emit_cmp_imm`'s lowering (the `mov_imm`+`cmp` fallback is the
  documented behavior; the spec references it).
- Do not change `AddImm`/`SubImm`/`AddSp`/`SubSp` sizing — they are correct.
- Do not "fix" this by clamping/rejecting large `cmp_imm` immediates unless that
  is chosen deliberately as the design (see Open Decisions); silently erroring is
  worse than sizing correctly.

## Blast Radius

- `emit_cmp_imm` (`emitter.rs:862`) — the only emitter whose word count is not a
  pure function of the add/sub chunking that `sizing.rs` assumes. Fixed by this
  bug.
- All `abi::compare_immediate` call sites (exhaustive search across `src/`): every
  literal argument is `<= 2048`; the dynamic arguments are small
  compiler-controlled quantities — ASCII-literal lengths (`fs_helpers_io.rs:1902`,
  internal scheme/mode strings), single bytes, `DECIMAL_EXPONENT_CLAMP = 1000`,
  clamped terminal dimensions (`term_draw.rs`), and named arena/thread/grapheme
  constants (all small). None exceed 4095 today → latent, not observed to fail.
- `sub_borrow` sizing (`sizing.rs:67`) is a constant 12 and `emit_sub_borrow`
  always emits 3 words = 12 — consistent, unaffected.

## Secondary finding (same class, LOW / defense-in-depth)

`instruction_size` sizes `AddCarry` by testing the **string** value of the
`carry_in` field (`sizing.rs:60-66`: `== "xzr"` → 8 bytes, else 12), whereas
`emit_add_carry` tests the **resolved register number** (`emitter.rs:654`:
`carry_in == 31` → 8 bytes, else 12). `reg()` maps `"xzr"`, `"sp"`, `"raw_sp"`,
and `"x31"` all to 31. Today every no-carry-in site passes the literal `"xzr"`
(`entry_and_arena.rs:1848,1849,1929,1930`), and register allocation never assigns
a vreg to x31, so the two tests always agree. But they encode the same predicate
two different ways: if any path ever spelled the no-carry input `"sp"`/`"x31"`,
sizing would say 12 while emit produced 8 — the identical size-mismatch hazard.
Fold this into the fix by keying the size on the resolved register number (or by
having both sites share one predicate).

## Fix Design

In `sizing.rs`, split `CmpImm` out of the `AddSp`/`SubSp` arm and size it to
mirror `emit_cmp_imm`: 4 bytes when `checked_imm12(rhs)` succeeds, else
`wide_imm_word_count(rhs) * 4 + 4` (the `mov_imm` words plus the `cmp` word).
`wide_imm_word_count` already exists in `sizing.rs` and is exactly the `mov_imm`
word count, so the size becomes a direct function of the same quantities
`emit_mov_imm`/`emit_cmp` use.

For the `AddCarry` secondary finding, change `sizing.rs` to resolve the field
through `reg(...)` and compare to 31 (matching `emit_add_carry`), rather than
string-comparing to `"xzr"`.

Rejected alternative: rejecting `cmp_imm rhs>4095` at emit time. That contradicts
the spec's documented fallback and would turn a currently-working-in-principle
path into a hard error; sizing correctly is strictly better.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Add an encoder unit test asserting `instruction_size(cmp_imm rhs=4096)`
      equals `emit_words("cmp_imm", rhs=4096).len()`, and similar for a value
      needing a multi-word `mov_imm` (e.g. `rhs=0x1_0000_0000`). Confirm they
      fail today (4 vs 8, etc.).
- [ ] Add an `add_carry` test with `carry_in="sp"` asserting size == emitted
      length (to lock the secondary finding), or document why it is out of scope.
- [x] Blast-radius audit complete (above).

Acceptance: the new size-vs-emit tests fail for the documented reason.
Commit: —

### Phase 2 — the fix

- [ ] Give `CmpImm` its own arm in `instruction_size` mirroring `emit_cmp_imm`.
- [ ] Key `AddCarry` sizing on the resolved register number, not the spelling.

Acceptance: Phase 1 tests pass; `rhs<=4095` and all add/sub/sp sizes unchanged.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] `scripts/test-accept.sh` — confirm zero golden movement (no current path
      emits `cmp_imm rhs>4095`, so `.ncode`/binaries must be byte-identical).
- [ ] Run `scripts/artifact-gate.sh` (codegen change).

Acceptance: full suite green; no golden deltas.
Commit: —

## Validation Plan

- Regression test(s): the size-vs-emit equality tests in
  `src/arch/aarch64/encode/tests.rs`.
- Runtime proof: none required — the fix is proven by the size/emit equality
  invariant plus byte-identical goldens (behavior is unobservable today because
  the path is unreached).
- Doc sync: none expected (spec already documents the emit fallback).
- Full suite: `scripts/test-accept.sh`, `scripts/artifact-gate.sh`.

## Open Decisions

- Should `cmp_imm rhs>4095` remain a supported (sized-correctly) path, or should
  codegen guarantee `cmp_imm.rhs <= 4095` and this be an assertion? Recommended:
  size it correctly (keeps the documented fallback honest); revisit only if the
  team wants to drop the fallback entirely.

## Summary

The engineering risk is entirely in getting `instruction_size(CmpImm)` to equal
`emit_cmp_imm`'s byte count for the out-of-range case; the change is a few lines
in `sizing.rs` and must not move any golden (the divergent path is unreached
today). The `AddCarry` string-vs-register predicate is the same class of latent
mismatch and is cheap to harden in the same edit.
