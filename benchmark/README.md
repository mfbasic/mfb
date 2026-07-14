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

In addition to that per-member surface, a second set of **pattern-throughput**
groups (plan-40) exercises the hot paths real programs hit — sustained churn,
chained pipelines, compile-once/run-many — rather than one call per member:

| group(s)            | what it exercises |
|---------------------|-------------------|
| `mapchurn`          | map grow/rehash, steady-state insert/`removeKey` churn, and `keys`/`values`/`mapValues`/`merge` materialization in a loop |
| `listchurn`         | build-by-`append`, `prepend` front-shift, and nested `List OF List` build + `flatten` + `groupBy` |
| `float matmul` + `mathpipe` | dense N×N `Float` matmul; a naive DFT (sin/cos interleaved with float ops); mean/variance/stddev reduction; and `finance`, an mfb-only `Money` running-balance calc |
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

The pre-existing `string unicode` and `liststr reshape` rows are small coverage
smoke-tests for this reason. The plan-40 rows carrying a `TODO(plan-39-A)` marker
— `string unibig`, `io binary`, the whole `arena` group, and `scalarbench
roundtrip`/`transform`/`listchurn` — are authored tiny **on purpose**: they are
the regression gate for the arena fix (plan-39-A). When that lands, each is bumped
from tiny to realistic size in the same commit and must stay **linear** — that
jump is the fix's acceptance criterion. (This is a runtime arena regression, not a
property of the benchmarked code; the C/Python mirrors keep the same tiny counts
only so the table lines up.)
