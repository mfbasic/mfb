# bug-746: linear-scan eviction around a compare-and-branch restores the victim only on the fall-through path

Last updated: 2026-09-26
Effort: medium (1h–2h)
Severity: HIGH
Class: Correctness

Status: Open
Regression Test: `src/codegen/engine/regalloc/tests.rs:eviction_around_rv64_compare_branch_restores_on_both_edges`

Found during bug-740 / plan-158-B/C. Pre-existing in the allocator; exposed on
riscv64 by the extra register pressure plan-158-B/C create.

## Failing Reproduction

`/tmp/t740r` (recreate from this text): a `probe(x, y)` that loops `dir` 0..3,
binds `newX = x + dir`, `newY = y + dir`, `oppX = x - dir`, `oppY = y - dir`,
tests `IF inMap(oppX, oppY) AND inMap(newX, newY)` where `inMap` is
`x >= 0 AND x < 79 AND y >= 0 AND y < 29`, and does
`hits = hits * 10 + dir + 1`; `probe(10, 10)` must be `1234`.
With the plan-158-B/C compiler (dev commit be0d7abdb): `-O0` prints `1234`,
`-O1` prints `1` on linux-riscv64 (box 2229). aarch64 host, linux-aarch64 (2226)
and windows-x86_64 correct.

## Root Cause (observed, fix not yet written)

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

## Fix Design (proposed)

Never evict around a block terminator: for a terminator, either pick a
scratch that needs no eviction, or emit the restore at the head of every
successor (split the edge). Simplest sound option: in the rewrite loop, when
the instruction `is_block_terminator` and needs an eviction, restore the victim
before the terminator is impossible (it is an operand)... so choose the
edge-split: emit `restore; b target` on a new label for the taken edge and
restore after for fall-through.
