# bug-770: Integer-key `Map` set/lookup hashes the 8 key bytes one at a time, and a bucket read-modify-write allocates three blocks

Last updated: 2026-09-28
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (performance)

Status: Open
Regression Test: tests/runtime/rt_map_integer_key_probe_cost.rs (to add, Phase 1)

The benchmark's Integer-keyed `Map` rows run at 24–28× C `-O0` (grade D, all
flagged RED). mfb is 1.1–1.8× slower than Python on the same rows.

| row (`benchmark/baseline/*.log`, min ms) | mfb -O1 | -O2 | -O3 | C -O0 | C -O2 | Python | ×C-O0 |
|---|---|---|---|---|---|---|---|
| `map lookup` (20,000 `set` + 20,000 `get`) | 2.505 | 2.497 | 2.501 | 0.090 | 0.042 | 1.365 | 27.8 |
| `map intkey` (5,000 `set` + 5,000×(`hasKey`+`get`+`getOr` miss)) | 0.960 | 0.943 | 0.934 | 0.037 | 0.011 | 0.897 | 25.9 |
| `map listagg` (2,000×(`getOr`+`append`+`set`) over 64 keys) | 0.448 | 0.445 | 0.445 | 0.019 | 0.014 | 0.294 | 23.6 |

(Ratios are from `./benchmark/rank.py --csv`. -O1, -O2 and -O3 are identical,
so no optimizer pass touches this.)

**Most of the time is not in the lookup.** In `map lookup`, 86% of the time is
the build: `set` costs 108 ns per new key and `get` costs 17 ns (measured in
§Failing Reproduction). There are three mechanisms:

- **A — the probe's per-key cost.** Every Map/Set probe treats an `Integer`
  key as 8 bytes. It hashes them one byte at a time: 8 serial
  `eor`+`mul` steps (FNV-1a), then a 64-bit `udiv` for `mod 2*capacity`. It
  compares keys the same way, one byte at a time in a loop. The inline hit path
  of one `get` is **215 instructions**. The whole iteration of C's lookup loop is
  about 35.
- **B — the build rehashes every key about 5 times.** A new-key `set` hashes
  the key once in the inline probe and again in `_mfb_rt_map_bucket_put`. Every
  home-slot collision hashes it once more in `_mfb_rt_map_probe`. Every capacity
  grow sets `bucketsReady = 0`, and the next probe then rebuilds the whole index
  (one more hash per live key). With ×1.5 growth above 1,024 entries, that is
  about 2.5 rebuild hashes per insert.
- **A′ — FNV-1a mod a non-power-of-two clusters.** At some capacities the
  distribution is poor. At n = 3,000–5,000 (`2*capacity` = 6,912 or 10,368) the
  average successful probe walks 2.5–3.7 slots, against 1.4 for a well-mixed
  hash at the same load. So 70% of hits miss the inline home slot and fall
  through to the re-hashing helper. `map intkey` (n = 5,000) sits exactly on that
  bad capacity.
- **C (`listagg` only) — three allocations per bucket update.**
  `MUT bucket = getOr(m, k, empty)` returns a *tight* copy
  (`capacity == count`). The next `append` therefore always reallocates:
  allocate a new block, copy again, free the old one. Together with the
  `LET empty = []` literal, that is exactly **3.0 arena allocations per
  iteration** (6,013 at n = 2,000).

**The single correct behavior a fix produces** is below. It is checkable
in-process against same-binary controls, and on the rows:

1. An `Integer`-key `get`/`getOr`/`hasKey` hit costs **≤ 2.5× a
   `List OF Integer` `get`** at every n ∈ {2,000; 3,000; 5,000; 20,000}. Today
   it is 4.2–9.2×.
2. That per-lookup cost is **flat in n**: the max/min ratio across those four
   sizes is ≤ 1.5. Today it is 2.2–3.0×.
3. Building a `Map OF Integer TO Integer` by repeated `set` costs **≤ 3.5× the
   same number of `List OF Integer` appends** at n = 20,000. Today it is
   10.3–11.0×.
4. The `getOr` → `append` → `set` bucket update does **≤ 2 arena allocations
   per iteration**. Today it does 3.0.
5. Row targets, from the peers:
   - `map lookup` ≤ **0.90 ms**. This matches a C `-O0` growing map with a
     word hash, which is 0.86–0.91 ms (§Goal).
   - `map intkey` ≤ **0.35 ms**. The same C model runs it in 0.30–0.35 ms.
   - `map listagg` ≤ **0.30 ms**. Python runs it in 0.294 ms, and Python copies
     the bucket every iteration too (`bucket = bucket + [i]`).

   Against the committed C `-O0` peer these are ≤ 10×, ≤ 9.5× and ≤ 16×. That
   is grade C, not A. The reason is that **the C peer does less work**: it
   pre-sizes its table outside the timed region, and `listagg` uses no map at
   all (§Goal).

References:

- `mfb spec memory collections` — *Map Hash Index* (`src/docs/spec/memory/05_collections.md`
  §"Map Hash Index"), *`get`*, *Self-updates → Cost*, *Capacity Headroom and Growth*.
- `.ai/collections.md` — bug-760 B1 (tombstones and settle exemptions), bug-760 B2
  (the reclaim cost; `rt_map_list_value_grow_cost`), "Map copy-then-bulk-insert pays
  grows: presize instead".
