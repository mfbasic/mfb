# bug-769: Inserting into a growing `Set`/`Map` costs ~90 ns per element, two thirds of it in the free-path entropy scrub; `toSet` grows from empty instead of sizing once

Last updated: 2026-09-28
Effort: large (3h–1d)
Severity: MEDIUM
Class: Other (Performance)

Status: Fixed
Regression Test: `tests/runtime/rt_set_insert_cost.rs`; `#[cfg(test)]` stream-identity tests in
`src/codegen/builtins/math/gen_rng_pcg64.rs`

`collections::toSet` over a 300-element `List OF Integer` costs ~97 ns per element
(`set (Fixed) toSet` 0.583 ms for 20 × 300 elements). C -O0 builds the same set in
0.031 ms and CPython in 0.056 ms, so the row is D-graded (19× C) and 10× Python. The
same per-insert cost shows up, smaller, in every row that builds a `Set`/`Map` from
empty (`set … add`, `map … set`, 300 inserts each).

Standalone, the cost is ~93 ns per inserted element at N = 300, rising to ~176 ns at
N = 300,000. A probe hit (adding an element already present) costs 11.8 ns. The
difference is growth. About 65% of the time is `_mfb_arena_fill_random` re-randomizing
freed blocks: the grow chain's outgrown blocks and the final set when it dies. That
comes to ~246 bytes scrubbed per inserted element, at ~4 bytes/ns. `toSet` adds to
this because its body starts from an empty set and grows it log₂(n) times, even though
`len(value)` bounds the result.

**The single correct behavior a fix produces:** `toSet(xs)` allocates its result once
and costs one insert per element. The free-path scrub keeps its spec'd contract (every
freed payload byte is re-randomized, in debug and release) but writes the same PCG64
stream faster. Checkable: at N = 300, `toSet` per element is ≤ 4× a probe-hit `add`
per element in the same run (today 7.9×), and the fill emits the identical PCG64 word
sequence.

## STATUS: FIXED (G1 `99c80478a`, G2 `a2d04434c` + spec; suffix helper `d3e30a5a5`; goldens `00d11c51b`)

**G1: native `toSet` allocates once.** Covers the fixed-width elements (Boolean, Byte,
Scalar, Integer, Float, Fixed, Money). String elements keep the `.mfb` path: passing a
packed String to the insert would need a per-element block or a reused scratch block,
which breaks "1 allocation per call". Allocations per call go from 9 (distinct) and 7
(with duplicates) to 1. The result is allocated at `capacity = len(value)` and filled
by the existing `lower_map_set_in_place`, whose grow arm is never taken.

**Deviation: the result's capacity with duplicates.** A distinct list gives a tight
result; the `.mfb` body left up to 2× headroom. A list with duplicates keeps
`capacity = len(value)`: the upper-bounded-builder shape. `mfb spec memory
collections` *Capacity Headroom and Growth* now says so in one bullet, since the
growth shape is "an implementation tuning detail, not an observable contract".

**`mapValues`.** Already allocates once per call (`map_values_fast_path`), so the
doc's claim is stale. It stays in the test as a guard.

**G2: 4 PCG64 words per iteration by jump-ahead, the identical stream.** Each word
j = 1..4 comes from the carried state with compile-time constants (M^j and I·Σ M^i).
The constants are hoisted out of the loop, and the old single step handles
`count mod 4`. `emit_pcg_step` and `math::rand` are untouched. K = 4 on every
architecture; on x86-64 (4 allocatable GPRs) K = 4 still measured fastest on real
hardware.

Stream-identity evidence:

- Unit tests: the constants, and a branch-for-branch model against the single-step
  stream for 0–64 bytes, 4096 words and odd sizes, 5 seeds.
- The helper's real machine code emulated (unicorn) for all five targets, old and new
  compiler: every word and the final state match a PCG64 model.
- An lldb run with the state pinned: old and new binaries give the identical sha256
  over 280 fills.
- The `--debug` counters are identical.

