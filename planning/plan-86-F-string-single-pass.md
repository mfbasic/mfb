# plan-86-F — string single-pass / memchr

Sub-plan **F** of [plan-86](plan-86-benchmark-perf.md). Open (F1 landed plan-64).

**Covers (2 P1 + 1 P2):** string case (48.7), strbuild splitjoin (11.35), string slice (36.7). All
genuine/linear.

## Root cause
The strings family copies **byte-at-a-time** through `emit_materialize_string_from_bytes`
(`builder_collection_layout.rs:2287-2295`) and inline split/join loops (no memchr / word-copy):
`lower_strings_case_map` (`builder_strings_builtins.rs:461`, two byte passes even after F1),
`lower_strings_split` (2-pass byte scan), `lower_strings_join` (byte loops), `lower_mid`
(`builder_search.rs`, byte-copy). Bounded, ~2×; op-count/allocation-bound.

## Fixes
> **MEASURED this session (release, `--run 10`, box-local) — the scout's "marginal" is CONFIRMED and QUANTIFIED,
> and the rows are ALLOC/CALL-bound, not copy-bound.** `string case` 47.2 ms, `string slice` 34.5 ms.
> `test_string_case` (`string.mfb:29`) = 50000 iters × 8 ops (upper/lower/caseFold/trim×4/normalizeNfc) on a
> ~20-byte string = **400 000 string ops at ~118 ns/op**. For a 20-byte string the byte-copy/pass F2 replaces is
> ~20 iterations ≈ a few ns — well under ~10 % of the 118 ns/op; the **arena alloc of the result + the builtin
> call dominate**, and F2/F3 reduce NEITHER. So F2 (word-copy) ≈ +6 %, F3 (one fewer pass) another small slice —
> `case` → ~44 ms (STILL P1 vs py 27.9), `slice` → ~30 ms (STILL P2). This is the same alloc/call-bound floor as
> `[[native-string-hof-rewrites-are-marginal]]`; the plan's own acceptance already says F is "a modest ~1.3–2×
> row improvement, NOT a clear." **Cost side: F2's widest swap (`emit_materialize_string_from_bytes`) is on the
> to_bytes/repeat/materialize funnel used by ~every string builtin → swapping it churns the `.ncode` of ~every
> string-touching byte-identity fixture (~15–20 builtins × 5 targets ≈ 75–100 `.ncodesum`) + many rt-behavior
> `.ncode`.** Real-but-small win, prohibitive golden churn, does not clear either row — LOW priority; if pursued,
> do the 4 byte-exact `emit_block_copy_advance` swaps together in ONE pass and budget a full multi-builtin
> `.ncodesum` regen. The edit points below are precise and correct.
- [x] **F1** — case_map ASCII quick-check (landed plan-64).
- [ ] **F2** — 8-byte word-copy + SWAR memchr. **TRACTABLE + near-zero-risk (scout, plan-86-A session), but
  MARGINAL (~1.3–1.8×, NOT band-clearing).** The word-copy helper ALREADY EXISTS: `emit_block_copy_advance`
  (`builder_collection_layout.rs:171-209`, an 8-byte load_u64/store_u64 loop + byte tail) is already used by
  the list/slice/map bulk paths — the string byte-copy loops are the outliers that never adopted it. **Swap
  these 4 byte-at-a-time loops to `emit_block_copy_advance` (dst/src/remaining are already set up at each):**
  (1) `emit_materialize_string_from_bytes` `builder_collection_layout.rs:2303-2311` (the widest — to_bytes/
  repeat/materialize funnel); (2) string `mid`/slice `builder_search.rs:910-918`; (3) split segment copy
  `builder_strings_package.rs:307-316`; (4) join delim+value copies `builder_strings_builtins.rs:1552-1560`
  and `1574-1582`. All byte-exact (moves UTF-8 verbatim), no Unicode concern. (5) **SWAR single-byte memchr**
  (no runtime memchr symbol exists — open-code it, model on `emit_ascii_scalar_fastforward` `builder_search.rs:33-56`,
  mask `0x8080808080808080`) for split's inner delimiter compare (`builder_strings_builtins.rs:1695-1697`,
  `1810-1812`), gated on `delimiterLen == 1` (the benchmark's `,`). **Why marginal:** the benchmark strings are
  SHORT (case/slice 13–21 B; split = 100 tiny fields), so the copy is a small fraction — allocation + call
  overhead dominate; word-copy cuts per-byte work ~8× but that's not the bottleneck. Same class as
  chunks/window/zip (`[[native-string-hof-rewrites-are-marginal]]`).
- [ ] **F3** — collapse case_map to a single ASCII pass. **HIGHER-RISK (control-flow restructure), do 2nd.**
  The ASCII fast path (`builder_strings_builtins.rs:521-568`) is ITSELF two byte passes — a quick-check scan
  (`:521-529`) then a transform-copy (`:559-568`). For ASCII, out-len == byte_len exactly, so allocate
  `byte_len+9` up front (byte_len already in scratch21 `:518`) and fuse to ONE transform-copy pass that bails
  to `case_slow` (`:574`, the Unicode two-pass, REQUIRED for ß→ss width changes) the moment a byte ≥0x80 is
  seen. The cheaper half — SWAR-ing the quick-check scan's per-byte `compare 128` (`:526`) to a word high-bit
  test — can land independently. **~1.5–2× on the case_map component, diluted by the row's 6 other ops** (trim*,
  normalizeNfc). **NET: F is a modest ~1.3–2× row improvement, not a clear** — the big benchmark wins are the
  O(container)-copy eliminations (groupBy 445×, removeKey 7.3×), not short-string byte-copy tuning.

## Acceptance
string/strbuild checksums + `scripts/artifact-gate.sh`.
