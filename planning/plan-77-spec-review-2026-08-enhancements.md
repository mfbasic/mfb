# Spec-review enhancements ledger (2026-08-02)

Last updated: 2026-08-02

> **Execution note (2026-08-02):** the sections above are the verified *research
> ledger*. The **Prerequisites**, **Implementation phases**, and **Corrections**
> sections at the bottom of this file make the ledger executable: every
> ✅-Confirmed item is turned into an ordered, independently-landable phase.
> Overall effort: **huge (>3d)** — 15 phases across 5 domains. Phases are ordered
> value-dense-and-tractable first (size → regex perf → csv), highest-risk last
> (R2 external-data, M6 new-analysis-pass). The domains are mutually independent:
> no phase depends on an earlier phase's *code* (only on the green baseline), so
> the order is a value/risk ranking, not a dependency chain. Anchors below were
> re-verified against source on 2026-08-02 (5-way read-only fan-out); drifted
> line numbers and the M6/R2 re-framings are in **Corrections**.

Source: a batch of ~35 AI-generated "suggestions" across unicode, threads, memory, regex, datetime, and csv. Every claim was verified against current source/spec (read-only fan-out, one verifier per domain). This file collects the ones that **verified as real, non-defect work** (perf / size / feature). The one real *correctness* bug is filed separately as **bug-425** (failed `thread::transfer` leaks the sender fd). Items marked ❌ below are recorded here **so they are not re-filed** — they are already-fixed, misreadings, or would regress intended behavior.

Nothing here is a correctness defect. None is urgent. Pick up individually if/when the size or hot-path cost matters.

## Verdict summary (all claims)

| # | Claim | Verdict | Disposition |
| --- | --- | --- | --- |
| U1 | `PackedProperty` carries 5 unread u16 fields (`decomp_*`, `*_seqindex`) | ✅ Confirmed | **Size** — see below (~82 KB/binary) |
| U2 | `sequences` table embedded but no codegen path reads it | ✅ Confirmed | **Size** — ~25 KB/binary |
| U5 | One coarse `module_uses_unicode_runtime_tables` gate emits all 14 tables | ✅ Confirmed | **Size** — feature-flag DCE |
| U4 | `mid`/`find` do byte-by-byte continuation scans | ✅ Confirmed | **Perf** — SWAR/NEON; only meaningful one of U3/U4/U6/U7 |
| U3 | NFC/NFD canonical ordering is gnome sort (O(n²)) | ✅ Confirmed | Perf (marginal — runs over ≤3-mark runs) |
| U6 | NFC recomposition is a linear scan | ✅ Confirmed | Perf (marginal — `comb_length` tiny) |
| U7 | `graphemeAt` lacks an early byte-length OOB short-circuit | ✅ Confirmed | Perf (error-path only) |
| R2 | `\p{Script=…}` hard-codes ~10 scripts | ✅ Confirmed | **Feature** — generate full script table |
| R3 | `__regex_makeCtx` recomputes code points per scalar | ✅ Confirmed | **Perf** — single-pass cps |
| R4 | Char classes are a linear item scan, no ASCII bitset | ✅ Confirmed | **Perf** — precompute ASCII bitset |
| R5 | `__regex_searchFrom` has no literal-prefix fast skip | ✅ Confirmed | **Perf** — first-literal scan |
| C3 | csv is fixed RFC-4180 (no dialect config) | ✅ Confirmed | **Feature** — custom delimiter/quote/EOL |
| C4 | csv has no streaming/iterator API | ✅ Confirmed | **Feature** — `csv::parseStream` |
| C5 | `__csv_stringify` allocates many intermediate strings | ✅ Confirmed | **Perf** — single pre-sized builder |
| C2 | csv parse doesn't pre-size collections | ⚠️ True, non-issue | Skip — collections already amortize; no reserve API exists |
| D2 | `toIso` delegates to the general char-by-char formatter | ✅ Confirmed | Perf (marginal) — hard-coded `__datetime_toIso` |
| M4 | Variable-width collection data region permutes out of index order | ✅ Confirmed | Perf/tradeoff — optional compaction pass |
| M6 | Non-escaping closures aren't scope-dropped | ✅ Confirmed | Perf — scope-drop for non-escaping closure env |
| M1 | Arena blocks never unmapped before `arena_destroy` | ⚠️ By design | Skip — documented deferred-coalescing tradeoff |
| M3 | Copies always shrink-to-fit tight | ⚠️ Partial/by design | Skip — moves already elide; no CoW by design |
| M5 | 80-byte resource tombstone never reclaimed | ⚠️ By design | Skip — tombstone required by close/alias semantics |
| T2 | Global stdin log high-water cap causes global stall | ⚠️ By design | Skip — intentional bounded-memory backpressure (plan-15 D3) |
| T3 | Trampoline re-runs `_mfb_linker_init` per worker | ⚠️ Required | Skip — LINK slots are **per-arena**, not process-immutable (bug-369) |
| T4 | Cancellation is cooperative only | ⚠️ By design | Skip — spec explicitly rejects `thread::stop`/back-edge checks |
| **T7** | Failed resource transfer leaks the sender's handle | ✅ **Correctness bug** | **→ bug-425** |
| C1 | csv parse builds a grapheme list first | ❌ Refuted | Already scalar-index scan (plan-39/64) |
| R1 | POSIX classes silently never match | ❌ Refuted | **Already fixed — bug-316** (runtime golden proves match) |
| R6 | No regex recursion/step guard (DoS) | ❌ Refuted | **Already guarded — bug-315/423** (step+depth+parse caps) |
| M2 | Large frees hit an address-ordered list (quadratic) | ❌ Refuted | Already fixed — hashed 64-head large bin (plan-25-A) |
| D5 | Instant/Duration ctors use intermediate arrays/tuples | ❌ Refuted | Arithmetic already inline; no such allocation |
| T1 | `thread.send` leaks copyable msg on failed enqueue | ❌ Refuted | Pending-free list reclaims it (bug-147.5b) |
| T5 | ErrorLoc captured as raw worker-arena ptr → UAF in `waitFor` | ❌ Refuted | Deep-copied before worker teardown (`emit_finalize_worker_error_source`) |
| T6 | `waitFor` result-retrieval race / double-read | ❌ Refuted | Mutex-guarded read-modify-write on the outbound queue lock |
| D1 | localOffset has no cache | ⚠️ Overstated + risky | Skip — "every call" is false; a range cache thrashes/mis-answers the DST-bracket calls |
| **D3** | `f` token truncates without rounding | ❌ **Reject — would regress** | Truncation is the ISO/Java/.NET convention; rounding `999999999ns` overflows the field |
| **D4** | `E` weekday token isn't validated on parse | ⚠️ Reject-ish | Lenient handling of a redundant token is defensible; validating would reject inputs that parse today |

