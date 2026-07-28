# plan-25-D: Map probe, io-write buffer, and parse accumulators

Last updated: 2026-07-05
Effort: medium (1h–2h)

Three independent Goal-1 gaps that share no code but are each a self-contained
codegen/source improvement:

1. **Map lookup/set pay a runtime-helper call per probe.** `map lookup` (6.1 ms
   vs Python 1.4) and `map set` (1.07 vs 0.19) call `_mfb_rt_map_probe` with a
   `bl`+relocation per operation and re-hash string keys every time.
2. **`fs::writeAll` copies its payload into the stdout/file buffer a byte at a
   time.** `io write` (26 ms vs 2.4) is dominated by the per-byte append loop.
3. **The csv/json parsers accumulate rows/fields with functional append and build
   strings with O(N²) concatenation.** `parse csv` (10.3 vs 0.72), `parse json`
   (8.4 vs 0.22).

It complements:

- `./mfb spec package collections` (map semantics), `./mfb spec package io`
  (buffered write contract from plan-14), `./mfb spec package csv` / `json`.
- `planning/plan-25-A` (arena) and `plan-25-B` (bulk copy) — D reuses B's
  `emit_block_copy_advance` for the io buffer.

## 1. Goal

- Map `get`/`set` on probe-eligible key types inline the hash probe (no per-op
  `bl` to `_mfb_rt_map_probe`) and avoid redundant re-hash of the same key in a
  build loop.
- `fs::writeAll` / stdout buffer append copies its payload with the word-loop
  helper, not per byte.
- csv/json parsers use MUT in-place accumulators (rows, fields) and chunked
  string building, dropping O(N²) to O(N).

### Non-goals (explicit constraints)

- No change to map iteration order, hashing algorithm result, or collection
  layout; no change to csv/json output values (byte-identical parse results).
- No change to the io buffering *contract* (plan-14 `io::setBuffered`/`flush`
  semantics) — only the copy loop inside it.

## 2. Current State

- **Map probe.** `emit_map_probe` (`builder_collection_query.rs:123-217`) emits a
  `bl` to the runtime probe helper per lookup and re-derives the FNV-1a hash and
  string comparison each call; map `set` (`builder_collection_mutate.rs:1697+`)
  goes through the same probe plus a payload copy even on the same-length update
  path.
- **io write buffer.** `emit_append_to_stdout_buffer`
  (`src/target/shared/code/io_helpers.rs:76-172`) has a byte-at-a-time
  `load_u8`/`store_u8` copy loop (`:157-165`); large chunks branch to a direct
  write.
- **csv parser** (`src/builtins/csv_package.mfb`): grapheme-split then
  `row = collections::append(row, field)` / `rows = collections::append(rows,
  row)` functional appends; row list grows O(N) per row → O(N²).
- **json parser** (`src/builtins/json_package.mfb`): `__json_parseArrayItems`
  uses MUT append (already), but `__json_parseString` builds via `current &
  chunk` string concat (`:~455`) → O(N²) per string.

## 3. Design Overview

- **D1 — inline map probe + key-hash caching.** Emit the FNV-1a hash and
  first-bucket probe inline for probe-eligible key types (the common Integer/String
  case), falling back to the helper only for the slow collision path. In a
  `set`-in-loop, cache the last key's hash/bucket to skip re-hash when the same
  key repeats.
- **D2 — word-copy io buffer.** Replace the byte loop in
  `emit_append_to_stdout_buffer` with `emit_block_copy_advance` (the same helper
  plan-25-B uses).