Throughput (addFresh, 300 inserts, macOS aarch64): 101.7 → 64.9 ns/insert. The fill's
share of `sample` top-of-stack fell from 65–67% to 49–50%. G2's ≤ 45% was a guess
target and was missed; the combined G3 bound is met.

**G3.** `toSet` per element is ×1.63 a probe-hit `add` (bound ×4; ×8.59 before).

**Benchmark** (`python3 /tmp/b769-bench.py 10`: the benchmark with `main` reduced to these
rows, pre-fix vs fixed, same host, loaded; medians in ms; checksums identical):

| row | before | after |
|---|---:|---:|
| `set (Fixed) toSet` | 1.171 | 0.274 |
| `set (Record-Fixed) toSet` | 1.012 | 0.534 |
| `set (State-Fixed) toSet` | 0.834 | 0.453 |
| `set (Fixed) add` | 0.045 | 0.018 |
| `map (Fixed/Record/State/key-Fixed) set` | 0.050–0.071 | 0.016–0.021 |

All three Fixed `toSet` rows take the native path (no `_mfb_ifn_collections_5FtoSet`
call in their `.ncode`). The String rows keep the `.mfb` body.

Gate: full suite 279 ok, artifact-gate 0 diffs, acceptance 1582 at -O1 and -O3.

**Found along the way.**

- `no_type_strings` caught `toSet`'s new `ParameterType::declared(<suffix>)`. Instead
  of raising the budget, nine collections fast-path suffix parses now share
  `collections::monomorph_suffix_type`, and `declared_sites` codegen tightened
  45 → 38.
- Not a bug, recorded for later: on aarch64 `add_carry` with `carry_out = xzr` still
  emits `cset xzr, cs`, a dead instruction on the fill's per-word critical path.
  Removing it changes the encoder's size table and two pinned encoder tests
  (`encodes_umulh_add_carry_and_rorv`, `add_carry_and_sub_borrow_sizes`), so it is a
  separate change.

References:

- `mfb spec memory arenas` — *Entropy Fill* (`src/docs/spec/memory/04_arenas.md`):
  "Freed chunks and freshly mapped blocks are filled with pseudo-random bytes — always
  on, in debug and release … The fill source is a dedicated per-arena PCG64 … Its output
  is never observable."
- `mfb spec memory collections` — *Capacity Headroom and Growth* ("The growth shape (an
  implementation tuning detail, not an observable contract) … Literals and known-size
  builders … allocate exactly").
- `planning/todo.md` — plan-133-B item 2, "Measure the entropy-fill cost … Measurement
  only; the fill stays": fill-off runs the whole benchmark ~30% faster by geomean
  (x0.701 / x0.710 over 485 rows). The owner decision recorded there is that the fill
  stays. This bug respects that.
- Found during the benchmark D/F write-up (cluster B, Map/Set mutation).

## Benchmark rows

`benchmark/baseline/*.log`, min column (ms). Sources are `benchmark/mfb/src/setops.mfb`
and `mapmatrix.mfb`. `toSet` row: `FOR k = 0 TO 19: acc += len(toSet(toList(base)))`
with `base` = 300 Integers. `add` / `set` rows: 300 inserts into a fresh
`Set OF Integer` / `Map OF Integer TO Integer`. The C and Python peers exist only for
the plain rows; the Record/State/key rows borrow them.

| row | mfb -O1 | -O2 | -O3 | C -O0 | C -O2 | Python | grade (÷ C -O0) |
|---|---:|---:|---:|---:|---:|---:|---|
| `set (Fixed) toSet` | 0.583 | 0.581 | 0.582 | 0.031 | 0.022 | 0.056 | D (18.8×) |
| `set (Record-Fixed) toSet` | 0.585 | 0.582 | 0.583 | (0.031) | | | D |
| `set (State-Fixed) toSet` | 0.592 | 0.591 | 0.592 | (0.031) | | | D |
| `set (Fixed) add` | 0.019 | 0.018 | 0.018 | 0.001 | 0.001 | 0.013 | D, small |
| `set (Record-Fixed) add` | 0.019 | 0.019 | 0.018 | (0.001) | | | D, small |
| `set (State-Fixed) add` | 0.025 | 0.028 | 0.022 | (0.001) | | | D, small |
| `map (Fixed) set` | 0.021 | 0.020 | 0.020 | 0.001 | 0.000 | 0.024 | D, small |
| `map (Record-Fixed) set` | 0.021 | 0.020 | 0.021 | (0.001) | | | D, small |
| `map (key-Fixed) set` | 0.021 | 0.021 | 0.021 | 0.001 | 0.000 | 0.024 | D, small |
| `map (State-Fixed) set` | 0.030 | 0.030 | 0.030 | (0.001) | | | D, small |

