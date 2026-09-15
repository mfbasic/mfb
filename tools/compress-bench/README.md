# compress-bench — throughput of the `compress` builtin

Measures each `compress` operation's MiB/s beside Python's `zlib` on the same bytes,
and checks that time grows linearly with input size. **Offline tooling only**: not
in CI, not a gate. Its numbers are recorded, dated, in the plan that changed the
operation (plan-137 letters record theirs in their Corrections sections) and in
`mfb spec stdlib compress`.

## Run

```
tools/compress-bench/run.sh target/release/mfb [op ...]
OPT_LEVELS="1" tools/compress-bench/run.sh target/release/mfb crc32
```

Needs `python3`. Builds `mfb/` once per optimization level (default `-O1` and `-O3`).

## What it measures

- **Corpus**: three kinds — seeded pseudo-random bytes, repetitive numbered text
  lines, all zeros — at 1, 4 and 16 MiB, generated fresh each run.
- **MFB time**: the program reads the file, then runs the op five times, timing each
  run with `datetime::monotonicNanos` around the op alone; the row shows the median.
  File reading and process start-up are excluded, which a whole-process timer such
  as `/usr/bin/time -p` (10 ms resolution) could not do at 1 MiB.
- **Python time**: the median of five in-process calls on the same bytes.
- **Correctness**: every MFB result must equal Python's. A mismatch fails the row.
- **Linearity**: for each op, corpus kind and level, the 16 MiB median must be at
  most 4.4× the 4 MiB median.

Exit `0` all rows correct and linear, `1` a mismatch or a non-linear row, `2` the
harness could not run.

Encoder ops (`deflate1`, `deflate6`, `deflate9`: `compress::deflate` at that level) time the
compression alone. Their correctness check compares what decompressing each side's output
gives, since `compress` does not reproduce zlib's bytes. Each row also shows both compressed
sizes and their ratio.

## Adding an op

1. `bench.py` — an `OPS` entry: a Python function over the bytes returning the
   result as a string.
2. `mfb/src/main.mfb` — a branch in `runOnce` returning the same string.
