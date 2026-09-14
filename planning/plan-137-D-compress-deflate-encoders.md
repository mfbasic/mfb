# plan-137-D: `deflate` / `zlibEncode` / `gzipEncode` — stored + fixed-Huffman LZ77

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-137-C (and through it B and A). If plan-137-C is not complete, this plan cannot
start, full stop. Whole-feature prerequisites: plan-137-A §Prerequisites.

This letter adds the encoding half of `compress::` with complete, valid output at every level:
level 0 writes stored blocks; levels 1–9 run a hash-chain LZ77 match finder whose effort grows
with the level and emit fixed-Huffman blocks. Behavioural outcome: for every input and every
level 0–9, Python's and Node's zlib decode our raw, zlib and gzip output back to the input, and
our own decoders do too; higher levels search harder. (Dynamic Huffman and lazy matching, which
close most of the ratio gap to zlib, are plan-137-E.)

Surface:

- `FUNC deflate(data AS List OF Byte, level AS Integer = 6) AS List OF Byte` — raw RFC 1951.
- `FUNC zlibEncode(data AS List OF Byte, level AS Integer = 6) AS List OF Byte` — RFC 1950.
- `FUNC gzipEncode(data AS List OF Byte, level AS Integer = 6) AS List OF Byte` — RFC 1952, one member.

`level` outside `0..9` raises `ErrInvalidArgument`.

References:

- RFC 1951 §3.2.4 (stored blocks), §3.2.5–3.2.6 (length/distance codes, fixed Huffman code), §4
  (compression, hash chains).
- zlib `deflate.c` `configuration_table` (per-level `good_length`, `max_lazy`, `nice_length`,
  `max_chain`) and `deflate_stored` / `deflate_fast` — **fetch; do not recite values**. This letter
  uses `max_chain` and `nice_length`; E uses the rest.
- plan-137-B §4.2 (shared length/distance tables), plan-137-A §4.4 (harnesses).

## Prerequisites

