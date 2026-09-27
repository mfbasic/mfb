# bug-746: linear-scan eviction around a compare-and-branch restores the victim only on the fall-through path

Last updated: 2026-09-26
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Closed
Regression Test: `src/codegen/engine/regalloc/tests.rs:eviction_around_rv64_compare_branch_restores_on_both_edges`

Found during bug-740 / plan-158-B/C. Pre-existing in the allocator; exposed on
riscv64 by the extra register pressure plan-158-B/C create.

## STATUS: FIXED (efd343f3a)

Shipped as the edge split proposed below, laid out inline rather than as an
out-of-line block. When the evicting instruction is a conditional terminator,
`linear_scan.rs:run` retargets it to a fresh `ra_evict_<i>_taken` label and emits:
the fall-through restore; `b ra_evict_<i>_join`; the taken label; the taken-edge
restore; `b <original target>`; the join label. `fresh_label` checks names against
every label already in the function. Goldens: `ec8ca5155`, the 4 linux-riscv64
byte-identity covers and nothing else. Spec: `architecture/06_native.md` (allocator
paragraph).

Verified on merged tip 9a5278d60 (main 4b38d5d1a merged in), both fixes applied:
`scripts/artifact-gate.sh target/release/mfb all` → 0 diffs / 2166 goldens (main-tip
baseline also 0), and full `cargo test --no-fail-fast` → 247 binaries ok.
Runtime: under qemu riscv64, the bug-746-only compiler repairs all four rt-behavior
fixtures that fail at base from this bug alone (table below). The riscv64 boxes
2229/2232 refused ssh, so there was no hardware run.

The doc's own `/tmp/t740r` repro still needs bug-740's pressure to reach this path
at HEAD (it does not evict around a terminator here). The standalone regression test
reproduces the mechanism without it.

## Failing Reproduction

`/tmp/t740r` (recreate from this text): a `probe(x, y)` that loops `dir` 0..3,
binds `newX = x + dir`, `newY = y + dir`, `oppX = x - dir`, `oppY = y - dir`,
tests `IF inMap(oppX, oppY) AND inMap(newX, newY)` where `inMap` is
`x >= 0 AND x < 79 AND y >= 0 AND y < 29`, and does
`hits = hits * 10 + dir + 1`; `probe(10, 10)` must be `1234`.
With the plan-158-B/C compiler (dev commit be0d7abdb): `-O0` prints `1234`,
`-O1` prints `1` on linux-riscv64 (box 2229). aarch64 host, linux-aarch64 (2226)
and windows-x86_64 correct.

## Root Cause

`src/codegen/engine/regalloc/linear_scan.rs` rewrite loop: when an instruction
needs more scratch than is free, it evicts a victim (`emit_spill` before,
`emit_reload` after the instruction). riscv64 selects compares into a fused
`rv.br lhs rhs cond target` — a block terminator. The restore is emitted after
it, so it runs only on the fall-through edge; on the taken edge the victim
register (here `t3`, holding parameter `x`) keeps the scratch value:

```
str_u64 t3 sp 928
ldr_u64 t3 sp 752
rv.br s9 t3 ge cmp_true_21
ldr_u64 t3 sp 928        <- fall-through only
```

AArch64/x86 compare with a separate non-terminator `cmp`, so the eviction
brackets the `cmp` and both edges see the restored register.

## Reproduction at HEAD without bug-740 (added by the fixing lane)

The allocator mechanism does not need plan-158-B/C's extra pressure. A standalone
fixture (the regression test above) builds a stream where v1..v12 fill the 12-register
rv64 integer pool across an `rv.br`, and v0, defined last, spills, so the branch reads
it through an evicted scratch. It is allocated with `Riscv64RegisterModel` and run on
a small model machine, and every value is stored on both edges. At HEAD this fails
with `taken edge: v1 was clobbered across the rv.br` (left `Some(0)`, i.e. v0's value,
right `Some(101)`).

The doc's `/tmp/t740r` program does NOT evict around a terminator at HEAD. An
instrumented HEAD compiler (a temporary `eprintln!` when `evictions` is non-empty on an
`is_block_terminator` op, never committed) swept `-ncode -O0/-O1` over every executable
project in `tests/{rt-behavior,byte-identity,rt-error}` and `examples/` for
linux-riscv64, linux-aarch64 and linux-x86_64. See the Blast Radius section for the result.

## Blast Radius (measured)

The instrumented sweep described above counted evictions around a block terminator
over 877 projects × {linux-riscv64, linux-aarch64, linux-x86_64} × {-O0, -O1}:

- **linux-riscv64: 43 projects** (86 builds). These are the byte-identity covers
  `collections`, `general`, `math` and `strings`, plus 33 rt-behavior fixtures
  (`strings/self-update-grow-valid` has 194 sites; most have 1–40).
- **linux-aarch64, linux-x86_64: 0.** None of their terminators names a register, so
  no eviction can bracket one, as the mechanism predicts.

Runtime, under qemu-user riscv64 (Alpine musl; boxes 2229/2232 refused ssh): every
one of the 33 rt-behavior hits was built at base and with the fix, and compared to
the host binary's output. At base **8 differ**; with the fix **all 33 match**.
Four of the eight have no bug-749 hazard at all (checker count 0), so bug-746
alone breaks them at HEAD:

- `collections/sort-string-gather-rt`: SIGSEGV (rc 139).
- `collections/sortby-string-gather-rt`, `collections/inplace-decline-aliasing-rt`,
  `math/bug128_fixed_atan2_overflow_min`: wrong output.

The other four (`money/money_operations`, `strings/self-update-{grow,window}-valid`,
`strings/strings-pad-to-width-rt`) also carry bug-749 hazards, so they need both fixes.

Site verdicts:

- `linear_scan.rs:run` eviction bracket around a conditional terminator: **fixed**.
- Eviction around an unconditional terminator (`b`, `ret`, `b .`): **unaffected**.
  None names a virtual register, so none can need a spill scratch.
- Def-spill stores after a terminator: **unaffected**. No terminator defines a
  virtual register.

## Fix Design (proposed)

Never evict around a block terminator: for a terminator, either pick a
scratch that needs no eviction, or emit the restore at the head of every
successor (split the edge). Simplest sound option: in the rewrite loop, when
the instruction `is_block_terminator` and needs an eviction, restore the victim
before the terminator is impossible (it is an operand)... so choose the
edge-split: emit `restore; b target` on a new label for the taken edge and
restore after for fall-through.

## Phases

### Phase 1 — failing test + audit

- [x] `eviction_around_rv64_compare_branch_restores_on_both_edges`: RED at base for the
      documented reason (taken edge sees v1 = v0's value).
- [x] Instrumented corpus sweep plus qemu runtime table (Blast Radius, measured).

Commit: efd343f3a (test landed with the fix; RED recorded in its message)

### Phase 2 — the fix

- [x] `linear_scan.rs`: taken-edge split for an eviction around a conditional
      terminator; `fresh_label`.
- [x] Spec `architecture/06_native.md`.

Commit: efd343f3a

### Phase 3 — regenerate expected outputs + full validation

- [x] Goldens regenerated with a bug-746-only compiler: exactly the 4 predicted
      linux-riscv64 covers.
- [x] Full `cargo test` and `artifact-gate all` on the merged tip.
- [x] Runtime: the four bug-746-only failures match the host under qemu riscv64.

Commit: ec8ca5155
