# bug-749: rv64 selector keeps a value in `t0` across spill code, and a >2 KiB frame's spill store overwrites it

Last updated: 2026-09-26
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: `src/arch/riscv64/select.rs:tests::no_selected_sequence_holds_t0_across_a_virtual_register`,
`src/target/linux_riscv64/mod.rs:tests::a_large_frame_spill_store_never_lands_in_a_t0_window`

A linux-riscv64 program gives a wrong result, a false trap or a wrong branch when
a function's frame is larger than 2 KiB and the allocator spills or evicts in the
middle of a compare-with-immediate or an overflow-checked add/subtract. The
checked-in `tests/byte-identity/general` fixture shows it. Built for linux-riscv64
and run under qemu (Alpine musl), it prints correctly up to `toByte`, then
`toMoney("1.25")` raises `Error: 7-705-0003 Text parse or non-finite numeric
representation conversion failed.` and exits 255. The host and the other targets
print `1.25`. The failure is silent in kind: every mis-evaluated branch goes
somewhere legal.

**The single correct behavior:** a register value the rv64 selector keeps between
two of its own instructions survives any spill code the allocator inserts between
them, whatever the frame size.

References:

- `mfb spec architecture riscv64-instruction-set` ("Flagless control flow").
- `src/arch/riscv64/encode/emitter.rs` module doc: `t0`–`t2` are reserved from
  allocation and reused by the encoder, and "every expansion is self-contained".
- Found while runtime-proving bug-746 (lane riscv-ret). It is independent of it:
  present with and without the bug-746 fix.

## Failing Reproduction

```
mfb build --target linux-riscv64 tests/byte-identity/general
# run build/general_codegen_cover_rt-musl.out on riscv64 (qemu-user, Alpine 3.20)
```

- Observed: `… 65` then `Error: 7-705-0003 … conversion failed.`, exit 255.
- Expected: `… 65`, `1.25`, `42.00`, `1.25`, `1.25`, `200.00`, …, exit 0.

A minimal `io::print(toString(toMoney("1.25")))` works. Dropping any one of the
fixture's `toString`/`toInt`/`toFloat`/`toFixed`/`toByte` blocks also makes it
pass, because that brings `main`'s frame under 2 KiB (measured: 2128 bytes
failing, 1984 passing).

## Root Cause

Two facts combine.

1. `src/arch/riscv64/encode/emitter.rs:emit_store` (and `emit_load_fp` /
   `emit_store_fp`) needs an address register once the offset exceeds the 12-bit
   immediate (> 2047), and it uses `t0`. `emit_load` avoids that by staging in `rd`.
   This is the one place spill code touches a lowering scratch register.
2. `src/arch/riscv64/select.rs` holds a value in `t0` from one emitted instruction
   to a later one, and the later instruction names a virtual register. Register
   allocation runs **after** selection, so it inserts that operand's reload, and
   under full pressure an **eviction store** (`str victim, [sp+evict_slot]`),
   between the two. In a frame over 2 KiB that store is `li t0, off; add t0, sp,
   t0; sd victim, 0(t0)`, and the held value is gone.

The observed site is `expand_fused`'s `CmpImm` (`mov_imm t0, imm; rv.br vX, t0`).
In the fixture's `toMoney(String)` parser, `IF c = "-"` ran as

```
mov_imm t0, 45
str_u64 t3, sp, 2096      <- evict save; offset > 2047 overwrites t0
ldr_u64 t3, sp, 1968      <- reload of the spilled char
rv.br   t3, t0, ne, …     <- compares against the frame address, not 45
```

Every character test in the parser was mis-evaluated, so `1.25` failed the
digit check.

## Goal

- The two regression tests pass, and `tests/byte-identity/general` prints the host's
  output under qemu riscv64.

### Non-goals (must NOT change)

- No shrinking of the allocatable pool (`.ai/arch-abi.md`: the pool-shrink
  allocator fault), and no use of `tp`/`gp` as a new scratch.
