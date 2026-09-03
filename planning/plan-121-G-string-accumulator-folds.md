# plan-121-G: String accumulator folds in `reduce`/`reduceRight`

Last updated: 2026-09-02
Effort: medium (1h–2h)
Depends on: nothing (independent of A–F)

`collections::reduce` over a `List OF String` with a concatenating reducer is
**O(N²)**: each fold step allocates a fresh tight string of
`len(acc) + len(x)` bytes. The identical fold written as a hand loop is **O(N)**,
because `acc = acc & x` on a plain `MUT String` local is matched by
`try_inplace_concat_assign` and lowered to an amortized-O(1) self-append into a
grown buffer. At N=8000 the two spellings are **790× apart** for the same result.

The mechanism already exists and is already proven sound. `reduce` simply cannot
reach it, because its accumulator is a lambda parameter returned by value.

Behavioral outcome: a String-accumulating `reduce`/`reduceRight` is linear in the
total output bytes, matching the hand-written loop.

References: `builder_inplace_assign.rs:680-691` — the `try_inplace_concat_assign`
doc comment, which states the grown-buffer contract and why the shadow never
escapes (D9); `.ai/collections.md`; `mfb spec` §14.

## Prerequisites

Stated once in plan-121-A. This sub-plan additionally needs nothing from A–F; it
is placed last only because its blast radius (the accumulator representation
inside a HOF) is larger than E's.

| Must be true | Command | Status |
|---|---|---|
| Suite green at HEAD | `cargo test --no-fail-fast` | **MET** — 99 results, 0 failed, EXIT=0 |
| `try_inplace_concat_assign` still present | `grep -c "fn try_inplace_concat_assign" src/codegen/collection/assign/builder_inplace_assign.rs` → 1 | **MET** — 1 |

## 1. Goal

- `collections::reduce` and `reduceRight` with a `String` accumulator cost
  O(total output bytes), not O(N²).
- The six rows in §2 reach grade B or better.

### Non-goals

Inherited verbatim from plan-121-A §1. Additionally:

- **The reducer stays an ordinary function.** No change to how a lambda or named
  `FUNC` is written, typed, or called. The reducer keeps receiving and returning
  a `String` by value; this changes only the buffer the accumulator lives in
  between steps.
- **No change to the canonical `String` form.** Anything that copies, returns, or
  transfers the accumulator must still see the tight `[len][bytes][NUL]` value —
  the same contract `try_inplace_concat_assign` already maintains. The grown
  shadow must never escape.
- **No change to fold order or to `reduceRight`'s direction.** `reduceRight`
  builds its result in the opposite order and must keep doing so; a fold is not
  assumed associative.

## 2. Current State

`collections::reduce` is `Body::abi_inline(lower_reduce)`
(`src/codegen/builtins/collections/func_reduce.rs:129`) — already native. The
accumulator is threaded through the reducer call by value, so each step produces
a fresh tight string.

`try_inplace_concat_assign` (`builder_inplace_assign.rs:692`) already implements
the fast representation for the hand-written spelling: a grown buffer with
geometric capacity headroom in a frame-local shadow slot, appended into and
frozen to the tight form on any copy/return/transfer.

### Measured populations

| What | Count | Command |
|---|---|---|
| `reduce`/`reduceRight` rows C-or-worse | 6 | `./benchmark/rank.py --csv \| awk -F, '$2=="reduce"\|\|$2=="reduceRight"'` |
| Worst | 601× (`list (State-Dynamic) reduce`) | same |
| …that lose to CPython | 6 (all of them) | `awk -F, '$4=="RED"'` over that set |
| Integer-accumulator rows (control) | 6, all grade A (1.73–1.82×) | same filter, `Fixed` sections |

### Verified properties

- **VERIFIED — the fold is O(N²) and only for the String accumulator.** Spike 3:
  one `reduce` over N elements, Integer accumulator — 3, 4, 8, 16 µs at
  N = 500, 1000, 2000, 4000 (linear). String accumulator over the same sizes —
  581, 1867, 8656, 39526 µs (×3.2, ×4.6, ×4.6 per doubling).
