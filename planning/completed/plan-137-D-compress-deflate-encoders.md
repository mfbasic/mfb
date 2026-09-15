# plan-137-D: `deflate` / `zlibEncode` / `gzipEncode` — stored + fixed-Huffman LZ77

Last updated: 2026-09-14
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
| plan-137-C complete | `ls planning/completed/plan-137-C-*` → one file | MET (2026-09-14 re-run: `planning/completed/plan-137-C-canvas-inflate-cutover.md`, archived in `821859191`). plan-137-A §Prerequisites re-run the same day. `ls bugs/completed/bug-621-*` → the append-growth fix (`bug-621-append-growth-over-reserves-data-capacity.md`) plus a second, unrelated `bug-621-loop-condition-temps-freed-once-per-loop-not-per-pass.md` that main reused the number for; the row's file is present. Release build → see Phase 1. Python zlib `1.2.12`, Node zlib `1.3.1-470d3a2`. `Cargo.lock:1142` flate2 `1.1.9`. `compress` present: plan-137-A's own "no package yet" row was its pre-start gate, and A created the package. |

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
| zlib `configuration_table` rows | 10 rows (2026-09-14), pasted below the table | Phase 1: fetch `deflate.c` for the Python-reported zlib version, paste the table here |
| Python zlib `FLEVEL` and gzip `XFL` per level | Measured 2026-09-14 (Python zlib 1.2.12, Node zlib 1.3.1-470d3a2, identical). zlib header: levels 0–1 `7801`, 2–5 `785e`, 6 `789c`, 7–9 `78da`. gzip `XFL`: levels 0–1 `4`, 2–8 `0`, 9 `2`. `OS` is `255` from Python `gzip.compress` (pure-Python header) and `19` from zlib's own gzip wrapper (`wbits=31`, Python and Node): the zlib wrapper writes its build's `OS_CODE`, and `255` "unknown" is what §1 specifies | Phase 1: `python3 -c "import zlib; [print(l, zlib.compress(b'x', l)[:2].hex()) for l in range(10)]"` and the `gzip` module equivalent (run: `zlib.compress(b'x', l)[:2]`, `gzip.compress(b'x', compresslevel=l, mtime=0)[8:10]`, `zlib.compressobj(l, DEFLATED, 31)` output `[8:10]`, Node `deflateSync`/`gzipSync` with `{level}`) |
| Encoder function emitted size (±1 MiB AArch64 branch range) | Measured 2026-09-14 (`/tmp/p137size-enc.py`, `mfb build --ncode` of a program calling all three encoders): `_mfb_ifn_compress_5FdeflateCore` **12,612 instructions on macos-aarch64 and linux-aarch64 = 50,448 B**, 20.8× under 1 MiB, so no split; linux-x86_64 12,378, windows-x86_64 12,382, linux-riscv64 12,895. `gzipEncode` 4,288, `zlibEncode` 2,450 (AArch64) | Phase 3: `--ncode` |

**zlib 1.2.12 facts, fetched 2026-09-14** from `https://raw.githubusercontent.com/madler/zlib/v1.2.12/deflate.c`
(sha256 `824ff399fae1934f57e48de6d5cac4410f36f709021e8ee79655f927b9bd0bce`) and `…/deflate.h`
(sha256 `9dd7224b61b43c6a336b30b867418372186ee34a1d1a9a51c338ee1adc2f530d`). The version is the one Python reports:
`zlib.ZLIB_RUNTIME_VERSION` → `1.2.12`. The text below is verbatim from those files:

