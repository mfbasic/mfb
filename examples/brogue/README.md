# brogue

A port of Brogue CE's dungeon generator to MFBASIC, written to exercise the
language and its runtime against a known-correct reference.

Brogue generates each dungeon level deterministically from a per-level seed,
using integer and fixed-point arithmetic only, so for a given seed the port
must reproduce the original's output exactly. Any difference is a bug in the
port or in MFBASIC.

## Layout

| Path | Contents |
|---|---|
| `src/main.mfb` | Entry point and the dump modes the checks compare |
| `src/rng.mfb` | Brogue's RNG (`oracle/src/brogue/Math.c`), ported bit for bit |
| `check/` | C drivers built from the oracle's own source, and the scripts that diff them against the port |
| `oracle/` | Brogue CE's C source, the reference implementation |

## Building and checking

```console
$ mfb build examples/brogue
$ ./examples/brogue/build/brogue.out rng 12345 20
$ examples/brogue/check/check-rng.sh
```

`check-rng.sh` compiles `check/rng_dump.c` against the oracle's `Math.c`, runs
it and the port over a fixed seed list covering both of Brogue's seed paths
(below and above 2^32), and diffs every draw. It exits 1 on the first mismatch.

## oracle/

- Upstream: https://github.com/tmewett/BrogueCE
- Pinned commit: `dedc315833c93826f632cd47fd87c0eb35541bcf` (2026-09-20)
- License: AGPL-3.0 (`oracle/LICENSE.txt`)
- Local changes: the upstream `.github/` CI directory is removed; the source is
  otherwise unmodified. Harness code lives in `check/`, not in `oracle/`.
