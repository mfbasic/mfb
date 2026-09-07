# bug-569: every HOF leaks the `String` block its callback returns — 64 B per element per call

Last updated: 2026-09-07
Effort: medium (the free is small; the completeness proof is the work)
Severity: **HIGH** (unbounded leak on the most ordinary `collections::transform` there is)
Class: Memory / callback ABI

Status: **FIXED** (2026-09-07, `073d43c23`)
Regression Test:
- `tests/runtime/rt_scope_drop_leaks.rs::transform_runs_at_constant_rss_with_a_string_callback`
  — 56.8 MB -> 112.6 MB at 50k/100k before; 1.0 -> 1.0 MB after.
- `…::sort_by_runs_at_constant_rss_with_a_string_key` — 27 -> 54 MB before.
- `…::group_by_runs_at_constant_rss_with_a_string_value` — 29 -> 59 MB before.
- `…::map_values_runs_at_constant_rss_with_a_string_callback` — 33 -> 66 MB before.
  A separate case per HOF, because each frees on its own path.
- `…::a_bare_parameter_callback_runs_at_constant_rss` — the POSITIVE pin: the one
  shape where the block the callback returns is the one the loop already freed.
- `…::a_fixed_width_callback_stays_flat` — the contrast that was always flat.
- `…::every_hof_frees_its_callback_result_exactly_once` — the VALUES, 25 runs of
  400 passes over all four HOFs plus a user-written HOF and six callback shapes.
- `tests/codegen/codegen_string_return_freshness.rs::transform_owns_the_string_block_its_callback_returns`
  and its `sort_by_` / `group_by_` / `map_values_` siblings — the owner count, per
  HOF, as a delta against the same HOF's fixed-width instantiation. Pre-fix all
  four deltas were 0.
- `…::a_callback_that_returns_its_own_parameter_is_freed_exactly_once` — the
  positive pin as a count.
- `…::an_indirect_callback_result_is_owned_regardless_of_a_shadowing_name`.
- `src/codegen/builtins/general/mod.rs::tests::every_builtin_that_can_be_a_callback_returns_boolean`
  — the guard the audit asked for.

## Relationship to bug-562

Found while fixing bug-562, and it is the *other half* of the same ABI.

bug-562 was the callee half: a `String`-returning callback could hand the
`FunctionRef` ABI a block the ABI did not own, and the ABI then freed it —
`[exit 139]`. That is fixed: the callee now copies on every return site whose
freshness lowering cannot prove.

**This is the caller half, and it is the exact mirror image: the ABI never frees
the block the callback returns at all.** So a correct, already-working callback
leaks one arena block per element per HOF call, and always has.

The two are independent — this one is measurable with a callback that predates
bug-562 entirely — but they touch the same code, so they should not be fixed in
the same change: bug-562 only ever ADDS a copy (it cannot double-free), while
this one ADDS a free (it can). See "Why this needs its own change" below.

## Failing reproduction

```
IMPORT io
IMPORT collections

FUNC deco(s AS String) AS String
  RETURN "<" & s & ">"
END FUNC

SUB main()
  LET xs AS List OF String = ["n0", "n1", "n2", "n3", "n4", "n5", "n6", "n7"]
  MUT i AS Integer = 0
  MUT acc AS Integer = 0
  WHILE i < 50000
    LET c AS List OF String = collections::transform(xs, deco)
    acc = acc + len(collections::get(c, 0))
    i = i + 1
  END WHILE
  io::print("acc=" & toString(acc))
END SUB
```

Peak RSS (`/usr/bin/time -l`), macos-aarch64, release:

| N (outer iterations) | peak RSS |
|---|---|
| 50 000 | 56.8 MB |
| 100 000 | 112.6 MB |

It doubles with the count: one leaked block per element per call. The same
program over a `List OF Integer` with `FUNC dbl(v AS Integer) AS Integer` is
**1.0 MB at both counts** — flat — which is what says the leak is the `String`
block and not the output list.

The callback shape does not matter. Measured at 50 000 / 100 000 outer
iterations over 8 elements:

| callback body | 50 000 | 100 000 |
|---|---|---|
| `RETURN s` | 25.6 MB | 50.2 MB |
| `RETURN toString(s)` (post-bug-562) | 25.6 MB | 50.2 MB |
| `RETURN "<" & s & ">"` | 56.8 MB | 112.6 MB |

