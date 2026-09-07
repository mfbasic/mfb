# bug-569: every HOF leaks the `String` block its callback returns — 64 B per element per call

Last updated: 2026-09-07
Effort: medium (the free is small; the completeness proof is the work)
Severity: **HIGH** (unbounded leak on the most ordinary `collections::transform` there is)
Class: Memory / callback ABI

Status: Open
Regression Test: — (an RSS case in `tests/runtime/rt_scope_drop_leaks.rs`)

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

(The concat row leaks twice per call: this bug's block, plus bug-570's dropped
interior temp.)

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