```c
/* deflate.c:134 */
local const config configuration_table[10] = {
/*      good lazy nice chain */
/* 0 */ {0,    0,  0,    0, deflate_stored},  /* store only */
/* 1 */ {4,    4,  8,    4, deflate_fast}, /* max speed, no lazy matches */
/* 2 */ {4,    5, 16,    8, deflate_fast},
/* 3 */ {4,    6, 32,   32, deflate_fast},

/* 4 */ {4,    4, 16,   16, deflate_slow},  /* lazy matches */
/* 5 */ {8,   16, 32,   32, deflate_slow},
/* 6 */ {8,   16, 128, 128, deflate_slow},
/* 7 */ {8,   32, 128, 256, deflate_slow},
/* 8 */ {32, 128, 258, 1024, deflate_slow},
/* 9 */ {32, 258, 258, 4096, deflate_slow}}; /* max compression */

/* deflate.c:860–870, the zlib header */
if (s->strategy >= Z_HUFFMAN_ONLY || s->level < 2)
    level_flags = 0;
else if (s->level < 6)
    level_flags = 1;
else if (s->level == 6)
    level_flags = 2;
else
    level_flags = 3;
header |= (level_flags << 6);

/* deflate.c:904, the gzip header (no gzhead) */
put_byte(s, s->level == 9 ? 2 :
         (s->strategy >= Z_HUFFMAN_ONLY || s->level < 2 ?
          4 : 0));

/* deflate.c:1924, deflate_fast: insert inside a match only when it is short */
if (s->match_length <= s->max_insert_length &&
    s->lookahead >= MIN_MATCH) {
    s->match_length--; /* string at strstart already in table */
    do {
        s->strstart++;
        INSERT_STRING(s, s->strstart, hash_head);
    } while (--s->match_length != 0);
    s->strstart++;
} else
{
    s->strstart += s->match_length;
    s->match_length = 0;
    ...
}

/* deflate.h:182 */ #   define max_insert_length  max_lazy_match
/* deflate.h:279 */ #define MIN_LOOKAHEAD (MAX_MATCH+MIN_MATCH+1)
/* deflate.h:284 */ #define MAX_DIST(s)  ((s)->w_size-MIN_LOOKAHEAD)
/* deflate.c:1314 (longest_match) */ chain_length >>= 2;   /* when prev_length >= good_match */
/* deflate.c:1319 */ if ((uInt)nice_match > s->lookahead) nice_match = (int)s->lookahead;
```

What D takes from these facts:
- **Chain budget and early stop:** the `chain` and `nice` columns per level.
- **Insertion inside a match (levels 1–3):** levels 1–3 insert every position inside a match only when the match
  length is ≤ the level's `lazy` column (`max_insert_length` is `max_lazy_match`); longer matches skip insertion.
  Levels 4–9 insert every position, which is the `deflate_slow` behaviour; D keeps greedy matching there, and E adds
  lazy evaluation.
- **zlib `FLEVEL`:** `0` for levels 0–1, `1` for 2–5, `2` for 6, `3` for 7–9. This matches the measured headers.
- **gzip `XFL`:** `2` at level 9, `4` at levels 0–1, else `0`. This matches the measured bytes.
- **Match window:** zlib stops a match `MIN_LOOKAHEAD` (262) short of its 32,768 window (`MAX_DIST`). DEFLATE allows
  32,768, and D's §4.2 uses the full RFC distance: a decoder accepts any distance ≤ 32,768.

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
`#[cfg(test)]` check. **Corrected by plan-137-A Corrections (2026-09-14):** build the 512-entry
table at program start with an MFB loop, not as a literal — a list literal lowers to ≈26 init
instructions per element (≈13,000 for this table); the `#[cfg(test)]` check pins the builder.

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