## Confirmed enhancements — detail

### Size (highest value; bundle U1+U2+U5)
Roughly **~110 KB shaved from every generated binary** if all three land, plus finer dead-code elimination.

- **U1 — strip 5 unread `PackedProperty` fields.** `src/unicode/runtime_tables.rs:26-38` packs 24 bytes/record; the codegen offset table (`src/target/shared/code/private/unicode.rs:3-13`) reads only offsets 0/12/14/16/18/20. Offsets 2/4/6/8/10 (`decomp_type`, `decomp_seqindex`, `casefold/uppercase/lowercase_seqindex`) are never read — an in-code comment already says so. Repack to ~14 bytes → ~82 KB off the 8385-record table. Coordinated change: stride constant + all read offsets shift + golden regen.
- **U2 — drop the dead `sequences` table.** `UNICODE_SEQUENCES_SYMBOL` is defined (`error_constants.rs:1005`) and emitted (`data_objects.rs:554-557`) with **zero read sites** (all runtime sequence reads use the flattened NFD/upper/lower/casefold tables). 12961 × u16 ≈ 25 KB.
- **U5 — feature-flag the table emission.** `src/target/shared/code/mod.rs:1680` emits all 14 tables when any Unicode facet is used. Split into `uses_case_mapping` / `uses_normalization` / `uses_graphemes` so e.g. `strings::graphemes` stops dragging in case-mapping + NFD tables.

### Perf — regex (worth doing together; the engine is the `.mfb` at `src/builtins/regex_package.mfb`)
- **R3** — `__regex_toScalars` (`:213-235`) computes all code points via `encoding::utf32Encode`, discards them, then `__regex_makeCtx` recomputes each per scalar via `__regex_scalarToCp` (another `utf32Encode` each). Return the cps already computed.
- **R4** — `__regex_classMatchOne` (`:588-609`) linearly scans `__regex_ClassItem`s per scalar. Precompute a 0..127 ASCII bitset on `__regex_Class` at compile time for O(1) membership.
- **R5** — `__regex_searchFrom` (`:968-981`) tries every offset. When `prog.root` starts with `__regex_Lit`, fast-skip non-matching starts (e.g. `string.indexOf` on the first literal).

### Perf — other
- **U4** — `lower_find`/`lower_mid` (`src/target/shared/code/builder_search.rs`) walk scalars byte-by-byte via `emit_scalar_skip_continuations`. SWAR/NEON lead-byte counting in 16-byte blocks helps long strings. (U3/U6/U7 are also confirmed but operate on inherently tiny inputs or only the error path — low priority.)
- **C5** — `__csv_stringify` (`src/builtins/csv_package.mfb:174-235`) allocates a fresh string per quoted cell (`__csv_quoteField` concatenates grapheme-by-grapheme) and re-concatenates rows. A single pre-sized builder writing escapes on the fly removes the intermediates.
- **D2** — `__datetime_toIso` (`src/builtins/datetime_package.mfb:730-732`) is literally `format(dt, "yyyy-MM-dd'T'HH:mm:ss.fffZ")`, walked char-by-char. A hard-coded fixed-width writer avoids the pattern scan. Marginal.
- **M6** — non-escaping capturing closures (`builder_values.rs:474-522`, 16-byte object + `captures.len()*8` env) are never scope-dropped (`is_freeable_flat_value` excludes closures, `builder_values.rs:225-232`). A correct scope-drop must recurse into the deep-copied captured values, so it's more than one `arena_free` — that's why it's not already covered.
- **M4** — variable-width collection value-grow tail-places payloads (`map_mutate.rs`, `collection_mutate.rs`), permuting the data region; reads stay correct via `valueOffset`, and compaction already happens implicitly at every tight-copy. An explicit threshold-triggered compaction is optional.