- `benchmark/RANKING.md` §2 (grades), §3 (work-equivalence: "suspect the work, not the code").
- Related, not duplicated: bug-760 (`bugs/completed/`) fixed the listagg *reclaim*
  scan (B2) and added tombstones (B1). No open doc in `bugs/` or `planning/` covers
  these rows (`grep -rilE "map lookup|map intkey|listagg|map_probe" bugs planning`
  returns only completed docs and man/spec-review findings).

## Failing Reproduction

All numbers below come from `/Users/justinzaun/Development/mfb/target/release/mfb`
(main at `7e53c82b9`), built with `mfb build -O 1`, on macOS-aarch64.

### R1 — the rows, standalone (`/tmp/dbC-rows`)

`src/map.mfb` is a verbatim copy of `benchmark/mfb/src/map.mfb`. `src/main.mfb`:

```
IMPORT io
IMPORT collections

TYPE BenchResult
  group AS String
  name AS String
  med AS Float
  avg AS Float
  min AS Float
  max AS Float
END TYPE

FUNC makeResult(grp AS String, nm AS String, times AS List OF Integer) AS BenchResult
  LET sorted AS List OF Integer = collections::sort(times)
  LET n AS Integer = len(sorted)
  LET perMs AS Float = 1000000.0
  LET mn AS Integer = collections::get(sorted, 0)
  LET med AS Integer = collections::get(sorted, n / 2)
  RETURN BenchResult[grp, nm, toFloat(med) / perMs, 0.0, toFloat(mn) / perMs, 0.0]
END FUNC

FUNC main() AS Integer
  MUT tests AS List OF BenchResult = []
  tests = test_map_lookup(10, tests)
  tests = test_map_intkey(10, tests)
  tests = test_map_listagg(10, tests)
  FOR i = 0 TO len(tests) - 1
    LET r AS BenchResult = collections::get(tests, i)
    io::print(r.name & " med " & toString(r.med, toByte(3)) & " min " & toString(r.min, toByte(3)))
  NEXT
  RETURN 0
END FUNC
```

Observed (two runs): `lookup min 2.629 / 2.624`, `intkey min 0.999 / 1.003`,
`listagg min 0.459 / 0.467` ms. The checksums match the benchmark
(`199990000`, `12497500`, `2063616`). Expected: ≤ 0.90 / 0.35 / 0.30 ms.

### R2 — where the `lookup` row goes: build vs get vs a `List` control (`/tmp/dbC-parts`)

```
IMPORT io
IMPORT collections
IMPORT datetime

FUNC runOne(n AS Integer) AS Nothing
  MUT bb AS Integer = 999999999999
  MUT bg AS Integer = 999999999999
  MUT la AS Integer = 999999999999
  MUT lg AS Integer = 999999999999
  MUT chk AS Integer = 0
  FOR r = 0 TO 9
    LET t0 AS Integer = datetime::monotonicNanos()
    MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer {}
    FOR i = 0 TO n - 1
      m = collections::set(m, i, i)
    NEXT
    LET t1 AS Integer = datetime::monotonicNanos()
    MUT s AS Integer = 0
    FOR i = 0 TO n - 1
      s = s + collections::get(m, i)
    NEXT
    LET t2 AS Integer = datetime::monotonicNanos()
    MUT xs AS List OF Integer = []
    FOR i = 0 TO n - 1
      xs = collections::append(xs, i)
    NEXT
    LET t3 AS Integer = datetime::monotonicNanos()
    FOR i = 0 TO n - 1
      s = s + collections::get(xs, i)
    NEXT
    LET t4 AS Integer = datetime::monotonicNanos()
    chk = chk + s
    IF t1 - t0 < bb THEN
      bb = t1 - t0
    END IF
    IF t2 - t1 < bg THEN
      bg = t2 - t1
    END IF
    IF t3 - t2 < la THEN
      la = t3 - t2
    END IF
    IF t4 - t3 < lg THEN
      lg = t4 - t3
    END IF
  NEXT
  io::print("n=" & toString(n) & " ns/op x10: map set(new) " & toString(bb * 10 / n) & " map get " & toString(bg * 10 / n) & " | list append " & toString(la * 10 / n) & " list get " & toString(lg * 10 / n) & " chk " & toString(chk))
END FUNC

FUNC main() AS Integer
  runOne(2000)
  runOne(3000)
  runOne(5000)
  runOne(20000)
  runOne(200000)
  RETURN 0
END FUNC
```

Observed, in ns/op (best of 10 reps; two runs shown as `a / b`):

| n | map `set` (new key) | map `get` (hit) | list `append` | list `get` | get ÷ list get | set ÷ append |
|---|---|---|---|---|---|---|
| 2,000 | 79.0 / 75.0 | 22.0 / 20.5 | 9.0 / 8.5 | 4.0 / 4.0 | 5.5 / 5.1 | 8.8 / 8.8 |
| 3,000 | 97.0 / 92.0 | 32.6 / 31.3 | 9.6 / 9.3 | 4.3 / 4.0 | 7.6 / 7.8 | 10.1 / 9.9 |
| 5,000 | 155.4 / 100.6 | 57.4 / 37.0 | 14.2 / 9.0 | 6.4 / 4.0 | 9.0 / 9.2 | 10.9 / 11.2 |
| 20,000 | 127.7 / 108.7 | 18.8 / 16.8 | 11.6 / 10.6 | 4.5 / 4.2 | 4.2 / 4.0 | 11.0 / 10.3 |
| 200,000 | 147.3 / 147.8 | 18.4 / 18.3 | 11.5 / 11.5 | 4.1 / 4.1 | 4.5 / 4.5 | 12.8 / 12.9 |