(The concat row leaks twice per call: this bug's block, plus the dropped
interior temp — filed as bug-570 with this report, since folded into **bug-567**.)

## Which HOFs

Every callback-invoking lowering that receives a `String`-typed result:

| HOF | callback result | leaks? |
|---|---|---|
| `collections::transform` | `U` | **yes** for `U = String` |
| `collections::sortBy` | `U` (the key) | **yes** for `U = String` |
| `collections::mapValues` | `U` | **yes** for `U = String` |
| `collections::groupBy` | `K`, `V` | **yes** for `String` |
| `collections::reduce` / `reduceRight` | `Acc` | **no** — see below |
| `collections::filter` / `all` / `any` / `findIndex` / `findLastIndex` / `partition` | `Boolean` | n/a (no block) |
| `collections::forEach` | `Nothing` | n/a |
| `json::parse` (reviver) | `json::Json` | n/a (not a bare `String`) |
| `http` handler | `http::Response` | n/a |

`reduce` is the exception and the model to copy. `gen_memory.rs` already tracks
per-iteration ownership of the accumulator (`reduce_acc_owned`) and frees the
superseded block, guarded by runtime pointer-identity checks against the item and
the old accumulator. Nothing equivalent exists in `transform`, `sortBy`,
`mapValues`, or `groupBy`.

The stated contract is already on the books — `collect_function_ref_names`
(`src/codegen/memory/data/data_objects.rs`) says the callback ABI "takes
OWNERSHIP of the callback's return value — e.g. `collections::groupBy` stores each
per-element result into a bucket and frees the callee's returned block after use."
That sentence is aspirational: `groupBy` does not.

## Where

`src/codegen/builtins/collections/func_transform.rs:lower_transform`. After
`lower_list_append_in_place` has copied the result's payload bytes into the output
buffer, nothing frees the result block:

```
    builder.free_collection_loop_item(free_slot, &element_type)?;      // frees the ARGUMENT
    builder.lower_list_append_in_place(output_slot, item_slot, &output_list_type, &output_type)?;
    builder.advance_collection_loop(...);                              // <- item_slot never freed
```

The symmetric fix is one more `free_collection_loop_item(item_slot, &output_type)`
after the append, and the same shape in `func_sort_by.rs`, `func_map_values.rs`,
and `groupBy`'s `.mfb` body.

## Why this needs its own change

Adding the free is only sound if **every** callback hands the HOF a block it
owns. bug-562 established that for user / `.mfb`-bodied / lambda callbacks
returning `String` (`function_returns_fresh_string` now delivers a copy on every
unprovable return site). The remaining case to prove before landing the free:

* a builtin passed directly as a callback. Today the complete set is
  `isEven isOdd isPositive isNegative isZero isEmpty isNotEmpty isNumeric`
  (`builtin_function_id`), **all `Boolean`**, so none can hand back a block — but
  that is a fact about the current list, not an invariant, and the free would make
  it load-bearing. It wants a guard test.

Getting it wrong here is a double free, not a leak, and the arena turns a wrong
`arena_free` into "Allocation failed" at some *later*, unrelated allocation. Pair
the free with a positive pin per HOF.

---

## The fix, as landed

Two changes, because the four HOFs reach their callback by two different routes
and only one of them is the `FunctionRef` loop.

### 1. `lower_transform` frees the block it collected

`src/codegen/builtins/collections/func_transform.rs`. After
`lower_list_append_in_place` has copied the result's payload bytes into the
output buffer, the result block has no remaining reader, and the loop frees it —
`free_collection_loop_item(item_slot, &output_type)`, the exact mirror of the
`free_collection_loop_item(free_slot, &element_type)` three lines above it that
releases the ARGUMENT. A no-op for every non-`String` `U`.

**This one lowering is three of the four HOFs.** The audit's most useful finding
is that `transform` is not merely the first case, it is the shared one:

