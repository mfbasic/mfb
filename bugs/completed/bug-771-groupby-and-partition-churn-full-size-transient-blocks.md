# bug-771: native `groupBy` and `partition` allocate, copy and scrub several full-size temporary blocks per call

Last updated: 2026-09-28
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (performance: O(n) transient bytes with a constant of 4–15×)

Status: Fixed
Regression Test: `tests/runtime/rt_hof_transient_bytes.rs`

The native lowerings of `collections::partition` and `collections::groupBy` return correct results. But each call
allocates, fills, copies and frees several temporary blocks, each as large as the input list or larger, and
builds only one result. Every freed byte is then scrubbed by the arena's always-on entropy fill (`mfb spec memory
arenas`, *Entropy Fill*). The cost is linear in n, but the constant is large:

| row (benchmark/baseline, `min` ms) | mfb -O1 | C -O0 | Python | grade |
|---|---:|---:|---:|---|
| `list (Dynamic) partition` | 12.264 | 0.474 | 8.104 | D 25.9× |
| `list (Record-Dynamic) partition` | 12.252 | 0.474 | 8.104 | D |
| `list (State-Dynamic) partition` | 12.255 | 0.474 | 8.104 | D |
| `list (Dynamic) groupby` | 0.252 | 0.013 | 0.130 | D 19.4× |
| `list (Record-Dynamic) groupby` | 0.252 | 0.013 | 0.130 | D |
| `list (State-Dynamic) groupby` | 0.260 | 0.013 | 0.130 | D |
| `pipeline groupagg` | 46.482 | 2.228 | 80.093 | D 20.9× |

(Numbers are from `benchmark/baseline/{mfb-O1,c-O0,python}.log`, and the grades from `./benchmark/rank.py`. The C and
Python peers are the `list (Dynamic)` ones, because the Record and State rows share them. `-O2`/`-O3` do not help: 11.903 / 11.997 ms
partition, 0.249 groupby and 45.844 / 45.774 groupagg in `mfb-O2.log` / `mfb-O3.log`.) The seven rows run the same two
lowerings. The Record/State variants only read the list out of a field first (`test_lrd_*` / `test_lsd_*` in
`benchmark/mfb/src/list.mfb`).

**The single correct behavior a fix produces:** each call allocates **one** result-sized block plus O(n × 8 bytes)
of projection/flag scratch, and **no** other temporary block of input size. Concretely, measured with the `--debug`
arena counters (`arena.0.alloc_bytes` per call, deterministic, see *Failing Reproduction*):

| call | today (measured) | target |
|---|---:|---:|
| `partition(List OF String n=1000)` | 191,890 B, 175,810 B scrubbed | ≤ 1.2 × the tight result (≤ 60,000 B) |
| `partition(List OF Integer n=1000)` | 32,209 B | ≤ 18,000 B |
| `groupBy(List OF String n=2000 → 4 keys)` | 739,229 B | ≤ 330,000 B |
| `groupBy(List OF Integer n=10000 → 7 keys)` | 1,240,568 B | ≤ 400,000 B |

In time, on the same input and callback: `partition` costs ≤ 1.3× `filter`, where `filter` keeps 990 of the same 1000 elements
(it is 2.3× today). `groupBy` costs ≤ 1.5× its own two projections, run alone as two `collections::transform` calls
(2.8× for String and 4.6× for Integer today). Both targets come from sibling operations that already exist, on the same
data. `filter` is the one-sided `partition`. `groupBy`'s documented contract is "project twice, then bucket". The bucketing
pass is one hash probe and one fixed-width or String copy per element, so it should cost less than the two
callback passes, as it does in C (`uniq_ll` sort) and Python (`dict.setdefault().append`).
For the rows that means an **expected** partition ≈ 6–7 ms, below the C band edge of 7.1 ms, and
groupagg ≈ 20–25 ms. These are guesses extrapolated from the sibling ratios. Only the byte and ratio targets are the
acceptance criteria.

The callback-failure leak and the skipped-`TRAP` defect found during this investigation are tracked separately in
`bugs/bug-777-a-failing-collection-callback-leaks-and-partition-skips-the-function-trap.md`.

## STATUS: FIXED (`d66a1d12c` partition, `c0f08ee26` groupBy, `c9ce929a0` valFn-failure leak)

Measured by `rt_hof_transient_bytes`: `--debug` `arena.0.alloc_bytes`, N vs 2N calls, less
a checksum-only control. The checksums are identical to the pre-fix compiler.

| call | before | after | target |
|---|---:|---:|---:|
| `partition(List OF String n=1000)` | 191,760 B | 45,024 B | ≤ 60,000 |
| `partition(List OF Integer n=1000)` | 32,192 B | 9,104 B | ≤ 18,000 |
| `groupBy(String 2000 → 4 keys)` | 738,864 B | 307,808 B | ≤ 330,000 |
| `groupBy(Integer 10000 → 7 keys)` | 1,237,840 B | 241,376 B | ≤ 400,000 |

Ratios from the agents' runs of the Failing Reproduction (host loaded, so ratios only):

- partition String ≈ 1.01× `filter` keep-990 (target ≤ 1.3×).
- groupBy String 1.41–1.46× and Integer 1.37–1.39× its two transforms (target ≤ 1.5×).

**Benchmark** (`python3 /tmp/b771-bench.py 10`: the benchmark with `main` reduced to these
rows, pre-fix vs fixed compiler, same host, back to back). Medians in ms; every checksum is
identical.

| row | before | after |
|---|---:|---:|
| `list (Dynamic) partition` | 12.658 | 5.631 |
| `list (Record-Dynamic) partition` | 12.556 | 4.650 |
| `list (State-Dynamic) partition` | 12.630 | 4.581 |
| `list (Dynamic) groupby` | 0.335 | 0.146 |
| `list (Record-Dynamic) groupby` | 0.277 | 0.128 |
| `list (State-Dynamic) groupby` | 0.260 | 0.131 |
| `pipeline groupagg` | 47.651 | 17.448 |

The Fixed rows improved too: partition 4.409/2.799/2.712 → 2.783/1.544/1.451, and
groupby 0.285/0.124/0.123 → 0.100/0.040/0.039. Gate: full suite 278 ok, artifact-gate
0 diffs, acceptance 1582 at -O1 and -O3.

**Deviations from the Fix Design.**

- **Partition predicate argument.** The in-place borrow is impossible for String: a packed
  String payload has no `[len]` header in the data region, and the `get` borrow rules
  exclude String for the same reason. A fresh block per element would have cost ~61 KB,
  over budget. Instead, one `[len][bytes][nul]` argument block, sized by a length
  pre-scan, is refilled for each element. This is sound because a callee never owns its
  argument (the old lowering freed each per-element block right after the call).
- **Partition flags.** 1 byte per element, not 8.
- **GroupBy map (Open Decision).** The recommended presized-`set` form measured
  396,880 B/call on the String case, over budget. So the buckets are laid straight into
  the map, at the offsets a `set` into an empty map would give. The hash index is left
  unbuilt (`bucketsReady` 0), as for a map literal. The probe checked the result map
  against the pre-fix compiler: iteration, `set`, `removeKey`, `hasKey`, `get`, 201
  further sets, and 5,000 distinct keys (to force table growth).

**Found and fixed here.** A failing `valFn` under a function-level `TRAP` leaked the
finished `keys` projection: 8,048 B per call over 1,000 Integers. The second transform's
failure exit freed only its own output. `groupBy` now captures that exit, frees `keys`,
and leaves through `emit_callback_failure_exit`. Test:
`rt_hof_transient_bytes::a_failing_value_callback_frees_the_key_projection`. It is RED
on the pre-fix compiler: 321,920 B live after 40 failing calls.

**Test corrected with proof.**
`codegen_string_return_freshness::group_by_owns_the_string_value_its_callback_returns`
expected two more String frees for `V = String`. One of them was the old bucket loop's
per-element materialisation, which no longer exists. The correction is `+2 → +1`, with
the 4-point proof in its commit.

References:

- `mfb spec memory collections` → *Capacity Headroom and Growth*: "Headroom is a property of a **mutable working
  buffer, never of a value** … Copying a collection value (pass-by-value, binding, **embedding**, thread transfer) is
  **shrink-to-fit**." `partition` embeds both reserved full-capacity lists into the `Partition` record (Root Cause A).
- `mfb spec memory arenas` → *`arena_free`* and *Entropy Fill*: every freed chunk is scrubbed past its 16-byte node
  with a PCG64 stream. The scrub stays (owner decision, `planning/todo.md` item 2, "Measurement only; the fill
  stays"). The measured suite-wide cost is a 29–30% geomean.
- `.ai/collections.md` → "Native String-HOF rewrites are marginal/capped (except groupBy)". This covers the earlier
  native `groupBy` (plan-64 D1), which removed the O(bucket²) `.mfb` get/set copy. This bug is the next constant factor
  in that lowering.
- `mfb man collections groupBy` / `partition` (the `DESC` consts in the two `func_*.rs` files): the documented
  callback order and bucket order that must not change.
- Found by: benchmark D-grade triage, cluster D (collection HOFs over String/Scalar data), 2026-09-28.

## Failing Reproduction

Build with the prebuilt compiler (`target/release/mfb`, main at `7e53c82b9`) in a scratch project:

```
mkdir -p /tmp/b771/src && cd /tmp/b771
printf '{"name":"b771","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}\n' > project.json
# src/main.mfb = the program below
/Users/justinzaun/Development/mfb/target/release/mfb build -O 1 -q . && ./build/b771.out
```

```
IMPORT io
IMPORT collections
IMPORT datetime

