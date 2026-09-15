# oracles/compress — the `compress` builtin against zlib

`compress` is written in MFBASIC and calls no system library, so it produces the
same bytes on every target — and, like `crypto`, it has no second opinion of its
own. This oracle is that second opinion: every operation is played against **two**
independent zlib builds, Python's stdlib `zlib` and Node's built-in `node:zlib`.

**Offline tooling only.** Nothing here is linked into the compiler or runtime, and
nothing here runs in CI. The in-CI cross-check is
`tests/interop/rt_compress_interop.rs` (against `flate2`, already a dependency);
this oracle covers the larger corpus and the second zlib that CI does not.

## Run

```
tools/oracles/compress/run.sh [path-to-mfb] [mode ...]
```

With no modes, every mode runs. With no `mfb` path it uses `target/release/mfb`
(building it if missing). Exit codes, shared with `tools/oracles/crypto/`:
`0` every case agreed with both judges, `1` a disagreement, `2` the harness could
not run (a build failed, a judge died, or a case count was wrong).

Needs `python3` (any build with `zlib`) and Node ≥ 22.2 (for `zlib.crc32`). No npm
install and no pip install.

## Modes

| Mode | Cases | What is compared |
|---|---|---|
| `crc32` | 118: lengths 0–17, then 100 random lengths up to 1 MiB | `compress::crc32(data)` in one call, and chained across a random split (`crc32(tail, crc32(head))`), against `zlib.crc32` |

## How it fits together

| Piece | Role |
|---|---|
| `python/gen.py <mode> <job>` | Writes a seeded job file and prints its case count |
| `mfb/` | The subject: reads the job, prints `case <index> <fields...>` per case |
| `python/oracle.py <mode> <job>` | Judge 1 — stdlib `zlib` / `gzip` |
| `node/oracle.mjs <mode> <job>` | Judge 2 — `node:zlib` |
| `run.sh` | Builds the subject, runs all three on each mode's job, compares line by line |

The subject and the judges **share no case table**: the generator writes the job
and every side reads the same file. The expected case count per mode is declared
in `run.sh` (`expected_cases`), never counted from a subject's output — a program
that stopped after case 3 would otherwise report "3 of 3 agreed".

Job layout (little-endian `u32`): the case count, then a `(length, aux)` pair per
case, then every case's bytes back to back. `aux` is per mode (the split point for
`crc32`).

## Adding a mode

Add the mode in all five places, or `run.sh` exits 2:

1. `python/gen.py` — a `MODES` entry returning `(data, aux)` cases.
2. `python/oracle.py` — a `MODES` entry printing `case <index> <fields...>`.
3. `node/oracle.mjs` — the same, from `node:zlib`.
4. `mfb/src/main.mfb` — a branch in `runJob` printing the same fields.
5. `run.sh` — the name in `ALL_MODES` and its count in `expected_cases`.
