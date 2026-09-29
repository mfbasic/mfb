# bug-777: a failing collection callback leaks the element it was handed, and a failing `partition` predicate skips the enclosing function's `TRAP` and leaks everything in flight

Last updated: 2026-09-28
Effort: medium (1h–2h)
Severity: HIGH
Class: Memory-safety (unbounded leak), plus Correctness (the error bypasses the function-level `TRAP`)

Status: Fixed
Regression Test: `tests/runtime/rt_callback_failure_frees.rs`; `partition` cases in
`tests/runtime/rt_callback_failure_reaches_trap.rs`

A user callback passed to a native collection lowering can fail (`FAIL`, or any raise inside it). The error must then
leave the lowering the way any failed call does: every block the lowering owns is freed, the enclosing function's own
live locals are freed, and control reaches that function's `TRAP` (`mfb spec language error-model`, function-level
`TRAP` "traps every error from the body"; `mfb spec memory fallible-call-abi`, *Leaving a function on an error*: the
function frees the failing statement's pending values). Two defects break this:

1. **`collections::partition`** (native fast path, `Integer`/`Float`/`Fixed`/`Money`/`String` elements) returns with
   a bare `ret` when no inline `TRAP` capture is active. The **enclosing function's `TRAP` never runs**: the program
   below dies with `Error: 7-705-0002` instead of printing `handler: three 4`. Both of partition's full-size working
   lists, the element copy, and **every owned local of the enclosing function** leak. Each call leaks 87,888 B for a
   1000-element `List OF String`, plus the caller's locals (+6,192 B for one 100-string list). A retry loop grows
   without bound.
2. **Seven other native callback loops** (copying `filter`, `transform`, `sortBy`, `forEach`, `reduce`, `reduceRight`,
   `findLastIndex`, and the in-place `x = filter(x, p)`) route the failure correctly, but they never free the
   **String element copy** they made to pass to the callback. That leaks the failing element's whole block, of any
   size: 16 B for `"s500"`, 64 B for a 45-byte element. This also applies to `groupBy`, whose projections run through
   `transform`. It happens once per failing call, so it is unbounded under retry.

Output is correct in every leak case, which makes the leaks silent.

**The single correct behavior a fix produces:** in every native collection lowering, after a callback failure,
`arena.0.live_bytes` at program exit is **0** for a program that retries the failing call N times, at every N, whether
the call is trapped by an inline `TRAP` or propagates to a function-level `TRAP` one or more frames up. And a failing
`partition` predicate runs the enclosing function's `TRAP`, exactly as `filter` does.

## STATUS: FIXED (merged from `worktree-B-777`)

`gen_flow.rs:emit_callback_failure_exit` now takes the member's owned blocks **and**
its loop item, freeing them in one register spill; every native callback site passes
its item, and `partition` routes both working lists and the item through it (its
private bare-`ret` arm is gone). Deviations from the design below, all measured with
the new test:

- **`reduce` also frees its accumulator.** The old comment held the in-flight item and
  accumulator "may still alias" the reducer's inputs; a *failed* reducer produced no
  result, the item is this iteration's fresh block, and the accumulator is exactly what
  the previous iteration committed with its ownership in `acc_owned_slot`. A
  `String`-accumulator case (not in the original matrix) leaked 150 blocks per 50
  failing calls before and 0 after.
- **The in-place `transform`/`sortBy` rows leaked under an inline `TRAP`** (the doc
  had measured them only under a function-level `TRAP`): a `TRAP`-wrapped assignment
  does not take the in-place arm, it lowers through the copying `transform`/`sortBy`,
  so those rows are fixed by the copying sites. `sortBy`'s String-element leak was
  likewise its key pass through the fixed path.
- **Found while writing the test: bug-780** (`bugs/completed/bug-780-*.md`) — every
  `FAIL` leaked the enclosing statements' pending temporaries, so every callback shaped
  `IF s = longKey() THEN FAIL …` leaked 64 B per failure at *every* site, including
  the guards. Fixed on the same branch; it also removed a real leak in the `audio`
  package's `#audio_mmlParse` error paths.

Commits: `21b26924e` (RED tests), `35df7364f` (bug-780), `d381eb637` (fix),
`a3d8c35f0` (73 `.ncodesum` goldens, localized to failure-path frees), `1d4ca7c8b` (fmt).