The C peer is not a like-for-like grow. `benchmark/c/setmatrix.c` `ISet` is a fixed
4096-slot table that never grows or frees during the row (`iset_new` mallocs once).
C's per-insert time (0.031 ms / 6000 ≈ 5 ns) is therefore the insert path alone.
CPython's set does grow; its `add` costs 0.013 ms / 300 ≈ 43 ns per insert, and its
`set(list)` costs 0.056 ms / 6000 ≈ 9 ns per element.

**Verdict on the small `add` / `set` rows (asked for by the cluster brief): a real
per-insert overhead, not the measurement floor.** Standalone at N = 300 … 300,000 the
per-insert cost is 93–176 ns (table below), against ~5 ns for C's insert path and ~43 ns
for CPython's growing `add`. Its causes are the same as `toSet`'s, minus the presizing
point, so this document covers them. The State-Fixed rows' extra ~6–9 µs per 300 ops
(`0.030` vs `0.021`, `0.025` vs `0.019`) is the `STATE` owner open/close per statement.
That is guess-level attribution, not measured separately, and out of scope.

## Failing Reproduction

`/tmp/dbB-toset/project.json` (same shape as any executable project):

```json
{"name":"dbbtoset","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
```

`/tmp/dbB-toset/src/main.mfb`:

```
IMPORT collections
IMPORT io
IMPORT datetime

FUNC buildL(n AS Integer) AS List OF Integer
  MUT xs AS List OF Integer = []
  FOR i = 0 TO n - 1
    xs = collections::append(xs, i)
  NEXT
  RETURN xs
END FUNC

FUNC viaToSet(xs AS List OF Integer, reps AS Integer) AS Integer
  MUT acc AS Integer = 0
  FOR k = 0 TO reps - 1
    acc = acc + len(collections::toSet(xs))
  NEXT
  RETURN acc
END FUNC

FUNC viaAddLoop(xs AS List OF Integer, reps AS Integer) AS Integer
  MUT acc AS Integer = 0
  FOR k = 0 TO reps - 1
    MUT s AS Set OF Integer = Set OF Integer { }
    FOR EACH x IN xs
      s = collections::add(s, x)
    NEXT
    acc = acc + len(s)
  NEXT
  RETURN acc
END FUNC

FUNC viaAddIdx(n AS Integer, reps AS Integer) AS Integer
  MUT acc AS Integer = 0
  FOR k = 0 TO reps - 1
    MUT s AS Set OF Integer = Set OF Integer { }
    FOR i = 0 TO n - 1
      s = collections::add(s, i)
    NEXT
    acc = acc + len(s)
  NEXT
  RETURN acc
END FUNC

FUNC viaMapSet(n AS Integer, reps AS Integer) AS Integer
  MUT acc AS Integer = 0
  FOR k = 0 TO reps - 1
    MUT m AS Map OF Integer TO Integer = Map OF Integer TO Integer { }
    FOR i = 0 TO n - 1
      m = collections::set(m, i, i)
    NEXT
    acc = acc + len(m)
  NEXT
  RETURN acc
END FUNC

FUNC viaToList(s AS Set OF Integer, reps AS Integer) AS Integer
  MUT acc AS Integer = 0
  FOR k = 0 TO reps - 1
    acc = acc + len(collections::toList(s))
  NEXT
  RETURN acc
END FUNC

FUNC bestOf(which AS String, n AS Integer, reps AS Integer) AS Integer
  LET xs = buildL(n)
  LET s0 = collections::toSet(xs)
  MUT b AS Integer = 9223372036854775807
  FOR r = 0 TO 6
    LET t0 AS Integer = datetime::monotonicNanos()
    MUT c AS Integer = 0
    IF which = "toSet" THEN
      c = viaToSet(xs, reps)
    END IF
    IF which = "addEach" THEN
      c = viaAddLoop(xs, reps)
    END IF
    IF which = "addIdx" THEN
      c = viaAddIdx(n, reps)
    END IF
    IF which = "mapSet" THEN
      c = viaMapSet(n, reps)
    END IF
    IF which = "toList" THEN
      c = viaToList(s0, reps)
    END IF
    LET ns AS Integer = datetime::monotonicNanos() - t0
    IF c <> n * reps THEN
      io::print("BAD " & which)
    END IF
    IF ns < b THEN
      b = ns
    END IF
  NEXT
  RETURN b
END FUNC

FUNC main AS Integer
  FOR EACH w IN ["toSet", "addEach", "addIdx", "mapSet", "toList"]
    MUT line AS String = w
    FOR EACH n IN [300, 3000, 30000, 300000]
      LET reps AS Integer = 3000000 / n
      LET ns AS Integer = bestOf(w, n, reps)
      line = line & "  n=" & toString(n) & ": " & toString(ns * 10 / (n * reps)) & " dns/elt"
    NEXT
    io::print(line)
  NEXT
  RETURN 0
END FUNC
```

