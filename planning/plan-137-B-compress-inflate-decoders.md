# plan-137-B: table-driven inflate + `inflate` / `zlibDecode` / `gzipDecode`

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-137-A (the `compress` package, its test homes, oracle and bench harnesses).
If plan-137-A is not complete, this plan cannot start, full stop. Whole-feature
prerequisites: plan-137-A §Prerequisites.

This letter adds the decoding half of `compress::`: a fast, strict, table-driven DEFLATE
decoder written in MFBASIC, and the three public decoders over it. Behavioural outcome:
anything Python's or Node's zlib produces — raw, zlib-wrapped or gzip, at every level and
strategy — decodes to the original bytes; anything they refuse, we refuse with
`ErrInvalidFormat` (one deliberate divergence: bytes after the end of a stream are ignored); and output past `maxBytes` raises `ErrTooLarge` without first building
the oversized result.

Surface:

- `FUNC inflate(data AS List OF Byte, maxBytes AS Integer = 67108864) AS List OF Byte` — raw RFC 1951.
- `FUNC zlibDecode(data AS List OF Byte, maxBytes AS Integer = 67108864, ignoreChecksum AS Boolean = FALSE) AS List OF Byte` — RFC 1950.
- `FUNC gzipDecode(data AS List OF Byte, maxBytes AS Integer = 67108864, ignoreChecksum AS Boolean = FALSE) AS List OF Byte` — RFC 1952,
  every member of a multi-member file, concatenated; `maxBytes` bounds the total.

Decided 2026-09-13 (plan-137-A §Decisions): bytes after the end of a stream are **ignored**;
`ignoreChecksum := TRUE` skips every checksum comparison (gzip header CRC-16, CRC-32, `ISIZE`;
zlib Adler-32) while every structural check still applies; a zlib stream with `FDICT` set is
refused with `ErrInvalidFormat`.

References:

- RFC 1951 §3.2 (block format, canonical Huffman codes, length/distance tables), RFC 1950 §2.2
  (CMF/FLG, FCHECK, FDICT, Adler-32), RFC 1952 §2.3 (member header, flags, CRC32, ISIZE).
- zlib `inftrees.c` / `inflate.c` — the behaviour the oracle encodes for code-set validity
  (over-subscribed vs incomplete sets, the one-distance-code case). Fetch; cite in the spec.
- `src/codegen/builtins/canvas/helper_inflate.rs` — the reference decoder being superseded
  (letter C deletes it); read its bit-order comment.
- `planning/completed/audit-3-decoders.md` DEC-57 / DEC-58 — the leniencies this decoder must not have.
- `.ai/collections.md` (in-place write rule), `.ai/codegen-invariants.md` (quadratic free list
  under mixed-size churn), memory note on the AArch64 ±1 MiB per-function branch range.
- plan-137-A §2 (shared current state and measurements), §4.4 (harnesses).

## Prerequisites

See plan-137-A §Prerequisites (whole-feature gate). Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-137-A complete | `ls planning/completed/plan-137-A-*` → one file | NOT MET |
| Error code 77050027 and the name `ErrTooLarge` are free on main | `git grep -c 77050027 main -- src` → no files; `git grep -c ErrTooLarge main -- src` → no files (take the next free code if not) | MET (2026-09-13; highest `7705` on main is 77050026 `ErrCanvasGroupLimit`) |

## 1. Goal

- The three decoders above, each refusing malformed input with `ErrInvalidFormat` (77050003),
  output beyond `maxBytes` with `ErrTooLarge` (new), and `maxBytes < 0` with `ErrInvalidArgument`.
- **Strictness parity with zlib**: over-subscribed code sets, incomplete literal/length code
  sets, incomplete distance sets beyond zlib's allowance, a distance reaching before the start
  of output, stored `LEN` ≠ `~NLEN`, reserved `BTYPE = 11`, invalid length/distance symbols
  (286, 287, 30, 31), a code-length repeat with no previous length or overrunning `HLIT+HDIST`,
  input ending mid-stream, a zlib header with `CM ≠ 8`, `CINFO > 7`, bad `FCHECK`, `FDICT` set,
  a gzip header with wrong magic, `CM ≠ 8`, reserved flag bits set, a truncated header or
  trailer — all refused.