- **D3 — parser accumulators.** Switch csv row/field building to MUT in-place
  append (benefits directly from plan-25-B's bulk/append fast paths); switch
  `__json_parseString` to accumulate chunks in a list and `join` once (or a MUT
  string builder) instead of repeated concat.

## Status: COMPLETE — commit 20e9e91d (1040/1040 acceptance green, zero .run diffs)

### Phase 1 — D2: io-write word copy — DONE

- [x] Replace the per-byte copy loop with the word-then-byte block copy
      (`emit_block_copy_advance` shape) in BOTH the stdout buffer
      (`io_helpers.rs`) and the file buffer (`fs_helpers_io.rs`).

Result: buffered byte stream byte-identical (regenerated control-flow-if /
parser-hello-world `.ncode`/`.mir`). The `io write` benchmark did **not** move
(26.5 ms): it opens a File (unbuffered by default) and writes 20000 × ~6-byte
chunks via `fs::writeAll`, so it is syscall-bound and never reaches the buffer
copy — and even buffered, a 6-byte payload is all byte-tail (< 8 B). The
word-copy speeds the buffered path for real payloads (`io::setBuffered(TRUE)` /
buffered Files), which this fixture does not exercise. The plan's −55% premise
("dominated by the per-byte append loop") did not hold for the benchmark.

### Phase 2 — D3: parser accumulators — DONE

- [x] csv `row`/`rows` accumulators already hit the in-place MUT append fast
      path (`try_inplace_append_assign`: single-element append, item type ==
      element type) — no source change needed (verified).
- [x] json: `__json_parseString` rewritten from per-character `current & ch`
      recursion to an iterative MUT chunk-list + one `strings::join`
      (`json_package.mfb`): O(n^2)/one-frame-per-char → O(n).
- [x] Tests: `func_csv_*` unchanged; `func_json_*` parse results byte-identical
      (regenerated `.ir` goldens).

Result: parse outputs byte-identical. `parse json`/`parse csv` were already
mostly improved by plan-25-A/B; the json rewrite fixes the real O(n^2) on *long*
JSON strings, but the benchmark only parses two short keys, so its median is
unchanged.

### Phase 3 — D1: inline map probe — DONE (key-cache deferred)

- [x] Inline the FNV-1a hash + first-bucket probe for probe-eligible keys in
      `emit_map_probe`; helper fallback only on the buckets-not-built and
      hash-collision paths (`builder_collection_query.rs`). Mirrors
      `lower_map_probe_helper` exactly → resolved entry byte-identical.
- [ ] Cache last-key hash/bucket across a `set` loop — DEFERRED: only helps a
      repeated-same-key (loop-invariant key) set loop, a pattern absent from the
      benchmark and idiomatic code; needs cross-statement loop-invariance
      analysis against the byte-identical gate for no measured gain.
- [x] Tests: `func_collection_get/set/hasKey_*` + runtime spot tests
      (String/Integer/Float/Byte/Boolean keys, 20k-key collisions, overwrite,
      removeKey rebuild) — iteration order + values byte-identical.

Result: correct and byte-identical, but `map lookup` (3.47→3.42) / `map set`
(0.18→0.20) barely moved — the removed per-op `bl` was not the dominant cost;
integer-key FNV hashing + division + the general get/set materialization
dominate and cannot change without altering the hash result. The −40%/−50% was
optimistic for these fixtures.

## Layout / ABI Impact

None. Hashing result, bucket layout, iteration order, parse output, and buffered
byte stream are all unchanged — only the emitted copy/probe code differs.

## Validation Plan

- Function tests: map get/set/hasKey (all key types), csv/json parse fixtures.
- Runtime proof: buffered io write byte-for-byte identical to unbuffered concat.
- Acceptance: `scripts/test-accept.sh`.

## Theorized gains (median)

| bench       | now (ms) | driver                         | Δ    |
|-------------|---------:|--------------------------------|-----:|
| io write    |   26.2   | D2 word-copy buffer            | −55% |
| parse csv   |   10.3   | D3 in-place accumulators (+A/B)| −75% |
| parse json  |    8.4   | D3 chunked string build (+A/B) | −65% |
| parse regex |   80.3   | benefits from A churn + string | −40% |
| map lookup  |    6.14  | D1 inline probe (+A)           | −40% |
| map set     |    1.07  | D1 inline probe + key cache    | −50% |

## Summary

Three unrelated, independently-landable improvements; D1 (map probe inlining)
carries the most risk because it touches hash codegen — gated by the byte-identical
map goldens. D2 and D3 are mechanical reuse of existing helpers and source-package
idioms.