| HOF | how it reaches the callback |
|---|---|
| `transform` | `abi_inline` — the loop itself |
| `sortBy` | fast path *gather* mode calls `lower_transform` to build the keys; the `.mfb` body calls `collections::transform(value, keyFn)`. The fast path's own key loop only accepts `Integer`/`Fixed`/`Money` keys, so it can never see a `String` result. |
| `groupBy` | fast path calls `lower_transform` twice (keys, values); the `.mfb` body does the same in source |
| `mapValues` | **neither** — its `.mfb` body invokes `f(e.value)` directly. Its fast path requires `V == U` in `Integer`/`Float`/`Fixed`/`Money`, so it can never see a `String` result. |

### 2. A call through a callable VALUE is classified from the value, not the name

`src/codegen/engine/value/builder_values.rs`. `mapValues` is the one HOF whose
callback result is an ordinary statement-scope temp rather than a loop item, and
that temp was never registered, because a `Call` carries only its target's NAME:
`f(e.value)` is indistinguishable by shape from a direct call to a top-level `f`.
`call_returns_fresh_string` resolved it through `functions`, which for an indirect
call is wrong twice over:

* the usual answer is "not found", which keeps the plan-25 exemption and leaks;
* where a top-level function happens to share the callable parameter's name, the
  answer is a promise made by a function this call never reaches.

`callable_value_return_type` resolves the target as a `FUNC(..) AS U` local or
global FIRST, so the binding wins over the shadowed name exactly as it does at the
call itself. A `String` there is fresh — see the soundness argument below — and a
call through a callable value is never a param borrow, so
`call_returns_param_borrow` declines on the same test.

