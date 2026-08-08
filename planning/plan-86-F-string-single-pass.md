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
- [x] **F1** — case_map ASCII quick-check (landed plan-64).
- [ ] **F2** — memchr single-byte delimiter scan + 8-byte word-at-a-time block copy in split/join/mid; fuse
  split's two scans for a single-char delimiter. Helps string case/slice + strbuild splitjoin.
- [ ] **F3** — collapse case_map to a single pass (one over-allocate-to-byte-len write).

## Acceptance
string/strbuild checksums + `scripts/artifact-gate.sh`.
