# plan-137-E: dynamic Huffman, lazy matching, per-block type choice — and whole-feature validation

Last updated: 2026-09-13
Effort: large (3h–1d)
Depends on: plan-137-D. If plan-137-D is not complete, this plan cannot start, full stop.
Whole-feature prerequisites: plan-137-A §Prerequisites.

This letter earns compression ratio. The encoder gains dynamic-Huffman blocks with
length-limited canonical codes, lazy matching for the levels zlib runs lazily, and a per-block
choice of the smallest of stored / fixed / dynamic. Behavioural outcome: output still decodes with
zlib at every level, and on the bench corpus our compressed size at levels 6 and 9 is recorded
against zlib's — with dynamic blocks never larger than the fixed or stored encoding of the same
block. It closes plan-137 with the whole-feature validation run.

References:

- RFC 1951 §3.2.7 (dynamic block header: `HLIT`, `HDIST`, `HCLEN`, code-length alphabet with
  repeat codes 16/17/18, the code-length code order).
- Length-limited Huffman construction: package-merge (Larmore & Hirschberg, 1990) — exact optimum
  under the 15-bit (literal/length, distance) and 7-bit (code-length) limits. Fetch a primary
  description; cite it in the spec.
- zlib `deflate.c` `deflate_slow` and `configuration_table` (`good_length`, `max_lazy`) — fetched
  in plan-137-D Phase 1; `trees.c` for the block-type choice (`_tr_flush_block`).