See plan-137-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-137-C complete | `ls planning/completed/plan-137-C-*` → one file | NOT MET (2026-09-13 re-run: `ls planning/completed | grep -c plan-137` → 0; blocked on plan-137-A's bug-621 row) |

## 1. Goal

- The three encoders above, valid at every level for inputs of 0 bytes, 1 byte, 2 bytes (shorter
  than a minimum match), 65,535 / 65,536 / 65,537 bytes (stored-block boundaries), highly repetitive
  input (258-byte matches, matches at distance 32,768), and incompressible input.
- zlib output: `CMF = 0x78`, `FLEVEL` bits set from `level` as zlib does, correct `FCHECK`,
  big-endian Adler-32 trailer. gzip output: `1f 8b 08`, `FLG = 0`, `MTIME = 0`, `XFL` from
  `level` as zlib does, `OS = 255` (unknown), CRC32 and ISIZE little-endian.
- Determinism: the same input and level give the same bytes on every target (checked on two boxes).
- Ratio and throughput recorded per level (no ratio floor; E improves it).

### Non-goals (explicit constraints)

- Not byte-identical to zlib. Not a streaming encoder. No dynamic Huffman or lazy matching here.
- `gzipEncode` writes no `FNAME`/`MTIME` (no way to pass them; deterministic output matters more).
- The decoders from B are not modified except to fix a bug the encoder tests expose (fix it, add
  the minimal case to B's tests, record it in Corrections).

## 2. Current State

- plan-137-A §2 (package, harnesses) and plan-137-B (decoders, shared length/distance tables in
  `helper_deflate_tables.rs`, bit-order conventions).
- No encoder exists in the tree (`grep -rln "FUNC __.*deflate\b\|__compress_deflate" src` → none at authoring).

### Measured populations

| What | Count | Command |
|---|---|---|
| zlib `configuration_table` rows | UNMEASURED (fetch) | Phase 1: fetch `deflate.c` for the Python-reported zlib version, paste the table here |
| Python zlib `FLEVEL` and gzip `XFL` per level | UNMEASURED | Phase 1: `python3 -c "import zlib; [print(l, zlib.compress(b'x', l)[:2].hex()) for l in range(10)]"` and the `gzip` module equivalent |
| Encoder function emitted size (±1 MiB AArch64 branch range) | UNMEASURED | Phase 3: `--ncode` |

## 3. Design Overview

- **§4.1 bit writer** — LSB-first accumulator flushed a byte at a time into the function-local
  output list.
- **§4.2 match finder** — hash of the next 3 bytes → `head` (most recent position) and `prev`
  (previous position with the same hash, indexed modulo the 32,768 window); greedy: take the
  longest match within `max_chain` candidates, stop early at `nice_length`.
- **§4.3 block emitter** — fixed-Huffman blocks, one per 64 KiB of input (the boundary is
  arbitrary but fixed and recorded); stored blocks at level 0 (≤ 65,535 bytes each).
- **§4.4 framing** — zlib / gzip wrappers.

Design uncertainty: MFB speed of the match finder (two table writes per input byte). Phase 2
measures level 1 and level 9 on the bench corpus before framing lands. Correctness risk: bit
packing and boundary sizes — Phase 3's zlib-decodes-ours sweep with the edge-size list in §1.

Rejected: emitting only stored blocks for levels 1–9 in this letter (a "level" that does nothing
is a stub); fixed Huffman with no LZ77 (same objection).

## 4. Detailed Design

### 4.1 Bit writer

State local to the encoder function: `acc` (Integer, ≤ 64 bits pending), `nbits`, `out`.
`put(code, len)`: `acc = bor(acc, sl(code, nbits))`, `nbits += len`, flush while `nbits ≥ 8`.
Huffman codes are written bit-reversed (RFC 1951 §3.1.1: codes are packed MSB-first into an
LSB-first stream) — the reversal comes from a module-level 9-bit reverse table generated by a
`#[cfg(test)]` check.

### 4.2 Match finder

`head` is a `List OF Integer` of `2^15` entries initialised to `-1`; `prev` is a `List OF Integer`
of 32,768 entries. Both are **locals of the encoder function**, written with in-place
`collections::set` (the only shape that stays in place: `.ai/collections.md`). Hash:
`band(bxor(bxor(sl(b0, 10), sl(b1, 5)), b2), 32767)`. Insert every position (levels 1–3 may skip
insertion inside long matches, as `deflate_fast` does — fetch and mirror). A candidate is valid
while `pos - cand ≤ 32768` and it is within the chain budget. Match length ≤ 258, ≥ 3.

### 4.3 Blocks

- Level 0: stored blocks `BFINAL`, `BTYPE=00`, byte-align, `LEN`, `NLEN`, raw bytes. Empty input →
  one final empty stored block.
- Levels 1–9: fixed-Huffman blocks (`BTYPE=01`) of literal/length and distance symbols with extra
  bits from B's tables, end-of-block 256. Empty input → one final empty fixed block.

### 4.4 Framing

zlib: header per Phase 1's measured `FLEVEL` mapping, Adler-32 trailer (B's helper). gzip:
10-byte header per §1, CRC32 (A's helper) and ISIZE trailer.

## Compatibility / Format Impact

New public functions `deflate`, `zlibEncode`, `gzipEncode`. No existing change.

## Phases

> **NOTE — keep the checkboxes current as you go** (see plan-137-A). **An unticked box means NOT DONE.**

### Phase 1 — fetch and measure the external facts

- [ ] Fill the UNMEASURED rows in §2 (configuration table, FLEVEL/XFL mapping).

Acceptance: §2 rows filled with pasted evidence.
  Check: the commands in §2 (est. 10 min).
Commit: —

### Phase 2 — raw `deflate`, levels 0–9

- [ ] `compress/helper_bit_writer.rs`, `helper_match_finder.rs`, `helper_deflate_core.rs`,
      `func_deflate.rs`; level validation; helpers gated `WhenUsed(&["deflate", "zlibEncode", "gzipEncode"])`.
- [ ] Oracle mode `encode-raw`: every §1 edge size and the bench corpus, levels 0–9 → Python
      `zlib.decompressobj(-15)` and Node `inflateRawSync` decode equal; our `inflate` decodes equal.
- [ ] Bench rows `deflate` level 1 / 6 / 9: MiB/s and ratio vs Python zlib at the same level.

Acceptance: zlib decodes every raw stream we produce, at every level and edge size.
  Check: `tools/oracles/compress/run.sh target/release/mfb encode-raw` → exit 0 (est. 15 min).
Commit: —

### Phase 3 — framing, tests, docs

- [ ] `func_zlib_encode.rs`, `func_gzip_encode.rs`; oracle modes `encode-zlib`, `encode-gzip`
      (Python `zlib.decompress`, `gzip.decompress`; Node `inflateSync`, `gunzipSync`; plus `gzip -t`
      on the host for gzip output).
- [ ] `tests/interop/rt_compress_interop.rs`: MFB encodes the seeded corpus at levels 0–9 in all
      three formats → `flate2` decodes equal; MFB `gzipEncode` of the same input twice → identical bytes.
- [ ] `tests/rt-behavior/compress/compress-encode-roundtrip-valid` (encode → decode with our own
      decoders, all formats and levels, prints lengths and a CRC of each output) and an rt-error
      fixture for `level := 10`.
- [ ] Determinism across targets: cross-build the roundtrip fixture for `linux-aarch64`, run on box
      2223, and compare its printed output CRCs with macOS (`FILTER=compress scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64 glibc`).
- [ ] Record emitted encoder size; man descriptors; `20_compress.md` encoder section (block policy,
      hash, chain budgets with the fetched table, FLEVEL/XFL mapping, determinism guarantee);
      byte-identity program extended and regenerated.

Acceptance: zlib decodes every framed stream; output identical on macOS and 2223; docs render.
  Check: `tools/oracles/compress/run.sh target/release/mfb encode-raw encode-zlib encode-gzip` → exit 0;
  `cargo test --test rt_compress_interop` → pass; `scripts/test-accept.sh target/release/mfb /tmp/p137d 'compress'` → 0 mismatches;
  `scripts/man-run-examples.sh compress --run` → pass (est. 25 min).
Commit: —

## Validation Plan

- Tests: oracle encode modes (not CI), interop round-trips and determinism (CI), rt fixtures.
- Coverage check: every §1 edge size appears by name in the oracle job generator and the interop test.
- Runtime proof: roundtrip fixture on macOS and 2223 with matching CRCs.
- Doc sync: man descriptors + `20_compress.md`.
- Final gate: plan-137-E.

## Open Decisions

- Block size for fixed-Huffman blocks — 64 KiB of input (recommended: bounded symbol buffers,
  arbitrary but fixed) vs. one block per call (unbounded symbol buffer on 64 MiB inputs).

## Corrections

<Filled in during execution, including per-level ratio and MiB/s.>

## Summary

The risk is bit-exact validity at boundaries, and MFB speed of a two-writes-per-byte match finder.
Every stream is judged by two external zlibs plus `flate2`; ratio is recorded, not gated, because
plan-137-E is where ratio is earned.
