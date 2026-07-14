# plan-00-B — Flagless Control Flow & Explicit Overflow

Last updated: 2026-06-29

Neutralize the most pervasive AArch64-ism in the MIR: **condition flags**. Today control
flow is `cmp`/`adds`/`subs`/`fcmp` set NZCV, then `b.eq`/`b.lt`/`b.vs`/… read it. RV64 has
no flags, so a flag-based MIR is unbuildable there. Replace flags with **compare-and-branch
+ compare-to-bool + explicit-overflow** ops (`mir.md §5`, §12.2 — resolved: flagless is the
long-term-best, no flags in the MIR).

Depends on plan-00-A. Stays AArch64-**byte-identical** under `-codegen mir`.

## 1. Goal

- MIR gains `br_<cc> a, b, Ltrue [, Lfalse]` (cc ∈ eq/ne/slt/sle/sgt/sge/ult/ule/ugt/uge),
  `set_<cc> dst, a, b` (→ 0/1), float `fbr_<cc>`/`fset_<cc>` with IEEE conditions **incl.
  unordered** (preserving plan-17 semantics: NaN compares → false; the `b.mi`/`b.ls`
  conditions), and explicit-overflow `add_ovf`/`sub_ovf`/`mul_ovf` (→ value + overflow vreg)
  replacing the `b.vs`/`b.vc` integer-overflow trap.
- MIR has **no flags op and no flag-reading branch**. `Cmp`/`Adds`/`Subs`/`FCmpD` +
  `Branch<cc>`/`b.vs` disappear from the MIR vocabulary.
- The AArch64 selector lowers `br_slt` → `cmp; b.lt`, `add_ovf` → `adds; cset/b.vs`, etc. —
  **byte-identical** to today.

### Non-goals

- No new ISA, no behavior change. The FP-finiteness boundary check stays the plan-17
  `fabs/fcmp vs +Inf` form (already flagless-friendly) — this plan only removes the
  *integer-flag* and *generic-compare-flag* coupling.

## 2. Current State

`regalloc/analysis.rs` already treats `Cmp`/`FCmpD` as `NoDef` and the `Branch*` as block
terminators (it rebuilds the CFG from the flat stream) — so the CFG model is flag-agnostic
already. The flag *coupling* is in instruction selection: the builders emit `cmp` then a
flag-branch as two separate ops with an implicit NZCV dependency.

## 3. Design

- NIR→MIR emits the flagless ops. The implicit `cmp→b.cc` pairing becomes one explicit
  `br_cc a, b, L` (operands carried, not a hidden flag dependency).
- AArch64 select: `br_cc` → `cmp; b.cc`; `set_cc` → `cmp; cset`; `fbr_cc` → `fcmp; b.cc`
  (with the exact plan-17 condition codes); `add_ovf` → `adds` + `cset`/`b.vs` consumer.
  The byte-identical requirement pins these mappings precisely.
- Overflow consumers (the integer-overflow trap path) read the explicit overflow vreg
  instead of the V flag — restructure the trap emit to take a bool.

## 4. Phases

1. Add the flagless MIR ops + the AArch64 selector mappings (cc ↔ `b.cc`, ovf ↔ `adds/cset`).
2. Retarget NIR→MIR control-flow lowering (`builder_control.rs`, comparisons, loop
   conditions) to emit `br_cc`/`fbr_cc`.
3. Retarget the integer-overflow trap path to `*_ovf` + bool consumer.
4. Byte-identical gate (suite self-diff under `-codegen mir`).

## 5. Validation

- Suite **byte-identical** (`mir` vs `direct`) after each phase. The plan-17 trap-location
  tests + the float `_invalid` traps must be byte-identical (the IEEE/unordered conditions
  are the silent-bug surface — pin them).
- No flags op remains reachable in any `-mir` dump.

## Summary

The single most *pervasive* neutralization — it touches every loop, condition, and integer
trap check — but conceptually simple: replace "set flags, branch on flags" with
"compare-and-branch," and "read V" with an explicit overflow value. Done byte-identically on
AArch64, it is what makes rv64 (no flags) even possible and removes the hardest-to-port idea
from the IR.