- plan-137-B §4.2 (the decoder's validation rules — every code set the encoder emits must pass them).

## Prerequisites

See plan-137-A §Prerequisites. Additionally:

| Must be true | Command | Status |
|---|---|---|
| plan-137-D complete | `ls planning/completed/plan-137-D-*` → one file | MET (2026-09-14 re-run: `planning/completed/plan-137-D-compress-deflate-encoders.md`, archived in `2f361a878`). plan-137-A §Prerequisites re-run the same day: `ls bugs/completed/bug-621-*` → the append-growth fix, plus main's unrelated reuse of the number; `cargo build --release --bin mfb` → `Finished … 58.62s`; Python zlib `1.2.12`, Node zlib `1.3.1-470d3a2`; `Cargo.lock:1142` flate2 `1.1.9` |

## 1. Goal

- Dynamic blocks: per block, count symbol frequencies; build literal/length and distance code
  lengths by package-merge with limit 15; RLE the combined length sequence with 16/17/18; build the
  code-length code with limit 7; emit `HLIT`/`HDIST`/`HCLEN` trimmed per RFC 1951.
- Every block is emitted as the cheapest of stored / fixed / dynamic by exact bit count.
- Lazy matching for the levels whose fetched `configuration_table` row uses `deflate_slow`, with
  `good_length` / `max_lazy` / `nice_length` / `max_chain` from that row; the fast levels keep D's
  greedy finder.
- Edge cases emitted correctly: a block with no distance symbols (emit one distance code, as zlib
  does — verify by oracle), a block whose literal alphabet has a single used symbol (a complete
  code needs two codes — pad per zlib's behaviour, verified by oracle), frequencies that force the
  15-bit limit (Fibonacci-distributed input).
- Recorded: size ratio vs zlib and MiB/s at levels 1, 6, 9 on the bench corpus.

### Non-goals (explicit constraints)

- Not byte-identical to zlib; no ratio floor that stops the plan (the numbers are recorded and the
  spec states them as dated measurements).
- No decoder change except a bug the encoder exposes (fixed, tested, recorded).
- No change to D's level-0 stored output or to the framing bytes.

## 2. Current State

- plan-137-D's encoder: greedy hash chains, fixed-Huffman blocks every 64 KiB input, stored at level 0.
- plan-137-B's `__compress_buildTable` validates code sets; the encoder must only emit sets that
  pass it.

### Measured populations

| What | Count | Command |
|---|---|---|
| D's ratio and MiB/s per level (the baseline this letter improves) | Copied 2026-09-14 from plan-137-D Corrections, "Phase 2 bench" (macos-aarch64, `-O1`, 16 MiB corpora, size vs Python zlib at the same level). Level 1: random 8.0 MiB/s 1.054×, text 51.9 1.412×, zero 111.0 2.225×. Level 6: random 8.0 1.054×, text 9.3 1.593×, zero 29.9 6.499×. Level 9: random 8.2 1.054×, text 4.1 1.588×, zero 30.6 6.499× | read it before Phase 1 |
| zlib's behaviour for zero-distance-symbol and single-literal-symbol blocks | Measured 2026-09-14 (`/tmp/p137e-phase1.py`, an independent RFC 1951 header parser; Corrections). The listed inputs give **no dynamic block**: `b'a'*1000` and `bytes(range(256))*4` → fixed (`BTYPE 1`), and `bytes(range(256))` → stored, because zlib keeps the cheapest encoding. Inputs that force a dynamic block, all at level 9, raw: `b'a'*100000` default strategy → `HLIT 286, HDIST 2, HCLEN 18`, literal/length lengths `{97:2, 256:3, 281:3, 285:1}`, distance `{0:1, 1:1}` (one distance code used, a second of length 1 added); `b'a'*100000` `Z_HUFFMAN_ONLY` → `HLIT 257, HDIST 2, HCLEN 18`, literal/length `{97:1, 256:1}`, distance `{0:1, 1:1}` (**no distance symbol used, yet two distance codes of length 1 are sent**); 10,000 bytes of `a`/`b` 9:1 `Z_HUFFMAN_ONLY` → literal/length `{97:1, 98:2, 256:2}`, distance `{0:1, 1:1}`. The rule, fetched (`trees.c` 1.2.12, sha256 `56644256…1678`, lines 641–652): "The pkzip format requires that at least one distance code exists, and that at least one bit should be sent even if there is only one possible code. So to avoid special checks later on we force at least two codes of non zero frequency" — padding with symbol 0 or 1 (`max_code < 2 ? ++max_code : 0`). Both trees get this rule | Phase 1: `python3 -c "import zlib; …"` on `b'a'*1000` and `bytes(range(256))*4` with `zlib.compressobj(9, zlib.DEFLATED, -15)`, then parse the block header with B's decoder in a debug probe |

## 3. Design Overview

- **§4.1 package-merge** — a pure function over a frequency list returning code lengths; the only
  genuinely algorithmic piece, and the correctness risk: a wrong length set produces a stream zlib
  refuses (caught) or a suboptimal one (caught by the size check vs fixed).
- **§4.2 block cost + choice** — exact bit counts for the three encodings.
- **§4.3 lazy matching** — one-position deferral per `deflate_slow`.

Design uncertainty: MFB speed of package-merge on 286 symbols per block (small; measured in
Phase 2). Correctness risk concentrates in §4.1 and in header trimming; both are judged by zlib on
adversarial frequency distributions.

Rejected: heuristic depth-limiting (zlib's `gen_bitlen` overflow fix-up) — package-merge is exact
and simpler to prove; a single global Huffman table for the whole input (worse ratio, bigger
symbol buffers).

## 4. Detailed Design

### 4.1 Package-merge

Input: frequencies of used symbols (unused get length 0; one used symbol gets a second, per Phase 1
evidence). Build `limit` levels of packages over the sorted leaf list, take the first `2n - 2`
items of the last level, count each leaf's appearances → its code length. Implemented with
function-local `List OF Integer` work arrays (in-place `set`/`append` only), returning a fresh
lengths list. A `#[cfg(test)]` Rust reference in `compress/mod.rs` is **not** enough on its own
(it would share the author's misunderstanding) — the oracle (zlib decodes, and Kraft equality
`Σ 2^-len = 1` asserted by B's builder) is the judge.

### 4.2 Block cost and choice

Stored: `(3 + pad-to-byte) + 32 + 8·bytes`. Fixed: Σ fixed code lengths + extra bits. Dynamic:
header bits (`5+5+4 + 3·HCLEN` + RLE'd length codes with their extra bits) + Σ dynamic code
lengths + extra bits. Emit the minimum; ties prefer fixed, then stored (deterministic).

### 4.3 Lazy matching

At position `p` with match `m`, if `len(m) < max_lazy`, look at `p+1`; if its match is longer,
emit a literal for `p` and continue from `p+1`; halve `max_chain` when `len(m) ≥ good_length`;
stop searching at `nice_length`. Mirror the fetched `deflate_slow` control flow and cite it.

## Compatibility / Format Impact

Output bytes of `deflate` / `zlibEncode` / `gzipEncode` at levels 1–9 change (smaller). They were
never promised stable across plan-137 letters; within a release they are deterministic. Level 0
output is unchanged from D.

## Phases

> **NOTE — keep the checkboxes current as you go** (see plan-137-A). **An unticked box means NOT DONE.**

### Phase 1 — measure

- [x] Fill §2's UNMEASURED row; copy D's recorded ratio/MiB/s into this file as the baseline.
      (2026-09-14: both rows filled, see §2.)

Acceptance: §2 filled with pasted evidence.
  Check: the commands in §2 (est. 10 min).
  (2026-09-14: `python3 /tmp/p137e-phase1.py` → the headers pasted in §2.)
Commit: —

### Phase 2 — package-merge + dynamic blocks + block choice

- [x] `compress/helper_package_merge.rs`, `helper_dynamic_block.rs`, `helper_block_cost.rs`; the
      encoder core emits the chosen block type.
      (2026-09-14.
      - `helper_package_merge.rs`: `__compress_codeLengths`, which pads to two codes as `build_tree` does, then runs
        package-merge over a node pool; plus `__compress_canonicalCodes` and `__compress_zeroList`.
      - `helper_dynamic_block.rs`: `__compress_treeRle`, a transcription of zlib's `send_tree`, and
        `__compress_dynamicHeader` with `HLIT`/`HDIST` and `HCLEN` trimmed as `build_bl_tree` does.
      - `helper_block_cost.rs`: exact dynamic, fixed and stored bits, the stored count covering multi-chunk blocks and
        the pad at the block's bit position.
      - The core buffers each 64 KiB block's symbols and frequencies, chooses by bit count with ties going to fixed,
        then stored (Open Decisions), and writes the choice. Level 0 is unchanged.
      - `helper_deflate_codes.rs` gains `__COMPRESS_LEN_SYM`.
      - `cargo test --bin mfb codegen::builtins::compress::tests` → `ok. 7 passed; 0 failed`.)
- [x] Oracle job generator gains adversarial distributions: Fibonacci frequencies (forces the
      15-bit limit), single-symbol blocks, no-match blocks, all-distinct 256-byte cycles.
      (`gen.py` `adversarial_payloads`, appended to the encode payloads:
      - 17,710 shuffled bytes of 20 symbols with Fibonacci counts 1…6,765;
      - `bytes(65 + s for s in de_bruijn(32, 3))`, 32,768 bytes with no 3-byte repeat, so no match at all;
      - `bytes(range(256)) * 64`;
      - `b"a" * 200000`, a single literal symbol across four 64 KiB blocks.

      Each mode now has 16 payloads × 10 levels = 160 cases, declared in `run.sh`.)
- [x] Oracle modes `encode-raw`/`encode-zlib`/`encode-gzip` re-run; add assertion in the MFB probe
      that no emitted dynamic block is larger than its fixed or stored cost (printed per block, checked by `run.sh`).
      (The check lives in the Python judge rather than the probe; see Corrections. `audit_blocks` re-decodes every
      produced stream bit by bit. For each dynamic block it compares the bits used with the fixed-code cost of the same
      symbols and the stored cost of the same bytes at that bit position. Any excess fails the case as `BLOCKCOST`.
      `run.sh … encode-raw encode-zlib encode-gzip` → `160/160` against Python and Node in each mode,
      `480 case(s), 0 failure(s)`. The audit line reads, per mode: `106 stored, 36 fixed, 126 dynamic; longest
      literal/length code 15 bits, distance 11 bits; 1 case(s) with a 15-bit code`.)

Acceptance: zlib decodes everything; the size invariant holds on every block.
  Check: `tools/oracles/compress/run.sh target/release/mfb encode-raw encode-zlib encode-gzip` → exit 0 (est. 15 min).
  (2026-09-14: exit 0, 480 cases, 0 failures, no `BLOCKCOST`; 126 dynamic blocks per mode were audited against their
  fixed and stored costs.)
Commit: 7cce7cce0

### Phase 3 — lazy matching + records + docs

- [x] Lazy finder per §4.3 for the slow levels.
      (2026-09-14, `helper_deflate_core.rs`: levels 4–9 follow zlib 1.2.12's `deflate_slow` (`deflate.c` 1974–2081, fetched,
      sha256 `824ff399…0bce`):
      - the held match at `pos - 1` and a search at `pos` only while it is shorter than `max_lazy`, and only for a longer
        match;
      - the chain budget quartered at `good_length`;
      - `TOO_FAR` 4096 for three-byte matches;
      - emit-held-match-and-insert-its-positions, or held-literal.

      Levels 1–3 keep D's greedy finder. A byte still held at a block end is flushed as a literal in that block
      (Corrections).
      - `run.sh target/release/mfb encode-raw encode-zlib encode-gzip` → `480 case(s), 0 failure(s)`, no `BLOCKCOST`.
      - The same job through all three framings gives byte-identical DEFLATE payloads (0 of 160 differ), and two runs
        are identical (0 differ).)
- [x] Bench: ratio vs zlib and MiB/s at levels 1, 6, 9 recorded in Corrections and in
      `20_compress.md` as dated measurements.
      (Corrections, "Phase 3 bench", and the spec's "Measured throughput and size".)
- [x] `tests/interop/rt_compress_interop.rs`: the level sweep re-run (sizes change, validity must
      not); add the adversarial distributions.
      (`encode_payloads` gains Fibonacci frequencies, a de Bruijn B(32,3), 256-byte cycles and 200,000 × `a`:
      11 payloads × 3 formats × 10 levels = 330 cases. `cargo test --test rt_compress_interop` → `ok. 6 passed; 0 failed`
      on the Phase 2 core and again on the lazy core. Re-run after the 65,535-byte block fix → `ok. 6 passed; 0 failed`.)
- [x] Byte-identity program regenerated; man `desc` of the three encoders states what `level`
      trades (effort vs size), no internals; spec encoder section updated.
      (Byte-identity: `scripts/sync-goldens.sh target/release/mfb byte-identity/compress compress-encode-roundtrip-valid compress-gzip-encode-level-invalid` → `synced 9 golden file(s) across 3 test(s)`; the five `.ncodesum` rebuilt by `/tmp/p137d-ncodesum.py` as the gate's pass 2 builds them; `bash scripts/artifact-gate.sh target/release/mfb compress` → `7 golden(s) checked, 0 diff(s)`. the same sync regenerated the two encoder rt fixtures, and only their `.ir` changed,
      because the injected helper source did. `git diff --stat` shows no `build.log` change. On the roundtrip fixture's
      2,672-byte input, every level's output has the same lengths and CRC-32s as under plan-137-D, so the bytes are
      identical; `scripts/test-accept.sh target/release/mfb /tmp/p137e 'compress-*'` → `acceptance tests passed (10 test(s) ran)`, and the roundtrip `build.log` still prints `TRUE` for every format and level (`grep -c FALSE` → 0).
      Man: the three `desc`s already say "`level` trades time for size … a higher level takes longer and usually gives
      a smaller result"; the dynamic blocks keep that true and name no internals. `scripts/man-run-examples.sh compress --run` → `examples: 12 built: 12 ran: 12 not run: 0 failed: 0`.
      Spec: `20_compress.md` "Blocks" (per-block choice, dynamic trees, package-merge, padding, header rules), "Matching"
      (greedy 1–3, lazy 4–9, `max_lazy`/`good_length` rows) and "Measured throughput and size", plus the `spec.md`
      bullet. `cargo test -p mfb --bins citations_resolve` → `1 passed`.)

Acceptance: validity everywhere; level 9 output ≤ level 1 output on every corpus file (recorded;
a violation is investigated and explained in Corrections, not hidden).
  Check: `cargo test --test rt_compress_interop`; `tools/compress-bench/run.sh target/release/mfb` (est. 15 min).
  (2026-09-14: interop `ok. 6 passed`; bench `OPT_LEVELS=1 ROUNDS=1 … deflate1 deflate6 deflate9`, exit 0, every row `ok`.
  Level 9 output ≤ level 1 output on every corpus file: yes (random 1 MiB: level 9 1,048,659 B, level 1 1,048,659 B; random 4 MiB: level 9 4,194,629 B, level 1 4,194,634 B; random 16 MiB: level 9 16,778,501 B, level 1 16,778,511 B; text 1 MiB: level 9 79,099 B, level 1 94,045 B; text 4 MiB: level 9 316,167 B, level 1 376,304 B; text 16 MiB: level 9 1,264,348 B, level 1 1,505,437 B; zero 1 MiB: level 9 1,220 B, level 1 4,792 B; zero 4 MiB: level 9 4,867 B, level 1 19,155 B; zero 16 MiB: level 9 19,452 B, level 1 76,606 B)..)
Commit: —

### Phase 4 — whole-feature validation (plan-137 final gate, runs once)

- [ ] `cargo build --release --bin mfb`, then
      `cargo test --no-fail-fast > /tmp/p137-final.log 2>&1; echo EXIT=$?` and
      `grep -c '^failures:' /tmp/p137-final.log` → 0 (includes `artifact_gate_all`).
- [ ] `scripts/test-accept.sh target/release/mfb /tmp/p137-accept` → 0 mismatches.
- [ ] `bash scripts/man-examples-gate.sh target/release/mfb`; `scripts/man-census.sh --memory-scope`;
      `scripts/spec-census.sh --citations`; `cargo test -p mfb --bins citations_resolve`.
- [ ] `tools/oracles/compress/run.sh target/release/mfb` (all modes) → exit 0.
- [ ] Runtime proof on the boxes: the roundtrip fixture and `compress-decode-valid` cross-built and
      run on 2223 (`linux-aarch64` glibc), 2230 (`windows-x86_64`, shipped with `scripts/remote-common.sh`
      `win_ship`; the first Windows execution of this package), and one short run on 2228
      (`linux-x86_64` glibc, emulated — only these two fixtures). Output CRCs identical to macOS.
- [ ] Size probe: `IMPORT io` + `IMPORT compress` with no call equals the `IMPORT io` baseline; record
      one-member deltas for `crc32`, `gzipDecode`, `gzipEncode`.
- [ ] Archive plan-137-E to `planning/completed/`. (Corrected 2026-09-14: A–D are archived as each
      completes — see Corrections.)

Acceptance: all gates above green, runtime proven on four targets, sizes recorded.
  Check: the commands above (est. 60 min — the full suite is required once by `.ai/testing-gates.md`;
  no scoped run covers every importer golden and the other packages' byte-identity).
Commit: —

## Validation Plan

- Tests: oracle (all modes), `rt_compress_interop`, `rt_compress_bounds`, rt fixtures, canvas decode tests.
- Coverage check: each public member is executed by an rt fixture and the interop test; each refusal
  class and each edge size is named in a test.
- Runtime proof: Phase 4's four-target run.
- Doc sync: man descriptors for all seven members; `20_compress.md` complete (formats, strictness,
  bounds, algorithms, determinism, dated measurements); `02_error-codes.md`; canvas docs (C).
- Final gate: Phase 4 — the only full-suite run of plan-137.

## Open Decisions

- Tie-break order between equal-cost block types — fixed, then stored, then dynamic (recommended:
  cheapest header to decode) vs. dynamic first.
  **Resolved 2026-09-14:** fixed, then stored, then dynamic, as recommended. zlib's `_tr_flush_block` prefers stored
  over fixed on a tie (Corrections). The difference shows only when the two cost exactly the same bits, and neither
  order is part of the contract, since the output is not byte-identical to zlib.

## Corrections

- **Phase 1's inputs could not show a dynamic header, and the parser is not B's decoder** (2026-09-14).
  `b'a'*1000` and `bytes(range(256))*4` at level 9 compress to fixed blocks (11 B and 280 B), so there is no `HLIT` /
  `HDIST` to read. The probe adds inputs that make zlib choose `BTYPE 2`: `b'a'*100000`, and `Z_HUFFMAN_ONLY` streams for
  the no-distance-symbol case. The headers are parsed by an independent Python RFC 1951 header reader rather than a
  debug build of `__compress_inflateCore`. B's core validates code sets but prints nothing, and adding a debug surface
  would be test-only code in the product. The facts come from zlib's output and from `trees.c`, not from our decoder's
  view of it.
- **zlib's tie order for the block type is stored, then fixed, then dynamic** (2026-09-14, `trees.c` `_tr_flush_block`,
  lines 950–981). It computes `opt_lenb = (opt_len+3+7)>>3` and `static_lenb`, sets `opt_lenb = static_lenb` when
  `static_lenb <= opt_lenb`, writes a stored block if `stored_len+4 <= opt_lenb`, else fixed if `static_lenb == opt_lenb`,
  else dynamic. The Open Decision's recommended order (fixed, then stored, then dynamic) differs only when stored and
  fixed cost the same; §4.2's order is settled in Phase 2 with this citation.

- **Phase 4 "Archive plan-137-A…E" contradicts the letters' own gates** (recorded by plan-137-A on
  2026-09-14, before this letter started). plan-137-B's Prerequisites row is
  `ls planning/completed/plan-137-A-*` → one file, and C, D and E gate the same way on B, C and D; archiving
  every letter only here would leave each of those rows NOT MET forever. Each letter is archived when it
  completes (plan-137-A: moved to `planning/completed/` in the commit after `5b9aaa579`); this letter's
  Phase 4 task archives E only.

- **The size-invariant check is in the oracle's Python judge, not printed by the MFB probe** (2026-09-14). The probe
  calls the public members, which expose no block costs. Printing them would mean a debug surface in the product's
  encoder, compiled into every program that compresses. The judge instead re-decodes each produced stream
  independently and recomputes, per block, the fixed-code cost of the same symbols and the stored cost of the same
  bytes. That is a stronger check than the encoder's own claim, and it runs in `run.sh` for all three encode modes.
- **The first Phase 2 build failed to parse: `step` is a keyword** (2026-09-14). The package-merge loop counter was
  renamed from `round` to `step`, and `STEP` is in the keyword set (`mfb spec language lexical-structure`: "… RES STEP
  TO TYPE …"). The injected helper failed with `MFB_PARSE_UNEXPECTED_STATEMENT` at `<builtin-compress_package_merge>`
  lines 126–134, and the oracle build stopped with exit 2. It is renamed `packLevel`. A scan of every new helper body
  for bindings named after keywords then found none.
- **Phase 3 bench** (2026-09-14, macos-aarch64,
  `OPT_LEVELS=1 ROUNDS=1 tools/compress-bench/run.sh target/release/mfb deflate1 deflate6 deflate9`). These are the
  settings of plan-137-D's baseline, so each row compares directly with it: the plan names no optimization level or
  round count.

  | level | corpus (16 MiB) | MiB/s | D MiB/s | Python MiB/s | size vs zlib | D size vs zlib |
  |---|---|---|---|---|---|---|
  | 1 | random | 7.1 | 8.0 | 58.2 | 1.000× | 1.054× |
  | 1 | text | 38.4 | 51.9 | 496.6 | 1.004× | 1.412× |
  | 1 | zero | 97.8 | 111.0 | 918.4 | 1.046× | 2.225× |
  | 6 | random | 7.2 | 8.0 | 56.3 | 1.000× | 1.054× |
  | 6 | text | 13.1 | 9.3 | 182.9 | 1.005× | 1.593× |
  | 6 | zero | 29.6 | 29.9 | 395.0 | 1.193× | 6.499× |
  | 9 | random | 7.2 | 8.2 | 55.8 | 1.000× | 1.054× |
  | 9 | text | 2.5 | 4.1 | 63.1 | 1.005× | 1.588× |
  | 9 | zero | 29.4 | 30.6 | 434.4 | 1.193× | 6.499× |

  Every row checked `ok`; the 16/4 MiB time ratios are 3.92–4.05.
- **The audit lines differ between modes because each mode's payloads are seeded differently** (2026-09-14). The lazy
  run printed a longest distance code of 12 bits for `encode-raw` and 11 for `encode-zlib`/`encode-gzip`, with 2/2/3
  cases hitting 15 bits, although the three formats wrap one core. Localized:
  - `gen.py` seeds `random.Random(f"{SEED}-{mode}")`, so the random and shuffled payloads differ per mode;
  - the same `encode-raw` job pushed through all three framings gives byte-identical DEFLATE payloads (0 of 160
    differ), and two runs are identical (0 differ), by `/tmp/p137e-framecmp.py`;
  - the judge's own `audit_blocks` over those outputs gives the same line for every framing: 126 dynamic blocks, 12-bit
    longest distance, 15-bit codes in cases 113 and 116 (`/tmp/p137e-auditcmp.py`).

  There is no encoder or audit defect.
- **Blocks were one byte too long to store in one piece; they are now 65,535 bytes of input** (2026-09-14). The first
  Phase 3 bench failed the acceptance clause "level 9 output ≤ level 1 output on every corpus file" on random 4 MiB:
  level 9 wrote 4,194,944 B, level 1 4,194,939 B. Localized with `/tmp/p137e-blocks.py`, which encodes that corpus at
  both levels and walks the blocks:
  - level 1 wrote 127 stored blocks, 63 of 65,535 bytes, 62 of 1 byte, one of 3 and one of 65,534;
  - level 9 wrote 128 stored blocks, 64 of 65,535 bytes and 64 of 1 byte.

  A block gathered from 65,536 input bytes cannot be one stored block (`LEN` is at most 65,535), so every
  incompressible block paid a second 5-byte header for its last byte. The two levels differed only in where a match
  moved a block end. Blocks now end at 65,535 input bytes (`helper_deflate_core.rs`, `20_compress.md`). This revises
  plan-137-D's recorded 64 KiB choice. After the fix: the same `/tmp/p137e-blocks.py` gives level 1 4,194,634 B in 66 stored blocks (64 × 65,535, one of 1 and one of 63,
  where a match ran past a block end) and level 9 4,194,629 B in 65 (64 × 65,535 and a final 64). Level 9 is now the
  smaller, and both levels save about 310 B on the 4 MiB corpus. Across the 480 encode cases the oracle still agrees on
  every one, and the per-mode audit counts 79 stored, 45 fixed and 126 dynamic blocks, where it counted 106 stored and 36 fixed.
- **A held byte is flushed at a block end** (not in zlib). zlib's `deflate_slow` keeps `match_available` across
  `FLUSH_BLOCK`. Here a block is priced as stored from exactly the bytes `[blockStart, pos)`, so its symbols must cover
  exactly those bytes. The byte held at `pos - 1` when the block ends is therefore written as a literal in that block,
  and the next block starts with no held match. At most one lazy decision per 64 KiB block differs from zlib's.
- **The 15-bit limit binds in one case per mode**, a single level of the Fibonacci payload (audit line above). At the
  other levels, matches take enough of the most frequent symbols out of the literal stream that the optimal code fits
  in 15 bits. The oracle shows the limited code is valid where it binds.
- **The compress byte-identity and roundtrip goldens are stale after this phase, by design.** Every program that calls
  an encoder now injects the dynamic-block helpers, and levels 1–9 emit different bytes. Phase 3's byte-identity
  regeneration task refreshes them, together with `compress-encode-roundtrip-valid`'s `build.log`, whose output lengths
  and CRCs change with the block choice.

## Summary

The engineering risk is package-merge and header trimming — judged by zlib on adversarial
distributions — and the final multi-target proof, which is the first time this package runs on
Windows. Nothing outside `compress::` changes in this letter.
