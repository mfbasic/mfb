# plan-86-G — bounds-check elimination (induction-var get/set)

Sub-plan **G** of [plan-86](plan-86-benchmark-perf.md). Open. Correctness-critical dataflow.

**Covers (1 P1 + 1 P2 + 2 P3):** scalarbench listchurn (10.6), bignum modmul (19.5)/modexp (10.9),
mathpipe memo (11.5).

## Root cause
Each hot loop does `collections::get`/`set` on a **loop-invariant-length** list indexed by a **loop-induction
variable**, paying a per-access bounds check the C peers (stack arrays) and Python don't. bignum is a
bounds-check row, not an algorithm swap — the C mirror `benchmark/c/main.c:365` is also bit-serial.

- [~] **G1 SAFE CUT LANDED (get elision for the `FOR i=0 TO len(L)-k` shape); the bignum/memo symbolic pass
  remains.** — emit an unchecked `get`/`set` when `0 ≤ index < len` is provable. **DONE: the FOR/len-exact GET
  elision, SOUND + verified.** Implementation: `provable_index_locals` (`i -> (L, headroom k)`) + `len_of_local`
  (`n -> L` from `LET n = len(L)`) + `for_bound_expr` (resolve the IR's synthetic `$for_end/$for_step` locals) +
  `collect_reassigned_locals` (the whole-body no-reassign proof). `recognize_provable_index` (in
  `lower_numeric_for`) records the fact for the body ONLY when `start==0`, `step==1`, `end` resolves to
  `len(L)-k` (`k>=1`), and `i`/`L`/`n` are NOT reassigned anywhere in the body; the IR's `LET i = $for_iterN`
  alias inherits the fact (Bind handler). `is_provable_index_access` proves `get(L, i)` (k>=1) / `get(L, i+1)`
  (k>=2); `lower_collection_get` threads `unchecked` into `lower_list_get_common`, which then skips the
  `0<=index<count` compares (the `count` reg is still allocated → vreg numbering unchanged; only the two
  compare/branch pairs are removed). **SOUNDNESS gated by 3 mandatory negative fixtures — each proves the OOB
  access STILL traps `7-705-0001` (the elision did NOT fire unsoundly):** `bounds_elim_reassigned_rt` (L
  reassigned in body), `bounds_elim_headroom_rt` (`i+1` with k=1), `bounds_elim_noninduction_rt` (`i*2`, not the
  induction var) — plus positive `bounds-elim-rt` (elision fires 9→3 `list_get_invalid` refs, output identical:
  s2=800/s1=450/tot=10). 3776 unit tests green. **Measured listchurn 10.6 → 10.36ms (~2%, MARGINAL): the two
  bounds checks are a small fraction of the per-pass cost (`toScalars(base)` dominates each of the 2000
  passes).** Commit: `<pending-G1>`. **REMAINING (the larger G1 half — bignum modmul 19.5 P2 / modexp 10.9 P3 /
  memo 11.5 P3):** they do NOT match the safe shape (WHILE loops with `r = set(r, …)` reassigning the list; memo
  bound is a const not `len`), so they need the symbolic-upper-bound range pass (extend `integer_strict_upper`
  to carry `< len(L)` + prove in-place `set` preserves `len`) + `set` elision — separately justified,
  runtime-validated, riskier. Original scout note (kept): **SCOUT VERDICT (plan-86-A
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