FUNC strIsShort(s AS String) AS Boolean
  RETURN len(s) <= 2
END FUNC
FUNC strIsLong(s AS String) AS Boolean
  RETURN len(s) > 2
END FUNC
FUNC isEvenN(n AS Integer) AS Boolean
  RETURN n - (n / 2) * 2 = 0
END FUNC
FUNC strLenKey(s AS String) AS Integer
  RETURN len(s)
END FUNC
FUNC strIdentity(s AS String) AS String
  RETURN s
END FUNC
FUNC buildStrRange(n AS Integer) AS List OF String
  MUT xs AS List OF String = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, "s" & toString(i))
  NEXT
  RETURN xs
END FUNC
FUNC buildRange(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC
FUNC report(name AS String, t0 AS Integer, t1 AS Integer, calls AS Integer, acc AS Integer) AS Nothing
  io::print(name & ": " & toString((t1 - t0) / calls) & " ns/call  (acc " & toString(acc) & ")")
END FUNC

FUNC main AS Integer
  LET s1k = buildStrRange(1000)
  LET i1k = buildRange(1000)
  LET s2k = buildStrRange(2000)
  LET i10k = buildRange(10000)
  MUT acc AS Integer = 0
  MUT t0 = datetime::monotonicNanos()
  FOR k = 1 TO 1000
    LET p AS Partition OF String = collections::partition(s1k, strIsShort)
    acc = acc + len(p.matched)
  NEXT
  report("partition  String  n=1000", t0, datetime::monotonicNanos(), 1000, acc)
  t0 = datetime::monotonicNanos()
  FOR k = 1 TO 1000
    LET m AS List OF String = collections::filter(s1k, strIsLong)
    acc = acc + len(m)
  NEXT
  report("filter     String  n=1000 keeps 990 (contrast)", t0, datetime::monotonicNanos(), 1000, acc)
  t0 = datetime::monotonicNanos()
  FOR k = 1 TO 1000
    LET p AS Partition OF Integer = collections::partition(i1k, isEvenN)
    acc = acc + len(p.matched)
  NEXT
  report("partition  Integer n=1000", t0, datetime::monotonicNanos(), 1000, acc)
  t0 = datetime::monotonicNanos()
  FOR k = 1 TO 1000
    acc = acc + len(collections::groupBy(s2k, strLenKey, strIdentity))
  NEXT
  report("groupBy    String  n=2000 -> 4 keys", t0, datetime::monotonicNanos(), 1000, acc)
  t0 = datetime::monotonicNanos()
  FOR k = 1 TO 1000
    acc = acc + len(collections::transform(s2k, strIdentity)) + len(collections::transform(s2k, strLenKey))
  NEXT
  report("2x transform String n=2000 (groupBy's projections, contrast)", t0, datetime::monotonicNanos(), 1000, acc)
  t0 = datetime::monotonicNanos()
  FOR k = 1 TO 100
    LET b AS Map OF Integer TO List OF Integer = collections::groupBy(i10k, LAMBDA(n AS Integer) -> n MOD 7, LAMBDA(n AS Integer) -> n)
    acc = acc + len(b)
  NEXT
  report("groupBy    Integer n=10000 -> 7 keys (groupagg shape)", t0, datetime::monotonicNanos(), 100, acc)
  t0 = datetime::monotonicNanos()
  FOR k = 1 TO 100
    acc = acc + len(collections::transform(i10k, LAMBDA(n AS Integer) -> n MOD 7)) + len(collections::transform(i10k, LAMBDA(n AS Integer) -> n))
  NEXT
  report("2x transform Integer n=10000 (groupBy's projections, contrast)", t0, datetime::monotonicNanos(), 100, acc)
  RETURN 0
END FUNC
```

Observed (macOS aarch64 M2 Max, 2026-09-28). The host was shared with other agents (load average 9.2 on 12 cores, `uptime`),
so absolute times are inflated and only the ratios matter. Two back-to-back runs:

```
partition  String  n=1000: 86092 / 93294 ns/call
filter     String  n=1000 keeps 990 (contrast): 38281 / 39668 ns/call
partition  Integer n=1000: 18138 / 18309 ns/call
groupBy    String  n=2000 -> 4 keys: 348426 / 376803 ns/call
2x transform String n=2000 (groupBy's projections, contrast): 126389 / 136748 ns/call
groupBy    Integer n=10000 -> 7 keys (groupagg shape): 719800 / 624890 ns/call
2x transform Integer n=10000 (groupBy's projections, contrast): 157450 / 153890 ns/call
```

An earlier run on a quieter host gave the same ratios: partition String 62,291, filter keep-990 27,037, partition
Integer 13,642, groupBy String 289,236, 2× transform String 99,457, groupBy Integer 10000→7 460,330 ns/call.

- Observed: `partition` costs 2.3× the `filter` that does the same predicate calls and keeps ~the same elements.
  `groupBy` costs 2.8× (String) and 4.1–4.6× (Integer) its own two projections.
- Expected: ≤ 1.3× and ≤ 1.5× respectively (see the Goal).

**Deterministic counters.** I reduced the same program to a single loop per operation (1000 calls; groupBy 10000→7 at 100
calls), built it with `--debug`, and divided `arena.0.*` by the call count:

| call | `alloc_calls` | `alloc_bytes` | `fill_free_calls` | `fill_free_bytes` |
|---|---:|---:|---:|---:|
| partition String n=1000 | 1,005 | 191,890 | 3 | 175,810 |
| filter String n=1000, keeps 990 (contrast) | 1,003 | 60,066 | 1 | 44,018 |
| partition Integer n=1000 | 3 | 32,209 | 3 | 32,160 |
| groupBy String n=2000 → 4 keys | 8,047 | 739,229 | 43 | 610,477 |
| groupBy Integer n=2000 → 100 keys (`bucketKey`/`identity`) | 615 | 343,344 | 615 | 333,503 |
| groupBy Integer n=10000 → 7 keys (lambdas) | 95 | 1,240,568 | 95 | 1,239,044 |

The input sizes set the scale. A 1000-element `List OF String` of `"s0".."s999"` is `40 + 1000×40 + 3,890` = 43,930 B
(`COLLECTION_HEADER_SIZE` = `COLLECTION_ENTRY_SIZE` = 40 in `src/codegen/error/constants/error_constants.rs`). A
1000-element `List OF Integer` is 8,040 B, and a 10000-element one is 80,040 B.

**`pipeline groupagg` is groupBy.** I timed the row's four stages separately (probe `/tmp/dbD-gagg`, 100 reps, µs per rep):
full chain 534, `groupBy` alone 483, `mapValues(reduce)` 28, `reduce` over 10,000 elements 21. So ~90% of the row is the
groupBy call measured above (10000 → 7 keys, `#collections_groupBy$Integer$Integer$Integer`, native path).

**Where the time goes.** I ran `sample <pid> 3` on 10⁶-call versions, then attributed the top-of-stack addresses to functions by
disassembly (`otool -tv`, helper script `/tmp/dbD-prof.py`). The function holding the PCG64 multiplier
`0x…4385DF649FCCF645` and a store loop is `arena_fill_random`. Its share of listed samples: **partition String 66.4%,
groupBy String 59.2%, groupBy Integer (n=2000 → 100) 71.5%.** Dividing the scrubbed bytes by the time spent scrubbing gives
≈ 0.26 ns per scrubbed byte (derived from one probe: groupBy Integer n=2000 → 100 at 121.7 µs/call × 71.5% scrub share / 333,503 scrubbed B/call). So on this code, eliminating a
temporary byte saves its allocation, its copy and its scrub.

## Root Cause

Three separate mechanisms. None is in the allocator. The allocator does what its spec says, and the lowerings hand it far more
bytes than the operations need.

### A. `partition` builds two full-size lists, then copies both, headroom included, into the record

`src/codegen/builtins/collections/func_partition.rs:CodeBuilder::lower_collection_partition_call`:

1. It reserves **both** outputs at the full source size:
   `lower_reserved_list(&collection.type_, collection_slot)` twice
   (`src/codegen/builtins/collections/gen_mutate.rs:CodeBuilder::lower_reserved_list`: `capacity = count(source)`,
   `dataCapacity = dataLength(source)`). That is 2 × 43,930 B for the String case, although the two sides together hold only
   one source's worth.
2. For String elements, each item is materialized (`load_collection_loop_item`: one `arena_alloc` plus a byte copy), passed
   to the predicate, appended by copying its bytes (`lower_list_append_in_place`), and freed
   (`free_collection_loop_item`). That is the 1,000 extra `alloc_calls` per call. The chunks are 16 B, so they are not
   scrubbed.
3. It builds the result with `emit_build_inlined_record(&Partition$T, [matched_slot, unmatched_slot])`
   (`src/codegen/memory/marshal/record.rs:emit_build_inlined_record_sized`). This sizes each inlined field from
   its block, **capacity included**, and copies the whole block. The `Partition` record is therefore ≈ 87,900 B for
   ~44 KB of content: its `matched` list carries 1000 slots of capacity for 10 elements. That breaks the spec's
   "embedding is shrink-to-fit" rule quoted above, and every consumer that later frees the record scrubs 2× the
   bytes.
4. It frees the two reserved lists (`free_intermediate_collection` ×2).

Bytes per String call: 43,930 + 43,930 (reserve) + 87,9xx (record) + 1000 × 16 (items) = 191,890 measured. Scrubbed:
all three large blocks, 175,810. `filter` does step 1 once and skips steps 3–4, which is why it runs at 44,018 scrubbed
bytes and 0.43× the time.

### B. `groupBy` sizes its hash scratch by element count and copies every bucket twice

`src/codegen/builtins/collections/func_group_by.rs:CodeBuilder::lower_collection_group_by_call` (the plan-64 D1
lowering, reached through `group_by_fast_path` for Integer keys with Integer/Float/Fixed/Money/String T and V; the
`.ncode` of the 10000→7 probe contains its `gb_el_loop`/`gb_probe` labels and the target
`#collections_groupBy$Integer$Integer$Integer`):

1. The two projections, `lower_transform` for keys and values, allocate n-element lists. This is required by the
   documented contract and stays.
2. **Four scratch lists sized by n, not by the number of distinct keys**: `hashKeys`/`hashOcc` at
   `tableSize = smallest pow2 ≥ 2n` (32,768 slots × 8 B = 262,184 B **each** for n=10000, used by 7 keys), and
   `bucketPtrs`/`keyOrder` at n slots (80,040 B each). Then a zeroing loop runs over all `tableSize` `hashOcc` words.
   For n=10000 that is 684,448 B of the 1,240,568 B, **55% of all bytes, to track 7 keys.**
3. Each bucket starts as a one-element block. The String path uses `lower_empty_collection` plus an append, the fixed
   path a `HEADER+8` alloc. Each bucket then grows by `lower_list_append_in_place`'s geometric realloc, and every realloc frees and scrubs the old
   block: 1→2→4→…→2048 for a 1,429-element bucket. The n=2000 → 100-key probe shows it: 615
   scrubbed frees per call, ~6 per bucket.
4. The final map is built with `lower_map_set_in_place(result, key, bucket)` per bucket, starting from an **empty**
   map literal. Each `set` copies the bucket's bytes into the map's data region and regrows the map geometrically.
   Each bucket is then freed and scrubbed (`free_intermediate_collection`). Every value byte is therefore copied into a
   bucket, copied again at each bucket regrow, copied into the map, and copied again at each map regrow.
5. For String V, each `vals[i]` is materialized (`emit_load_collection_payload`), appended by copy, and freed. That is
   1 of the 4 per-element allocations. The other 3 are in the two String `transform`s: 6,007 allocs per 2000-element
   call for the transforms alone.

### C. Callback-failure leaks: moved to bug-777

The callback-failure defects found during this investigation are tracked in `bugs/bug-777-a-failing-collection-callback-leaks-and-partition-skips-the-function-trap.md`. They are: `partition` returns with a
bare `ret` that skips the enclosing function's `TRAP` and leaks 87,888 B plus the caller's locals per failing call, and
eight callback lowerings leak the failing String element. bug-777 owns the fix for every site. This bug's partition
rewrite must route its failure branch through the helper bug-777 fixes.

## Goal

- The byte targets in the table above hold (Phase 1 test), and so do the two time ratios against the sibling operations
  (Phase 3 probe).
- The `Partition` record is shrink-to-fit: its size equals the header plus two tight lists (`capacity == count`,
  `dataCapacity == dataLength`).

### Non-goals (must NOT change)

Owner constraint: the fix changes **no language specification and no memory-model rule**, only lowering, fast paths and
internal data layout invisible to programs.

- **Observable semantics are unchanged.** `partition` calls `predicate` exactly once per element, in index order,
  with no short-circuit. `groupBy` runs `keyFn` over the **whole** list, then `valFn` over the whole list, both up
  front before any bucket is written (`mfb man collections groupBy` says so explicitly). So **fusing the two
  projections into one per-element loop is forbidden**: it would change which error surfaces first and the order of
  callback side effects. Also unchanged: order within `matched`/`unmatched` and within each bucket, bucket insertion
  order (first appearance), the error codes (`ErrOutOfMemory` only; a callback's own error propagates unchanged), and
  value semantics (the result owns copies, and the input is not modified).
- **The memory model stays as it is** (`mfb spec memory`): arena allocator, drop contract, copy/mutate rules. The
  **entropy scrub stays on**, per `planning/todo.md` item 2. Making `arena_fill_random` faster, or skipping it, is out of
  scope here. The fix reduces the bytes handed to the allocator, which is what the memory spec already asks
  for (*Capacity Headroom and Growth*: embedding is shrink-to-fit, and known-size builders allocate exactly).
- Result types and names are unchanged: `Partition OF T` (mangled `Partition$T`) and `Map OF K TO List OF V`. No new
  builtin, signature or syntax.
- The eligibility gates of `group_by_fast_path` / `partition_fast_path` are not narrowed, and the `.mfb` bodies stay the
  fallback.
- **Tempting wrong fixes, forbidden:** editing the benchmark rows or their sizes, since the C/Python peers define the work.
  Making C "do more". Skipping the scrub for "collection blocks". Reusing the caller's input block as an output
  (that breaks value semantics). Returning a `Partition` whose lists alias the source.

## Blast Radius

`grep -rn "lower_reserved_list(\|emit_build_inlined_record(\|emit_callback_failure_exit(" src/codegen`, plus the hash sizing in
`func_group_by.rs`:

- `func_partition.rs:lower_collection_partition_call`: the double reserve plus the record copy with headroom (A).
  Its callback-failure defects (the skipped `TRAP`, the leak) are `bugs/bug-777-a-failing-collection-callback-leaks-and-partition-skips-the-function-trap.md`.
  **Fixed by this bug.**
- `func_group_by.rs:lower_collection_group_by_call`: the n-sized scratch, bucket growth, and map-copy (B). **Fixed by
  this bug.**
- `func_filter.rs:lower_filter`: `filter`'s output keeps its full-source reservation (`filter` keep-10 scrubs 44,018 B per
  call, the same as keep-990). The spec permits this ("`transform`/`filter` outputs aside"), so it is not a defect. It is
  latent, out of scope, and a candidate for a later shrink.
- `func_sort_by.rs` (6 `lower_reserved_list` sites) and `func_sort.rs:351`: each reserved block is used at its full count,
  since sort outputs are the same size as the input. Unaffected by A/B.
- Every callback-failure site (`emit_callback_failure_exit` callers): measured per site in bug-777's Blast Radius.
  They are out of scope here.
- `func_zip.rs:509` `emit_build_inlined_record(&record_type, [av_slot, bv_slot])` builds a `Pair` from two
  **materialized elements**, not reserved lists, so there is no headroom. Unaffected by A.
- Other `emit_build_inlined_record` callers (`construct_helpers.rs`, `builder_value_semantics.rs`, `builder_values.rs`,
  `astrings`, `crypto`, `vector`): they receive already-tight values (user constructor args, which are copies). Unaffected.
- `.ai/collections.md` "Big exception — groupBy" documents the plan-64 lowering. Update it with the new shape (Phase 3).

## Fix Design

**Partition (A).** Replace the one-pass "reserve both, append, re-copy" with two passes:

1. *Predicate pass.* Walk the source in index order and call `predicate` per element. Record each result in a private
   fixed-width flags scratch (a kind-2 `List OF Integer` or byte list of n), and accumulate `countMatched` and
   `dataMatched` (the String payload bytes). On failure, route through `emit_callback_failure_exit` with cleanup of
   the flags scratch and the current item, never a raw `ret` (the helper and its item cleanup are bug-777's fix).
2. *Build.* Allocate the `Partition$T` record **once**, at its exact shrink-to-fit size: `8×2` slots, plus the matched block
   (`HEADER + countMatched×stride + dataMatched`), plus the unmatched block (the complement). Write both headers
   tight.
3. *Copy pass.* Walk the source a second time and copy each element's entry/payload **directly from the source block** into its
   side, as the tight-copy path copies payload bytes. No materialize and no per-element free.

This lowers transient bytes to the flags scratch (8n B), and the result is 1× the content instead of 2×. For the predicate
argument, passing a **read-only borrow** of the source element would also remove the 1,000 per-call item allocations. That is
the same seam `collections::any` uses through `predicate(collections::get(value, i))` (13 ns/elem scan-and-call against 23 for
`filter`'s materialize-per-element, measured with the /tmp/dbD-part2 probe). It is optional, and allowed only where the
existing get-borrow rules (`.ai/collections.md`, "get read-only borrow") already permit a borrowed callback argument.

**GroupBy (B).** Keep the two projections exactly as today (Non-goals). Then:

1. *Distinct-key table.* Start the open-addressing table small (e.g. 16 slots) and **grow by rehash** at load > ½,
   so `hashKeys`/`hashOcc`/`keyOrder` are O(distinct keys). Keep one pass over `keys` that assigns each element its
   bucket id, overwriting the private `keys` scratch in place (it is dead after this pass; the key per bucket is
   kept in `keyOrder`). Count per-bucket elements, and for String V the per-bucket payload bytes read from the `vals` entries.
2. *Exact result.* Allocate the result map **presized**: `nb` entries, and data equal to the sum of the tight bucket blocks. Use the
   `copy_collection_with_capacity` idea `.ai/collections.md` already describes for bulk inserts, so no `set` regrows. Then write
   each bucket **directly into its map value slot** in first-appearance order. The better option is to allocate each bucket
   once at its exact count, fill it by walking `vals`, and `set` it into the presized map. The best is to lay the tight
   bucket sub-blocks straight into the map's data region and skip separate bucket blocks altogether. Either way, there is
   no geometric bucket growth and no map regrowth.
3. For String V, copy the payload bytes from the `vals` entry into the bucket, with no materialize/free round-trip.

This lowers transient bytes to `keys` + `vals` + an O(distinct) table (+ one exact bucket set). For 10000 → 7 that is
≈ 80 + 80 + ~0 + 80 KB, plus the result, which fits the ≤ 400,000 B target. The risk is in the map-construction step. The map hash index, the
tombstone/`capacity` invariants (`mfb spec memory collections` → *Map Hash Index*, *Header Fields*) and bucket
insertion order must match what `lower_map_set_in_place` produces today. The existing
`tests/rt-behavior/collections/groupby-string-value-native-rt` and the checksum cross-validation in
`benchmark/run.sh` guard the observable result.

Rejected alternatives: (1) a faster `arena_fill_random` (e.g. several interleaved PCG lanes). It is a real, separate
lever, but it changes the arena runtime helper the memory spec names, needs an owner decision, and is outside this
cluster. `bugs/bug-769-set-map-insert-pays-the-free-scrub-of-every-grow.md` (a parallel D-row doc) owns that lever. Any speed-up it lands lowers this bug's absolute times, but not its byte targets. It is recorded here only as evidence that each removed byte is worth ≈ 0.26 ns today. (2) Fusing the key and value
callbacks is forbidden by Non-goals. (3) Sorting keys, as the C peer does, would change bucket order and callback semantics.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] Add `tests/runtime/rt_hof_transient_bytes.rs`, modeled on
      `tests/runtime/rt_debug_arena.rs::fill_counters_match_known_allocations`: build each operation's single-loop program
      with `--debug` at N and 2N calls, and assert `delta(alloc_bytes)/N` against the target table (partition String
      ≤ 60,000, partition Integer ≤ 18,000, groupBy String 2000→4 ≤ 330,000, groupBy Integer 10000→7 ≤ 400,000).
      Also assert the program's printed checksum, so a broken result cannot pass. It must fail today (191,890 / 32,209 /
      739,229 / 1,240,568).

Acceptance: the test fails for the documented reason.
Commit: —

### Phase 2 — the fix

- [x] `func_partition.rs`: implement the two-pass, exact-size partition (Fix Design). Its predicate-failure branch must
      use `emit_callback_failure_exit` with the item cleanup that bug-777 adds. If bug-777 has not landed, land it first.
- [x] `func_group_by.rs`: implement the grown distinct-key table, the counting pass, exact buckets, and the presized map.

Acceptance: the Phase 1 tests pass; `hof-callback-failure-rt`, `partition-native-trap-rt`,
`groupby-string-value-native-rt`, `callback-string-return-identity-rt` and `bug752-res-element-callbacks-rt` still pass
unchanged; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] Any `.ncode`/codegen golden that captures these two lowerings shifts only inside `lower_collection_partition_call` /
      `lower_collection_group_by_call` / the failure exits. Localize any other diff to one fixture before accepting it
      (AGENTS.md).
- [x] Re-run the Failing Reproduction and check the two ratio targets (partition ≤ 1.3× filter keep-990; groupBy ≤ 1.5× two
      transforms).
- [x] `./benchmark/run.sh 10` (or `--run 10` of the list/pipeline groups). The seven rows improve, all checksums still cross-
      validate against C and Python, and no other row regresses beyond noise.
- [x] Update `.ai/collections.md` ("Big exception — groupBy") and the `mfb spec memory collections` text, if any
      describes the partition/groupBy build shape.
- [x] Full suite.

Acceptance: full suite green; golden deltas are only the intended lowering; the benchmark checksums are unchanged.
Commit: —

## Validation Plan

- Regression test: `tests/runtime/rt_hof_transient_bytes.rs` (byte budgets). The callback-failure tests belong to bug-777.
- Runtime proof: the Failing Reproduction ratios, plus the seven benchmark rows from `./benchmark/run.sh 10`, with
  `./benchmark/rank.py` showing each row's new grade.
- Doc sync: `.ai/collections.md` groupBy section. No `mfb man` change (the behavior is identical). Check
  `mfb spec memory collections` for any sentence about partition/groupBy construction (`scripts/spec-census.sh --citations`).
- Full suite: `cargo test` (the release gate in `.ai/compiler.md` → *Hard Completion Gate*), plus `artifact-gate`/`test-accept`
  per `.ai/testing-gates.md`.

## Open Decisions

- Should the predicate get a borrowed element in partition? Recommended: yes, where the get-borrow rules already allow it
  (as `any` does). The alternative is to keep materialize-per-element, which still meets the byte target, since 16 B chunks
  are not scrubbed.
- Should groupBy write buckets straight into the map data region, or set exact buckets into a presized map? Recommended: the
  presized-`set` form first (less layout risk). Take the direct layout only if the ratio target is still missed.

## Summary

Neither lowering does anything asymptotically wrong. Both hand the allocator 4–15× the bytes the operation needs, and the
always-on scrub makes each byte cost ≈ 0.26 ns on top of its allocation and copy. `partition` also embeds its working
headroom into the result value, contrary to the memory spec. The engineering risk is concentrated in groupBy's map construction,
which must reproduce today's map layout and bucket order exactly. The partition rewrite is contained.
