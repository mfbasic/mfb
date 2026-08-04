# Benchmarks

A cross-language micro-benchmark suite comparing **MFBASIC** against **C** (at
`-O0` and `-O2`) and **CPython**. Each language is a single self-contained
program that times every micro-benchmark internally and prints a grouped
`median / average / min / max` table in milliseconds.

- `mfb/`    — the MFBASIC project (`mfb build` → `build/benchmark.out`)
- `c/`      — compiled at `-O0` and `-O2`
- `python/` — run under `python3`
- `empty/`  — standalone process-startup benchmark (run `./empty/run.sh`)

Each language program is split into one file per package surface so the coverage
for each package lives on its own (the same split in all three):

| file                | group(s)          | what it exercises |
|---------------------|-------------------|-------------------|
| `main.*`            | recurse, float, record, bignum, parse, io, primes, thread + driver | the cross-language reference workloads (C's `parse` lives in `parsebench.c`) |
| `list.*`            | `list (Fixed/Dynamic)`, `list (Record-Fixed/Dynamic)`, `list (State-Fixed/Dynamic)` | every `collections::` list op replicated across a 6-way matrix (bug-430): **Integer** (`Fixed`) vs **String** (`Dynamic`) element, held in a plain `MUT` local vs a record field (fields before + after the list) vs a `File STATE` field — the wrapped groups expose the whole-record-rebuild O(n²) on mutation ops. mfb-internal (no C/Python peers) |
| `map*.* `           | `map`             | every map-shaped `collections::` op over **Integer-valued** and **String-valued** maps (String keys) |
| `mapmatrix.*`       | `map (Fixed/Dynamic)`, `map (Record-Fixed/Dynamic)`, `map (State-Fixed/Dynamic)`, `map (key-Fixed/key-Dynamic)` | the map value-matrix (bug-430): each map op (`set`/`get`/`getOr`/`hasKey`/`removeKey`/`keys`/`values`/`mapValues`/`merge`) split one-per-function and replicated across the same 6-way container grid as `list`/`set`, varying the **value** type (Integer `Fixed` / String `Dynamic`) with the **key fixed at Integer**. Plus a **key-hash pair** — `map (key-Fixed)` (Integer key) and `map (key-Dynamic)` (String key), both Integer value, plain standalone only (the key-hash path is container-independent, so no Record/State split; `key-Fixed` is the same workload as `map (Fixed)`). `mapValues`/`merge` are map-producing (reduced size, arena caveat below). mfb-internal (no C/Python peers) |
| `math*.*`           | `math`            | the libm-severed Float kernels + coverage of every `math::` member across Integer / Float / Fixed and the array (SIMD) overloads |
| `vector*.*`         | `vector`          | every `vector::` member across the Float / Fixed / Integer families |
| `bits*.*`           | `bits`            | every `bits::` bitwise / shift / rotate op, **one row per op** (`band`/`bor`/`bxor`/`bnot`/`sl`/`sr`/`sra`/`rl32`/`rr32`/`rl64`/`rr64`/`clz`/`ctz`/`popCount`/`bswap16`/`bswap32`/`bswap64`); Integer-only, so no Fixed/Dynamic split. Checksums match mfb/C/Python bit-for-bit |
| `string*.*`         | `string`          | `&` concat + every `strings::` member (case, search, slice, Unicode) |
| `encoding*.*`       | `encoding`        | `encoding::` serialize/deserialize round-trips (base64 / hex / percent) over the `List OF Byte` ↔ String seam |
| `datetimeb.* / datetimebench.*` | `datetime` | `datetime::` civil arithmetic (addDays/addMonths/daysInMonth/between) and an ISO format/parse round-trip |
| `dispatch*.*`       | `dispatch`        | control-flow dispatch: union + `MATCH` tag-dispatch expression eval, and inline-`TRAP` error recovery |
| `crypto*.*`         | `crypto`          | `crypto::` hashes (SHA-256/512), HMAC-SHA-256, PBKDF2, constant-time compare, a fresh-message hash churn, and deterministic Ed25519 sign+verify — the portable software core over `bits`, cross-checked against `hashlib`/`hmac`/pyca |
| `serialize*.*`      | `serialize`       | `json::stringify` / `csv::stringify` — the encode direction (recursive tree walk + escape + number/field rendering) complementary to the `parse` group |
| `setops*.*`         | `set (Fixed/Dynamic)`, `set (Record-Fixed/Dynamic)`, `set (State-Fixed/Dynamic)` | the `Set OF T` collection type (plan-63), each op split one-per-function (`add`/`remove`/`contains`/`toList`/`toSet`/`union`/`intersection`/`difference`/`symmetricDifference`/`isSubset`/`isSuperset`/`isDisjoint`) and replicated across the same 6-way matrix as `list` (bug-430) — Integer/String element × plain/record/`STATE` container. Set-producing rows run at a reduced size (arena-churn caveat below). mfb-internal (no C/Python peers) |

In addition to that per-member surface, a second set of **pattern-throughput**
groups (plan-40) exercises the hot paths real programs hit — sustained churn,
chained pipelines, compile-once/run-many — rather than one call per member:

| group(s)            | what it exercises |
|---------------------|-------------------|
| `mapchurn`          | map grow/rehash, steady-state insert/`removeKey` churn, and `keys`/`values`/`mapValues`/`merge` materialization in a loop |
| `listchurn`         | build-by-`append`, `prepend` front-shift, and nested `List OF List` build + `flatten` + `groupBy` |
| `float matmul` + `mathpipe` | dense N×N `Float` matmul; a naive DFT (sin/cos interleaved with float ops); mean/variance/stddev reduction; a bottom-up coin-change `memo` DP over a `List` memo table; and two mfb-only `Money` rows — `finance` (running balance) and `money` (per-line tax/tip pipeline) |
| `strbuild`          | `&`-concat vs `strings::join` string building, `split`/`join` round-trip, and a `replace`/`trimChars`/`stripPrefix`/`padLeft` cleaning chain |
| `regexbench`        | compile-once/match-many, capture-group rewrite, `\|`-alternation find-all, and pattern-driven replace |
| `arena`             | mixed-size transient-churn / long-lived+short-lived / grow-shrink — the **regression gate for the arena free list** (see below) |
| `scalarbench`       | the `Scalar` primitive (plan-41): string↔`List OF Scalar` round-trip, `is*` classification sweep, `toInt`/`toScalar` transform pipeline, and the 4-byte `List OF Scalar` payload width |

The `io` group also gains `readnum` (read+parse), `buf_on`/`buf_off` (buffered vs
unbuffered write, quantifying the buffering win), `format` (mixed Int/Float/String
formatting), and `binary` (`strings::toBytes` + `fs` byte round-trip); the `string`
group gains `unibig` (realistic-size Unicode churn). These new rows live in
per-theme files (`mapchurn.*`, `listchurn.*`, `mathpipe.*`, `strbuild.*`,
`regexbench.*`, `arena.*`, `scalarbench.*`) mirrored across all three languages.

Three more per-member groups extend the existing surface (plan-45): the `map`
group gains `intkey` (Integer-keyed `hasKey`/`get`/`getOr` sweep — the distinct
Integer hash/probe path), `intchurn` (sliding-window insert + `removeKey` over
Integer keys), and `listagg` (group N ints into a `Map OF List` and append into
buckets); the `list` group gains `sort_asc`/`sort_desc`/`sort_rand` (`collections::sort`
over pre-sorted / reverse / coprime-stride-scrambled permutations of `0..N-1` — the
merge sort's best/worst/average shapes, all sorting to the same canonical order so
one shared order-sensitive checksum proves each shape sorted correctly).

Two critical-feature groups (plan-65) close remaining coverage gaps. The `crypto`
group is the first benchmark of the `crypto::` package — a portable software core
over `bits`, so hashing/HMAC/KDF/AEAD is a pure integer/bit-shuffling throughput
hot path with deterministic, byte-identical output and exact peers: `sha256`/
`sha512` bulk hash, `hmac` (HMAC-SHA-256), `pbkdf2` (PBKDF2-HMAC-SHA-256 work
factor), `cte` (`constantTimeEqual`), `churn` (hash many fresh small messages), and
`ed25519` (deterministic RFC-8032 sign+verify with a fixed seed). Every row folds
the digest/tag/signature bytes into an integer checksum that matches the C
(hand-rolled FIPS/RFC cores) and Python (`hashlib`/`hmac`/pyca) columns bit-for-bit
— the cross-language checksum is the proof of correctness. The `serialize` group
benchmarks the encode direction the `parse` group never touched: `json`
(`json::stringify` a pre-built tree), `roundtrip` (`json::parse`+`stringify`), and
`csv` (`csv::stringify` a String grid, quoting only where needed). Its checksum is
the length of the emitted text (order-independent, so it matches even when json
object members emit in a different order); the canonical inputs use only ASCII
strings and integers so every compact serializer produces the same length.

The `set (*)` groups benchmark the `Set OF T` collection type (plan-63) across the
same 6-way container matrix as `list` (bug-430). Each op of the set surface is
split one-per-function — `add`/`remove` (mutation), `contains`/`toList`/`isSubset`/
`isSuperset`/`isDisjoint` (bool/list read), and the source-generic algebra
`toSet`/`union`/`intersection`/`difference`/`symmetricDifference` (set-producing) —
run over a `Set OF Integer` (`Fixed`) and a `Set OF String` (`Dynamic`) element,
held in a plain `MUT` local, a record field (fields before + after the set), or a
`File STATE` field. The wrapped groups exercise the whole-record-rebuild path: a
`Set`-in-record/`STATE` mutation rebuilds and re-inlines the whole set on every
step, so the `add`/`remove` rows show the bug-430 O(n²) against the plain baseline.
Each row folds set sizes into an integer checksum that matches across all three
containers (the matrix's correctness proof); unlike the former bundled `set`
group, these rows are mfb-internal with no C/Python peer, and the set-producing
algebra rows run at a reduced size (arena-churn caveat below).

Only two crypto rows measure at realistic sizes today — `sha256` and `cte`, whose
transients stay in the arena quick bins and whose per-call cost is flat across the
run loop. The rest (`sha512`, `hmac`, `pbkdf2`, `churn`, `ed25519`) and the whole
`serialize` group are arena-gated (see below): mfb's crypto/serialize cores
allocate transient `List OF Byte`/`String` values per operation, so at realistic
sizes they hit the runtime arena's quadratic free-list path and climb cumulatively.
They are authored tiny with a `TODO(plan-64-A)` marker, to be bumped to realistic
sizes in the commit that lands the arena fix. Ed25519 has no in-suite C peer
(deterministic RFC-8032 signing has no libc entry point the suite hand-rolls), so
its C column prints `--`; mfb and Python agree because the seed is fixed.

## Running

```sh
./benchmark/run.sh                 # 10 iterations per test (default)
./benchmark/run.sh --run 50        # 50 iterations per test
./benchmark/run.sh 50              # shorthand
BENCH_RUNS=50 ./benchmark/run.sh   # environment override
```

`run.sh` builds all four targets, runs each in turn, echoes its table, and
writes a timestamped log per target:

```
mfb-<ts>.log   c-O0-<ts>.log   c-O2-<ts>.log   python-<ts>.log
```

Logs, built `*.out` binaries, and generated `*.mfp` packages are git-ignored.
**Prefer the median** — the average is dragged up by occasional OS-scheduling
outliers. Use a higher `--run` (e.g. 50+) when you care about the stats
columns; a single-sample run leaves `median == average`.

## Coverage vs. throughput

Every `collections::`, `math::`, `vector::`, `bits::`, and `strings::` member is
invoked with every element/numeric type it accepts, so the suite doubles as an
API-surface coverage check. Two kinds of asymmetry are intentional.

The mfb-only rows (`math fixed`, `vector fixed`, `mathpipe finance`, `mathpipe
money` — see below) still appear in the C and Python tables, printed as `--`, so
every target's output has the **same rows in the same order** and lines up
row-for-row. In C a `record(group, name, NULL, 0)` call emits the placeholder; in
Python `record(group, name, None)` does.

- **`parse` group (csv/json/regex)** — C has no standard-library CSV or JSON
  parser, so `parsebench.c` vendors two widely-used single-purpose libraries:
  [parson](https://github.com/kgabis/parson) (MIT) for JSON and
  [libcsv](https://github.com/rgamble/libcsv) (LGPL-2.1) for CSV. Regex needs no
  dependency — POSIX `<regex.h>` (`regcomp`/`regexec`) is in libc. All three
  languages build the same materialized structure (CSV grid, JSON DOM) and
  produce matching checksums (`csv=6003000`, `json=5000`, `regex=200`). The
  vendored sources are committed alongside the hand-written bench files.
- **`Fixed`-typed rows** — `math fixed` and `vector fixed` — exist only for
  `mfb`. C and Python have no fixed-point type, so those rows have no
  cross-language counterpart. (`math simd` and `vector math`/`float` operate on
  `Float` arrays, not `Fixed`, and are implemented in all three languages.) The
  `math int` and `vector int` rows use a self-contained deterministic generator
  where mfb uses its PCG, and the `string unicode`/`string unibig`
  grapheme/normalization counts are approximated in C/Python, so those checksums
  are stable but not expected to match mfb bit-for-bit.
- **`mathpipe finance`** — a `Money` running-balance calc — is mfb-only, like the
  `Fixed` rows (C/Python have no exact base-10 decimal type). It marks `Money` as
  a tracked feature.
- **`scalarbench` classification/transform** run over **ASCII** input so the five
  `is*` counts and the ROT-13 codepoints match across a libc/Python/mfb triple
  (non-ASCII Unicode classification is not guaranteed identical across all three);
  mfb still pays its Unicode-category-table lookup per scalar. The `roundtrip` and
  `listchurn` rows use a mixed-script string but compare only scalar counts and
  code-point order, which do agree everywhere.
- **`regexbench`** inputs are small (≈ the `parse regex` row): mfb's regex cost
  grows quadratically in text length, so the rows exercise distinct *shapes*
  (capture / alternation / replace) rather than large volumes, and the match
  counts still match across all three (ASCII patterns).

### Arena-churn caveat + regression gate (plan-39-A)

Several rows use **deliberately tiny** iteration counts because MFBASIC's runtime
arena free list degrades quadratically under mixed-size **transient** churn — the
short-lived `List`/`String` temporaries that
`strings::graphemes`/`graphemeAt`/`graphemesCount`/`toBytes`/`normalizeNfc`/`toScalars`
allocate, and the String copies that `collections::sort`/`window` make. The
degradation is process-global and cumulative across the `run` loop (a fresh row
starts fast, each repeat gets dramatically slower): a few hundred such allocations
stay linear; tens of thousands hang the suite for minutes.

The pre-existing `string unicode` row is a small coverage smoke-test for this
reason (the former `liststr reshape` smoke-test is superseded by the reduced-size
rows in the collection-container matrix documented below). The plan-40 rows carrying a `TODO(plan-39-A)` marker
— `string unibig`, `io binary`, the whole `arena` group, and `scalarbench
roundtrip`/`transform`/`listchurn` — are authored tiny **on purpose**: they are
the regression gate for the arena fix (plan-39-A). When that lands, each is bumped
from tiny to realistic size in the same commit and must stay **linear** — that
jump is the fix's acceptance criterion. (This is a runtime arena regression, not a
property of the benchmarked code; the C/Python mirrors keep the same tiny counts
only so the table lines up.)

The plan-45 rows carrying a `TODO(plan-44-J)` marker — `encoding base64`,
`datetime iso`, and `map intchurn` — are the same kind of gate for the current
arena sub-plan (plan-44-J, successor to plan-39-A): each allocates a fresh
`List`/`String`/`Map` per call and is authored tiny on purpose, to be bumped to a
realistic `BUF`/`reps`/`W`/`steps` in the commit that lands plan-44-J and must
stay linear. (`encoding hex`/`percent`, `datetime civil`, `map intkey`/`listagg`,
`mathpipe memo`, and the `list sort_*` rows are Phase 1 — not arena-sensitive at
their authored sizes — and run at realistic counts today.)

The plan-65 rows carrying a `TODO(plan-64-A)` marker are the same kind of gate for
the current arena sub-plan (plan-64-A, successor to plan-44-J): the `crypto`
`sha512` (`reps`), `hmac` (`reps`), `pbkdf2` (`iterations`), `churn` (`msgs`), and
`ed25519` (`reps`) rows, plus the whole `serialize` group (`json`/`roundtrip`/`csv`
`reps`). mfb's software crypto/serialize cores allocate transient `List OF
Byte`/`String` values per operation, so at realistic sizes they cross the arena
quick-bin threshold (SHA-512's >2 KB working set) or fill the quick bins by sheer
volume (PBKDF2's thousands of HMAC passes), hitting the O(n²) free-list path and
climbing cumulatively. At their realistic target sizes they explode in-suite —
`sha512` reps=64 → 1.6 s (26 ms in a clean process), `pbkdf2` iters=4096 → ~8 s
(224 ms clean), `hmac` reps=64 climbing 180 ms → 1300 ms across `--run 10` — so
each is authored tiny on purpose, to be bumped to a realistic `reps`/`iterations`
in the commit that lands plan-64-A and must stay linear. (Only `crypto sha256` and
`crypto cte` are Phase 1 — their transients stay in the quick bins and their
per-call cost is flat across the run loop — and run at realistic counts today.)

The bug-430 **collection-container matrix** (`list.mfb`, `setops.mfb` — each
collection op replicated across a `Fixed`/`Dynamic` × plain/`Record`/`State` grid)
is arena-churn-sensitive on its **collection-producing** rows, which allocate a
fresh `Set`/`List` per iteration. These are authored at deliberately small sizes
as coverage smoke-tests (a `union` of two 1000-element `Set`s at 200 reps takes
~12 s in-suite from cumulative free-list degradation, but ~0.1 s at the reduced
size). When the arena fix lands (successor to plan-64-A), the reduced rows below
bump to full size and must stay **linear**. Affected items:

- **`set (*)` set-producing ops — every group** (`Fixed`, `Dynamic`, `Record-Fixed`,
  `Record-Dynamic`, `State-Fixed`, `State-Dynamic`): `toSet`, `union`,
  `intersection`, `difference`, `symmetricDifference` run at a small `alg` size
  (Integer 300 elems × 20 reps; String 100 × 15).
- **String set groups** (`set (Dynamic)`, `set (Record-Dynamic)`,
  `set (State-Dynamic)`): all remaining ops also run at reduced sizes (base 200 vs
  the Integer groups' 1000; `add`/`remove` 400 vs Integer's 300 accumulation cap)
  because `Set OF String` churn allocates String bytes on top of the set blocks.
- **String list reshape-family rows** (`list (Dynamic)`, `list (Record-Dynamic)`,
  `list (State-Dynamic)`): `sort`, `window`, `zip`, `flatten`, `chunks` allocate
  many short-lived nested `List OF String` temporaries — lower-risk than the set
  rows (they run at full size today) but the same family, and candidates to shrink
  if they degrade at higher `--run` counts.
- **`map (*)` map-producing ops — every group**: `mapValues`, `merge` allocate a
  fresh `Map` per iteration, so they run at a small `prod` size (Integer 300 × 20
  reps; String 100 × 15). The String-valued groups (`map (Dynamic)`,
  `map (Record-Dynamic)`, `map (State-Dynamic)`) also run their other ops smaller
  (read base 200 vs the Integer groups' 1000; `set`/`removeKey` 400 vs 300).

Not affected — realistic sizes today: the whole `list (Fixed)`/`(Record-Fixed)`/
`(State-Fixed)` Integer matrix, and the Integer set groups' cheap ops (`add`,
`remove`, `contains`, `toList`, `isSubset`/`isSuperset`/`isDisjoint`), whose
transients stay in the arena quick bins.

**Comparability caveat — the `Dynamic` (String) and `Fixed` (Integer) groups are
sized differently on purpose** (for `list`, `set`, `map`, and any later matrix).
Because the arena mixed-transient-churn ceiling binds far sooner on `String`
elements, the `Dynamic` groups run at smaller base sizes and rep counts than their
`Fixed` twins (e.g. set algebra Integer 300×20 vs String 100×15; set read base
1000 vs 200; set `add`/`remove` Integer 300 vs String 400). Consequently the
**container axis is apples-to-apples** — compare `Fixed` vs `Record-Fixed` vs
`State-Fixed` (all one size) to read the bug-430 whole-record-rebuild cost — but
the **element axis is not**: a `Fixed` row and the corresponding `Dynamic` row are
running different-sized workloads, so a raw `Fixed`-vs-`Dynamic` time difference is
mostly the size gap, not an Integer-vs-String property. (This is why, e.g.,
`set (Fixed) union` reads ~110 ms against `set (Dynamic) union` ~7 ms — the Integer
set is 3× larger — while `add`/`remove` run the other way because String is both
larger there and heavier per element.) The per-element sizes live in the
`INT`/`STR` tables of `gen_list.py` / `gen_set.py` / `gen_map.py`.