`mfb build -O 1 /tmp/dbB-toset && /tmp/dbB-toset/build/dbbtoset.out` (macOS aarch64,
`target/release/mfb` at `7e53c82b9`; units are deci-nanoseconds per element):

```
toSet  n=300: 931 dns/elt  n=3000: 1111 dns/elt  n=30000: 1415 dns/elt  n=300000: 1776 dns/elt
addEach  n=300: 928 dns/elt  n=3000: 1114 dns/elt  n=30000: 1394 dns/elt  n=300000: 1800 dns/elt
addIdx  n=300: 926 dns/elt  n=3000: 1108 dns/elt  n=30000: 1401 dns/elt  n=300000: 1760 dns/elt
mapSet  n=300: 938 dns/elt  n=3000: 1133 dns/elt  n=30000: 1424 dns/elt  n=300000: 1739 dns/elt
toList  n=300: 58 dns/elt  n=3000: 58 dns/elt  n=30000: 58 dns/elt  n=300000: 58 dns/elt
```

- `toSet` = a hand-written `add` loop = `Map` `set` loop, to within 2%. `toSet` has no
  cost of its own beyond its inserts. It is the per-insert cost, paid while growing
  from empty.
- `toList` (5.8 ns/element) is not a factor in the benchmark row: 20 × 300 × 5.8 ns
  ≈ 0.035 ms of the row's 0.583 ms.

Contrast `/tmp/dbB-addsplit` has the same project shape. Its three loops of N
`add`s × reps are: `fresh` (new set each rep), `existing` (every element already
present, so a probe hit with no write or grow) and `empty` (`acc = acc + 1`):

```
fresh n=300: 926 dns/op  n=30000: 1412 dns/op
existing n=300: 118 dns/op  n=30000: 338 dns/op
empty n=300: 21 dns/op  n=30000: 20 dns/op
```

A new element costs 80.8 ns more than a probe hit at N = 300. That is the write
plus growth.

Where the time goes, for `addFresh(300, 400000)` in `/tmp/dbB-addprof` (the same loop,
120 M inserts, 11.37 s user → 94.8 ns/insert):

- `sample <pid> 4` put 1934 of 2984 top-of-stack samples (64.8%) in the loop at image
  offsets `0x25e8`–`0x26ac`. `otool -tv` shows that loop is the PCG64 XSL-RR word
  generator (multiplier `0x2360ed051fc65da4_4385df649fccf645`) called from the tail of
  the arena free routine (`bl 0x1000025e8` after the bin push, with
  `x0 = ptr + 16, x1 = size − 16`). That is `_mfb_arena_fill_random` called by
  `_mfb_arena_free`.
