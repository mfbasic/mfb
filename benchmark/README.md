# Benchmarks

A cross-language micro-benchmark suite comparing **MFBASIC** against **C** (at
`-O0` and `-O2`) and **CPython**. Each language is a single self-contained
program that times every micro-benchmark internally and prints a grouped
`median / average / min / max` table in milliseconds.

- `mfb/`    — the MFBASIC project (`mfb build` → `benchmark.out`)
- `c/`      — compiled at `-O0` and `-O2`
- `python/` — run under `python3`
- `empty/`  — standalone process-startup benchmark (run `./empty/run.sh`)

Each language program is split into one file per package surface so the coverage
for each package lives on its own (the same split in all three):

| file                | group(s)          | what it exercises |
|---------------------|-------------------|-------------------|
| `main.*`            | recurse, float, record, bignum, parse, io, primes, thread + driver | the cross-language reference workloads (C's `parse` lives in `parsebench.c`) |
| `list.*`            | `list`, `liststr` | every `collections::` list op over **Integer** lists and over **String** lists |
| `map*.* `           | `map`             | every map-shaped `collections::` op over **Integer-valued** and **String-valued** maps |
| `math*.*`           | `math`            | the libm-severed Float kernels + coverage of every `math::` member across Integer / Float / Fixed and the array (SIMD) overloads |
| `vector*.*`         | `vector`          | every `vector::` member across the Float / Fixed / Integer families |
| `bits*.*`           | `bits`            | every `bits::` bitwise / shift / rotate op |
| `string*.*`         | `string`          | `&` concat + every `strings::` member (case, search, slice, Unicode) |

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
API-surface coverage check. Two kinds of asymmetry are intentional:

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
  where mfb uses its PCG, and the `string unicode` grapheme/normalization counts
  are approximated in C/Python, so those checksums are stable but not expected to
  match mfb bit-for-bit.

### Arena-churn caveat (two coverage-only rows)

The `string unicode` row and the `liststr reshape` row use **deliberately tiny**
iteration counts. MFBASIC's runtime arena free list degrades quadratically under
mixed-size **transient** churn — the short-lived `List`/`String` temporaries that
`strings::graphemes`/`graphemeAt`/`graphemesCount`/`toBytes`/`normalizeNfc`
allocate, and the String copies that `collections::sort`/`window` make — and the
degradation is process-global and cumulative across the `run` loop (a fresh row
starts fast, each repeat gets dramatically slower). A few hundred such
allocations stay in the linear regime; tens of thousands hang the suite for
minutes. Both rows are therefore small coverage smoke-tests of their surface, not
throughput measurements. (This is a runtime arena regression, not a property of
the benchmarked code; the C/Python mirrors keep the same tiny counts only so the
table lines up.)
