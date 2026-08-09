# plan-86-G — bounds-check elimination (induction-var get/set)

Sub-plan **G** of [plan-86](plan-86-benchmark-perf.md). Open. Correctness-critical dataflow.

**Covers (1 P1 + 1 P2 + 2 P3):** scalarbench listchurn (10.6), bignum modmul (19.5)/modexp (10.9),
mathpipe memo (11.5).

## Root cause
Each hot loop does `collections::get`/`set` on a **loop-invariant-length** list indexed by a **loop-induction
variable**, paying a per-access bounds check the C peers (stack arrays) and Python don't. bignum is a
bounds-check row, not an algorithm swap — the C mirror `benchmark/c/main.c:365` is also bit-serial.

- [ ] **G1** — emit an unchecked `get`/`set` when `0 ≤ index < len` is provable. **SCOUT VERDICT (plan-86-A
  session): the SAFE minimal cut helps ONLY scalar listchurn (P1, 10.6ms) — memo and bignum do NOT match and
  need a strictly larger, riskier pass.** Why: (a) **listchurn** (`scalarbench.mfb:151-161`) is the clean shape
  — `LET scalars` (never reassigned), `n=len(scalars)`, `FOR i=0 TO n-2`, `get(scalars,i)`/`get(scalars,i+1)`,
  all provably in range; (b) **memo** (`mathpipe.mfb:185-195`) FAILS — the bound is `maxAmount` (a const), NOT
  `len(ways)`, AND `ways = set(ways,…)` reassigns `ways` every iter; (c) **bignum** modmul/modexp
  (`main.mfb:647-733`) FAILS — WHILE loops (not `FOR i=0 TO len-1`) with `r=set(r,…)` reassigning the list.
  **Reusable substrate (do NOT build from scratch):** plan-39 I1's range facts `integer_strict_upper`/
  `integer_lower_bounds` (`mod.rs:423,429`), the mutation tracker `scan_loop_locals` (`function_lowering.rs:413-510`
  → gives "L not reassigned in the loop body"), and the **conservative-default-false + clear-on-every-loop/
  Match/Trap/Assign-boundary** discipline (`builder_control.rs:842,1100-1112`) — model G1 exactly on how I1
  threads `elide_overflow` (`builder_numeric.rs:134-135,748`). **Edit points:** (1) in `lower_numeric_for`
  (`builder_control.rs:1149`), when the loop is `FOR i=0 TO len(L)-1` (or `-2` for `i+1` headroom) with `L` a
  Local AND `scan_loop_locals` shows `L ∉ top_assigns∪excluded`, record `provable_index_locals: {i→(L,k_hi)}`,
  cleared on every boundary; (2) thread an `unchecked: bool` (default false) into `lower_list_get_common`
  (`builder_collection_query.rs:70`, skip the `:121-125` compares) and `lower_list_set_in_place`
  (`list_mutate.rs:1860`, skip `:1909-1919`); compute the per-access proof from raw `NirValue`s in
  `lower_collection_get`/`lower_collection_set` (`builder_collection_queries.rs:25`, `collection_mutate.rs:163`)
  before lowering. (3) **MANDATORY negative fixtures** (`.ai/compiler.md:66-70`): list reassigned in body /
  bound not `len(L)` / index not the induction var — each asserts an intentionally-OOB access STILL traps
  `ErrIndexOutOfRange` (proves the elision did not fire). **An unsound elision is a silent OOB arena read/write
  that may only SIGSEGV past a threshold** (`.ai/compiler.md:80-90`). Land the FOR/`len`-exact cut first
  (listchurn); memo/bignum need a genuine symbolic-upper-bound range pass (extend `integer_strict_upper` to
  carry `< len(L)` + prove in-place `set` preserves `len`) — separately justified, runtime-validated.
- [ ] **G2 (memo only)** — strength-reduce constant `MOD 1000000007` to a conditional subtract. Lowering:
  `emit_integer_binary` MOD arm (`builder_numeric.rs:906-921`) is a full `sdiv`+`msub`; replace with
  `dst=left; if dst>=m: dst-=m` when `right` is the const `m` AND `left` (= `A+B` where each of A,B is a prior
  `MOD m` result, so `∈[0,2m)`) is provably non-negative and `< 2m`. **Gap: no existing "value < m" fact** —
  add a narrow one (result of `x MOD constM` is in `[0,constM)` for non-negative `x`; both memo operands are
  non-negative `get(ways,…)` counts), thread it conservative-default-false, ship with a negative fixture (a
  `MOD` whose operand isn't provably `<m` keeps `sdiv`/`msub`).

## Acceptance
bignum/memo/scalar checksums + overflow behavior + `scripts/artifact-gate.sh` + a targeted out-of-bounds
**negative test proving the elision does NOT fire when the bound is not provable**.