- The same program with 1000 reps built `--debug` reports `arena.0.alloc_calls 9001`
  (9 allocations per set of 300: the empty literal plus 8 grows), `fill_free_calls 9000`
  and `fill_free_bytes 73,728,000`. That is **245.8 bytes scrubbed per inserted
  element**.
- Derived fill throughput: 245.8 B / (0.648 × 94.8 ns) ≈ **4.0 bytes/ns**.
- The benchmark `add` row's 63 ns/insert (0.019 ms / 300) against this probe's 93 ns is
  consistent with the benchmark dropping `nums` after its `t1`. The final block's scrub
  then falls outside the timed region. This is a guess: the attribution was not
  measured separately.

## Root Cause

Three layers, measured in the order of their share.

**1. Every freed grow block is entropy-scrubbed, one serial PCG64 step per 8 bytes
(~65% of insert time).**
`src/codegen/memory/arena/arena.rs:lower_arena_free` parks the chunk, then calls
`ARENA_FILL_RANDOM_SYMBOL` over `[ptr+16, ptr+size)`. That is the spec'd *Entropy
Fill*. `src/codegen/builtins/math/gen_rng_pcg64.rs:lower_arena_fill_random` emits one
PCG64 step (`emit_pcg_step`: 128-bit multiply-add) plus the XSL-RR output per 8-byte
word. In the linked binary the loop is 35 instructions per word, 8 of them
re-materializing the two 128-bit constants each iteration (`mov`/`movk` ×4 twice,
`otool -tv` at `0x2610`–`0x2698`). Every word also waits on the previous word's
128-bit multiply, so the loop is bound by the multiply chain's latency, not by
instruction count. Guess, supported by the C prototype below: interleaving
independent steps raises throughput with the same instructions per word.

A growing `Set`/`Map` frees each outgrown block (`map_mutate.rs` in-place `set`/`add`
grow path: `_mfb_arena_alloc`, copy, `_mfb_rt_drop_owned_collection` of the old block;
see `mapset_grow_48` … `mapset_write_49` in the `viaAddIdx` `.ncode`). With the doubling
shape (4 → 1024, then ×1.5; *Capacity Headroom and Growth*) and a 40-byte entry plus
8 data bytes plus 16 bucket bytes per slot, a set freed at the end of its life has had
~246 bytes per element scrubbed. C's `free` and CPython's set resize do not scrub.

C prototype of the same stream (`/tmp/dbB-fill/fill.c`, `cc -O2`, 4096-word fills,
best of 20 × 1000): single chain **5.36 B/ns**, 2-way jump-ahead **7.88 B/ns**,
4-way **9.33 B/ns**. All three produce the identical word sequence (checked word by
word). Jump-ahead means computing `s₊₁ = M·s + I` and `s₊₂ = M²·s + I(M+1)` (… `M⁴`)
from the same `s`, so the chains are independent and the stream is unchanged.

**2. `toSet` builds by growing from empty although `len(value)` bounds the result.**
`src/codegen/builtins/collections/func_to_set.rs` `BODY` is
`MUT result AS Set OF T = Set OF T { }` / `FOR EACH x IN value: result =
collections::add(result, x)` / `RETURN result`, monomorphized per call site
(`_mfb_ifn_collections_5FtoSet_24Integer`, 755 instructions, reached from `viaToSet` by
`-ncode`). For 300 elements that is 9 allocations, 8 full copies, 8 scrubs of the
outgrown block, and 8 invalidations of the hash index (a grow resets `bucketsReady`;
the next probe rebuilds the index, re-hashing every entry). A presized result needs 1
allocation, 0 copies, 0 rebuilds, and scrubs only the final block when the set dies.