The `get` cost is not flat. It peaks at n = 3,000–5,000, which is the
clustering in §Root Cause A′. An earlier variant of the same probe
(`/tmp/dbC-lookup`) gave these ns/op at n = 20,000:

| op | ns/op |
|---|---|
| `get` | 16.9 |
| `getOr` hit | 17.2 |
| `getOr` miss | 20.7 |
| `hasKey` | 16.6 |
| empty `FOR` loop | 3.3 |

So the three lookups cost the same. The build accounts for
20,000 × 108 ns ≈ 2.16 ms of the row's 2.5 ms.

### R3 — `listagg`: where an iteration goes (`/tmp/dbC-agg`, `/tmp/dbC-agg2`)

The ns-per-iteration figures come from two probe programs: `/tmp/dbC-agg`
(variants A–F) and `/tmp/dbC-agg2` (variants P–U). The allocation counts come
from `/tmp/dbC-alloc{0,2000,4000}`, built with `mfb build -O 1 --debug` and read
from `perf.mfb_alloc.count`.

| variant (n = 2,000, 64 keys) | ns / iteration |
|---|---|
| A — the benchmark shape (`getOr` → `append` → `set`) | 221 / 222 |
| P — `LET empty AS List OF Integer = []` alone | 15–17 |
| Q — `getOr` returning a 16-element list (copy) | 68–78 |
| R — Q + `append` to the copy | 149–169 |
| U — `MUT b = sixteen` (a plain local copy) + `append` | 121–131 |
| F — `getOr` on a 64-key `Map OF Integer TO Integer` (scalar value) | 14 |

`append` adds 80 ns to R, and the same pattern costs the same in U. So the
realloc-after-tight-copy is not specific to `Map`. The probe itself (F) is
14 ns, about 6% of A.

The listagg loop alone, as a whole program (`agg(n)` below), gives:

| n | `perf.mfb_alloc.count` |
|---|---|
| 0 | 2 |
| 2,000 | 6,013 |
| 4,000 | 12,015 |

That is 3.0 allocations per iteration.

```
FUNC agg(n AS Integer) AS Integer
  MUT m AS Map OF Integer TO List OF Integer = Map OF Integer TO List OF Integer {}
  FOR i = 0 TO n - 1
    LET key AS Integer = i MOD 64
    LET empty AS List OF Integer = []
    MUT bucket AS List OF Integer = collections::getOr(m, key, empty)
    bucket = collections::append(bucket, i)
    m = collections::set(m, key, bucket)
  NEXT
  RETURN len(m)
END FUNC
```

### Contrast cases that are already fast (regression guards)

- **`String` keys:** `map set` builds and reads back 1,000 `toString(i)` keys in
  0.099 ms vs C `-O0` 0.104 (grade S). `int_ops` and `str_ops` are S as well.
  The byte loop is the right shape for a variable-length key, and this bug must
  not regress it.