### Feature requests
- **R2** — generate a full Unicode script table for `\p{Script=…}` (`__regex_scriptTest` at `regex_package.mfb:371-460` hard-codes 10 scripts; anything else returns FALSE). Mirror the general-category table generation.
- **C3** — csv dialect config: custom delimiter / quote char / output line-ending. Today fixed in `src/builtins/csv.rs:18-52` (2 functions, no options) and `csv_package.mfb` (delimiter 44, quote 34, `"\n"` output).
- **C4** — csv streaming (`csv::parseStream` / row iterator). Today `parse` returns the whole `List OF List OF String` (`csv.rs:29`); no streaming entry point exists.

## Do NOT implement (recorded so these don't get re-filed)

- **R1 / R6 / M2 — already fixed.** R1 (POSIX classes never match) is bug-316; the runtime golden `tests/rt-behavior/regex/regex-posix-classes-rt` proves all six classes match today. R6 (regex DoS) is bug-315/423 — step budget (2M) + match-depth cap (600) + parse-depth cap (200), all raising catchable `77050003`. M2 (quadratic large-free list) was fixed by the hashed 64-head large bin (plan-25-A). Filing any of these would duplicate closed work.
- **C1 / D5 / T1 / T5 / T6 — misreadings of current behavior.** The described "current behavior" does not exist (see verdict table for the disproof + file:line).
- **D3 — rounding fractional seconds would regress.** Truncation is the ISO 8601 / Java `DateTimeFormatter` / .NET convention; rounding `999999999ns` with `.fff` yields `1000` — a 4-digit overflow of a 3-digit field that could roll into the seconds place. Keep truncation.
- **D1 — localOffset cache is unsafe where it's hottest.** The "every call" premise is false (UTC/fixed zones never hit the seam), and `__datetime_resolveLocal` deliberately calls `offsetAt` with four *different* epochSeconds to bracket a DST transition; a range-keyed cache spanning that window would return the wrong offset across exactly the boundary the code is probing.
- **D4 — validating the `E` weekday is a debatable strictness change, not a fix.** The weekday is redundant with the date; lenient handling matches common formatters and rejecting mismatches would break inputs that parse today. Treat as an opt-in strict-mode feature at most, not a bug. (For the record: 2026-08-01 is a Saturday, so the suggestion's example was factually right.)
- **M1 / M3 / M5 / T2 / T3 / T4 — intentional design tradeoffs**, documented in spec/source (deferred-coalescing arena; value-semantics + move elision; resource tombstone; bounded stdin backpressure; per-arena LINK slots; cooperative-only cancellation). Any of these is a redesign/feature, not a defect.

---

# Prerequisites

This ledger has no cross-plan dependency. The only gate is a **green baseline** in
the worktree, so every phase's acceptance can be attributed to that phase.

| # | Check | Command | Status |
| --- | --- | --- | --- |
| P1 | Release binary builds | `cargo build --release --bin mfb` → exit 0 | ✅ MET (2026-08-02, exit 0) |
| P2 | Compiler unit tests green | `cargo test --bin mfb` → `test result: ok` | ✅ MET (2026-08-02: 3757 passed; 0 failed) |

Both MET → phases run to completion. Either NOT MET → stop and report both rows.

# Standing requirements (fold into every phase)

- **Compiler/codegen/runtime/diagnostics work** → read `.ai/compiler.md` first
  (runtime completion gate, register lifetimes, validation/function tests).
- **Editing an embedded builtin `.mfb`** (`regex_package.mfb`, `csv_package.mfb`,
  `datetime_package.mfb`, `unicode_gencat.mfb`) **ripples to importer `.ir`/`.ast`
  goldens** of every fixture that imports it, and often to per-target `.ncodesum`.
  Grep the symbol in `tests/**/*.ir`, regenerate with `sync-goldens.sh`, and prove
  the delta is only the intended change. (mem: builtin-mfb-source-ripples-to-importer-ir-goldens)
- **Any byte change to an embedded unicode table** shifts `.ncode` of ~every
  string-using fixture; goldens are RELEASE-generated, no accept mode — regen via
  `-ncode` + `shasum` per target (mem: unicode-table-byte-change-wide-golden-blast).
- **Do NOT run the full artifact-gate per phase** (~15–20 min); do a targeted
  per-phase check, ONE full gate at finalization (mem: dont-run-full-gate-per-phase,
  no-concurrent-artifact-gate). `mfb_exe()` may reuse a stale release binary — `rm`
  target/release/mfb before a CLI subprocess test if in doubt.
- **Never edit/weaken a golden or test to pass** — prove-4 first (AGENTS.md).

# Implementation phases

Ordering = value/risk ranking (§Execution note). Each phase is independently
landable and lands with its goldens regenerated + a green targeted suite.

## Phase 1 — Size/U2: drop the dead `sequences` table (~25 KB)

Anchors (verified): symbol `UNICODE_SEQUENCES_SYMBOL` def `error_constants.rs:1004`;
emission `data_objects.rs:552-558` (symbol line 553); **zero read sites**
(`grep -rn UNICODE_SEQUENCES_SYMBOL src/` → def+emit+1 doc ref only); table
`tables.sequences` len 12961 × u16 ≈ 25 KB (`runtime_tables.rs:486`). Distinct from
the live `nfd_/uppercase_/lowercase_/casefold_sequences`.

- [x] Remove the 7th data object (`UNICODE_SEQUENCES_SYMBOL`) from the vec in `data_objects.rs:552-558` and renumber the surrounding emission.
- [x] Delete the now-unused const at `error_constants.rs:1004`; `grep -rn UNICODE_SEQUENCES_SYMBOL src/` returns 0 (only the doc ref remained, now removed).
- [x] Drop the `.sequences` field build (`runtime_tables.rs`): struct field, `sequences_hex()`, the `parse_numeric_array("utf8proc_sequences")` construction, the `len()==12961` assert, and the `every_hex_serializer` test entry. `grep '\.sequences\b'` → 0 (NFD/case sequences are separate live fields, untouched). Verified the 5 usages don't derive any live table.
- [x] Update the spec: removed the obsolete "utf8proc sequences table" section (`01_tables-and-algorithms.md`), clearing the two now-dangling `[[…UNICODE_SEQUENCES_SYMBOL]]` citations.
- [x] Regenerated the affected `.ncodesum` goldens via `-ncode` + `shasum` per target. Blast measured empirically (host-detection sweep): exactly **11 unicode-table-emitting fixtures** changed (crypto, csv, datetime, encoding, http, json, net, regex, strings, term + crypto-ec-valid + macos-app-mode-term), 55 golden files across 5 targets. `.ir`/`.ast` byte-identical → runtime behavior unchanged. 13 non-unicode fixtures unaffected.
- **Acceptance:** ✅ `_mfb_unicode_sequences` absent from a fixture's emitted `-nobj`; byte-identity strings macos-aarch64 MATCHES regenerated golden; `.ir`/`.ast` unchanged; `cargo test --bin mfb unicode::runtime_tables` 14/14 green. **Size drop: 25,922 B/binary** (12961 u16 records, the asserted table dimension) off every unicode-emitting executable.
- **Commit:** `03859f3e8`

## Phase 2 — Size/U1: repack `PackedProperty`, strip 5 unread fields (~82 KB)

Anchors: struct `runtime_tables.rs:25-38`, **24-byte stride** (11×u16 + pad@22),
**8385 records**; codegen reads only offsets 0/12/14/16/18/20 (`private/unicode.rs:4-13`
constants; read sites 335/353/361/369/377/388/408/1270/1275/1280). Dead: offsets
2/4/6/8/10 = `decomp_type`, `decomp_seqindex`, `casefold_seqindex`,
`uppercase_seqindex`, `lowercase_seqindex`. NB the in-file comment `unicode.rs:5-8`
only names 6/8/10 — it undercounts; the real dead set is 5.

- [x] Removed the 5 dead fields from `struct PackedProperty` and their `parse_properties` writes; also removed the now-dead `decomp_type_value` helper + its `parse_value` arm + 2 tests (mirrors the bug-343 A4 category-lookup removal).
- [x] Recomputed `encode_le`: 6 live fields, **no trailing pad**, 12-byte stride (offsets 0/2/4/6/8/10).
- [x] Updated `UNICODE_PROPERTY_SIZE` 24→12 and all `UNICODE_PROPERTY_OFFSET_*` (`private/unicode.rs`) to 0/2/4/6/8/10; verified all read sites go through the constants (no hardcoded 24) and the writer/reader offsets match.
- [x] Updated the fixed-size assertion (`* 12 * 2`) and the `data_objects.rs` "12 bytes each" / `* 12`.
- [x] Rewrote the spec `PackedProperty` section (record diagram, offset table, prose) to the 12-byte/6-field layout; also fixed two **Phase-1 leftover** stale `sequences`/24-byte references in the same doc (recorded in Corrections).
- [x] Regenerated the `.ncodesum` golden blast (same 11 fixtures, 55 files). `.ir`/`.ast` byte-identical.
- **Acceptance:** ✅ Runtime smoke test executed `strings::upper/lower/caseFold/normalizeNfc/displayWidth/graphemes/graphemeAt` — all correct output (NFC composition, width=5, graphemes=3 all read the repacked records at the new offsets); host byte-identity MATCH for all 10 affected byte-identity fixtures; `cargo test --bin mfb unicode::runtime_tables` 13/13 green. **Stride 24→12 B/record: saved 12 B × 8385 records = 100,620 B ≈ 98 KB/binary** (properties table was 201,240 B, now 100,620 B). Exceeds the ledger's ~82 KB estimate because I dropped all 5 dead fields + the pad (to 12 B) rather than the estimated ~14 B.
- **Commit:** `23f7aa594`

## Phase 3 — Size/U5: feature-flag the 14-table emission

Anchors: gate `mod.rs:1680` (`references_unicode_table || module_uses_unicode_runtime_tables`);
emit `data_objects.rs:528-630` (single 14-object vec); coarse detector
`module_analysis.rs:917-987`, match arm **929-936**. No case/normalization/grapheme
split exists yet. Shared-by-all: stage1/stage2/properties (objects 1-3).

- [x] **Went finer than a family set — per-symbol.** Made `unicode_runtime_data_objects(Some(&referenced))` (`data_objects.rs`) emit each table iff its `_mfb_unicode_*` symbol is in the referenced set. This is strictly better than 3 families: a `caseFold`-only program emits ONLY the 2 casefold tables (the case path never indexes the base trie — verified), not a whole "case-mapping family + base".
- [x] Rewrote the `mod.rs:1675` gate to collect the referenced `_mfb_unicode_*` symbols from `code_functions` relocations (the comment's "ground truth") and drive per-symbol emission; kept `module_uses_unicode_runtime_tables` only as an empty-reloc fallback (`None` → full set). Did NOT rewrite the 28-site NIR walk — unnecessary once emission is relocation-driven.
- [x] **Direct symbol-scan pins it** (the acceptance's original ask, now achievable): the `.ncode` dump carries the `_mfb_unicode_*` symbol names (the `.nobj` dump does not). `grep -oE '_mfb_unicode_[a-z0-9_]+' <pkg>.ncode` on a runtime-input (non-foldable) program shows: **graphemes-only → `stage1 stage2 properties` only (zero case/nfd symbols)**; caseFold-only → `casefold_entries casefold_sequences` only (no base trie); a `upper`+nfc+fold+graphemes program → all 11 referenced tables but **no `lowercase_*`** (never called `lower`) — per-symbol precision. Plus a unit test `unicode_runtime_data_objects_emit_only_referenced_tables` pinning the filter, and the 11 regenerated byte-identity ncodesum goldens.
- **Acceptance:** ✅ symbol scan: graphemes-only `.ncode` contains no `_mfb_unicode_casefold_*`/`_uppercase_*`/`_lowercase_*`/nfd/combinations symbols while producing correct grapheme output; ✅ all 11 affected fixtures **fully link** (no dropped-but-referenced table, incl. term's base-trie backend); ✅ runtime full-surface program correct; ✅ unit test + `cargo test --bin mfb` green. Size: graphemes-only ncode 817 KB vs full 2.27 MB.
- **Commit:** _(next commit)_

## Phase 4 — Regex/R3: single-pass code points in `makeCtx`

Anchors: `__regex_scalarToCp` `regex_package.mfb:205-211`; `__regex_toScalars`
`:213-226` (computes `cps` via `encoding::utf32Encode` line 219, **discards** it);
`__regex_makeCtx` `:228-235` rebuilds cps per-scalar via `__regex_scalarToCp`
(line 232); `TYPE __regex_Ctx` `:126-130` = `{text, cps, n}`.

- [ ] Have `__regex_toScalars` return both the scalar strings and the `cps` it already computed (parallel return / small struct), instead of discarding cps.
- [ ] Populate `__regex_Ctx.cps` in `makeCtx` from that returned array; delete the per-scalar `__regex_scalarToCp` loop.
- [ ] Regenerate regex importer goldens (`tests/rt-behavior/regex/*`, `tests/rt-behavior/threads/thread-regex-rt`).
- **Acceptance:** all regex rt-behavior fixtures produce identical match results; N per-scalar `utf32Encode` calls eliminated (verify via the lowered `.ir` no longer inlining `#regex_scalarToCp` in the ctx loop).
- **Commit:** _(fill on land)_

## Phase 5 — Regex/R4: ASCII bitset for char classes

Anchors: `__regex_classMatchOne` `:588-610` (linear `FOR EACH item` over the 4-arm
union, re-run per position); types `__regex_Range/Single/Short/Prop` `:29-48`,
`TYPE __regex_Class` `:58-62`; caller `__regex_classMatch` `:620-639` (handles
`neg`/`fold`); class construction site `:1286+`.

- [ ] Add a precomputed `0..127` membership representation to `TYPE __regex_Class` (`:58-62`) — e.g. `ascii AS List OF Boolean` (128 entries) or a bitmask list.
- [ ] Populate it once at class construction (`:1286+`) by evaluating the items for cp 0..127 (bake in `fold`; leave `neg` to the matcher).
- [ ] In `__regex_classMatch` (`:620`), for `cp <= 127` index the bitset directly; fall back to `__regex_classMatchOne` only for cp > 127.
- [ ] Regenerate regex goldens.
- **Acceptance:** all regex rt-behavior fixtures (esp. `regex-posix-classes-rt`, which pins all 6 POSIX classes) produce identical results; class matches for ASCII inputs no longer walk the item list (verify via lowered `.ir`).
- **Commit:** _(fill on land)_

## Phase 6 — Regex/R5: literal-prefix fast-skip in `searchFrom`

Anchors: `__regex_searchFrom` `:968-981` (resets `__regex_steps`, then
`WHILE s <= ctx.n` tries `__regex_tryAt` at *every* offset); `TYPE __regex_Lit`
`:51-54`; `prog.root` `TYPE __regex_Program` `:132-136`, root union `:83-92`,
`__regex_Concat` `:67-69`.

- [ ] Derive a required first scalar/cp from `prog.root` when it is a leading `__regex_Lit` (or the first part of a `__regex_Concat` that is a `Lit`); compute once per search.
- [ ] In the `WHILE` at `:973`, when a required first cp exists, advance `s` to the next `ctx.cps` position equal to it before calling `__regex_tryAt` (respect the step budget / bug-315 semantics).
- [ ] Fall back to the current every-offset behavior when no literal prefix exists.
- [ ] Regenerate regex goldens.
- **Acceptance:** all regex rt-behavior fixtures identical; a literal-prefixed pattern over a long non-matching string skips offsets (verify match count/behavior unchanged; step budget still catches pathological patterns).
- **Commit:** _(fill on land)_

## Phase 7 — csv/C5: single pre-sized builder in `stringify`

Anchors: `__csv_stringify` `csv_package.mfb:174-186` (per-row `out = out & …`,
O(rows²)); `__csv_stringifyRow` `:188-200`; `__csv_encodeField` `:202-207`;
`__csv_quoteField` `:225-235` (grapheme-by-grapheme concat, `strings::graphemes`
materializes a list). Parse side already uses the buffer approach (plan-64 A3).

- [ ] Thread one shared string buffer through `__csv_stringify` → `__csv_stringifyRow` → `__csv_encodeField`/`__csv_quoteField` so each level appends into it instead of returning a fresh String the caller re-concatenates (collapse points :181/:183, :195/:197, :229/:231).
- [ ] Replace the grapheme-by-grapheme loop in `__csv_quoteField` with a scan that appends escaped runs into the buffer without a `strings::graphemes` list.
- [ ] Regenerate the two csv `.ir` goldens (`tests/byte-identity/csv/...`, `tests/rt-behavior/csv/...`) — they hard-code source line numbers, so even additive edits shift them.
- **Acceptance:** `tests/rt-behavior/csv` fixtures produce byte-identical stringify output; intermediate per-cell/per-row String allocations gone (verify lowered `.ir`); `cargo test --bin mfb` green.
- **Commit:** _(fill on land)_

## Phase 8 — csv/C3: dialect config (additive)

Non-goal: **must not** change the existing `csv.parse(String)` / `csv.stringify(grid)`
signatures. Anchors: descriptors `csv.rs:18-36` (parse) / `:37-50` (stringify),
single overload each, `DefaultValue::None`; hard-coded delimiter 44, quote 34,
CR/LF 13/10, output `"\n"` in `csv_package.mfb` (parse :47/:70/:80/:145/:155-168;
stringify :181/:195/:209-223/:226-234). Static parity tables `csv.rs:71-85`.

- [ ] Design a dialect record (delimiter / quote char / output EOL) and add it as an **optional trailing parameter or a second overload** on `csv.parse` and `csv.stringify` (`csv.rs`), keeping the 1-arg overloads resolving unchanged.
- [ ] Update the mirrored static tables `call_param_names`/`expected_arguments` (`csv.rs:71-85`) and the parity test.
- [ ] Thread the dialect scalars into `__csv_parse`/`__csv_stringify` (`csv_package.mfb`), replacing the hard-coded 44/34/`"\n"` with the passed values (default to RFC-4180 when absent).
- [ ] Add man/spec entries for the new option surface (per `.ai/man_template.md` / `.ai/specifications.md`).
- [ ] Add rt-behavior fixtures: TSV (delimiter 9), single-quote, `\r\n` output; regenerate csv goldens.
- **Acceptance:** new fixtures round-trip a non-comma dialect correctly; existing 1-arg csv fixtures unchanged (byte-identical); man/spec updated; suites green.
- **Commit:** _(fill on land)_

## Phase 9 — csv/C4: streaming `csv.Reader` resource

Anchors: `csv.parse` returns whole grid (`csv.rs:28`, `csv_package.mfb:31`
accumulates `rows`, `RETURN rows :99`); no streaming entry point (grep 0).
Mirror the resource-handle pattern (`resource.rs` `ResourceRegistry`/`ResourceInfo`;
precedents `fs.rs` File, `net.rs` Socket/Listener) — a `csv.Reader` open/next/close
triad registered through the resource table, additive (leaves `csv.parse` intact).

- [ ] Define a `csv.Reader` resource type + `RESOURCE_TABLE`/`LINK … CLOSE BY …` wiring, modeled on `fs.rs`/`net.rs`, registered in `resource.rs`.
- [ ] Add `csv::parseStream(String) AS csv.Reader` (or a reader-from-source ctor) and a `readRow(reader) AS List OF String` / iteration surface that yields one row at a time; close reclaims the reader.
- [ ] Ensure the reader carries the parse cursor/state incrementally (reuse `__csv_*` field/record decode helpers without materializing the full grid).
- [ ] man/spec entries for the streaming API.
- [ ] rt-behavior fixture: stream a multi-row csv, assert row-by-row equality with `csv.parse`; resource close verified (no leak).
- **Acceptance:** streaming a csv yields the same rows as `csv.parse` in order; the `csv.Reader` resource opens/closes with no leak (resource-state verify); suites green.
- **Commit:** _(fill on land)_

## Phase 10 — Unicode perf/U4: SWAR/NEON lead-byte counting in find/mid

Anchors: `lower_find` `builder_search.rs:4`, `lower_mid` `:572`; helper
`emit_scalar_skip_continuations` `private/unicode.rs:53-72` (per-byte
load/AND 0xC0/cmp 0x80/branch); 4 call sites (find :187, :229; mid :734, :756),
each `add cursor,1; sub remaining,1; skip; label(advanced); add scalar_index,1`.

- [ ] Add a block lead-byte-count primitive: over 16-byte blocks, count bytes with `b & 0xC0 != 0x80`, bump `scalar_index` by that popcount, advance cursor 16 (SWAR; NEON where available). Keep the existing per-byte path as the `<16 bytes remaining` tail.
- [ ] Slot the block scan in at the loop-head level of all 4 call sites (find locate + advance_candidate; mid locate_start + locate_end).
- [ ] Regenerate any `.ncodesum` / byte-identity goldens for strings fixtures.
- **Acceptance:** `strings::find`/`strings::mid` rt-behavior fixtures produce identical indices/substrings across ASCII + multibyte inputs (correctness of the block count vs the byte scan); long-string case walks in blocks (verify lowered `.ir`/disasm).
- **Commit:** _(fill on land)_

## Phase 11 — Unicode perf/U3+U6+U7: marginal normalization/grapheme fast-paths

All in `builder_strings_builtins.rs`. Marginal (tiny inputs / error-path only) —
grouped as one phase. U3 gnome sort `order_loop:958-991`; U6 recompose
`compose_loop:1004-1091` (scan :1052-1062, `comb_length` :1033); U7 `graphemeAt`
`lower_strings_grapheme_at:2802` (OOB gap after :2822, before the full segmentation :2823).

- [ ] U7: add an early OOB short-circuit in `lower_strings_grapheme_at` after the index type-check/spill (:2822) and before `lower_strings_graphemes` (:2823) — load `value` byte length (String layout offset 0, cf. `builder_search.rs:715`), branch to `invalid` when `index < 0 || index >= byte_len`. Success path unchanged.
- [ ] U3: bound/streamline the gnome-sort back-step (`order_loop`) — it only runs over ≤3-mark runs, so a small insertion sort or a run-length cap is a correctness-preserving tidy; keep output identical.
- [ ] U6: leave the sorted-table early-out as-is unless a cheap win exists; document if no change is warranted (mark moot with evidence).
- [ ] Regenerate affected strings goldens.
- **Acceptance:** `strings::graphemeAt` out-of-range raises `77...` without segmenting (verify error path); normalizeNfc rt-behavior fixtures byte-identical; any moot sub-task carries its evidence.
- **Commit:** _(fill on land)_

## Phase 12 — datetime/D2: hard-coded fixed-width ISO writer

Anchors: `__datetime_toIso` `datetime_package.mfb:730-732` = `__datetime_format(dt,
"yyyy-MM-dd'T'HH:mm:ss.fffZ")`; delegate `__datetime_format` `:697-728` is a
char-by-char pattern interpreter. Golden ripple: `.ir`/`.ast` + `.ncodesum` across
5 targets.

- [ ] Replace `__datetime_toIso` body with direct field extraction + zero-padding emitting the 24-char `yyyy-MM-ddTHH:mm:ss.fffZ` layout in one pass (no pattern scan, no `__datetime_formatToken` dispatch). Truncate fractional seconds (matches D3 verdict — do NOT round).
- [ ] Regenerate datetime `.ir`/`.ast` goldens and the 5-target `.ncodesum`.
- **Acceptance:** `strings`/datetime rt-behavior fixtures calling `toIso` produce byte-identical output vs the old formatter across a spread of datetimes (incl. sub-second, Z); suites green.
- **Commit:** _(fill on land)_

## Phase 13 — Memory/M4: optional threshold compaction on in-place value-grow

Anchors: maps `map_mutate.rs:242-289` (`value_grow` tail-append leaves dead slack;
reads via `valueOffset`); lists `collection_mutate.rs:200-229` (bug-365 comment).
Implicit compaction already happens at every tight copy (`copy_collection_tight`
`builder_collection_layout.rs:364`; fixup `emit_offset_compaction_fixup`
`collection_buffer.rs:337`). OPTIONAL — only helps repeated in-place grows with no
intervening copy.

- [ ] Add a `deadSlack / dataLength > threshold` guard after the tail-append + repoint in the `value_grow` block (`map_mutate.rs:~289`); on trip, compact via `copy_collection_tight` + `emit_offset_compaction_fixup`.
- [ ] Pick + justify the threshold; ensure no regression to the common (single-grow) path.
- [ ] Regenerate affected goldens.
- **Acceptance:** a fixture that grows one map value many times in place shows bounded data-region slack (measured) while producing identical reads; existing collection/map suites byte-identical. If measurement shows no meaningful win, mark the phase moot with the numbers.
- **Commit:** _(fill on land)_

## Phase 14 — Regex/R2: full Unicode script table for `\p{Script=…}`

**Open decision / risk:** needs an external `Scripts.txt` (Unicode 16.0.0) — Python
`unicodedata` has no script API. Anchors: `__regex_scriptTest`
`regex_package.mfb:371-460` (10 hand-coded scripts, else FALSE); gc precedent =
generated `src/builtins/unicode_gencat.mfb` via `scripts/gen_regex_unicode.py`,
embedded through `include_str!` + `.replace` (`strings.rs:523`); regeneration guarded
by `scripts/check-generated.sh` (Unicode 16.0.0 pin).

- [ ] Obtain the Unicode 16.0.0 `Scripts.txt` data source (record provenance); resolve the Open Decision on how it is vendored/fetched for `check-generated.sh`.
- [ ] Extend `gen_regex_unicode.py` to emit a run-length `__regex_scriptOf(cp) AS String` (or per-script range table) into a new generated `.mfb`, mirroring the gc generator.
- [ ] Embed it the same `include_str!`/`.replace` way; replace the 10-branch `__regex_scriptTest` (`:371-460`) with a lookup against the generated table.
- [ ] Wire the new generated file into `scripts/check-generated.sh`.
- [ ] man/spec: document the now-full `\p{Script=…}` support; regenerate regex + generated-file goldens.
- **Acceptance:** `\p{Script=…}` matches scripts beyond the original 10 (rt-behavior fixture over e.g. Armenian/Thai/Devanagari), the original 10 still match, unknown script names still parse-then-not-match; `check-generated.sh` green.
- **Commit:** _(fill on land)_

## Phase 15 — Memory/M6: closure escape analysis + recursive scope-drop (HIGHEST RISK)

**Re-framed (see Corrections):** there is *no* closure escape analysis today and
closures are *never* freed — this is a new analysis pass **plus** a new recursive
free, with two-sided correctness exposure. Anchors (`builder_values.rs`): closure
arm `434-553`, env block `471-513` (`arena_alloc captures.len()*8`; captures
deep-copied via `lower_value_owned` :500 — a native `d` float :507 is stored by
value, **must not** be freed), object `514-547` (`CLOSURE_OBJECT_SIZE`=16,
`error_constants.rs:283-285`); `is_freeable_flat_value` `225-232` (excludes
closures). A closure = **N+2 arena blocks** (object + env + one per flat capture).
Address-taken-local hazard flagged at `builder_exits.rs:231`.

- [ ] Build a closure escape analysis: a closure is non-escaping iff it is not returned, not stored in a binding/global/collection/record, and not passed to an escaping call. Be conservative — default to "escapes" on any doubt (a false non-escape → UAF).
- [ ] Add a recursive closure free path: load env ptr (offset 8), walk N slots, free each *freeable-flat* capture (recurse into nested composites), free env, free the 16-byte object — in that order; skip native by-value slots (the `d` float at :507).
- [ ] Gate the free on the escape verdict; integrate with the existing owned-value scope-drop (extend beyond `is_freeable_flat_value`, or a dedicated closure drop).
- [ ] RED tests first (per `.ai/compiler.md`): (a) a non-escaping closure with String/record captures frees all N+2 blocks (no leak); (b) an escaping closure (returned/stored) is NOT freed (no UAF); (c) the address-taken-local hazard case.
- **Acceptance:** the RED tests pass; a leak-check fixture shows a non-escaping capturing closure's env + captures reclaimed; no escaping-closure UAF; full `cargo test --bin mfb` + strings/closure rt-behavior green.
- **Commit:** _(fill on land)_

# Finalization

- [ ] Merge current `main` if it advanced; re-run acceptance.
- [ ] `cargo fmt --all` + second pass `--manifest-path repository/Cargo.toml`; commit churn.
- [ ] ONE full `artifact-gate.sh` run (execution-free codegen gate) — green.
- [ ] Full acceptance/CI green in the worktree.

# Corrections

- **M6 re-framed (verified 2026-08-02).** Plan-77's ledger said "non-escaping
  closures aren't scope-dropped." Reality (fan-out over `builder_values.rs`,
  `src/ir/resource_escape.rs`, all `escape*` machinery): **no closure escape
  analysis exists at all** — the `escape`/`non_escaping` code is exclusively about
  `RES` resources and vector-promotion. Closures are *never* freed (simply excluded
  from `is_freeable_flat_value:225-232`). So M6 is not "add a drop for the
  non-escaping case"; it is *build the escape analysis from scratch* + *a recursive
  N+2-block free*. Effort raised to large, risk to highest, placed last (Phase 15).
- **R2 needs external data (verified 2026-08-02).** The gc generator
  `scripts/gen_regex_unicode.py` uses Python `unicodedata`, which has **no script
  API**; a full script table requires an external Unicode 16.0.0 `Scripts.txt`.
  Added as an Open Decision in Phase 14 (how to vendor/fetch it under
  `check-generated.sh`).
- **Line-number drift corrected against source (2026-08-02):** U2 `UNICODE_SEQUENCES_SYMBOL`
  def is `error_constants.rs:1004` (ledger said ~1005), emission `data_objects.rs:552-558`
  symbol line 553 (ledger said 554-557). R3 `__regex_toScalars` is `:213-226` and
  `__regex_makeCtx` `:228-235` (ledger's `:213-235` conflated the two). R4
  `__regex_classMatchOne` body runs to `:610` (ledger `:588-609`). D2 delegate is
  `__datetime_format` (ledger wrote bare `format`). M6 closure arm is `434-553`
  (ledger's `474-522` is only the env-alloc sub-block). All other anchors verified
  exact.
- **U1 dead-field set is 5, not the 3 the source comment names.** The in-file
  comment `private/unicode.rs:5-8` names only offsets 6/8/10; the actual unread set
  is 2/4/6/8/10 (`decomp_type`, `decomp_seqindex`, + the 3 case seqindexes). Phase 2
  fixes the comment too.
- **Phase 1 left two stale spec references (caught + fixed in Phase 2).** Removing
  the `sequences` table (Phase 1) left `01_tables-and-algorithms.md` still listing
  `sequences = 12961 u16` in the reference-sizes line and a `sequences` row in the
  emitted-symbols table. Both are now removed. Lesson: a table removal must sweep
  the spec's *size list* and *symbol table*, not only the prose section.
- **U1 also removed dead Rust code the ledger didn't mention.** Dropping the
  `decomp_type` field made `decomp_type_value` (a 16-arm helper), its `parse_value`
  match arm, and 2 unit tests dead; removed per AGENTS.md "no dead code" (same
  pattern as the bug-343 A4 category-lookup removal noted in the source).
- **U5 implemented per-*symbol*, not per-*family*.** The ledger/plan proposed a
  3-way `uses_case_mapping`/`uses_normalization`/`uses_graphemes` split. Driving
  emission off the actual `code_functions` relocations (already the "ground truth"
  per the mod.rs comment) is simpler AND finer: it emits exactly the referenced
  tables, so a `caseFold`-only program drops even the base trie (the case path
  never indexes it). No rewrite of the 28-site NIR walk was needed. Verified all
  affected fixtures still link (no dropped-but-referenced table).
- **Phase-1 also left "fourteen tables" stale in the spec (fixed in Phase 3).**
  `## Embedding` said "all fourteen tables"; after the sequences removal it is
  thirteen, and after U5 the wording had to change from all-or-nothing to
  per-table anyway. Same lesson as the sequences size-list/symbol-table leftovers:
  a table-count change must sweep every count in the doc.