**3. The insert path itself, ~20–30 ns (derived: 92.6 − 61.4 fill ≈ 31 ns minus the
amortized copy and rebuild).** In the `viaAddIdx` `-ncode`, a probe hashes the 8-byte
key with the byte-at-a-time FNV loop (`map_probe_hloop`, 6 instructions × 8 bytes)
then `udiv`/`msub` for the home slot. A new key then takes the `mapset_not_found` →
`mapset_write` path (~70 instructions, every temporary through a stack slot at -O1)
and calls `_mfb_rt_map_bucket_put`. A probe hit alone costs 11.8 ns (`existing`
above). This layer is the ordinary -O1 code quality of the map arms. It is listed for
completeness and is not the target of this bug.

## Goal

- G1 (`toSet` sizes once): a `--debug` build of a program that calls `toSet` R times
  on a 300-element list reports `arena.0.alloc_calls` rising by exactly R for
  those calls (today 9R). Same for a list with duplicates.
- G2 (fill throughput, same stream): `_mfb_arena_fill_random` emits the same PCG64
  word sequence as today for any length and alignment remainder. On the
  `/tmp/dbB-addprof` loop the fill's share of samples drops from 64.8% to ≤ 45%.
  Guess target, from the prototype's 1.74× (9.33 / 5.36) over a single chain.
- G3 (the checkable end-to-end behavior, `tests/runtime/rt_set_insert_cost.rs`): at
  N = 300, `toSet` per element ≤ 4× a probe-hit `add` per element in the same run
  (today 93.1 / 11.8 = 7.9×). Estimated after G1 + G2: ≈ 31–38 ns / 11.8 ns ≈ 2.6–3.2×
  (guess, from the decomposition above).
- G4 (benchmark, expected consequence, guess): `set (Fixed|Record-Fixed|State-Fixed)
  toSet` ≤ 0.23 ms at -O1 (≤ 7.5× C -O0, B grade; ≤ 4.1× Python). `set … add` /
  `map … set` rows improve by the fill speed-up only (they really do grow from empty).

### Non-goals (must NOT change)

Owner constraint: no change to the language specification or the memory-management
model.

- **The entropy fill stays, on every free and every fresh block, in debug and
  release.** Skipping, shortening or deferring the scrub of any freed payload byte
  (e.g. "don't scrub a block that only held integers", "skip the grow-chain frees",
  "scrub lazily on reuse"), or turning it off in release, changes `mfb spec memory
  arenas` *Entropy Fill*. That is a **blocker** for this bug, not a fix. The owner
  decision in `planning/todo.md` (plan-133-B item 2) is "the fill stays".
- **The fill stays the dedicated per-arena PCG64 stream** (same state words at arena
  offsets 16/24, same seeding, same `--debug` counters `fill_grow_*` / `fill_free_*`
  with the same values). Jump-ahead emits the same words in the same order, so the
  spec text stays true as written. Replacing the generator (xorshift, ChaCha, a
  per-block SplitMix) would need a spec edit and is out of scope. It would be an owner
  decision, not part of this fix.