- **Overwriting an existing key** (`set` on a present key): 18–23 ns/op at
  n = 2,000 and 20,000 (R2's earlier variant). No grow and no `bucket_put`.
- **`List OF Integer`** `get`/`append`: 4 and 9–12 ns/op (R2).
- **Native `collections::groupBy`** already uses a private identity-hash table
  (`slot = key & mask`, `src/codegen/builtins/collections/func_group_by.rs`). It
  is the in-tree precedent for word-hashing an `Integer` key.

## Root Cause

Evidence is from `.ncode` dumps
(`mfb build -O 1 --ncode --target macos-aarch64 /tmp/dbC-lookup` and
`/tmp/dbC-rows`), flattened with `/tmp/dbC-dump.py`.

### A — the per-probe cost: byte-at-a-time FNV-1a, `udiv`, byte compare

`src/codegen/builtins/collections/gen_map.rs:emit_map_query_key` hands an
`Integer`/`Float`/`Fixed` key to the probe as `(stack-slot pointer, length 8)`.
`emit_map_probe` in the same file then does the following on the inline hit path
of a `get` in `/tmp/dbC-lookup` `runOne` (counted from the dump):

| step | instructions |
|---|---|
| key spill and header geometry | 35 |
| FNV-1a hash loop (`map_probe_hloop`), 8 iterations × 8 instructions plus the exit | 66 |
| `udiv` + `msub` (`hash mod 2*capacity`), bucket load, entry address, key-length compare | 17 |
| key compare loop (`map_probe_cloop`), 8 iterations × 10 instructions plus the exit | 82 |
| store the entry address, load the value | 15 |
| **total per inline hit** | **215** |

The critical path is 8 dependent `eor`→`mul` pairs, then a 64-bit `udiv`, then
two dependent loads. That is about 50 cycles of latency, which matches the
measured 17 ns hit at n = 20,000.

The same hash is emitted in four other places, all driven by the constants
`FNV1A_BASIS`/`FNV1A_PRIME` in `src/codegen/error/constants/error_constants.rs`:

- `src/codegen/engine/builder/mod.rs:lower_map_probe_helper` (`_mfb_rt_map_probe`)
- `lower_map_build_buckets_helper` (`_mfb_rt_map_build_buckets`)
- `lower_map_bucket_put_helper` (`_mfb_rt_map_bucket_put`)
- `src/codegen/collection/map/map_bucket_remove.rs:home_slot` (used by
  `_mfb_rt_map_bucket_delete` and `_mfb_rt_map_compact_from`)

All of them loop over bytes, whatever the key width. The byte loop exists because
one helper serves every probe-eligible key type
(`gen_map.rs:map_key_probe_eligible`: `String`, `Integer`, `Float`, `Fixed`,
`Byte`, `Boolean`, `Nothing`). The helper sees only `(ptr, len)`.

**The C model reproduces mfb's numbers.** `/tmp/dbC-c/split.c`, compiled with
`cc -O0`, implements mfb's exact algorithm: byte-wise FNV-1a, `% (2*cap)`, a
byte compare, 4→×2→×1.5 growth, and a full rebuild per grow. It measures the
build at 88–101 ns/op and `get` at 21–23 ns/op at n = 20,000, against mfb's
108 and 17. At n = 5,000 it measures 103–110 and 43–45, against mfb's 101–155
and 37–57. **mfb's code generation is not the problem; the algorithm is.**
Swapping only the hash for a one-multiply word mix, still with `udiv` and ×1.5
growth, drops the model to 52–55 / 16.6 ns. Using a mask and ×2 growth drops it
further, to 33–35 / 12 ns.

### A′ — FNV-1a over little-endian integer bytes, reduced `mod 2*capacity`, clusters

`/tmp/dbC-sim.py` replays the exact bucket geometry, including mfb's capacity
schedule `4, ×2 to 1024, then ×1.5` from
`COLLECTION_GROW_LOOKUP_INIT`/`_TAPER`. The average probes per successful lookup,
FNV vs a splitmix64 finalizer at the same bucket count:

| n (keys `0..n`) | buckets | load | FNV-1a probes | splitmix probes | FNV hits in home slot |
|---|---|---|---|---|---|
| 2,000 | 4,608 | 0.43 | 1.38 | 1.40 | 74% |
| 3,000 | 6,912 | 0.43 | 2.52 | 1.40 | — |
| 5,000 | 10,368 | 0.48 | **3.71** | 1.48 | **30%** |
| 5,000 (even keys: `intkey`) | 10,368 | 0.48 | **3.13** | 1.44 | **28%** |
| 20,000 | 52,488 | 0.38 | 1.12 | 1.31 | 94% |

The inline probe checks only the home slot. Any other slot branches to
`map_probe_fallback`, which calls `_mfb_rt_map_probe`. That helper **re-hashes
from scratch** and walks the run (`lower_map_probe_helper`). So at the bad
capacities about 70% of hits pay two hashes, a call, and a multi-slot byte-compare
walk. This is the n = 3,000–5,000 spike in R2, and `map intkey` (n = 5,000, even
keys) lands on it. Lookups stay O(1) average, so there is no complexity defect.
The spike is a hash-quality constant that swings 2.5–3× with capacity.

### B — a new-key `set` hashes the key about 5 times

`src/codegen/collection/map/map_mutate.rs` (the `set` arm, via `emit_map_probe`
at `map_mutate.rs:333`), dumped in `/tmp/dbC-lookup` `runOne` `mapset_*`:

1. The inline probe hashes the key (miss → `mapset_not_found`). On a home-slot
   collision it goes to the fallback, and `_mfb_rt_map_probe` hashes it again.
2. After the entry and data are written, `bl _mfb_rt_map_bucket_put` hashes it
   **again** to find an empty slot (`lower_map_bucket_put_helper`). It does not
   reuse the slot where the probe stopped.
3. When `count == capacity`, the grow path (`mapset_grow_*`) calls
   `_mfb_rt_map_reclaim` (bug-733). It allocates the ×2 (or ×1.5 once past
   1,024) block and copies the entries and data. It writes the new header with
   `str_u32 0` at offset 4, so **`bucketsReady = 0`**. The next `set`'s probe
   falls back to `_mfb_rt_map_probe`, which calls `_mfb_rt_map_build_buckets`
   and re-hashes **every live key**.

   Growing 1,024 → 20,000 at ×1.5 rebuilds at about
   1,536 + 2,304 + … + 17,496 ≈ 49k keys (guess; derived from the schedule, not
   instrumented). Adding the 2,048 keys hashed below 1,024 gives about 2.5
   rebuild hashes per insert. The C model measures one full rebuild at
   14.9–16.5 ns per entry with the byte hash, and 7.3 ns with a word hash.
4. On a hit, the `set` arm recomputes the entry index as
   `(entryAddr − map − 40) / 40` with a second `udiv` (`mapset_found_handle`
   prologue in the dump). The probe already knew the index.

### Verified NOT the cause

- **bug-760 B1 `_mfb_rt_map_settle` on reads.** Checked in
  `/tmp/dbC-settle`, which runs `removeKey` on the map first so that the settle
  machinery is live in the function. The `get`, `getOr` and `hasKey` loops and
  the later `set` loop contain no `bl _mfb_rt_map_settle`. Only
  `_mfb_rt_map_probe` / `_mfb_rt_map_bucket_put` / `_mfb_rt_map_reclaim` appear.
  The three benchmark functions (`test_map_lookup`/`intkey`/`listagg` in
  `/tmp/dbC-rows`) contain no settle call either. So `get`/`getOr`/`hasKey`
  (and `len`) are exempt, as `.ai/collections.md` says.
- **Index-not-ready rebuilds triggered by reads.** Reads never invalidate the
  index. The only rebuilds are the ones that follow a grow (B.3), and they land
  on the next write's probe.
- **Bounds and error checks.** The overflow `adds`/`b.vc` and the
  `count == 0` / `bucketsReady` tests are 2–4 instructions each, out of about
  300 per iteration.
- **Page faults / fresh memory.** The `--debug` report for `/tmp/dbC-rows`
  shows `arena.0.maps 15` for the whole program and `hit_quick_bin 60,758` of
  61,132 allocations. Steady-state reps reuse arena memory.

### C — `listagg`: tight copy then append ⇒ an extra allocation and copy per iteration

`src/codegen/memory/owned.rs:materialize_owned_element` copies the fetched
`List` value through `copy_flat_block` →
`src/codegen/collection/layout/builder_collection_layout.rs:copy_collection_tight`.
The copy is shrink-to-fit by design ("headroom is a property of a mutable working
buffer, never of a value", plan-01 §4.3). The header it writes has
`capacity == count` (`/tmp/dbC-rows` `test_map_listagg`: `str_u64 x9` at offsets
8 **and** 16).

The very next statement, `bucket = collections::append(bucket, i)`, is an
in-place self-update. It finds `count >= capacity` and takes
`append_inplace_realloc`: `_mfb_arena_alloc`, copy, `_mfb_arena_free`. The copy
is required by value semantics. The *second* allocation and copy are not: they
exist only because the copy that feeds a working buffer was made tight.

The third allocation per iteration is the `LET empty AS List OF Integer = []`
argument (P: 15 ns). The probe is 14 ns (F). The `set` value-grow and relocation
into the map's data region is the remainder. bug-760 B2 already made its reclaim
cheap.

## Goal

- R2 at n ∈ {2,000; 3,000; 5,000; 20,000}: Integer-key `get` ns/op ≤ **2.5×**
  `List OF Integer` `get` ns/op at each n. The max/min of the map `get` ns/op
  across the four sizes must be ≤ **1.5**.
- R2 at n = 20,000: map `set` (new key) ns/op ≤ **3.5×** list `append` ns/op.
- R3: the `agg(2000)` program does ≤ **4,015** arena allocations, i.e. ≤ 2 per
  iteration (`--debug`, `perf.mfb_alloc.count`).
- R1 / the benchmark, mfb -O1 min: `map lookup` ≤ **0.90 ms**, `map intkey`
  ≤ **0.35 ms**, `map listagg` ≤ **0.30 ms**. All three checksums unchanged.

**Why these targets, and why not "within 2.5× C -O0".**
`benchmark/c/mapbench.c` does less work than mfb and Python on all three rows:

- `test_map_lookup` and `test_map_intkey` `calloc` a 32,768- or 16,384-slot
  table **before** `t0`. The table never grows and never rehashes. The hash is
  one multiply plus a mask. mfb and Python both grow from empty.
- `test_map_listagg` uses **no map**. It indexes a 64-element array of vectors
  by `key` and appends in place with no copy.

The fair peers are these:

- `lookup` and `intkey`: the C `-O0` model of a *growing* word-hash map
  (`/tmp/dbC-c/peer.c`, word-hash + mask, ×2 growth). It measures
  **0.86–1.09 ms** for `lookup` and **0.30–0.35 ms** for `intkey`, and that is
  with non-inlined `-O0` calls, where mfb's probe is inline.
- `listagg`: Python, which also copies the bucket each iteration
  (`benchmark/python/mapbench.py:test_map_listagg`, `bucket = bucket + [i]`).
  It measures 0.294 ms. The C value-semantics model (copy out, append, copy
  back) is 0.17–0.25 ms at `-O0`.

The targets therefore mean parity with an honest C `-O0` implementation, or with
Python. Against the committed C column that is ≤ 10×, ≤ 9.5× and ≤ 16× (grade C).
Closing that last gap would take pre-sizing, or eliding a value-semantics copy
the language requires. Both are out of scope; see Non-goals.

### Non-goals (must NOT change)

**Owner constraint:** the fix changes no language specification and no memory
model, only internal implementation. It stays within these sections of
`mfb spec memory collections`:

- *Map Hash Index*: the index stays "derived, lazy, and recomputed — never
  authoritative", keeps `2 * capacity` `U64` slots, and keeps linear probing.
- *Self-updates → Cost*: "A lookup costs … an O(1)-average hash probe". This is
  unchanged, and so is the per-element cost of a self-update.
- *Copy*: a `get`/`getOr` result is still an owned copy.
- *Capacity Headroom and Growth*: headroom is only ever added to a working
  buffer, never to a stored value or a returned value.

Specifically:

- **Observable semantics are unchanged.** Map iteration order stays entry
  (insertion) order; the buckets never influenced it. Key equality stays the
  bitwise comparison of the stored key bytes: `Float` `+0.0 ≠ -0.0`,
  `NaN = NaN`, a `Fixed` compares by bit pattern. The results, errors and error
  codes of `get`/`getOr`/`hasKey`/`contains`/`set`/`removeKey` are unchanged, and
  so are all checksums. The hash is not observable to a program. No builtin,
  signature, syntax or `mfb man` text changes.
- **Value semantics are unchanged.** `getOr`/`get` still returns a copy. The map
  still holds its own value, and `set` still stores a copy. **Forbidden:** eliding
  the `getOr` copy by "moving" the value out of the map slot on the assumption
  that the later `set` overwrites it. That is an alias/move rule change. It breaks
  §14.2's "the old value remains live" on a failure path (`append` can raise
  `ErrOutOfMemory` between the two), and it is not a provably-dead source under
  the current analysis. **If someone wants the `listagg` row at C-array speed,
  that is the blocker:** the copy is required by value semantics, and only
  copy-elision where the source is provably dead is allowed. This bug does not
  attempt it.
- **Memory model unchanged.** No change to the arena, the ownership/drop
  contract, the stored-value shrink-to-fit rule, or the header and entry layout
  (header offset 4 keeps `bucketsReady` in the u8 and `deadEntries` in the upper
  24 bits). No change to the `2*capacity` bucket region size, or to thread
  transfer or `.mfp` bytes (buckets are never serialized; `bucketsReady = 0` on
  every copy).
- **`String` keys keep a byte hash.** The `set` row is at C parity today.
  Word-at-a-time hashing of long strings is a separate optimization and is not
  part of this bug.
- **Forbidden fixes:**
  - changing `benchmark/mfb/src/map.mfb`, the C or Python peers, or the row
    sizes;
  - pre-sizing the map from a benchmark-specific constant;
  - re-baselining `benchmark/baseline/*.log` to hide the rows;
  - relaxing `rt_map_list_value_grow_cost`'s ×1.5 bound;
  - special-casing the benchmark's key pattern, such as sequential keys or an
    identity hash with no mixing. An identity hash (`key mod B`) is forbidden
    because arithmetic-progression keys (`i*2`, `i*1024`) would then pile into
    one residue class.

## Blast Radius

All sites that hash a map/set key (found with `grep -rn "FNV1A_BASIS\|FNV1A_PRIME"
src`). There are exactly three files. **They must all change together**,
because the index is only correct if every writer and reader computes the same
home slot:

- `gen_map.rs:emit_map_probe` (inline probe used by `get` `:366`, `getOr` `:490`,
  `gen_set.rs:49` `contains`, `map_mutate.rs:333` `set`, `map_mutate.rs:1295`
  `removeKey` / `remove`, `builder_emit_helpers.rs`) — **fixed by this bug**.
- `builder/mod.rs:lower_map_probe_helper`, `lower_map_build_buckets_helper`,
  `lower_map_bucket_put_helper` — **fixed by this bug** (same hash).
- `map_bucket_remove.rs:home_slot` (backward-shift delete `_mfb_rt_map_bucket_delete`,
  `_mfb_rt_map_compact_from`) — **fixed by this bug**. Its backward-shift
  "home lies cyclically in `(gap, slot]`" test depends on the home slot. If one
  site kept FNV, entries would become unreachable **silently** (wrong
  `hasKey = FALSE`). This is the correctness risk; see Fix Design.
- `func_group_by.rs` private scratch table (`key & mask`) — unaffected. It never
  writes the result map's buckets; `bucketsReady = 0` on the built map.
- `builder_collection_layout.rs:3407` and `runtime/thread/value_functions.rs:63` —
  unaffected. They are FNV over a *type name*, used to make a symbol name, not a
  map key.
- Other rows on the same probe that should improve (latent, same cost, not
  D-grade): `map (key-Fixed)`/`(Fixed)`/`(Record-*)`/`(State-*)` `get`/`getOr`/`hasKey`
  (B, 2.7–4.2×), `set (Fixed) contains` (B, 4.3×), `set (Fixed) add` (D, 19×,
  `small`), `map (Fixed) set` (D, 21×, `small`), `set (Fixed)` `toSet`/`union`/
  `intersection`/`difference` (C/D). The ×-figures come from `rank.py --csv`. All
  of these are Integer-keyed and share B.1–B.3. Record them as expected
  improvements; they are not gates.
- The C3 pattern "tight copy into a `MUT` working buffer, then an in-place
  `append`" is not map-specific. R3 variant U, a plain `MUT b = xs` followed by
  `append`, pays the same 80 ns. **In scope only for the `get`/`getOr` →
  `MUT` binding site** that the `listagg` row uses. The general
  `lower_value_owned` copy is latent: same hazard, not a D-grade row. Do not
  widen it here.

## Fix Design

Every item is an internal lowering or runtime-helper algorithm change.

**Fixed-width keys get a word hash and word compare, from one shared definition
(A, A′).** For a key whose stored length is 8 (`Integer`/`Float`/`Fixed`):

- hash = a 64-bit mix of the key word, for example
  `z = k * 0x9E3779B97F4A7C15; z ^= z >> 29` or a splitmix64 finalizer;
- slot = `umulh(hash, 2*capacity)`. This is the multiply-high range reduction:
  it keeps the `2*capacity` bucket count, with no layout change and no `udiv`.
  `umulh` is already an abstract op; the grow path uses it for overflow checks.

The dispatch must be on **the key bytes' length at every site**, not on the
static type, so that all five hash sites agree even inside the shared helpers:

- `len == 8` → word path;
- any other length → today's FNV-1a;
- 1-byte keys may keep FNV.

A `String` key that happens to be 8 bytes long would also take the word path.
That is harmless: its hash is still a pure function of its bytes, the same at
every site. On riscv64, do not load a `String`'s 8 bytes as one unaligned word.
Either gate the word path on the probe-eligible *fixed-width* entry type and pass
that as a flag argument to the helpers, or assemble the word from 8 byte loads
(check `.ai/arch-abi.md` for riscv64 alignment). Key equality becomes one `u64`
compare, which is bit-identical to the 8-byte compare, so the *Map Hash Index*
equality rule holds.

**Walk the run inline; hand off only when the index is not built (A′, B.1).**
`emit_map_probe` should loop over the linear-probe run itself. A home-slot
collision should not call a helper that starts over. `_mfb_rt_map_probe` stays
for the `bucketsReady == 0` path.

**Reuse the probe's end slot for insertion (B.2).** On a `set` miss, the probe
has already found the empty slot that ends the run. Pass it to the bucket write
(or inline the store) instead of calling `_mfb_rt_map_bucket_put` to re-hash.
This is only valid if no grow happened in between. After a grow, keep today's
invalidate-and-rebuild.

**Carry the entry index (B.4).** Have the probe yield `index`, not only
`entryAddr`, so `set`'s found path drops its `udiv` by 40.

**Rebuild cost (B.3).** With the word hash, a rebuild is about 2× cheaper
(C model: 14.9 → 7.3 ns/entry). Leave the ×1.5 lookup taper
(`COLLECTION_GROW_LOOKUP_TAPER`, shared with `List`) alone unless the Phase 2
measurement misses the build target. It is an Open Decision because it trades
memory.

**`listagg` (C).** When `get`/`getOr`'s owned copy is bound to a `MUT` local
that the same scope self-updates with `append`, emit the copy with growth
headroom rather than tight. The shape is `copy_collection_with_capacity` from
`.ai/collections.md` "Map copy-then-bulk-insert", with
`capacity = step(count)` and `dataCapacity = step(dataLength)`.

This is allowed because headroom on a **working buffer** is exactly what plan-01
§4.3 permits. The value is still an independent copy, and nothing observable
changes. The *stored* value stays tight: `set` still copies tight into the map,
and every other store (return, field, global, thread transfer) still goes
through the tight copy.

The `append` then uses the headroom, so there is one allocation instead of two,
and one copy instead of two. It stays within the self-update cost contract
("plus at most one grow"): the copy is the grow.

**Rejected alternatives:**

- An identity hash (see Non-goals).
- A power-of-two bucket count with a mask. This is a layout change to the
  documented `2*capacity` region, and `umulh` gets the same benefit without it.
- Copying the buckets on grow instead of rebuilding them. The slots change with
  capacity, so a rebuild is needed anyway.
- Caching the hash in the entry. That changes the entry layout, and the 40-byte
  entry has no free word.
- A move-out of the map value for `listagg` (see Non-goals: a value-semantics
  blocker).

**Risk** is in the Blast-Radius agreement. `home_slot` in `map_bucket_remove.rs`
is the easiest one to miss, and a miss there is silent (a key present but not
found). Phase 1's differential test (below) is the guard.

**Doc sync.** The contributor spec *Map Hash Index* bullet says
"`FNV-1a(keyBytes) mod (2*capacity)`". It must be updated in the same change to
describe the new internal function. That is an as-is description of an internal
index, not a language or memory-model rule, and nothing else in
`mfb spec memory` changes. **If the owner treats that bullet as frozen, that is
a blocker** for A/A′: the row targets cannot be met with the FNV byte chain
kept. That is a guess, from the C model: keeping byte-FNV and `udiv` but walking
inline still leaves about 12–14 ns per hit.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [ ] Add `tests/runtime/rt_map_integer_key_probe_cost.rs`, modeled on
      `tests/runtime/rt_map_list_value_grow_cost.rs`: one in-process program,
      best of 5, with same-binary controls and a checksum line.
      - `an_integer_key_lookup_costs_within_2_5x_a_list_get`: R2's program at
        n ∈ {2,000; 3,000; 5,000; 20,000}. Assert map `get` / list `get` ≤ 2.5
        at each n, and max/min of map `get` ≤ 1.5. **Fails today** (4.0–9.2×;
        3.0×/2.2× spread).
      - `building_an_integer_key_map_costs_within_3_5x_list_appends`: n = 20,000.
        Assert ≤ 3.5. **Fails today** (10.3–11.0×).
      - `a_list_value_read_modify_write_allocates_twice_per_iteration`: build
        R3's `agg(2000)` with `--debug` and assert `perf.mfb_alloc.count`
        ≤ 4,015. **Fails today** (6,013). Check whether `tests/common` already
        has a `--debug` harness helper (`grep -rn "perf.mfb_alloc" tests`) and
        reuse it.
      - A differential correctness test that is **green today and must stay
        green**. For `Map OF Integer`, `Map OF Float` (with `+0.0`, `-0.0` and
        `NaN` keys) and `Map OF Fixed`, plus a `Set OF Integer`, at sizes
        crossing every grow (1…6,000), interleave `set` / `removeKey` (in-place
        and tombstone paths) / `hasKey` / `getOr` / `contains` over
        arithmetic-progression keys (`i`, `2i`, `1024i`, negatives, `MIN`/`MAX`).
        Assert against a `List`-scan oracle. This is the guard for the
        all-sites-agree invariant. Model it on
        `rt_map_fifo_removal_flat.rs`'s checksum.
- [ ] Record the Blast-Radius verdicts above; re-run the FNV grep to prove the
      list is complete.

Acceptance: the three cost tests fail for the documented reasons; the
differential test passes.
Commit: —

### Phase 2 — the fix

- [ ] A/A′: add the length-8 word hash plus `umulh` reduction and the `u64`
      compare to all five hash sites together (`gen_map.rs:emit_map_probe`, the
      three helpers in `builder/mod.rs`, `map_bucket_remove.rs:home_slot`). Put
      the hash in one shared emitter so the sites cannot drift.
- [ ] A′/B.1: make `emit_map_probe` walk the linear-probe run inline; keep the
      helper for `bucketsReady == 0`.
- [ ] B.2 / B.4: make a `set` miss insert at the probe's end slot (no
      `bucket_put` re-hash when no grow happened), and make a `set` hit use the
      probe's index (no `udiv` by 40).