- [x] Fill the UNMEASURED rows in §2 (configuration table, FLEVEL/XFL mapping).
      (2026-09-14: both rows filled. The table and the `FLEVEL`/`XFL` expressions are pasted verbatim from the fetched
      1.2.12 sources with their sha256, and the per-level bytes are measured in Python and Node. The encoder-size row is
      Phase 3's by design.)

Acceptance: §2 rows filled with pasted evidence.
  Check: the commands in §2 (est. 10 min).
  (2026-09-14: `curl -sSfL …/v1.2.12/deflate.c` then `grep -n "configuration_table\[10\]" -A12` → the rows above; the
  Python/Node level sweep → the mapping above.)
Commit: 23a447a9f

### Phase 2 — raw `deflate`, levels 0–9

- [x] `compress/helper_bit_writer.rs`, `helper_match_finder.rs`, `helper_deflate_core.rs`,
      `func_deflate.rs`; level validation; helpers gated `WhenUsed(&["deflate", "zlibEncode", "gzipEncode"])`.
      (Files as shipped: `helper_deflate_codes.rs` (the fixed codes, bit-reversed at program start), `helper_deflate_core.rs`
      (bit writer, match finder and blocks in one function), `helper_deflate.rs` (`__compress_deflate`, level validation),
      `func_deflate.rs`; see Corrections for the layout. Codes and core are gated on the three encoders; the wrapper only
      on `deflate`. `helper_deflate_tables.rs`'s gate gains the encoders, since the code builders call its table
      functions. Unit test `fixed_code_builder_matches_rfc1951` pins the §3.2.6 ranges (Kraft sum 1) against the builder;
      `cargo test --bin mfb codegen::builtins::compress::tests` → `ok. 7 passed; 0 failed`.)
- [x] Oracle mode `encode-raw`: every §1 edge size and the bench corpus, levels 0–9 → Python
      `zlib.decompressobj(-15)` and Node `inflateRawSync` decode equal; our `inflate` decodes equal.
      (`gen.py` `encode_payloads` names every §1 edge (0/1/2 bytes, 65,535/65,536/65,537, 100,000 zeros for 258-byte
      matches, a 32,768-byte block ×3 for distance 32,768, 100,000 random bytes) plus the three oracle corpora: 12
      payloads × 10 levels = 120 cases. The probe writes its outputs to `MFB_COMPRESS_OUT`, and the judges decode that file
      and require exact equality with no trailing bytes. `run.sh … encode-raw` → `120/120 agreed with python`,
      `120/120 agreed with node`.
      Negative control (`/tmp/p137d-neg.py`): flipping one byte in case 96's stream makes the Python judge report
      `err … invalid literal/length/distance code`, where the clean file differs from the probe on 0 lines.)
- [x] Bench rows `deflate` level 1 / 6 / 9: MiB/s and ratio vs Python zlib at the same level.
      (`tools/compress-bench` gains ops `deflate1`/`deflate6`/`deflate9`: only the compression is timed, the result
      checked is the decompressed length and CRC-32, and both sizes are shown. Figures are in Corrections, "Phase 2 bench".)

Acceptance: zlib decodes every raw stream we produce, at every level and edge size.
  Check: `tools/oracles/compress/run.sh target/release/mfb encode-raw` → exit 0 (est. 15 min).
  (2026-09-14: exit 0, 120 cases, 0 failures; re-run on the Phase 3 core with its `prefix` parameter → 120/120 against
  both judges again.)
Commit: 9c75bf7f0

### Phase 3 — framing, tests, docs

- [x] `func_zlib_encode.rs`, `func_gzip_encode.rs`; oracle modes `encode-zlib`, `encode-gzip`
      (Python `zlib.decompress`, `gzip.decompress`; Node `inflateSync`, `gunzipSync`; plus `gzip -t`
      on the host for gzip output).
      (Framing lives in `helper_zlib_encode.rs` / `helper_gzip_encode.rs`, which hand their header to the core as its
      output prefix. The Adler-32 and CRC-32 helper gates gain `zlibEncode` / `gzipEncode`. The Python judge decodes
      with `zlib.decompressobj(15/31)` (the zlib-level decoder: `gzip.decompress` is a pure-Python header parse that
      accepts what zlib refuses, plan-137-B probe), requires the header bytes to equal zlib's own at the level (zlib
      `CMF`/`FLG`; gzip `1f8b08`, `FLG 0`, `MTIME 0`, `XFL`, `OS 255`), and runs host `gzip -t` on every member.
      `run.sh … encode-raw encode-zlib encode-gzip` → 120/120 per mode against both judges, `360 case(s), 0 failure(s)`.)
- [x] `tests/interop/rt_compress_interop.rs`: MFB encodes the seeded corpus at levels 0–9 in all
      three formats → `flate2` decodes equal; MFB `gzipEncode` of the same input twice → identical bytes.
      (`encoders_output_decodes_with_flate2_and_is_deterministic`: 7 payloads (0/1/2 bytes, 65,535 and 65,537 random
      bytes, 70,000 zeros, seeded text) × 3 formats × 10 levels = 210 cases. Each is compressed twice in one run and
      compared byte for byte, and the program is run twice and its output files compared.
      `cargo test --test rt_compress_interop` → `ok. 6 passed; 0 failed`.)