The shadowing half was live and measurable: the identical `mapValues` program
registered **1** owner with the callback named `deco` and **2** with it named `f`
(the `.mfb` body's own parameter name). It is 2 either way now.

## Soundness: why adding a free here cannot double-free

Every other leak in this cluster was fixed by adding a check or a copy. This one
adds a FREE, so the argument has to be that **every** callback hands the HOF a
block it owns. That rests on bug-562's completeness, so the enumeration was
re-verified rather than inherited.

### What a `String`-returning callback can BE

| callback source | in `module.functions`? | in `callback_referenced`? | `function_returns_fresh_string`? |
|---|---|---|---|
| a named user `FUNC` | yes | yes (a `NirValue::FunctionRef`) | yes — `param_borrow` excludes the callback set, so the fresh predicate admits it, and the callee copies |
| a `.mfb`-package function | yes (merged before `collect_function_ref_names` runs) | yes | yes |
| a capture-less `LAMBDA` | yes (`context.lambdas`) | yes — lowered as `FunctionRef` | yes |
| a capturing `LAMBDA` | yes (`context.lambdas`) | **no** — `collect_function_ref_names` collects `FunctionRef` only, not `Closure` | yes, and it does not need the set: a lambda body is a single expression, so a body that IS a bare parameter has no free variables and is therefore lowered as a `FunctionRef`, not a `Closure`. A param-borrow `Closure` cannot be written. |
| a builtin passed directly | **no** | n/a | n/a — the complete set is `isEven isOdd isPositive isNegative isZero isEmpty isNotEmpty isNumeric`, all `Boolean`, so none can hand back a block |

The builtin row was the one the bug report asked for a guard on, because it is a
fact about a list rather than an invariant, and the free makes it load-bearing. It
is an invariant now:
`every_builtin_that_can_be_a_callback_returns_boolean` asserts both that the
admitted set is still exactly eight names and that every one of them resolves to
`Boolean` for every argument type it accepts.

`callback_referenced_functions` is computed **once for the whole module, after
package merge** (`builder/mod.rs`, `collect_function_ref_names(module)`), so an
imported package's exported function used as a callback in the importing module is
in the same set as a local one.

### The free does not rely on that enumeration anyway

`reduce` is the model the bug report pointed at, and what it actually does is
guard: `gen_memory.rs` compares the reducer's result pointer against both the loop
item and the superseded accumulator before freeing either. `transform` now does
the same — a runtime pointer-identity check against the item it materialised,
emitted only when the element type is `String` (for a fixed-width element the item
register holds a scalar and cannot compare equal to an arena pointer).

So the one alias that has ever existed here — a callback that returns its own bare
parameter, i.e. the very block `free_collection_loop_item` released on the way in —
leaves exactly one free even if plan-86 K1's forced copy were ever to regress. The
guard is four instructions per element and it is what makes the change
independent of a predicate maintained elsewhere.

### Where it is NOT sound, and therefore not done

Nothing. Every callback-invoking lowering was enumerated by its
`emit_direct_callable_branch` site:

| site | callback result | action |
|---|---|---|
| `func_transform.rs` | `U` | **freed** (this fix) |
| `func_sort_by.rs` fast path | key, `Integer`/`Fixed`/`Money` only | no block |
| `func_map_values.rs` fast path | `U == V`, fixed-width only | no block |
| `gen_memory.rs` (`reduce`/`reduceRight`) | `Acc` | already freed, with its own identity guards |
| `func_filter.rs`, `func_partition.rs`, `func_find_last_index.rs` | `Boolean` | no block |
| `func_for_each.rs` | `Nothing` | no block |
| `func_all/any/find_index` | `Boolean`, via `.mfb` | no block |
| `json::parse` reviver | `json::Json` | not a bare `String`; untouched |
| `http` route handler | `http::Response` | untouched |

## Measured

`/usr/bin/time -l`, macos-aarch64, release, peak RSS at N and 2N outer passes over
an 8-element source. The callback is `RETURN toString(s)` — the identity — rather
than a concat, because a concat return leaks a SECOND block (bug-567, untouched)
and would read as a leak at both counts.

| probe | 50 000 | 100 000 | after (50k / 100k) |
|---|---|---|---|
| `transform`, `RETURN "<" & s & ">"` (the report's own repro) | 56.8 MB | 112.6 MB | 29 / 58 MB — **bug-567's remaining half** |
| `transform`, `RETURN toString(s)` | 25.6 MB | 50.2 MB | **1.0 / 1.0 MB** |
| `transform`, `RETURN s` | 25.6 MB | 50.2 MB | **1.0 / 1.0 MB** |
| `sortBy`, `String` key | 27 MB | 54 MB | **1.0 / 1.0 MB** |
| `groupBy`, `String` value | 29 MB | 59 MB | **1.0 / 1.0 MB** |
| `mapValues`, `Map OF Integer TO Integer` -> `String` | 33 MB | 66 MB | **1.0 / 1.0 MB** |
| `transform`, `RETURN len(s)` (`Integer`) — contrast | 1.0 MB | 1.0 MB | 1.0 / 1.0 MB |

The `transform` concat row halving rather than flattening is the arithmetic the
report predicted: that shape leaked twice per element, this bug's block plus
bug-567's dropped interior temp (this report filed it as bug-570; it has since
been folded into bug-567, which carries the `clear_pending_temps_to` analysis).
One of the two is gone.

### What the `mapValues` number is NOT

`mapValues` over a `Map OF String TO String` still grows, and it is not this bug.
A bare `FOR EACH e IN <Map OF String TO String>` that only reads `e.key`/`e.value`
grows **50 -> 99 MB** at the same counts with no callback in the program at all —
the per-entry `String` materialisation the loop never frees. That is why the
regression case uses a `Map OF Integer TO Integer` source: with fixed-width keys
and values the callback's result is the only block either loop can own, which is
what makes the measurement an isolation rather than a total. Filed separately.

## §14 memory-semantics

The clause the fix realizes is **§14.3 Function calls and returns** — "Returning a
value moves it into the caller's return slot" — read under §14's opening
invariant that "each live value is owned by exactly one binding, container slot,
**temporary**, closure environment, thread message, or return slot", and "values
are reclaimed by deterministic drop at the end of the owning scope".

bug-562 made the callee's side of that true: a `String` callback now moves a
solely-owned block into the return slot. This fix supplies the other end. The
HOF's per-iteration temporary is that block's one owner, and its scope is the
iteration, so §14.7's "live bindings are dropped in reverse declaration order
within each scope" is the drop that was missing. **§14.6** says why the drop is
sound rather than merely required: "inserting into a container copies or moves the
inserted value into the container; it never stores a non-owning alias" — the
append copies the payload bytes, so the output list is an independent owner and
the callback's block is dead the moment the copy finishes.

The change therefore only ADDS a free the ABI's own contract already promised in
words. `collect_function_ref_names`'s doc comment
(`src/codegen/memory/data/data_objects.rs`) already stated that the callback ABI
"takes OWNERSHIP of the callback's return value — e.g. `collections::groupBy`
stores each per-element result into a bucket and frees the callee's returned block
after use." That sentence described no code. It does now.

Block identity is not observable from source (§14.3.1: "source code only observes
the value-model rules above: copies are independent"), so no program can tell the
difference except by not exhausting memory.

## Gates

Measured against a detached worktree at the same base sha (465b602a9, bug-562's
tip — this lands on top of it).

| gate | result |
|---|---|
| `scripts/artifact-gate.sh target/release/mfb all` | 1412 tests, 1578 builds, **1973 goldens checked, 0 diffs** |
| `scripts/test-accept.sh` | **1434 test(s) ran, passed** |
| `cargo test --release --no-fail-fast` | 183 binaries, **0 failed** (3879 unit passed / 0 failed; baseline 3878 / 0 — the +1 is the new builtin guard) |
| `rustup run 1.96.0 cargo fmt --all --check` | clean |
| `cargo check --all-targets` | 0 warnings |

The baseline tree at 465b602a9 was run to the same point and its unit binary read
3878 passed / 0 failed. Its integration sweep was stopped part-way: it stalled on
`rt_canvas_metal::a_frame_whose_polygons_together_overflow_the_edge_region_falls_back`,
whose spawned canvas child spins at 100% CPU under load — the fixed tree reached
the identical test and behaved the same way before finishing, so it is a slow
GPU-stress fixture on a contended machine, not a difference this change makes.

### Why the golden delta is zero

The `transform` free fires only where a callback returns a bare `String`, and the
callable-value classification only where a `String`-returning callable value is
INVOKED. **No committed fixture satisfies either**, which is both why 1973 goldens
are byte-identical and why a leak this ordinary survived this long: the tree's
`transform`/`sortBy`/`groupBy` fixtures map to fixed-width results, and its
`sortBy` `String` fixtures (`sortby-string-gather-rt`, `sort-string-gather-rt`)
sort `String` ITEMS by a fixed-width key, which is the gather path — the callback
result there is the `Integer` index, not a block.

The independent positive pin is sharper than the gate because it is a program that
DOES exercise the path: `a_callback_that_returns_its_own_parameter_is_freed_exactly_once`
and the four per-HOF deltas are all comparative counts on programs written for it,
and `a_fixed_width_callback_stays_flat` pins that the shapes that already worked
are untouched.

As an owner count, per HOF, on the `String` instantiation against the otherwise
identical fixed-width one:

| HOF | measured on | before (String / fixed) | after (String / fixed) |
|---|---|---|---|
| `transform` | `loop_item_free_size` slots in `_mfb_fn_main` | 1 / 1 | **2** / 1 |
| `sortBy` | `…sortBy$Integer$String` vs `$Integer$Float` | 0 / 0 | **1** / 0 |
| `groupBy` | `_mfb_fn_main` (fast path), `$…$String` vs `$…$Integer` | 3 / 2 | **4** / 2 |
| `mapValues` | `pending_temp` + `_mfb_rt_drop_owned_string` in `…mapValues$Integer$Integer$U` | 1 / 1 and 0 / 0 | **2** / 1 and **1** / 0 |

Every row was 0 before. Each is a separate test, because each HOF frees on its own
path and a pin on `transform` says nothing about `groupBy`.

## What this fix does NOT do

Three other leaks share these programs and none of them is this bug. Each was
isolated by a contrast that removes the callback entirely:

* **bug-567** — `RETURN <nested concat>` drops an interior pending temp unfreed
  (`clear_pending_temps_to`). This report filed it as bug-570; it was folded into
  bug-567, which had reproduced the same defect first. Visible above as the
  `transform` concat row halving (56.8/112.6 -> 29/58) rather than flattening,
  exactly as the report predicted.
* **bug-571** (filed with this change) — `FOR EACH` over a `List OF String` or a
  `Map OF String TO …` leaks the per-element `String` materialisation. 25 -> 50 MB
  for a loop whose body is `acc = acc + len(e)`, with no callback, no HOF and no
  `collections` import. It is why `mapValues` over a `Map OF String TO String`
  still grows and why this change's `mapValues` case uses a
  `Map OF Integer TO Integer` source.
* **bug-572** (filed with this change) — a capturing `LAMBDA` leaks its closure
  environment per call. 13 -> 25 MB with a `Boolean` predicate, which allocates no
  result block at all; dropping the capture makes it flat.