- [ ] C: `get`/`getOr` → `MUT` binding that is `append`-self-updated gets a
      headroom copy (`copy_collection_with_capacity`); stored values stay tight.
- [ ] Update `mfb spec memory collections` *Map Hash Index* (hash description and
      the `[[…]]` citations) and `.ai/collections.md` (a lesson line: fixed-width
      keys word-hash, and all five sites must agree).

Acceptance: the Phase 1 tests pass; the contrast cases still hold
(`map set` String row, overwrite `set`, `rt_map_list_value_grow_cost`
≤ ×1.5, `rt_self_update_cost`, `rt_map_fifo_removal_flat`); nothing in
Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [ ] Codegen goldens / `.ncode` snapshots that contain the map probe will shift.
      This is expected **only** in the map/set probe, helper and `set`/`getOr`-copy
      sequences. Inspect one fixture per shifted family before accepting it
      (AGENTS.md: an unexpected diff is a bug-hunt trigger).
- [ ] Run the full suite (`cargo test`, plus `scripts/` gates per
      `.ai/testing-gates.md`), and `cargo test --bin mfb spec` for the spec
      edit.
- [ ] Re-run R1–R3 and the benchmark map group, then `./benchmark/rank.py`.
      Confirm the row targets and that no other row regressed (String-key rows
      especially).