- [x] `tests/rt-behavior/compress/compress-encode-roundtrip-valid` (encode → decode with our own
      decoders, all formats and levels, prints lengths and a CRC of each output) and an rt-error
      fixture for `level := 10`.
      (`compress-encode-roundtrip-valid` covers a 2,672-byte input at levels 0–9 plus empty input, every line `TRUE`.
      `rt-error/compress/compress-gzip-encode-level-invalid` gives `before`, `Error: 7-705-0002`, `compress::gzipEncode: level
      must be from 0 to 9`. Goldens were created by `scripts/sync-goldens.sh` over empty placeholders, since the script
      never creates a file; the generated `build.log` and `.run` were read before keeping. `.run` is empty, as for every
      existing compress fixture. `scripts/test-accept.sh target/release/mfb /tmp/p137d 'compress-*'` →
      `acceptance tests passed (10 test(s) ran)`.)
- [x] Determinism across targets: cross-build the roundtrip fixture for `linux-aarch64`, run on box
      2223, and compare its printed output CRCs with macOS (`FILTER=compress scripts/linux-runtime-proof.sh target/release/mfb 2223 linux-aarch64 glibc`).
      (2026-09-14: `linux runtime proof: linux-aarch64/glibc on port 2223 — 9 passed, 0 failed, 0 not run`. The nine
      are the compress rt-behavior and rt-error fixtures. The roundtrip fixture's golden is the macOS output, with 33
      length-and-CRC-32 values, so the linux-aarch64 bytes are identical.)
- [x] Record emitted encoder size; man descriptors; `20_compress.md` encoder section (block policy,
      hash, chain budgets with the fetched table, FLEVEL/XFL mapping, determinism guarantee);
      byte-identity program extended and regenerated.
      (Size: §2 row. Man: `deflate`/`zlibEncode`/`gzipEncode` descriptors and the package `MODULE_INTRO`/`MODULE_DESC`.
      `scripts/man-run-examples.sh compress --run` → `examples: 12 built: 12 ran: 12 not run: 0 failed: 0`. Spec:
      `20_compress.md` "Encoding" (formats and header bytes, guarantee, blocks, matching with the fetched table, emitted
      size, bench table, verification) and the `spec.md` bullet; `cargo test -p mfb --bins citations_resolve` →
      `spec_citations_resolve ... ok`. Byte-identity: the program calls `deflate`, `zlibEncode(data, 0)` and `gzipEncode(data, 9)`.
      `.ast`/`.ir` came from `sync-goldens.sh`, whose diff is the program's six lines plus 703 added `.ir` lines. The five
      `.ncodesum` were rebuilt by `/tmp/p137d-ncodesum.py` exactly as the gate's pass 2 builds them. Then
      `bash scripts/artifact-gate.sh target/release/mfb compress` → `7 golden(s) checked, 0 diff(s)`.)

Acceptance: zlib decodes every framed stream; output identical on macOS and 2223; docs render.
  Check: `tools/oracles/compress/run.sh target/release/mfb encode-raw encode-zlib encode-gzip` → exit 0;
  `cargo test --test rt_compress_interop` → pass; `scripts/test-accept.sh target/release/mfb /tmp/p137d 'compress'` → 0 mismatches;
  `scripts/man-run-examples.sh compress --run` → pass (est. 25 min).
  (2026-09-14: the three encode modes exit 0; interop `ok. 6 passed; 0 failed`; `compress-*` acceptance 10 passed; man examples
  12/12 ran; box 2223 9 passed.)
Commit: 9c75bf7f0

## Validation Plan

- Tests: oracle encode modes (not CI), interop round-trips and determinism (CI), rt fixtures.
- Coverage check: every §1 edge size appears by name in the oracle job generator and the interop test.
- Runtime proof: roundtrip fixture on macOS and 2223 with matching CRCs.
- Doc sync: man descriptors + `20_compress.md`.
- Final gate: plan-137-E.

## Open Decisions

