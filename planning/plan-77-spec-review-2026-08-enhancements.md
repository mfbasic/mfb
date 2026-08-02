# Spec-review enhancements ledger (2026-08-02)

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