Acceptance: full suite green; golden deltas confined to the probe/copy
sequences; the R1 rows meet the Goal on macOS-aarch64, and a Linux x86-64 /
riscv64 smoke run of the differential test passes.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_map_integer_key_probe_cost.rs` (three
  cost bounds plus the differential oracle).
- Runtime proof: R1 (`/tmp/dbC-rows`, the verbatim `map.mfb`) meets
  ≤ 0.90 / 0.35 / 0.30 ms with unchanged checksums, and the benchmark map group
  confirms it.
- Doc sync: `src/docs/spec/memory/05_collections.md` *Map Hash Index* (and *`get`*,
  which says "FNV-1a bucket probe"), and `.ai/collections.md`. No `mfb man`
  change, since nothing is user-visible.
- Full suite: `cargo test` (all targets) and the acceptance/artifact gates in
  `.ai/testing-gates.md`.

## Open Decisions

- Word mix choice: one multiply plus shift-xor (cheapest) vs a full splitmix64
  finalizer (2 muls). Recommend the cheapest one that passes the flatness
  bound at all four n in the Phase 1 test, and also at arithmetic-progression
  keys. Measure; don't infer.
- Lookup growth taper (×1.5 above 1,024, shared with `List`): leave it alone
  unless the build bound fails after A/B. Changing it trades memory, and it is
  shared.
- `LET empty AS List OF Integer = []` allocates on every iteration (15 ns). A
  shared static empty block would need a no-free sentinel in the drop contract.
  That is a memory-model question, so it is **out of scope**; listed only so it
  isn't re-proposed silently.

## Summary

The Integer-key map rows are slow because the map treats an 8-byte key as a
byte string. It hashes it with 8 serial FNV-1a steps plus a `udiv`, and a
new-key `set` repeats that about 5 times: probe, fallback re-probe, `bucket_put`,
and about 2.5 rebuild hashes per grow. That FNV mod non-power-of-two also
clusters, to 3.7 probes per hit at n = 5,000. `listagg` adds a third allocation
per iteration because the owned copy it appends to is tight.

A `cc -O0` model of the same algorithm reproduces mfb's timings. The fix is
algorithmic and internal: a word hash, an inline probe walk, slot and index
reuse, and headroom on a working-buffer copy. The engineering risk is keeping
all five hash sites in agreement, which the differential test guards. Value
semantics, the memory model, iteration order, key equality and the stored-value
layout stay untouched.
