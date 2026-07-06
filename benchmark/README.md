# Benchmarks

A cross-language micro-benchmark suite comparing **MFBASIC** against **C** (at
`-O0` and `-O2`) and **CPython**. Each language is a single self-contained
program that times every micro-benchmark internally and prints a grouped
`median / average / min / max` table in milliseconds.

- `mfb/`    — the MFBASIC project (`mfb build` → `benchmark.out`)
- `c/`      — one `main.c`, compiled at `-O0` and `-O2`
- `python/` — one `main.py`, run under `python3`
- `empty/`  — standalone process-startup benchmark (run `./empty/run.sh`)

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

> Note: the C program does **not** implement the `parse` group (csv/json/regex),
> so those rows exist only for `mfb` and `python`.

## MVP goals

The compiler MVP targets two bars:

1. **Everything is faster than Python** (mfb median < python median, every row).
2. **Math is within ±1 ms of C `-O0`** (unoptimized C) — `|mfb − c‑O0| ≤ 1 ms`.

## Current status — run `20260705-130531`

Scored on the median column. ✅ meets the bar, ❌ misses it.

### Goal 1 — faster than Python

Passing comfortably: all `recurse`, all `float`, all `math`, `distinct`, `set`,
map `set`, string `concat`, record `update`, `vector`, `primes`, `thread`.

Still **slower than Python** (❌):

| bench            | mfb (ms) | python (ms) | note |
|------------------|---------:|------------:|------|
| io read          | **7141** |        1.11 | ❌ pathological — 7 s with a 3.6–10.6 s spread; almost certainly a bug, not slow codegen |
| bignum modmul    |   239.4  |      192.9  | ❌ loses to interpreted Python |
| bignum modexp    |   130.8  |      106.8  | ❌ loses to interpreted Python |
| list copy        |    32.35 |        2.50 | ❌ ~13× (verbatim-copy path) |
| io write         |    27.52 |        2.49 | ❌ |
| parse csv        |     5.76 |        0.86 | ❌ |
| parse json       |     4.79 |        0.23 | ❌ |
| parse regex      |     4.60 |        0.017| ❌ |
| groupby          |     4.65 |        0.13 | ❌ value-grow churn |
| map lookup       |     3.00 |        1.44 | ❌ |
| list prepend     |     1.91 |        0.31 | ❌ |
| list append_batch|     0.905|        0.005| ❌ |
| list append      |     0.051|        0.035| ❌ (sub-0.1 ms — likely noise) |
| list sort        |     0.011|        0.003| ❌ (sub-0.1 ms — likely noise) |

The collection micro-ops fight CPython's hand-tuned C list/dict; the meaningful
misses are `io read`, `bignum`, `list copy`, `io write`, `parse`, and `groupby`.

### Goal 2 — math within ±1 ms of C `-O0`

**Not met on any op.** MFBASIC's software kernels run 3–8× C‑O0:

| op    | mfb (ms) | c‑O0 (ms) | Δ vs c‑O0 |
|-------|---------:|----------:|----------:|
| sqrt  |    8.87  |    7.57   |   +1.3 (closest) |
| exp   |   19.29  |    8.14   |  +11.2 |
| atan  |   24.04  |   14.45   |   +9.6 |
| atan2 |   30.57  |   13.95   |  +16.6 |
| asin  |   28.92  |   10.21   |  +18.7 |
| acos  |   29.71  |    8.87   |  +20.8 |
| sin   |   32.19  |    8.09   |  +24.1 |
| cos   |   32.55  |    8.01   |  +24.5 |
| log   |   32.43  |    8.04   |  +24.4 |
| log10 |   34.10  |    8.07   |  +26.0 |
| tan   |   72.36  |    9.43   |  +62.9 |
| pow   |   96.05  |   15.59   |  +80.5 |

`sqrt` is the only op close (1.3 ms over). The rest need substantial kernel
throughput work to reach the ±1 ms bar.

## Summary

- **Blocker:** `io read` at ~7 s is an anomaly to fix first (buggy read loop).
- **Goal 1 gaps:** bignum (loses to Python), list copy, io write, parse, groupby,
  map lookup, list prepend/append_batch.
- **Goal 2:** every transcendental is well outside ±1 ms of C‑O0; `sqrt` is nearest.