References:

- `tests/runtime/rt_callback_failure_reaches_trap.rs` module doc: the same bare-`ret` defect was fixed for
  `emit_callback_failure_exit` (found during plan-142-B). That test's `CASES` list names `filter`, `transform`,
  `forEach`, `reduce`, `sortBy`, `mapValues` and `findLastIndex`, **but not `partition`**, which keeps its own
  hand-written copy of the pre-fix code.
- `mfb spec memory fallible-call-abi` → *Leaving a function on an error*; `mfb spec language error-model` §8 (function-
  level `TRAP`); `mfb man collections partition` ("an error raised inside predicate … propagates out of the
  collections::partition call to the caller, abandoning the partially built result").
- Split out of `bugs/bug-771-groupby-and-partition-churn-full-size-transient-blocks.md` (found during the cluster-D
  benchmark triage, 2026-09-28).

## Failing Reproduction

### 1. `partition` skips the function-level `TRAP` (wrong control flow)

```
IMPORT collections
IMPORT io

FUNC failOnThree(n AS Integer) AS Boolean
  IF n = 3 THEN
    FAIL error(77050002, "three")
  END IF
  RETURN n > 0
END FUNC

FUNC run() AS Integer
  LET xs AS List OF Integer = [1, 2, 3, 4]
  LET r AS Partition OF Integer = collections::partition(xs, failOnThree)
  io::print("not reached")
  RETURN 0

  TRAP(e)
    io::print("handler: " & e.message & " " & toString(len(xs)))
    RETURN 1
  END TRAP
END FUNC

FUNC main() AS Integer
  io::print("run -> " & toString(run()))
  RETURN 0
END FUNC
```

Built with `target/release/mfb build -q` (main `7e53c82b9`, macOS aarch64):

- Observed: stderr `Error: 7-705-0002` / `three`, exit 255. Neither `handler:` nor `run -> ` is printed.
- Expected (and what the same program prints with `collections::filter(xs, failOnThree)` in place of `partition`):
  `handler: three 4` then `run -> 1`, exit 0.

### 2. The leak matrix

The harness generates one program per case (`/tmp/dbD-lk/gen.py`). A 1000-element `List OF String` `"s0".."s999"`
(plus a 1000-element `List OF Integer`, and a 1000-entry `Map OF Integer TO String`) is passed to `doOp`. `doOp` runs
one statement whose callback FAILs on element 500 (`"s500"`, or `500` for Integer). `tryOnce` calls `doOp` under a
function-level `TRAP`, and `main` calls `tryOnce` 200 times:

```
FUNC predFail(s AS String) AS Boolean
  IF s = "s500" THEN
    FAIL error(77050002, "boom")
  END IF
  RETURN FALSE
END FUNC
' keyFail / mapFail / eachFail / accFail / predFailT / ipredFail / imapFail: the same FAIL on element 500
FUNC doOp(xs AS List OF String, ixs AS List OF Integer, m AS Map OF Integer TO String) AS Integer
  ' variant "extra": LET extra AS List OF String = buildStrRange(100)  (and a later read of it)
  <statement>
  RETURN 0
END FUNC
FUNC tryOnce(xs AS List OF String, ixs AS List OF Integer, m AS Map OF Integer TO String) AS Integer
  RETURN doOp(xs, ixs, m)
  TRAP(err)
    RETURN -1
  END TRAP
END FUNC
FUNC main AS Integer
  ' sb = buildStrRange(1000), ib = buildRange(1000), mb = 1000-entry map
  MUT acc AS Integer = 0
  FOR k = 1 TO 200
    acc = acc + tryOnce(sb, ib, mb)
  NEXT
  io::print(toString(acc))
  RETURN 0
END FUNC
```

Each case is built `-O 1 --debug`, and I report `arena.0.live_bytes` at exit ÷ 200. Every run printed `-200`, so the
callback failed and was trapped every time.

| statement in `doOp` | lowering | leak / failing call | with an owned local `extra` |
|---|---|---:|---:|
| control: `IF predFail(collections::get(xs, 500)) …` | — | 0 | 0 |
| `LET p AS Partition OF String = collections::partition(xs, predFail)` | `func_partition.rs:lower_collection_partition_call` | **87,888** | **94,080** (+`extra`) |
| `LET p AS Partition OF Integer = collections::partition(ixs, ipredFail)` | same | **16,096** | **22,288** (+`extra`) |
| `collections::filter(xs, predFail)` | `func_filter.rs:lower_filter` | 16 | 16 |
| `collections::transform(xs, mapFail)` | `func_transform.rs:lower_transform` | 16 | 16 |
| `collections::groupBy(xs, keyFail, mapFail)` | `func_group_by.rs` → `lower_transform` | 16 | 16 |
| `collections::sortBy(xs, keyFail)` | `func_sort_by.rs:lower_collection_sortby_call` | 16 | 16 |
| `collections::reduce(xs, 0, accFail)` | `gen_memory.rs:lower_collection_reduce_impl` | 16 | 16 |
| `collections::reduceRight(xs, 0, accFail)` | same | 16 | 16 |
| `collections::forEach(xs, eachFail)` | `func_for_each.rs:lower_for_each` | 16 | 16 |
| `collections::findLastIndex(xs, predFail)` | `func_find_last_index.rs:lower_collection_find_last_index_call` | 16 | 16 |
| `MUT ys = xs` / `ys = collections::filter(ys, predFail)` | `builder_inplace_shrink.rs:try_inplace_filter_assign` | 16 | 16 |
| `collections::mapValues(m, mapFail)` | `func_map_values.rs:lower_collection_map_values_call` | 0 | 0 |
| `mm = collections::mapValues(mm, mapFail)` | `builder_inplace_setmap.rs:try_inplace_map_values_assign` | 0 | 0 |
| `ys = collections::transform(ys, mapFail)` | `builder_inplace_rewrite.rs:try_inplace_transform_assign` | 0 | 0 |
| `ys = collections::sortBy(ys, keyFail)` | `builder_inplace_sort.rs:try_inplace_sort_by_assign` | 0 | 0 |
| `collections::any(xs, predFail)` / `all(xs, predFailT)` / `findIndex(xs, predFail, 0)` | `.mfb` bodies (`predicate(collections::get(value, i))`) | 0 | 0 |
| Integer `filter` / `transform` / `sortBy` (`ixs`) | as above | 0 | 0 |

The 16 B is the failing element's own block. When element 500 is made 45 bytes long (`/tmp/dbD-lk/gen_long.py`), the
leak for `filter`, `forEach`, `findLastIndex` and `sortBy` becomes **64 B** per call (8 B length + 45 + NUL, rounded up
to the 16-byte granule), so it scales with the element.

I also checked the same failures under an **inline** `TRAP` in `doOp` (`/tmp/dbD-lk/gen_it.py`): `partition` String
16 B, `filter` 16 B, `forEach` 16 B, `partition` Integer 0. So the inline-capture arm of partition frees its two lists
but still leaks the element, and the 87,888 B / skipped-`TRAP` defect is specific to the no-capture arm.

## Root Cause

**A. partition's bespoke failure arm.** `src/codegen/builtins/collections/func_partition.rs:
CodeBuilder::lower_collection_partition_call` handles a failed predicate itself:

```rust
match self.raw_result_capture_label() {
    None => self.emit(abi::return_()),
    Some(capture) => { /* spill 4 result regs, drop matched + unmatched, reload, branch */ }
}
```

The `None` arm is the exact code that `rt_callback_failure_reaches_trap.rs` says was replaced in the shared helper
`src/codegen/builtins/collections/gen_flow.rs:CodeBuilder::emit_callback_failure_exit`. There the `None` arm is now
`self.emit_call_error_exit()?`, which runs the function-level `TRAP` and frees the scope. A bare `ret` from the middle of
the function body bypasses that exit entirely. It skips the `TRAP` handler, never drops the function's owned locals (the
`extra` column), never drops `matched`/`unmatched` (2 × 43,930 B reserved at the full source size: see bug-771 A), and
never frees the element copy. The error registers still hold the callee's error, so the *caller* sees a failed call: the
error surfaces one frame too high, which is why the leak repro's `tryOnce` still catches it.

**B. The element copy is not part of any failure cleanup.** For a `String` element, the callback loops obtain the argument
with `load_collection_loop_item` (a fresh arena block: length word + bytes + NUL), store it in `item_slot`, call the
callback, and free it after the success path with `free_collection_loop_item(item_slot, …)`. On the failure branch,
`emit_callback_failure_exit(cleanup)` takes only **one** optional block, and every caller passes its output list
(`filter`, `transform`, `sortBy`'s keys) or `None` (`forEach`, `reduce`, `findLastIndex`, in-place `filter`). No caller
passes `item_slot`, so the copy leaks. The unaffected sites confirm the mechanism:

- `try_inplace_transform_assign` calls `self.free_collection_loop_item(item_slot, &element_type)?` **before** its
  `emit_callback_failure_exit(None)`. This is the correct sibling pattern.
- `mapValues` (both forms) passes a pointer into the map's own payload (no copy).
- `any`/`all`/`findIndex` are `.mfb` bodies whose `predicate(collections::get(value, i))` argument is a read-only borrow.
- `Integer` elements are never materialized.

## Goal

- Every row of the leak matrix reads 0 in both columns, for inline and function-level `TRAP`s alike.
- The §1 program prints `handler: three 4` / `run -> 1` and exits 0 for `partition` with every fast-path element type.
- The callback's error (code, message, `ErrorLoc`) reaches the handler unchanged.

### Non-goals (must NOT change)

Owner constraint: no language-spec or memory-model change. The fix only makes these lowerings obey the scope-edge rule
that `mfb spec memory fallible-call-abi` already states.

- Error semantics are unchanged: which callback call fails first, the error value, no retry, and no partial result
  (`partition`/`filter`/… still abandon their result; the in-place forms still leave `x` untouched, as plan-142-A's
  failure-atomicity rule and `tests/runtime/rt_inplace_failure_atomic.rs` require).
- The success path is not changed: no extra instructions per element, and the success-path free stays where it is.
- Callback call order and count are not changed.
- **Tempting wrong fixes, forbidden:** borrowing the element for every callback just to dodge the free. That is a
  different change with its own aliasing rules (bug-771 discusses it for `partition` only). Also forbidden: making the
  test pass by catching the error with an inline `TRAP` in the fixture, or loosening `rt_callback_failure_reaches_trap`.

## Blast Radius

Every native collection lowering that invokes a user callback, found with
`grep -rn "emit_callback_failure_exit(\|raw_result_capture_label()" src/codegen` and
`grep -rn "emit_direct_callable_branch" src/codegen/builtins/collections src/codegen/collection`. Each was measured with
the harness above (String elements unless noted):

| site | verdict |
|---|---|
| `func_partition.rs:lower_collection_partition_call` (String, Integer) | **Fixed here**: A (skipped `TRAP`, 87,888 / 16,096 B plus caller locals) and B (16 B under inline `TRAP`) |
| `func_filter.rs:lower_filter` | **Fixed here**: B, 16 B |
| `func_transform.rs:lower_transform`, and `groupBy`, which calls it for both projections | **Fixed here**: B, 16 B |
| `func_sort_by.rs:lower_collection_sortby_call` | **Fixed here**: B, 16 B |
| `func_for_each.rs:lower_for_each` | **Fixed here**: B, 16 B |
| `gen_memory.rs:lower_collection_reduce_impl` (`reduce`, `reduceRight`) | **Fixed here**: B, 16 B each |
| `func_find_last_index.rs:lower_collection_find_last_index_call` | **Fixed here**: B, 16 B |
| `builder_inplace_shrink.rs:try_inplace_filter_assign` | **Fixed here**: B, 16 B |
| `builder_inplace_rewrite.rs:try_inplace_transform_assign` | unaffected, measured 0: frees the item before the exit (the pattern to copy) |
| `builder_inplace_sort.rs:try_inplace_sort_by_assign` | unaffected, measured 0 |
| `func_map_values.rs:lower_collection_map_values_call`, `builder_inplace_setmap.rs:try_inplace_map_values_assign` | unaffected, measured 0: the value is passed by pointer into the map |
| `any`, `all`, `findIndex` (`.mfb` bodies, borrowed `get` argument) | unaffected, measured 0 |
| every site above with `Integer` elements, except `partition` | unaffected, measured 0: nothing is materialized |
| `collection/map/map_bucket_remove.rs`, `collection/sort/sort_string_list.rs` `abi::return_()` | unaffected: these are returns of standalone runtime-helper functions with their own frames, not mid-body exits past a callback |

`collections::find` takes a value, not a predicate
(`func_find.rs` `expected_arguments`).

## Fix Design

1. Give `emit_callback_failure_exit` a way to free the loop item as well as the output block. For example, a
   `loop_item: Option<(usize, ParameterType)>` parameter that calls `free_collection_loop_item` inside the existing
   four-register spill/reload, since `arena_free` clobbers the result registers, as the helper's comment already notes.
   Pass `item_slot` at the eight **Fixed here** sites. A no-op for fixed-width elements keeps their code unchanged.
2. Delete partition's bespoke `match`. Drop `unmatched` with `emit_owned_value_drop` inside the same spill, then call
   `emit_callback_failure_exit(Some((matched_slot, …)), item)`. If bug-771's partition rewrite lands first, the
   predicate pass there owns only a flags scratch and the item, and it must use the same helper. Either order works, but
   this bug owns the fix at every site.
3. Add `partition` to `rt_callback_failure_reaches_trap.rs`'s `CASES`, so the shared-helper invariant covers every
   callback lowering.

Rejected: freeing the item *before* testing the result tag at every site, as `try_inplace_transform_assign` does. It is
correct, but it puts an extra free on the success path of the sites that free after the append, and those need the item
alive for the append. The failure-branch free keeps the hot path unchanged.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] Add `partition` (Integer, and a String variant) to `CASES` in `tests/runtime/rt_callback_failure_reaches_trap.rs`.
      It fails today (no `handler:` line).
- [x] Add `tests/runtime/rt_callback_failure_frees.rs`, modeled on
      `tests/runtime/rt_list_element_record_borrow.rs::bounded` / `assert_no_leak` (`alloc - free` and `live_bytes`
      from a `--debug` run at N and 2N). One case per **Fixed here** site, with a 45-byte failing element, both with an
      inline `TRAP` and under a caller's function-level `TRAP`, and with an owned local in the failing frame. Each case
      requires `live_bytes == 0`. It fails today at the measured sizes.
- [x] Keep the unaffected sites in the same test as guards (they pass today).

Acceptance: the new cases fail for exactly the documented reasons; the guards pass.
Commit: `21b26924e`

### Phase 2 — the fix

- [x] `gen_flow.rs:emit_callback_failure_exit` gains the item cleanup; the eight sites pass `item_slot`.
- [x] `func_partition.rs`: route through the shared helper (both lists and the item).

Acceptance: Phase 1 tests pass; `hof-callback-failure-rt`, `partition-native-trap-rt`, `rt_inplace_failure_atomic.rs`
and `rt_scope_drop_leaks.rs` pass unchanged.
Commit: `d381eb637` (and `35df7364f`, bug-780)

### Phase 3 — full validation

- [x] Codegen/`.ncode` goldens may shift only on the failure branches of the listed lowerings. Localize any other diff to
      one fixture before accepting it (AGENTS.md).
- [x] Re-run the leak matrix harness: all zeros. Re-run the §1 program.
- [x] Full suite.

Acceptance: full suite green; the harness and §1 behave as in the Goal.
Commit: `a3d8c35f0`, `1d4ca7c8b`

## Validation Plan

- Regression tests: `tests/runtime/rt_callback_failure_frees.rs`; the extended `rt_callback_failure_reaches_trap.rs`.
- Runtime proof: the leak-matrix harness (every row 0) and the §1 program's output.
- Doc sync: none expected. The spec already requires this behavior. `.ai/collections.md` could gain one line under the
  predicate-scan note: "free the materialized item on the callback-failure branch too".
- Full suite: `cargo test` per `.ai/compiler.md` *Hard Completion Gate*.

## Open Decisions

- Change the helper's signature, or add a second helper? Recommended: one helper with an `Option` item argument, so that
  no future callback site can forget it. The alternative is a wrapper that frees the item and then calls the existing
  helper.

## Summary

The risk is low and local: failure branches only, with a proven sibling pattern (`try_inplace_transform_assign`) and a
shared helper that already does the hard part (register spill plus the correct exit). The one behavior-visible defect,
a skipped function-level `TRAP` in `partition`, is the same bare-`ret` bug the shared helper already fixed once. It
survived because `partition` kept a private copy of the old code, and the test that pins the rule does not list it.