- No change to any other target.
- Not "fixing" it by capping frame size or reordering the frame. That only moves the
  threshold.

## Blast Radius

Selector sequences that write a scratch in one instruction and read it in a later one
(`select.rs`, `v128.rs`), each checked for a virtual-register operand in between:

- `expand_fused` `CmpImm` (`mov_imm t0; rv.br vX, t0`): **fixed**. It is the observed site.
- `expand_fused` `Adds`/`Subs`: the sum/difference sits in `t0` across two `eor`s
  that read `lhs`/`rhs`, and across `mov dst, t0`, whose `dst` may spill. **Fixed.**
- `store_flag_rhs` (`mov_imm t0; add t0, s11, t0; str rhs, [t0]`): the address sits
  in `t0` across the store of the virtual register `rhs`. **Fixed.**
- `select_riscv64` standalone-branch arms (`mov_imm t0; rv.br gp, t0`;
  `load_flag_rhs` into `t0`; `rv.br gp, t0`): unaffected. No virtual register is
  in between, so the allocator inserts nothing.
- `float_branch`: `rv.fcmp t0; rv.br t0` has nothing between. The `vs`/`vc` forms
  hold `t1` across the second `rv.fcmp` and `t2` across the `and`. **Unaffected:**
  spill code never touches `t1`/`t2` (fact 1).
- `FCmpZeroD` holds `ft1` across `rv.fcmp`. **Unaffected:** FP spill code uses only `t0`.
- `v128.rs` scalarization: lanes are held in `t0`/`t1`/`ft*` and the slot base in
  `t2`. The only instructions with virtual-register operands (`LdrQ`/`StrQ`'s
  `base`) move the value in `t1` and are preceded only by the `t2` slot-base
  setup. **Unaffected**, measured by the corpus checker below.
- Measured: a post-allocation checker (`t0` written, then an `sp`-based store or FP
  memory op before `t0` is read) swept over every executable project's linux-riscv64
  `-ncode` in `tests/{rt-behavior,byte-identity,rt-error}` and `examples/`. The
  counts are recorded in Phase 1 below.

## Fix Design

Only `t0` is ever clobbered by spill code (fact 1), so the selector must never hold
`t0` across an instruction that names a virtual register. `t1`/`t2` are safe there.

- `CmpImm`: materialize the immediate in `t1`.
- `Adds`/`Subs`: keep the result in `t2`, compute the sign word in `t1`, and use `t0`
  only for the intermediate `t2 ^ lhs` (written and consumed in adjacent
  instructions with no virtual register in between). `mov dst, t2` then precedes the
  branch on `t1`.
- `store_flag_rhs`: build the address in `t1`.

Rejected: teaching `emit_store` another scratch. There is none free. `t1`/`t2`
carry selector values. `tp`, `gp` and the pool are ruled out above. Moving `sp`
temporarily is also out, because it exposes live frame memory to a signal handler.

Expected output shift: every linux-riscv64 golden whose code contains an immediate
compare or an overflow-checked add/subtract changes register names (`t0` → `t1`/`t2`).
No other target moves.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] Selector invariant test: no fused `CmpImm`/`Adds`/`Subs` expansion and no bare
      register compare holds `t0` across an instruction naming a virtual register.
- [ ] Whole-pipeline test: `tests/byte-identity/general`'s real source, lowered for
      linux-riscv64, has no `t0` window broken by an `sp` store at offset > 2047.
- [ ] Corpus checker counts, at base and after.

Commit: —

### Phase 2 — the fix

- [ ] `select.rs`: `CmpImm` → `t1`; `Adds`/`Subs` rework; `store_flag_rhs` → `t1`.
- [ ] Spec: the riscv64 selection section states the scratch rule.

Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Regenerate the linux-riscv64 goldens it moves; classify the diff by target.
- [ ] Full `cargo test`; `scripts/artifact-gate.sh target/release/mfb all`.
- [ ] `tests/byte-identity/general` under qemu riscv64 matches the host.

Commit: —
