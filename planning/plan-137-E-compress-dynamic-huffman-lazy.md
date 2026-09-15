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
| plan-137-D complete | `ls planning/completed/plan-137-D-*` → one file | NOT MET (2026-09-13 re-run: `ls planning/completed | grep -c plan-137` → 0; blocked on plan-137-A's bug-621 row) |

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
| D's ratio and MiB/s per level (the baseline this letter improves) | from plan-137-D Corrections | read it before Phase 1 |
| zlib's behaviour for zero-distance-symbol and single-literal-symbol blocks | UNMEASURED | Phase 1: `python3 -c "import zlib; …"` on `b'a'*1000` and `bytes(range(256))*4` with `zlib.compressobj(9, zlib.DEFLATED, -15)`, then parse the block header with B's decoder in a debug probe |

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

- [ ] Fill §2's UNMEASURED row; copy D's recorded ratio/MiB/s into this file as the baseline.

Acceptance: §2 filled with pasted evidence.
  Check: the commands in §2 (est. 10 min).
Commit: —

### Phase 2 — package-merge + dynamic blocks + block choice

- [ ] `compress/helper_package_merge.rs`, `helper_dynamic_block.rs`, `helper_block_cost.rs`; the
      encoder core emits the chosen block type.
- [ ] Oracle job generator gains adversarial distributions: Fibonacci frequencies (forces the
      15-bit limit), single-symbol blocks, no-match blocks, all-distinct 256-byte cycles.
- [ ] Oracle modes `encode-raw`/`encode-zlib`/`encode-gzip` re-run; add assertion in the MFB probe
      that no emitted dynamic block is larger than its fixed or stored cost (printed per block, checked by `run.sh`).

Acceptance: zlib decodes everything; the size invariant holds on every block.
  Check: `tools/oracles/compress/run.sh target/release/mfb encode-raw encode-zlib encode-gzip` → exit 0 (est. 15 min).
Commit: —

### Phase 3 — lazy matching + records + docs

- [ ] Lazy finder per §4.3 for the slow levels.
- [ ] Bench: ratio vs zlib and MiB/s at levels 1, 6, 9 recorded in Corrections and in
      `20_compress.md` as dated measurements.
- [ ] `tests/interop/rt_compress_interop.rs`: the level sweep re-run (sizes change, validity must
      not); add the adversarial distributions.
- [ ] Byte-identity program regenerated; man `desc` of the three encoders states what `level`
      trades (effort vs size), no internals; spec encoder section updated.

Acceptance: validity everywhere; level 9 output ≤ level 1 output on every corpus file (recorded;
a violation is investigated and explained in Corrections, not hidden).
  Check: `cargo test --test rt_compress_interop`; `tools/compress-bench/run.sh target/release/mfb` (est. 15 min).
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

## Corrections

- **Phase 4 "Archive plan-137-A…E" contradicts the letters' own gates** (recorded by plan-137-A on
  2026-09-14, before this letter started). plan-137-B's Prerequisites row is
  `ls planning/completed/plan-137-A-*` → one file, and C, D and E gate the same way on B, C and D; archiving
  every letter only here would leave each of those rows NOT MET forever. Each letter is archived when it
  completes (plan-137-A: moved to `planning/completed/` in the commit after `5b9aaa579`); this letter's
  Phase 4 task archives E only.

## Summary

The engineering risk is package-merge and header trimming — judged by zlib on adversarial
distributions — and the final multi-target proof, which is the first time this package runs on
Windows. Nothing outside `compress::` changes in this letter.