- **Observable semantics:** `toSet` returns the same set (first-occurrence insertion
  order, same `len`, same errors: none of its own, `ErrOutOfMemory`). No new builtin,
  signature or syntax. `mfb man collections toSet` is unchanged; its description ("folds
  over `value` … adds each element") remains true of the result.
- **Memory model:** the result is still a new value (`toSet` stays pure; `value` is not
  mutated or aliased). Capacity is not observable (*Capacity Headroom*: "an
  implementation tuning detail"), and a copy of the result is still shrink-to-fit.
  The self-update contract and the allocator's bin/flush behavior are unchanged.
- **Growth shape of in-place `add`/`set`** (4 → 1024 doubling, then ×1.5) stays. Changing
  it trades memory for scrub bytes and is tuning, not this bug.
- **Forbidden:** editing the benchmark rows (e.g. timing `toSet` without the final
  drop), or making the C peer grow to "even out" the ratio.

## Blast Radius

Searches: `grep -ln "Set OF [A-Z] { }\|Map OF [A-Z] TO [A-Z] {}" src/codegen/builtins/*/func_*.rs`,
`grep -rn "ARENA_FILL_RANDOM_SYMBOL" src/codegen`.

- `func_to_set.rs` — fixed by this bug (G1).
- `func_map_values.rs` — `MUT result AS Map OF K TO U = Map OF K TO U {}` then one
  `set` per entry of `value`: the result size is **exactly** `len(value)`. Same fix
  applies. **In scope**: the presizing mechanism serves it with an exact count.
- `func_intersection.rs`, `func_difference.rs` (result ≤ `len(a)`),
  `func_symmetric_difference.rs` (≤ `len(a) + len(b)`) — grow from empty. Latent, same
  pattern, **out of scope**: presizing to an upper bound can over-allocate a mostly
  empty result, so each needs its own call. Not in the D/F list.
- `func_union.rs` — starts from `a`, so it grows at most log₂(1 + len(b)/len(a)) times.
  Unaffected by G1.
- `lower_arena_free` / `lower_arena_alloc` fresh-block fill — every free and every
  arena grow in every program calls the same helper, so G2 speeds up all of them.
  `planning/todo.md` measured fill-off at x0.70 geomean over the suite. **Fixed (made
  faster) by G2**, and the `.ncode` goldens that embed the helper body change (see Fix
  Design).
- `math::rand` (`gen_rng_pcg64.rs`, state at arena offsets 88/96) — shares
  `emit_pcg_step` but is an observable, reproducible stream. **Must not change.** The
  jump-ahead goes in the fill helper only.
- In-place `set`/`add` grow in `map_mutate.rs` (`lower_map_set_in_place`) —
  unchanged. It really does grow and is sped up only by G2.

## Fix Design

**G2, fill jump-ahead (the dominant layer, every program).** In
`lower_arena_fill_random`, hoist the multiplier/increment materialization out of the
loop and emit a K-way unrolled body (K = 4 recommended by the prototype). From the
loop-carried state `s`, compute `s·Mʲ + I·(Mʲ⁻¹ + … + 1)` for j = 1..K as independent
128-bit multiply-adds. Store the K XSL-RR words, and carry `s ← s·Mᴷ + …`. A
remainder loop (the existing single step) handles `count mod K`. The constants are
compile-time u128 values (compute them in Rust). The word sequence, the state written
back to offsets 16/24, and the `--debug` counters are identical. The risk sits in the
x86-64 lowering: `mul`/`umulh` clobber `rax`/`rdx`, as the helper's own comment notes.
Register pressure rises with K, so check the vreg allocator places K chains without
spilling on x86-64 and riscv64. Fall back to K = 2 where it spills.

**G1, `toSet` sized once.** Give `toSet` a native lowering that allocates the result
`Set` with `capacity = len(value)` (and the matching fixed-width data capacity and
bucket region), then runs the existing insert-without-grow path per element. This
follows the native builder lowerings already used for other collection builtins (the
`Body::Mfb` source body stays as the reference semantics and for any instantiation
the native path declines). Equivalent alternative: an internal capacity-hinted empty
literal that the monomorphized body uses. Recommended against, because it adds an
internal builtin surface for one caller; see Open Decisions. `mapValues` takes the
same presize with its exact count.

Rejected alternatives:

- **Skip or shrink the scrub** (only the dirty prefix, or none for the grow chain):
  blocker, changes *Entropy Fill* (Non-goals).
- **Grow in place** (extend the block into an adjacent free chunk instead of
  allocate + copy + free): the arena has no realloc-extend today. It would help every
  grow, but it is an allocator change with its own design risk. Its benefit here is
  unmeasured (guess). Out of scope; a separate plan if pursued.
- **Faster generator:** would need a spec edit (Non-goals).

Expected output shifts: the `.ncode` goldens that embed `runtime.arena_fill_random`
(its body only), and any golden with a `toSet` / `mapValues` call (the native
lowering replaces the call to `_mfb_ifn_collections_5FtoSet_*` / `…mapValues…`). No
`.ir`/`.mir` change, and no stdout change anywhere.

## Phases

### Phase 1 — failing tests + audit (no behavior change)

- [x] `tests/runtime/rt_set_insert_cost.rs`, modeled on
      `tests/runtime/rt_map_fifo_removal_flat.rs` (in-process timing, best of N, a
      ratio bound rather than an absolute time): `toSet` per element at N = 300 ≤ 4×
      probe-hit `add` per element. Also a `--debug` allocation-count case modeled on
      `tests/runtime/rt_debug_arena.rs` (G1: one allocation per `toSet`). Confirm both
      fail today (7.9×; 9 allocations).
- [x] `#[cfg(test)]` in `gen_rng_pcg64.rs`: the jump-ahead constants equal
      `Mʲ`, `I·Σ Mⁱ` computed with `u128`. A reference model of the unrolled loop over
      lengths 0..=64 bytes plus 4096 words produces the single-step sequence (passes
      trivially today; it is the Phase 2 guard).
- [x] Record the `mapValues` / set-algebra verdicts above as final.

Acceptance: the cost and allocation tests fail for the documented reasons.
Commit: —

### Phase 2 — the fix

- [x] `lower_arena_fill_random`: K-way jump-ahead, constants hoisted. Verify on
      aarch64, x86-64 (SysV and Win64) and riscv64 (`.ai/arch-abi.md`).
- [x] Native presized `toSet`; the same for `mapValues`.

Acceptance: Phase 1 tests pass; `/tmp/dbB-addprof` fill share ≤ 45% of samples; the
fill stream is identical; nothing in Non-goals changed.
Commit: —

### Phase 3 — regenerate expected outputs + full validation

- [x] Re-baseline only the goldens whose delta is the fill helper body or the
      `toSet` / `mapValues` call site. Inspect one fixture per class first.
- [x] Full suite (`cargo test`) before any golden re-baseline.
- [x] Benchmark `setops` / `mapmatrix` rows (G4), and a whole-suite geomean as a
      side-effect check of G2.
- [x] Spec sync: *Entropy Fill* may gain one sentence ("emitted K words per iteration,
      same stream"). `mfb spec` for `toSet`'s lowering if the collections spec lists
      native builders. Gate: `cargo build`, `cargo test --bin mfb spec`.

Acceptance: full suite green; golden deltas are exactly the intended ones; G3/G4 hold.
Commit: —

## Validation Plan

- Regression tests: `tests/runtime/rt_set_insert_cost.rs` (fails → passes); the
  `gen_rng_pcg64.rs` stream-identity unit test.
- Runtime proof: `/tmp/dbB-toset` shows `toSet` per element below `addEach` at every
  N. `/tmp/dbB-addprof` under `sample` shows the fill share ≤ 45%. The `--debug`
  counters `fill_free_bytes` for the same program are unchanged by G2 alone (same
  bytes scrubbed, faster).
- Doc sync: `mfb spec memory arenas` *Entropy Fill* (optional one sentence); no `mfb man`
  change.
- Full suite: `cargo test`; the benchmark suite for G4.

## Open Decisions

- `toSet` presize mechanism: a native lowering (recommended; no new surface) vs. an
  internal capacity-hinted literal used by the source body (smaller change, but a new
  internal builtin, which the owner constraint discourages).
- Unroll factor K: 4 (recommended by the aarch64 prototype) vs 2 (lower register
  pressure on x86-64). Decide per architecture from a spill check.
- Presizing the upper-bounded set-algebra bodies (`intersection`, `difference`,
  `symmetricDifference`): separate decision, not in this bug.

## Summary

The D grade on `toSet` and the small D grades on `add`/`set` are one cost: growing a
`Set`/`Map` from empty frees a chain of blocks, and every freed byte is re-randomized
by a latency-bound single-chain PCG64 loop at ~4 B/ns. The scrub is spec'd and stays.
The fix makes the same stream ~1.7× faster (prototype) and stops `toSet` /
`mapValues` from growing when the input bounds the result. The engineering risk sits
in the multi-architecture fill helper, not the `toSet` lowering. No observable
behavior, layout, allocator policy or spec rule changes.
