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
| `decode-raw` | 150: 3 corpora (text, random, mixed runs) × levels 0–9 × 5 strategies (default, filtered, Huffman-only, RLE, fixed) | length and CRC-32 of `compress::inflate` over Python zlib's raw DEFLATE (`wbits -15`), against Python `zlib.decompressobj(-15)` and Node `inflateRawSync` |
| `decode-zlib` | 150: the same matrix, zlib-wrapped (`wbits 15`) | length and CRC-32 of `compress::zlibDecode`, against Python `zlib.decompressobj(15)` (requiring `eof`) and Node `inflateSync` |
| `decode-gzip` | 159: the same matrix gzip-wrapped (`wbits 31`); 3 multi-member files (two members, three members, a member plus non-member padding); 6 hand-built headers with `FEXTRA`, `FNAME`, `FCOMMENT`, `FHCRC` alone and combined | length and CRC-32 of `compress::gzipDecode`, against Python's zlib member loop (`zlib.decompressobj(31)` while the rest starts `1f 8b`) and Node `gunzipSync`; case 152 is a declared divergence for Node |
| `mutate` | 600: seeded 1–3 byte replacements, deletions or insertions of 36 valid streams (3 payloads × levels 0/1/6/9-fixed × raw/zlib/gzip) | verdict only (`ok <len> <crc32>` or `err`) three ways. **Fails** when `compress` accepts what both zlibs refuse, refuses what both accept, or all accept with different output. Cases where Python and Node disagree with each other are bucketed and listed for inspection, not failed. The probe must answer every case — a crash or hang is a harness failure (exit 2) |

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

## The behaviour probe

```
tools/oracles/compress/probe.sh
```

Measures the **judges**, not the MFB decoder: `python/probe_streams.py` builds edge streams
bit by bit with its own DEFLATE writer (no zlib encoder involved) — trailing bytes, header
flags, bad trailers, and code sets at the edges of zlib's `inflate_table` rules — and the
script prints how Python's `zlib` / `gzip.decompress` and Node's `node:zlib` treat each. Its
table is the evidence behind `compress`'s strictness rules and the divergences below.

## Declared divergences

Cases where `compress` deliberately does not do what a judge does. A decode mode must skip or
expect these rather than report them as disagreements. Evidence: `probe.sh`, 2026-09-14
(Python zlib 1.2.12, Node zlib 1.3.1).

| Case | `compress` | Judge behaviour |
|---|---|---|
| gzip stream followed by bytes that do not begin `1f 8b` | decodes; the bytes are ignored | Python `gzip.decompress`: `BadGzipFile: Not a gzipped file`; Node `gunzipSync`: `Z_BUF_ERROR` (1 byte) or `incorrect header check` (2+ bytes). Python `zlib.decompressobj(31)` agrees with `compress` (leaves them in `unused_data`). In `run.sh`: `decode-gzip` case 152 is skipped for the Node judge only (`declared_divergences`) |

Where Python's pure-Python `gzip.decompress` is more lenient than zlib — it accepts a wrong
header CRC-16 and reserved flag bits, which both zlibs refuse — `compress` follows zlib, so a
gzip decode mode must judge headers with a zlib-backed decoder (`zlib.decompressobj(31)` or
Node), not `gzip.decompress`.

## Adding a mode

Add the mode in all five places, or `run.sh` exits 2:

1. `python/gen.py` — a `MODES` entry returning `(data, aux)` cases.
2. `python/oracle.py` — a `MODES` entry printing `case <index> <fields...>`.
3. `node/oracle.mjs` — the same, from `node:zlib`.
4. `mfb/src/main.mfb` — a branch in `runJob` printing the same fields.
5. `run.sh` — the name in `ALL_MODES` and its count in `expected_cases`.