- Block size for fixed-Huffman blocks — 64 KiB of input (recommended: bounded symbol buffers,
  arbitrary but fixed) vs. one block per call (unbounded symbol buffer on 64 MiB inputs).
  **Resolved 2026-09-14:** 64 KiB, as recommended. With fixed codes the encoder writes symbols straight into the
  bit writer and buffers none, so the boundary matters only as the place plan-137-E's per-block choice will cut.

## Corrections

- **File layout differs from Phase 2's names** (2026-09-14). `helper_bit_writer.rs` and `helper_match_finder.rs` are not
  separate files. The bit writer and the match finder are inlined into `__compress_deflateCore`, for the reason
  plan-137-B's decoder core is one function: its Phase 1 measurement showed per-symbol helper calls and threaded state
  cost throughput. What would have been those files is `helper_deflate_codes.rs` (tables) plus `helper_deflate.rs`
  (validation).
- **No separate 512-entry reverse table** (§4.1). Only fixed codes exist in D, so they are reversed once at program start
  by `__compress_reverseBits` into the 288-entry literal/length table, the per-length table (259 entries, code plus
  extra bits packed) and the 30-entry distance table. Distances map to codes through zlib's 512-entry `dist_code`
  layout (`__COMPRESS_DIST_CODE`). The reversal function remains for plan-137-E's dynamic codes.
- **The core takes an output `prefix`** (not in §4). The zlib and gzip wrappers pass their header as the start of the
  output list, so the compressed data is never copied to prepend it. The Phase 2 oracle and bench ran before this change.
  The oracle was re-run after it (120/120 per judge). The bench was not: the change adds one copy of at most ten bytes
  per call.
- **Phase 2 bench** (2026-09-14, macos-aarch64,
  `OPT_LEVELS=1 ROUNDS=1 tools/compress-bench/run.sh target/release/mfb deflate1 deflate6 deflate9`). One `-O1`
  interleaved round was used, not the tool's default three rounds at `-O1` and `-O3`: the plan names no optimization
  level or round count, and one round keeps level 9 on 16 MiB of text affordable. Every result checked `ok`, and every
  16/4 MiB ratio is 3.77–4.30.

  | level | corpus (16 MiB) | MiB/s | Python MiB/s | size vs zlib |
  |---|---|---|---|---|
  | 1 | random | 8.0 | 58.8 | 1.054× |
  | 1 | text | 51.9 | 458.0 | 1.412× |
  | 1 | zero | 111.0 | 924.5 | 2.225× |
  | 6 | random | 8.0 | 55.5 | 1.054× |
  | 6 | text | 9.3 | 168.4 | 1.593× |
  | 6 | zero | 29.9 | 436.0 | 6.499× |
  | 9 | random | 8.2 | 56.0 | 1.054× |
  | 9 | text | 4.1 | 64.6 | 1.588× |
  | 9 | zero | 30.6 | 436.1 | 6.499× |

  The gap to zlib is the fixed code, and every factor above is measured:
  - random input costs its literal codes, where zlib stores the data;
  - a 258-byte match costs 13 bits, which is 105,993 B for 16 MiB of zeros against zlib's 16,310 B;
  - on the 12 oracle payloads, 100,000 zeros take 974 B at levels 1–3 and 635 B at levels 4–9, the effect of levels
    1–3 not inserting positions inside long matches, as `deflate_fast` does.

  Plan-137-E's dynamic blocks and per-block choice are the letter that earns ratio.
- **The first interop run failed 210/210 on my test, not the encoder** (2026-09-14). The MFB program printed each
  determinism flag with `toString(Boolean)`, which MFBASIC spells `TRUE`, while the test compared with `"true"`. No
  `flate2` decode failed: 210 failures for 210 cases, all "two calls differ". Proof that `TRUE` is the spelling: the
  roundtrip fixture's generated output (`… 1523927077 TRUE | …`). The comparison is now `"TRUE"`, with a comment.
- **`sync-goldens.sh` creates no golden** (its header: "New golden files are never created"). The two new fixtures'
  goldens were produced by syncing over empty placeholder files; each generated file was read before it was kept.

## Summary

The risk is bit-exact validity at boundaries, and MFB speed of a two-writes-per-byte match finder.
Every stream is judged by two external zlibs plus `flate2`; ratio is recorded, not gated, because
plan-137-E is where ratio is earned.