- **Checksums:** with `ignoreChecksum = FALSE` (the default) a wrong zlib Adler-32, gzip header
  CRC-16 (when `FHCRC` is set), gzip CRC-32 or `ISIZE` raises `ErrInvalidFormat`; with `TRUE` the
  same streams decode to the bytes their data blocks describe. The checksum bytes must still be
  present — `TRUE` skips the comparison, not the structure.
- **Trailing bytes are ignored:** `inflate` stops after the final block, `zlibDecode` after the
  Adler-32; `gzipDecode` decodes another member only while the remaining bytes begin with the
  magic `1f 8b` and ignores anything else after the last member. Bytes that begin with the magic
  but are not a valid member are refused — they are a member, not trailing bytes.
- Throughput recorded; decode time linear in output size; faster than canvas's inflate on the
  same stream.

### Non-goals (explicit constraints)

- No encoder in this letter. No change to canvas (letter C). No streaming state object.
- No leniency beyond what the oracle shows zlib accepts, other than the two decided behaviours
  (trailing bytes ignored; checksum comparisons skipped only when the caller passes
  `ignoreChecksum := TRUE`).
- **Decoder state lives in one place** (§4.1) — not spread across helper locals — so a later
  streaming plan is a refactor, not a rewrite. This must not be achieved by threading a record
  through calls in the hot loop (copies; `.ai/collections.md`).
- No trading a check for speed. A check found slow is made cheaper, never removed.

## 2. Current State

plan-137-A §2 holds the shared facts. Specific to this letter:

- `__canvas_inflate` reads each bit as `(toInt(getOr(data, pos / 8, 0)) / __canvas_pow2(pos MOD 8)) MOD 2`,
  where `__canvas_pow2` loops (`helper_inflate.rs` `INFLATE_BITS`), decodes Huffman symbols by
  walking lengths 1..15 bit by bit with no table (`__canvas_huffDecode`), rebuilds the
  length/distance base tables as list literals on every call, and returns a fresh two-element
  list per symbol. It bounds output with a `limit` check before every append and refuses rather
  than truncates.
- It accepts over-subscribed trees (DEC-58) and does not read the Adler-32 trailer (DEC-57).
- Output built by in-place append to a function-local list is amortized O(1) (35 allocations
  for 16 MiB, `/tmp/cbench/adbg` probe, plan-137-A §2); its memory footprint is governed by
  bug-621's fix (prerequisite).

### Measured populations

