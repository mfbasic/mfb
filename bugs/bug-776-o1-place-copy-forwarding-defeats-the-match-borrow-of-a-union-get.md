# bug-776: at -O1+, a union element fetched by `collections::get` and only `MATCH`ed is copied — place copy forwarding removes the `$match` temp that the plan-86 E borrow keys on

Last updated: 2026-09-28
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Other (performance regression; no wrong output observed)

Status: Open
Regression Test: tests/runtime/rt_list_element_record_borrow.rs (new case `a_binding_read_only_through_a_match_of_a_union_element_is_bounded`), plus a unit test in `src/codegen/engine/analysis/borrow_get.rs`

Take `LET e AS SomeUnion = collections::get(xs, i)` where `e` is read only as a `MATCH`
scrutinee and through that match's variant extracts. By `mfb spec memory collections`
§*`get`*, that binding is a **borrow**: no copy, no scope-drop free. At -O0 it is. At -O1
and -O2, since plan-149-A2 (a082faf5a, 2026-09-27), every such `get` allocates, copies
and frees the element. At -O3 the same thing already happened before that commit, through
the Level-3 aggregate-copy row.

The cost scales with the element's size, and every freed byte is PRNG-filled
(`mfb spec memory arenas` §*Entropy Fill*). The hottest known victim is the regex matcher:
`__regex_simpleMatchAt` does this `get` + `MATCH` on every node visit. There, a
one-class scan over a 1 350-scalar subject costs 2 822 µs at -O1 against 629 µs at -O0
for a 15-item class. That regex cost is tracked in bug-773.

**The single correct behavior a fix produces:** at every optimization level, the
`get` + `MATCH` borrow classification is the same as at -O0. The R1 program below makes
**6** allocations at -O0, -O1, -O2 and -O3. Its output is unchanged, and it ends with
`live_bytes 0` and `double_free_skips 0`.

References:

- `mfb spec memory collections` §*`get`*. It says the copy is skipped for "a binding read
  only as a borrow … whose every read is a field access or a `MATCH` over it (with that
  match's variant extracts)", *pinned* when the container is an immutable local.
  `[[src/codegen/engine/analysis/borrow_get.rs:collect_borrow_gets]]`. This bug violates it
  at -O1+.
- `mfb spec language memory-semantics` §14.6 (a read is an owned value; skipping the copy
  when unobservable is the compiler's business).
- `src/optimizer/catalog.rs` row "Place copy forwarding" (Level 1: "same operations, less
  waste"). Here it adds waste.
- plan-86 E and bug-689 (`bugs/completed/bug-689-list-element-record-get-copies-whole-element.md`):
  where the borrow comes from. plan-149-A2: where the forwarding row comes from.
- bug-773, where this was found. It is split out because it is a general compiler
  regression, not a regex cost.

## Failing Reproduction

### R1 — allocation count (`/tmp/dbE-bm-exhaustive`)

This is the shape of the fixture's own `evalAt`
(`tests/rt-behavior/collections/get-borrow-match-rt/src/main.mfb`), in a loop:

```
IMPORT io
IMPORT collections
TYPE Num
  value AS Integer
END TYPE
TYPE Add
  left AS Integer
  right AS Integer
END TYPE
UNION Expr
  Num
  Add
END UNION
FUNC evalAt(nodes AS List OF Expr, i AS Integer) AS Integer
  LET e AS Expr = collections::get(nodes, i)
  MATCH e
    CASE Num(n)
      RETURN n.value
    CASE Add(a)
      RETURN a.left + a.right
  END MATCH
  RETURN 0
END FUNC
FUNC main() AS Nothing
  LET nodes AS List OF Expr = [Num[3], Add[1, 2]]
  MUT n AS Integer = 0
  FOR k = 1 TO 1000
    n = n + evalAt(nodes, k MOD 2)
  NEXT
  io::print(toString(n))
END FUNC
```

`mfb build -O <n> --debug <dir>`, then run it and read `arena.0.alloc_calls`:

| compiler | -O0 | -O1 | -O2 | -O3 |
| --- | --- | --- | --- | --- |
| HEAD (`target/release/mfb`, main @ 7e53c82b9) | 6 | **1006** | **1006** | **1006** |
| 99b11f282 (parent of a082faf5a) | 6 | 6 | 6 | **1006** |
| a082faf5a (plan-149-A2) | 6 | **1006** | **1006** | **1006** |
| a082faf5a + the one-term fix below (throwaway, `/tmp` worktree) | 6 | 6 | 6 | 6 |

- Observed: one extra allocation per call, which is an owning copy of the element, at -O1+.
- Expected: 6 at every level. Stdout is `3000` at every level and every compiler.

Contrast, fast today: a **record** element read only through fields keeps its borrow at
-O1 (`/tmp/dbE-bg-d`: 3 allocations for 1 000 calls), so only the `MATCH` path regressed.

The bisect compilers were built in `git worktree add --detach` checkouts under `/tmp`
(`/tmp/dbE-wt-parent`, `/tmp/dbE-wt-intro`) with `CARGO_TARGET_DIR=/tmp/dbE-target` and
`cargo build --release --bin mfb`.

### R2 — the NIR rewrite

`mfb build -O <n> --nir` emits the post-optimizer NIR. The only difference in `evalAt`
between -O0 and -O1 (HEAD) is:

```
-O0:  bind e = call collections.get(nodes, i)
      bind $match0 = local e
      match local $match0
        CASE Num: bind n = unionExtract(Num, local $match0)
        CASE Add: bind a = unionExtract(Add, local $match0)
-O1:  bind e = call collections.get(nodes, i)
      match local e
        CASE Num: bind n = unionExtract(Num, local e)
        CASE Add: bind a = unionExtract(Add, local e)
```

`mfb build -O 1 -v` on R1 reports `Place copy forwarding: 1`, and none of the other
NIR-stage rows that could rewrite a bind fire. At -O3, HEAD reports
`Place copy forwarding: 1`. At -O3, the parent compiler reports
`Object / aggregate copy propagation: 1` and no `$match0` in its NIR: the Level-3 row
`src/optimizer/opt1/aggcopy.rs` did the same rewrite before plan-149.

### R3 — the regex symptom (bug-773)

`findAll` over `strings::repeat("z", 1350)`, no match, min of 30:

| pattern | -O1 (HEAD) | -O0 (HEAD) | -O1 (a082faf5a + fix) |
| --- | --- | --- | --- |
| `[o]` | 435 µs | 491 µs | 302 µs |
| `[cdbfo]` | 1 007 µs | 575 µs | 363 µs |
| `[abcdefghijklmno]` | 2 822 µs | 629 µs | 414 µs |

## Root Cause

**Confirmed by a unit-test probe**, not only by reading the code. A throwaway `#[cfg(test)]`
module was appended to `borrow_get.rs` in the a082faf5a worktree. It lowers R1's shape at
`OptLevel(0)`, classifies `main` with `collect_borrow_gets`, applies
`crate::optimizer::opt1::placefwd::forward` at `OptLevel(1)`, and classifies again:

```
PROBE before: $match0 present=true names={"$match0", "e"} pinned={"e", "$match0"}
PROBE after placefwd::forward: $match0 present=false names={} pinned={}
PROBE nir_for_src at O1: $match0 present=false names={}
```

(`src/codegen/engine/analysis/borrow_get.rs` is byte-identical between a082faf5a and HEAD,
per `git diff a082faf5a HEAD --stat -- src/codegen/engine/analysis/borrow_get.rs`, so the
classification half holds at HEAD. HEAD's forwarding half is R2.)

The mechanism:

1. **The desugar.** The IR lowers `MATCH e` to `$matchN = e; MATCH $matchN`, and each case
   binds `x = UnionExtract(Local $matchN)`.
2. **The forward.** `src/optimizer/opt1/placefwd.rs:forward` (plan-149-A2) sees
   `LET $match0 = <place Local(e)>`. All reads of `$match0` fall in a window with no write
   to `e`, so it points them at `e` and deletes the bind. Its gates (not a param, not
   address-taken, not captured, no write in the window) all pass.
3. **The classification.** In `src/codegen/engine/analysis/borrow_get.rs:collect_borrow_gets`,
   a `get` binding is borrow-eligible only if
   `read_only = !assigned(e) && reads(e) >= 1 && reads(e) == members(e) + scrut(e) + temps.len()`.
   - Before forwarding, `e` has one read (the `$match0 = e` bind); that read is the match
     temp, so `1 == 0 + 0 + 1` holds.
   - After forwarding, `e` has 3 reads: the scrutinee and two `UnionExtract`s. The extracts
     are counted in `total_reads` and in `union_extract_reads`, but `read_only` does not add
     `union_extract_reads` for the binding itself; only `is_match_temp` allows extracts, and
     only for a *temp*. So `3 == 0 + 1 + 0` fails.
   - `e` is then neither pinned nor windowed. `src/codegen/memory/owned.rs:materialize_owned_element`
     performs the owning copy and the Bind arm registers a scope-drop free.
4. **Proof of the term.** Adding `+ extract(e)` when `scrut(e) >= 1` to that sum, and
   nothing else, makes the probe report `after placefwd::forward: names={"e"} pinned={"e"}`.
   R1 then drops to 6 allocations at every level (table above).

What remains unconfirmed:

- The -O3 variant's origin commit was not bisected. `aggcopy.rs` was added in 9f2e43f22
  (2026-08-30, "optimizer: land the Level-3 rows"). That it introduced the -O3 copy is a
  guess.
- Whether the one-term fix is sound for every shape still needs the Phase 1 tests.
  Observed so far on the patched a082faf5a compiler (build `-O1`/`-O3` `--debug`, run),
  the `get-borrow-match-rt` fixture:
  - prints stdout identical to the HEAD compiler (`v0=7 … stress=210000`);
  - ends with `live_bytes 0` and `double_free_skips 0`.

Why the contrast is immune: a record binding read through fields is `members(e)` reads,
which `read_only` already counts, and there is no `$match` temp to forward.

## Goal

- R1: `alloc_calls` = 6 at -O0/-O1/-O2/-O3. Stdout is `3000`, `live_bytes 0`, and
  `double_free_skips 0`.
- A unit test pins it: after `placefwd::forward`, `collect_borrow_gets` still names the
  directly-`MATCH`ed `get` binding as pinned.
- Every existing negative case still copies: `get-borrow-match-rt`'s "escape" and
  "reassign" cases, and the `rt_list_element_record_borrow.rs` `a_*_is_a_copy` cases.

### Non-goals (must NOT change)

- **Owner constraint.** Nothing observable changes: no change to the language
  specification or the memory model.
  - Value semantics hold: `get`/`LET`/argument/return still behave as a copy. The borrow
    applies only where §*`get`* already says no program can tell the difference.
  - `collections::*` results, errors and codes are unchanged. There is no new syntax,
    builtin or signature.
  - The ownership/drop contract (`mfb spec memory arenas` §*Scope-Drop Frees*), the
    in-place self-update contract, and RES semantics are untouched.

  This fix makes the implementation match the spec text (§*`get`*) at every optimization
  level; the spec needs no edit.
- **No widening of the borrow.** A binding whose extract escapes (is stored, returned, or
  outlives a write to the container) must still copy. The new allowance counts only
  `UnionExtract` reads whose result binding is the match's own read-only variant binding,
  exactly what `is_match_temp` already allows for the temp.
- **Forbidden wrong fixes:**
  - disabling or demoting `Place copy forwarding` (or `aggcopy`): both are correct rows,
    and the analysis must understand their output;
  - special-casing `$match*` names in the forwarder: that leaves a user-written
    `LET m = e` + `MATCH m` exposed and keeps -O3 broken through aggcopy;
  - rewriting regex (or any builtin) source to dodge the shape;
  - weakening `get-borrow-match-rt` or any `_is_a_copy` test.

## Blast Radius

Searches:

- `grep -rn "is_match_temp\|match_temp\|\$match" src/codegen src/optimizer` (outside
  builtins) finds only `borrow_get.rs` and `last_use.rs`;
- a Python scan of `src/codegen/builtins/**` for `LET x AS T = collections::get[Or](…)`
  followed by `MATCH x`;
- `src/optimizer/catalog.rs` for rows that delete binds.

**Analyses keyed on the removed temp:**

- `src/codegen/engine/analysis/borrow_get.rs:collect_borrow_gets` `read_only` — **the bug;
  fixed here**.
- `borrow_get.rs` match-temp propagation (the loop "A match temp `$matchN = Local(e)` that
  copies a get-borrow `e` also borrows"): after forwarding there is no temp, so there is
  nothing to propagate. **Unaffected** once `e` itself is classified.
- `src/codegen/engine/analysis/last_use.rs` "MATCH views" (a view is a local bound from
  `x`/`x.f` read only as scrutinee/`UnionExtract`, and an owning view lets case aliases move
  fields) — **latent, same hazard, not observed**. After forwarding, the scrutinee is the
  source local itself. If that local is bound from a call it is not a view, so its aliases
  are never move sites.
  - Probe `/tmp/dbE-lu` (`LET e AS U = mk(k)`, `MATCH e`, `CASE A(a)`, `RETURN a.items`)
    does **not** regress: 5001 allocations at -O1 on the parent compiler, 4001 at
    a082faf5a and at HEAD.
  - The Phase 1 audit must add a `last_use` probe for a case where a field moves out of an
    alias (`stack = c.nxt` style) at -O0 vs -O1, and either clear this site or fix it here.
- `src/codegen/engine/analysis/handover.rs` and `module_analysis.rs` treat `NirOp::Match`
  generically and do not key on a temp name. **Unaffected** by grep; the audit should
  confirm.

**Rows that delete such binds:**

- `src/optimizer/opt1/placefwd.rs` (Level 1) — the -O1/-O2 cause. **Left as is**; the fix
  is in the analysis.
- `src/optimizer/opt1/aggcopy.rs` (Level 3) — the pre-existing -O3 cause. **Fixed by the
  same analysis change** (R1: 6 at -O3 with the fix).

**Affected program sites (`get` + `MATCH` of a union element):**

- `regex/helper_simple_match_at.rs:__regex_simpleMatchAt` (`leaf`, per node visit) —
  **fixed here**. The largest known cost, per bug-773 R4.
- `regex/helper_run.rs:__regex_run` (`anchorLeaf`, per anchor visit) — **fixed here**.
- `regex/helper_flatten.rs:__regex_flatten` (`firstLeaf`, once per compile) —
  **fixed here**; negligible cost.
- `regex/helper_flatten.rs:__regex_flatten` (`node = get(work, last)`, then `removeAt`) —
  **unaffected**: the container is written in the window, so it was never a borrow.
- `canvas/helper_render.rs` (`LET item AS DrawItem = collections::getOr(…)`, then `MATCH item`)
  — **unaffected**: `item` is passed whole to `__canvas_appendDraw`, so it was never a
  borrow.
- User programs that `LET x = collections::get(…)` a union and only `MATCH` it (e.g. an
  expression-tree evaluator) — **fixed here**.

**Test coverage gap:** `tests/rt-behavior/collections/get-borrow-match-rt` pins stdout
only, not the allocation count, and `rt_list_element_record_borrow.rs` has no union-MATCH
`bounded` case. That is why no gate caught the regression.

**Side note (not a live bug):** at a082faf5a, `last_use::tests::collect_last_use_moves_follows_the_hand_derived_table`
and `…finds_the_five_decoder_append_sites` fail (`cargo test --bin mfb analysis::` in the
worktree, with or without the fix). 86677797e ("last_use tests: probe shapes at -O0")
fixed them at HEAD.

## Fix Design

In `borrow_get.rs:collect_borrow_gets`, count a binding's own `UnionExtract` reads as
borrow-transparent when that binding is itself a `MATCH` scrutinee. That mirrors the
allowance `is_match_temp` gives the temp:

```rust
let read_only = !assigned(e) && reads(e) >= 1
    && reads(e) == members(e) + scrut(e) + (if scrut(e) >= 1 { extract(e) } else { 0 }) + temps.len();
```

The one-term version above was validated on the a082faf5a worktree (R1 all levels = 6; the
fixture output matches HEAD; `live_bytes 0`). The implementer must still prove what
`is_match_temp` relies on, for the direct form:

- the extract bindings are the match's own variant bindings (`Bind x = UnionExtract(Local e)`),
  not an arbitrary `UnionExtract(e)` elsewhere (e.g. inside a `RETURN`), which would be an
  escape. If the collector cannot tell them apart, count only `UnionExtract`s that are the
  direct value of a `Bind` in a case body of a `MATCH` whose scrutinee is `Local(e)`;
- an extract binding that escapes whole is still covered by the existing Bind-arm gate
  (`materialize_owned_element` copies the extract), as it is today for the temp. Confirm
  with the Phase 1 negative test.

This stays within `mfb spec memory collections` §*`get`* (pinned/windowed borrow, "a
`MATCH` over it (with that match's variant extracts)") and §14.6. No allocator,
layout or drop-contract change.

Rejected alternatives:

- Stopping `placefwd` (and `aggcopy`) from forwarding `$match` temps. It works for -O1, but
  it is special-cased by name, leaves user-written temps exposed, and gives up the row's
  genuine win: `/tmp/dbE-lu` saves 1 allocation per call with forwarding.
- Re-running `collect_borrow_gets` before the optimizer. The classification must describe
  the NIR that codegen actually lowers.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] `tests/runtime/rt_list_element_record_borrow.rs`: add
      `a_binding_read_only_through_a_match_of_a_union_element_is_bounded`, modeled on
      `a_binding_read_only_through_fields_of_an_immutable_list_is_bounded`, using `bounded()`
      (REPS vs 2×REPS allocation counts equal) with R1's `evalAt` body. The default
      `mfb build` is -O1, so it is RED today. Add an `-O3` variant if `run()` can pass `-O`.
- [ ] Add a negative case: `a_match_extract_that_escapes_is_a_copy`, where a variant binding
      is appended to another list and then the container is modified. Its output must equal
      the -O0 output. GREEN today; it must stay green.
- [ ] Unit test in `borrow_get.rs` `mod tests`:
      `a_directly_matched_binding_after_place_forwarding_is_pinned`. Lower at `OptLevel(0)`,
      apply `placefwd::forward` under `with_opt_level(OptLevel(1), …)`, and assert
      `names`/`pinned` contain `e`. RED today; the probe above is the template.
- [ ] Audit: a `last_use` MATCH-view probe (field moved out of an alias) at -O0 vs -O1, and
      the `handover.rs`/`module_analysis.rs` confirmation; write verdicts into Blast Radius.

Acceptance: the new bounded test and unit test fail for the documented reason; the negative
test passes; the audit is complete.
Commit: —

### Phase 2 — the fix

- [ ] `borrow_get.rs:collect_borrow_gets`: the direct-scrutinee extract allowance (Fix Design),
      restricted to the match's own variant bindings.
- [ ] Fix the `last_use.rs` views the same way if the audit finds a regression (else record
      the "clear" verdict).
- [ ] Update the module comment in `borrow_get.rs` to describe the forwarded form.

Acceptance: the Phase 1 tests pass; R1 = 6 at every level; the `get-borrow-match-rt` stdout
is unchanged; all `rt_list_element_record_borrow.rs` and `rt_recursive_value_*` tests pass.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Run the full suite first. `.ncode`/NIR goldens of functions with `get` + `MATCH` (the
      regex helper bodies, any fixture that has the shape) lose one alloc/copy/free
      sequence. Inspect one fixture per class, and accept only that delta.
- [ ] Re-run bug-773's R3/R4 probes and record the numbers there.
- [ ] Benchmark: re-baseline only as its own commit after the suite is green.

Acceptance: full suite green; the golden deltas are exactly the removed copies.
Commit: —

## Validation Plan

- Regression tests:
  `tests/runtime/rt_list_element_record_borrow.rs::a_binding_read_only_through_a_match_of_a_union_element_is_bounded`
  (+ the negative case), and the `borrow_get.rs` unit test.
- Runtime proof: R1 at -O0..-O3 shows 6 allocations, stdout `3000`, `live_bytes 0`; the
  `get-borrow-match-rt` fixture's stdout is unchanged; R3 on the regex probe.
- Doc sync: none expected. `mfb spec memory collections` §*`get`* already states the
  behavior the fix restores. Update only the `borrow_get.rs` module docs, then
  `cargo test --bin mfb spec` if a citation moves.
- Full suite: `cargo test`.

## Open Decisions

- **Where to fix.** Recommended: the analysis (`borrow_get.rs`), not the forwarding rows.
  See Fix Design.
- **Whether to bisect the -O3 origin.** Recommended: no. The same fix covers it, and R1
  pins -O3.

## Summary

This is a one-term blind spot in `collect_borrow_gets`, exposed when plan-149's Level-1
forwarding row began deleting the `$match` temp at -O1 (aggcopy already did this at -O3).
It is proven by bisect (6 → 1006 allocations between 99b11f282 and a082faf5a) and by a
unit probe of the classifier before and after `placefwd::forward`.

The risk is in proving that the widened allowance only admits the match's own variant
bindings, and in the `last_use` views audit. Neither the language spec nor the memory
model changes: the fix makes -O1+ do what §*`get`* already promises.