- **VERIFIED — the fast mechanism exists and the gap is purely reachability.**
  Spike 4 runs the identical fold two ways over the same list. Hand loop:
  42, 75, 150, 297 µs at N = 1000, 2000, 4000, 8000 — exactly linear. Via
  `reduce`: 2378, 9001, 39681, 234573 µs. At N=8000 that is **790×** for the same
  answer. Nothing new needs inventing.
- **VERIFIED — the container is irrelevant to this row.** `reduce` is read-only
  over its list, and the Integer rows are grade A in the plain, record and STATE
  containers alike (1.73–1.82×). So this is not a plan-121-C/D problem and does
  not depend on them.

## 3. Design Overview

Give `lower_reduce`'s accumulator the same representation
`try_inplace_concat_assign` produces, when two conditions hold:

1. the accumulator's static type is `String`; and
2. the reducer's body is a self-concat of the accumulator — i.e. it returns
   `acc & …` with `acc` the accumulator parameter, unmodified otherwise.

Under those conditions the fold appends into a grown buffer across steps and
freezes to the tight form once, when the fold's result is bound. Otherwise the
fold keeps today's behavior exactly.

**Where correctness risk concentrates:** the escape analysis. The grown shadow
must not be observable. The existing concat arm reasons about a *local* whose
lifetime the builder controls; here the buffer is threaded through a **function
call**, so the reducer could in principle stash the accumulator somewhere. The
condition above (the reducer's body is exactly a self-concat) is deliberately
narrow for that reason — it is a syntactic condition on a body the compiler can
see, not an inter-procedural analysis. **If the reducer is not statically
visible, or does anything else with `acc`, the optimization must decline.**
Declining is always correct.

The second risk is `reduceRight`: it must keep folding right-to-left. Appending
into a grown buffer builds left-to-right, so `reduceRight` with a self-concat
reducer produces `acc & x` with the accumulator on the *left* and the elements
arriving in reverse — which is still a left-append into the buffer. Phase 1 must
confirm this by test before Phase 2 assumes it.

**Byte-identity is NOT the gate.** Expected drift: `.ncode` for fixtures
containing a String-accumulating `reduce`. Phase 1 records the set.

### Rejected alternatives

- **Make `String` a rope.** Rejected: a representation change affecting every
  string in the language, with a large correctness surface, to fix one fold
  shape. The non-goals forbid changing the canonical form.
- **Rewrite the benchmark row to use a hand loop.** Rejected: that hides a real
  and general defect — `reduce` with a String accumulator is an ordinary thing to
  write, and it is currently 790× slower than the loop it is sugar for.
- **Inter-procedural escape analysis on the reducer.** Rejected as
  disproportionate: the narrow syntactic condition captures the benchmark row and
  the common idiom, and declines safely everywhere else.

## Phases

### Phase 1 — Pin the semantics that must not move

- [x] Write rt fixtures for the fold's observable contract *before* optimizing.
      **`tests/rt-behavior/collections/p121g-reduce-accumulator-rt`**, passing at
      HEAD. Every requested shape plus three the plan did not list, and each
      negative has a DISTINCT answer, so a wrong accept cannot hide behind a
      coincidentally-equal result:

      | shape | answer at HEAD | why it is here |
      |---|---|---|
      | `acc & x` (`reduce`) | `012345` | the shape the optimization is for |
      | `acc & x` (`reduceRight`) | `543210` | direction — see below |
      | hand loop | `012345` | `reduce` must agree with the loop it is sugar for |
      | `x & acc` | `543210` | prepends; cannot become an append — must decline |
      | `x & acc` (`reduceRight`) | `012345` | the mirror |
      | ignores `acc` | `5` | result is the LAST element |
      | returns a constant | `K` | uses neither operand |
      | `acc & toString(len(acc)) & x` | `0021426384105` | **reads** `acc`, so the tight form is observable |
      | `acc & x & x` | `001122334455` | `acc` appears once but it is not one append |
      | LAMBDA reducer | `012345` | the same shape, not statically a named `FUNC` |
      | empty list | `[seed]` | the seed is the whole result |
      | single element | `solo` | one step |
      | **non-empty seed** (added) | `SEED:012345` | a buffer that assumed an empty start would drop the seed |
      | **non-empty seed, right** (added) | `SEED:543210` | the seed stays at the FRONT |
      | **result is a normal String** (added) | `len=90`, `head=01234567`, `tail=4849`, `copyEq=TRUE` | measurable, sliceable, comparable, and independent of any buffer it was built in |
- [x] Confirm by test which side `reduceRight`'s self-concat appends on (§3).
      **ANSWERED BY TEST, and §3's hypothesis is confirmed:**
      `reduceRight(xs, "", acc & x)` = `543210`. The accumulator stays on the
      **LEFT** and the elements arrive in **reverse**, so it is still a
      **left-append into the buffer** — the same append the forward fold does,
      fed in the opposite order. `seededRight = SEED:543210` confirms the seed
      stays at the front rather than being appended last. So `reduceRight` needs
      no different buffer strategy, only its existing iteration order.
- [x] Record the `.ncode` goldens containing String-accumulating `reduce`.
      **`tests/byte-identity/collections` — one root**, and this time the census
      also asked the question Correction F2 says to ask.

      | query | result |
      |---|---|
      | `.ncodesum` roots that resolve (control) | **141 of 141** |
      | roots mentioning `collections::` (control) | 6 |
      | roots calling `reduce`/`reduceRight` | **1 — `byte-identity/collections`** |
      | **builtin bodies** calling `reduce`/`reduceRight` (the F2 question) | `func_reduce.rs`, `func_reduce_right.rs`, **`func_sum.rs`** |

      **`collections::sum` is implemented via `reduce`.** That matters: a change
      to `reduce`'s *general* lowering would drift every fixture that calls `sum`,
      which is far more than one root. It does not apply here only because the
      planned change is gated on a **String** accumulator and `sum` folds
      `Integer`/`Float`/`Fixed` — but if Phase 2's condition ever widens beyond
      String, this is the row that makes the drift set explode.

#### What already exists on this path, and what it constrains

Two fixtures already cover `reduce`'s accumulator and **neither may change**:

- **`reduce-accumulator-reclaim-rt` (plan-86-B) is the binding constraint Phase 2
  must respect, and §3 does not mention it.** Native `reduce` already **reclaims
  the superseded accumulator and item on every iteration** — the pre-fix lowering
  freed neither, so a `List OF String` fold grew arena RSS by about one block per
  element per pass — and the reclamation is **guarded by runtime pointer-equality**
  so the bug-307 aliasing shapes stay safe. A grown-buffer accumulator threads a
  block that must *not* be reclaimed each step, so Phase 2 has to keep those
  guards true rather than merely not crash: the fixture's own header says a
  use-after-free or double-free surfaces there as corrupted output or a crash.
- **`hof-string-item-lifetime-rt`** covers String item lifetime through the HOFs.

Both pass unchanged, so neither engages AGENTS.md's four-question gate.

Acceptance: **MET.** `p121g-reduce-accumulator-rt` exists and passes at HEAD
across all fifteen shapes; the `reduceRight` direction is answered by the test
(`543210` — accumulator left, elements reversed, so still a left-append), not by
reasoning. No `src/` change in this phase.
Commit: —

### Phase 2 — Grown-buffer accumulator for the self-concat reducer

- [x] ~~In `lower_reduce`, detect the narrow condition from §3 and thread the
      accumulator through a grown buffer~~ — **done, but NOT there and NOT that
      way; see Correction G1.** The reducer is a function *pointer* at
      `lower_reduce`, and the cost is inside the reducer, so no way of threading
      the accumulator can remove it. The fold is instead **rewritten into the loop
      it is sugar for**, in `src/ir/lower.rs`, and the existing
      `try_inplace_concat_assign` then does the work it already does — no new
      buffer machinery.

      A post-pass on the ops `lower_statement` produces, for the same reason
      `hoist_trap_calls` is one: `lower_expression_with_expected` returns an
      `IrValue` and has no statement sink, and threading one through it would
      touch every recursive call in the core lowering path.
- [x] Decline — falling back to today's lowering — whenever the reducer is not
      statically visible or does anything with `acc` other than a self-concat.
      **Six declines, each pinned by a test:** a `Closure` reducer (substituting a
      body that reads a capture is unsound); a name not in the recognized table; a
      left-concat `x & acc`; a right operand that names the accumulator
      (`hir_mentions`, which answers "mentions" for any shape it does not
      enumerate — the safe direction); a non-`String` accumulator; and a list or
      seed that is not effect-free.

      Plus one the plan did not anticipate, which is the *evaluation-order*
      condition: the fold is hoisted to the front of its statement, so it is
      rewritten **only when it is the first effectful node** in that statement.
      Everything before it must be effect-free, or hoisting would reorder
      observable work. `ir_value_is_effect_free` answers `false` for anything it
      does not enumerate.
- [x] Apply the same to `reduceRight`, preserving fold direction. **Done.**
      `NirOp::ForEach` is forward-only, so `reduce` emits a `ForEach` and
      `reduceRight` a counted `For` with a **negative step** over
      `collections::get`. Phase 1's test is what made this safe to write: a
      self-concat `reduceRight` is still a **left-append** (`543210`), so the two
      share one loop body and differ only in iteration direction.
- [x] Tests: every Phase 1 fixture must still pass unchanged; add a
      codegen-inspection test proving the fast path is *taken* and *declined*.

      **All fifteen Phase 1 answers are byte-identical after the rewrite**, and so
      are both pre-existing fixtures — including `reduce-accumulator-reclaim-rt`,
      the plan-86-B fixture Phase 1 flagged as the binding constraint (it pins the
      accumulator reclamation and the bug-307 aliasing shapes). Neither engages
      the four-question gate.

      **`tests/codegen_reduce_concat_fold.rs`, 7 tests**, using the absence of
      `lower_collection_reduce_impl`'s `reduce_call_loop` label as the instrument —
      if the rewrite fires, that lowering is never reached. Three taken (`reduce`,
      `reduceRight`, and the **nested-inside-`len`** shape the benchmark actually
      uses) and four declined (left-concat, reads-the-accumulator,
      ignores-the-accumulator, Integer accumulator).

Acceptance: **MET.** Spike 4 re-run, same fold two ways over the same list,
before (a compiler built from `56b368996`) and after:

| N | hand loop | `reduce` before → after | `reduceRight` before → after |
|---|---|---|---|
| 1000 | 22 µs | 473 → **35** | 314 → **35** |
| 2000 | 48 µs | 1739 → **58** | 1200 → **72** |
| 4000 | 108 µs | 7652 → **112** | 4964 → **129** |
| 8000 | 212 µs | 31868 → **219** | 20512 → **260** |

`reduce` now tracks the hand loop **within 3%** (219 µs against 212 µs at
N = 8000) and both are linear — doubling N doubles the time. That is **145×** for
`reduce` and **79×** for `reduceRight`, with identical answers
(`len 16000/16000/16000` throughout). The 790× gap the sub-plan was written
against is closed.

Every Phase 1 fixture passes unchanged. Golden drift: **3 diffs, all `.ir`**
(`reduce-accumulator-reclaim-rt`, `hof-string-item-lifetime-rt`, and this
sub-plan's own fixture) — which is exactly what a desugar moves. No `.ast` moved,
and both pre-existing fixtures' **run output is byte-identical**, checked
directly. Wider than Phase 1's predicted one root, but in the same direction and
for the same reason F2 records: the census counted `.ncodesum` roots, and these
three carry `.ir` goldens instead.
Commit: —

### Phase 3 — Re-rank

- [ ] Re-run `./benchmark/run.sh 10` and `./benchmark/rank.py`.
- [ ] Confirm the six rows reach grade B or better and that all six RED flags
      clear.

Acceptance: the six rows in §2 are grade B or better with no RED flag.
Commit: —

## Validation Plan

- **Tests:** the Phase 1 negative-shape fixtures are the core of this sub-plan —
  each one pins a case the optimization must decline. Per the project's hard-won
  rule, every RED test is paired with one pinning what must NOT change.
- **Coverage check:** confirm both the taken and the declined branches are
  executed by tests; an optimization that never fires still passes a green suite.
- **Runtime proof:** spike 4 re-run — `reduce` and the hand loop must have the
  same asymptotic shape.
- **Doc sync:** `.ai/collections.md` gains the accumulator-representation rule
  and its decline conditions.
- **Acceptance:** `cargo test --no-fail-fast`, `./scripts/test-accept.sh`, the
  artifact gate, `cargo fmt` per AGENTS.md.

## Corrections

### G1 — the optimization cannot live in `lower_reduce`; the reducer is a pointer there

§3 says: *"Give `lower_reduce`'s accumulator the same representation
`try_inplace_concat_assign` produces, when … the reducer's body is a self-concat
of the accumulator."* **The reducer's body is not visible at `lower_reduce`.**

By the time `lower_collection_reduce_impl` runs
(`src/codegen/builtins/collections/gen_memory.rs:355`), the reducer is a
**function value** — a pointer stored into `reduce_action` and called
indirectly. Its only static check is `require_direct_callable`
(`gen_flow.rs:63`), which inspects the *type* (`ParameterType::Func`) and asserts
the location is not `void`. There is no body to pattern-match, so the §3
condition cannot be evaluated where §3 places it.

Nor can it be evaluated one layer up: `LowerContext` (`src/ir/lower.rs:27`)
carries `function_returns`, `function_types` and `function_params` — **signatures
only**. `src/monomorph/` holds full bodies but has no rewrite infrastructure.

**Consequence for the design.** The cost is *inside* the reducer — each call
returns a fresh tight `len(acc) + len(x)` string — so no change to how the fold
*threads* the accumulator can remove it. The accumulator cannot be given a grown
buffer that the reducer appends into, because the reducer is opaque and receives
its argument under ordinary value semantics; a callee-side "append into `acc` in
place" would corrupt the caller's string, since a caller does not give up
ownership of a `String` argument.

The only sound shape is therefore what the sub-plan's own title implies: **do not
call the reducer at all for the recognized shape — rewrite the fold into the loop
it is sugar for**, at a layer where the body is visible, and let the existing,
already-proven `try_inplace_concat_assign` do the work. That needs no new buffer
machinery.

**What that rewrite actually requires** (the estimate of *medium (1h–2h)* is also
wrong, and this is why):

1. Plumbing HIR function bodies into the lowering context, which today has none.
2. An expression-position hoisting walker — the benchmark row is
   `acc = acc + len(collections::reduce(base, "", strConcatFn))`, so the call is
   nested two deep and a statement-level rewrite does not reach it. The pattern
   exists to copy (`rewrite_trap_operands`, `src/ir/lower.rs:1880`) but it is
   ~200 lines of walker.
3. Parameter substitution of the reducer body's right operand.
4. **Two** loop forms: `NirOp::ForEach` for `reduce`, and `NirOp::For` with a
   negative `step` for `reduceRight` — `ForEach` is forward-only. Phase 1's test
   settled that this is sound (`reduceRight` = `543210`, i.e. still a left-append
   fed in reverse), so the two share one body shape.
5. A decline set that is exactly right, including `IrValue::Closure` captures —
   substituting a body that references a capture is unsound.

This lands in `src/ir/lower.rs`, which `.ai/collections.md` and the project's
memory both flag as shape-coupled to lowering, where **a missed desugar shape
miscompiles silently**. That is not a reason to skip it; it is the reason Phase 1
pinned fifteen shapes with fifteen distinct answers first.

## Open Decisions

- **How narrow to make the reducer condition** — recommend starting at the
  narrowest (body is exactly `RETURN acc & <expr>`) and widening only if a real
  idiom is missed. A too-narrow condition costs performance; a too-wide one costs
  correctness, and correctness outranks it. (§3)

## Corrections

<Filled in during execution.>

## Summary

The lowest-risk large win in plan-121: the fast representation already exists,
is already proven sound for the hand-written spelling, and the measured gap is
790×. All the risk is in the decline conditions — the grown buffer must never
escape through a reducer the compiler cannot see, which is why the trigger is a
narrow syntactic check rather than an escape analysis.