| What | Count | Command |
|---|---|---|
| Canvas inflate throughput on the bench corpus | UNMEASURED | Phase 1 |
| Python / Node decode behaviour on trailing bytes (sets the oracle's declared divergences), a wrong `FHCRC`, `FDICT`, the one-distance-code block, the no-distance-code block, an incomplete literal set | UNMEASURED | Phase 1 oracle probe |
| Emitted code size of the inflate core function (AArch64 conditional-branch range is ±1 MiB per function) | UNMEASURED | Phase 2: `mfb build --ncode` of the fixture, count the function's instructions × 4 B |

## 3. Design Overview

- **§4.1 core** — `__compress_inflateCore(data, start, maxBytes)` owns all decoder state, raises
  on malformed input, and returns the output list. The framing layers also need the byte position
  where the DEFLATE stream ended; §4.1 gives two candidate ways to report it, chosen in Phase 2
  by measured cost. A shape that allocates per symbol or copies the output per block is rejected.
- **§4.2 Huffman tables** — built once per block into function-local `List OF Integer`s.
- **§4.3 framing** — zlib and gzip parse/verify around the core.
- **§4.4 bounds** — `maxBytes` checked before every output write.

Design uncertainty first: Phase 1 measures canvas's inflate and a minimal table-driven
prototype (fixed + dynamic Huffman only, no framing) on the same zlib stream, and settles the
two open shapes (§4.1 end-position reporting; `bits::sr` vs integer `/` for the bit buffer) by
measurement. Correctness risk last and behind the oracle: Phase 3's mutate mode.

## 4. Detailed Design

### 4.1 The inflate core and "state in one place"

One MFBASIC function, `__compress_inflateCore`, owns **all** decoder state as locals declared
together at its top, under a comment naming them as the state set:

`pos` (input byte cursor), `bitBuf` / `bitCount` (LSB-first bit buffer, ≤ 57 valid bits),
`final` (BFINAL seen), `phase` (between blocks / stored / Huffman), `out` (the output
`List OF Byte`), `litTable` / `distTable` (current block's decode tables), `storedLeft`.

Helpers are pure: they take scalars or read-only lists and return scalars or freshly built
tables (`__compress_buildTable(lengths, count) AS List OF Integer`). No helper takes `out`.

**End-position reporting** (framing needs to know where the DEFLATE stream ended to read a
trailer or the next gzip member). Two candidate shapes, chosen in Phase 2 by measured cost:

- (a) the core returns `out` and the framing layer calls `__compress_inflateEnd(data, start)`,
  a scan that re-walks block structure without writing output — doubles decode work;
- (b) the core returns `out` with the end position appended as 8 trailing bytes that the
  wrapper removes with one `collections::slice`-style call — one pass, one extra copy of `out`.

Record the choice and its measured cost in Corrections. A shape that copies `out` per block or
per symbol is rejected outright.

**Bit reader.** Refill a byte at a time while `bitCount < need` (need ≤ 15 + 13 extra):
`bitBuf = bor(bitBuf, sl(byte, bitCount))`. Peek `band(bitBuf, mask)`; consume
`bitBuf = sr(bitBuf, n)`. Phase 1 measures `sl`/`sr` (count-checked branches) against `*`/`/`
by table powers of two; take the faster, record both numbers.

**Output.** `out = collections::append(out, b)` for literals; back-references copy byte by byte
from `getOr(out, len(out) - dist)` so overlapping matches work by construction. Before any
write: `IF len(out) + n > maxBytes THEN FAIL error(<ErrTooLarge>, …)`.

### 4.2 Huffman decode tables

Canonical codes from code lengths (RFC 1951 §3.2.2). Validation while building, matching
zlib's `inftrees.c`: count codes per length; walk lengths accumulating `left = left*2 - count[len]`;
`left < 0` → over-subscribed → refuse; `left > 0` → incomplete → refuse, **except** the cases
the Phase 1 oracle probe shows zlib accepts (expected: a distance code set with exactly one
code of length 1, and a block with no distance codes that uses none). Record the exceptions
with the oracle output in the spec.

Table shape: a primary table indexed by the next `ROOT` bits (9 for literal/length, 6 for
distance) whose entries pack `symbol * 16 + length`; codes longer than `ROOT` use a second-level
table addressed from the primary entry (entry flag + offset). Tables are function-local
`List OF Integer` built by in-place `append` in the builder helper and returned once per block.
Fixed-Huffman tables are built once per `__compress_inflateCore` call, not per block.

Length base/extra and distance base/extra are module-level `LET` tables (read-only), generated
by a `#[cfg(test)]` check against the RFC 1951 §3.2.5 formula, not typed from memory.

### 4.3 Framing

- **zlib:** `CMF`, `FLG`; `CM = 8`, `CINFO ≤ 7`, `(CMF*256 + FLG) MOD 31 = 0`; `FDICT` set →
  `ErrInvalidFormat` (no dictionary API yet); core; the 4-byte big-endian Adler-32 trailer must be
  present and, unless `ignoreChecksum`, equal `__compress_adler32(out)` (s1/s2 mod 65521; reduce
  every 5,552 bytes as zlib's `NMAX` does, which keeps every intermediate far below 2^63). With
  `ignoreChecksum` the sum is not computed at all. Bytes after the trailer are ignored.
- **gzip:** per member — `ID1 = 0x1f`, `ID2 = 0x8b`, `CM = 8`, reserved `FLG` bits 5–7 = 0;
  skip `MTIME`/`XFL`/`OS`; `FEXTRA` (`XLEN` + bytes), `FNAME` / `FCOMMENT` (zero-terminated);
  `FHCRC`: the 2 bytes must be present and, unless `ignoreChecksum`, equal the low 16 bits of the
  CRC-32 of the header bytes before them (RFC 1952 §2.3.1); core; `CRC32` (A's `crc32` helper) and
  `ISIZE` (output length mod 2^32) must be present and, unless `ignoreChecksum`, match. Then, if at
  least 2 bytes remain and they are `1f 8b`, decode the next member (its errors raise); otherwise
  stop and ignore the rest. With `ignoreChecksum` no CRC-32 is computed.
- **raw:** core only; bytes after the final block are ignored.

### 4.4 Bounds and hostile input

- `maxBytes` bounds output; the check precedes the write, so a bomb fails at `maxBytes + 1`
  bytes of work, never after building more.
- Input cursor reads use `getOr(data, pos, 0)` plus an explicit `pos > len(data)` end-of-input
  check before decoding each symbol, so a truncated stream refuses rather than decoding zeros.
- No recursion; every loop advances `pos` or output, so hostile input cannot loop forever.

## Compatibility / Format Impact

New public functions `inflate`, `zlibDecode` and `gzipDecode` (the last two with `ignoreChecksum`); new error constant `ErrTooLarge`
(`errorcode/mod.rs` row + `02_error-codes.md` row; `errorCode::ErrTooLarge` becomes nameable).
Nothing existing changes.

## Phases

> **NOTE — keep the checkboxes current as you go** (see plan-137-A). **An unticked box means NOT DONE.**

### Phase 1 — measure first (no public surface)

- [ ] Re-run plan-137-A §Prerequisites and this letter's rows.
- [ ] Oracle behaviour probe (`tools/oracles/compress/python/oracle.py probe`, `node/oracle.mjs probe`):
      trailing bytes after raw / zlib / gzip streams; `FHCRC` with a wrong CRC-16; `FDICT` set;
      one-distance-code block; no-distance-code block; incomplete literal set. Paste outputs
      here. Where zlib refuses trailing bytes that we ignore, write the case into the oracle's
      declared-divergence list (so `mutate` does not report it as leniency); the code-set cases
      set §4.2's exceptions.
- [ ] Canvas baseline: an `--app`-free harness can't reach `__canvas_zlibInflate`, so measure
      through `canvas::loadImage` on a headless build of a generated 4096×4096 PNG with one IDAT
      compressed by Python `zlib.compress(raw, 6)` (pattern: `tests/canvas/rt_canvas_image_decode.rs`
      `decode_bounded` helpers). Record decode ms and the PNG's inflated size.
- [ ] Prototype `__compress_inflateCore` (fixed + dynamic blocks, stored blocks, no framing) in
      a scratch builtin build; measure the same stream: `sl`/`sr` vs `*`/`/` bit buffer, and the
      §4.1 (a)/(b) end-position shapes. Record MiB/s for each variant at `-O1` and `-O3`.

Acceptance: every UNMEASURED row in §2 filled; the oracle's declared-divergence list and §4.2's
code-set exceptions each cite the probe output behind them.
  Check: the probe scripts print their tables; bench numbers pasted in Corrections (est. 45 min —
  it is the plan's design experiment; nothing smaller can measure the decoder's speed).
Commit: —

### Phase 2 — the core and `inflate`

- [ ] `compress/helper_inflate_core.rs`, `helper_huffman_table.rs`, `helper_deflate_tables.rs`
      (+ table-check tests), `func_inflate.rs`; `ErrTooLarge` row in `errorcode/mod.rs` and
      `02_error-codes.md`; `errors: vec!["ErrInvalidFormat", "ErrTooLarge", "ErrInvalidArgument"]`.
- [ ] Gate helpers `WhenUsed(&["inflate", "zlibDecode", "gzipDecode"])`.
- [ ] Record the core's emitted size (§2 row); if within 4× of the ±1 MiB AArch64 limit, split
      the table builders further before continuing.

Acceptance: `inflate` round-trips Python `zlib.compressobj(level, DEFLATED, -15)` output for
levels 0–9 and strategies default/filtered/huffman-only/RLE/fixed.
  Check: `tools/oracles/compress/run.sh target/release/mfb decode-raw` → exit 0 (est. 8 min).
Commit: —

### Phase 3 — framing, strictness, bounds

- [ ] `helper_adler32.rs`, `helper_zlib_frame.rs`, `helper_gzip_frame.rs`, `func_zlib_decode.rs`,
      `func_gzip_decode.rs`.
- [ ] Oracle modes `decode-zlib`, `decode-gzip` (incl. multi-member, `FNAME`/`FCOMMENT`/`FEXTRA`/`FHCRC`
      headers written by Python's `gzip` module and by hand-built headers), and `mutate`
      (1–3 byte edits of valid streams: replace / delete / insert; robustness is a hard
      assertion — the probe answers every case; agreement buckets printed and every
      *we-accepted-they-refused* case inspected and either fixed or recorded as a documented
      divergence with evidence).
- [ ] `tests/interop/rt_compress_interop.rs`: `flate2` (miniz_oxide backend) encodes raw / zlib /
      gzip at levels 0–9 over a seeded corpus → MFB decodes equal; `flate2` streams tampered at
      a seeded byte → MFB refuses; one hand-built over-subscribed-tree stream and one
      bad-Adler stream → refused (the DEC-57/58 regressions).
- [ ] Same file, the decided behaviours: a stream with a corrupted Adler-32 / CRC-32 / `ISIZE` /
      `FHCRC` → refused by default, and decoded to the original bytes with `ignoreChecksum := TRUE`;
      a structurally broken stream (bad `NLEN`) → refused even with `ignoreChecksum := TRUE`;
      `FDICT` set → refused; raw / zlib / gzip streams followed by 1, 7 and 1,000 junk bytes (not
      starting `1f 8b`) → decoded, junk ignored; two gzip members → concatenated; a valid member
      followed by `1f 8b` plus garbage → refused.
- [ ] `tests/runtime/rt_compress_bounds.rs` (`common::run_bounded_with_rss`): a 64 MiB+1 zero
      bomb under default `maxBytes` → `ErrTooLarge` within a time and RSS bound derived from
      bug-621's fixed growth rule (derivation written in the test); decode time for n and 4n
      output ratio ≤ 4.4.
- [ ] rt-behavior fixture `compress-decode-valid` (small streams generated by Python zlib,
      embedded as hex with the generating command in a comment) and rt-error fixtures for
      `ErrInvalidFormat` (a bad checksum, and an `FDICT` stream), `ErrTooLarge`, `ErrInvalidArgument`.

Acceptance: decoders agree with zlib on the corpus, refuse every tamper class in §1, and bound
hostile input.
  Check: `tools/oracles/compress/run.sh target/release/mfb decode-raw decode-zlib decode-gzip mutate` → exit 0;
  `cargo test --test rt_compress_interop --test rt_compress_bounds` → pass;
  `scripts/test-accept.sh target/release/mfb /tmp/p137b 'compress'` → 0 mismatches (est. 20 min).
Commit: —

### Phase 4 — speed record, docs

- [ ] Bench rows `inflate` / `zlibDecode` / `gzipDecode` over the corpus; record MiB/s and the
      ratio to canvas (Phase 1) and to Python zlib in Corrections.
- [ ] Man descriptors for the three members (errors, `maxBytes` and `ignoreChecksum` meaning,
      trailing bytes ignored, multi-member gzip,
      examples that round-trip a literal stream produced by a documented command).
- [ ] `20_compress.md`: formats, bit order, table construction and validation rules with the zlib
      exceptions and their oracle evidence, the decided behaviours (trailing bytes, `ignoreChecksum`,
      `FDICT`) and the gzip next-member rule, bounds, end-position shape
      chosen and why, throughput recorded as a dated measurement.
- [ ] `tests/byte-identity/compress/` program extended to call the decoders; regenerate its eight
      goldens; confirm no other package's `.ncodesum` moved.

Acceptance: decoders are faster than canvas's inflate on the Phase 1 stream; docs render and
examples run; byte-identity drift is confined to `tests/byte-identity/compress/`.
  Check: `tools/compress-bench/run.sh target/release/mfb`; `scripts/man-run-examples.sh compress --run`;
  `cargo test --bin mfb spec`; `scripts/artifact-gate.sh target/release/mfb compress` (est. 15 min).
Commit: —

## Validation Plan

- Tests: oracle modes (not CI), `rt_compress_interop` and `rt_compress_bounds` (CI), rt fixtures.
- Coverage check: every refusal class in §1 has a named case in the interop test or an rt-error
  fixture (list them in the test's module doc).
- Runtime proof: `compress-decode-valid` on macos-aarch64 and box 2223
  (`FILTER=compress scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64 glibc`).
- Doc sync: man descriptors + `20_compress.md` + `02_error-codes.md`.
- Final gate: plan-137-E.

## Open Decisions

- End-position reporting shape (a) re-scan vs (b) appended trailer — decided in Phase 2 by
  measured cost; (b) expected cheaper.

## Corrections

<Filled in during execution, including the Phase 1 measurements.>

## Summary

The risk is strictness (a lenient decoder is a silent security and interop bug) and speed; both
are measured before the public surface lands and gated by an oracle that includes mutation.
Canvas keeps its own decoder until letter C.
