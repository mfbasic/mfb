# bug-625: astrings::fromString leaks its empty spans list (48 B per AttributedString)

Last updated: 2026-09-13
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness (memory)

Status: Open
Regression Test: tests/runtime/rt_scope_drop_leaks.rs (to add, Phase 1); tests/runtime/rt_debug_soak.rs (`a_paint_loop_keeps_live_bytes_constant`, plan-133-A)

Every `AttributedString` built by `astrings::fromString` leaves one 48-byte block live after
the value is dropped. The block is the empty `spans` list the constructor builds and copies
into the record but never frees (§ Root Cause). It doesn't matter whether the value was bound
to a `LET`, was an element of a `List OF AttributedString`, or was a record field. A terminal UI that repaints rows as
attributed strings leaks one block per row per repaint. The browser example's
`display::paint` leaks 1,632 B per paint of the `BASIC` page this way (plan-133-A § 2).

**The single correct behavior a fix produces:** dropping an `AttributedString` frees every
block it owns, so the reproduction below reports equal `live_bytes` at N=1000 and N=2000.

References:

- `src/docs/spec/memory/04_arenas.md` (scope drop); the `astrings` man page.
- Found by plan-133-A Phase 2: the canvas remainder of the paint stage, isolated by bisecting
  scratch copies of `examples/browser/display` (`/tmp/plan-133-a/bisect_display*.py`).

## Failing Reproduction

`/tmp/plan-133-a/stages/as2_single.mfb`, `{N}` = 1000 and 2000, `target/release/mfb build
--debug`, macOS, main `14c9fc1ca`:

```
IMPORT io
IMPORT astrings

SUB main()
  MUT total AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < {N}
    LET a AS AttributedString = astrings::fromString("ab" & toString(i))
    total = total + 1
    i = i + 1
  END WHILE
  io::print("total=" & toString(total))
END SUB
```

- Observed (`arena.0.*`): N=1000 `live_bytes 75040`, `alloc_calls 4006`, `free_calls 3002`;
  N=2000 `live_bytes 123040`, `alloc_calls 8006`, `free_calls 6002`. Each iteration makes 4
  allocations and 3 frees, leaving one 48 B block.
- Expected: `live_bytes` equal at both N.

Shapes (N=1000 → 2000):

| Shape | `live_bytes` | Per iteration |
|---|---|---|
| `LET a AS AttributedString = astrings::fromString(…)` (`as2_single`) | 75,040 → 123,040 | 1 block, 48 B |
| `LET l AS List OF AttributedString = [fromString(…), fromString(…)]` (`as1_list`) | 123,040 → 219,040 | 2 blocks, 96 B |
| a record `{rows AS List OF AttributedString, count}` returned from a helper that appends 4 (`as3_record`) | 219,040 → 411,040 | 4 blocks, 192 B |
| `display::paint` of a 5-row page, dom's bug-620/621 sites rewritten (`pt_paint`) | 37,520 → 61,520 at N=100 → 200 | 240 B = 5 × 48 |
| the same with paint's final `FOR EACH rowText IN cv.rows` loop (which builds the attributed rows) removed (`display-v11`) | 13,520 → 13,520 | 0 |
| plain `String` values in the same loops (`fe_field`, `fe_field_let`) | 13,520 → 13,520 | 0 |

## Root Cause

The leak is in the constructor, not the drop (from reading the code, plan-133-A; the fixed
48 B size and the 3-of-4 free count both agree).

- `AttributedString` is an ordinary two-field record, `text AS String` plus
  `spans AS List OF AttrSpan` (`src/codegen/engine/validation/validation.rs`). Every field is
  flat, so `is_freeable_flat_value` is true, and the binding's drop
  (`emit_owned_value_drop`, `src/codegen/cleanup/owned/builder_owned_cleanup.rs`) makes one
  `arena_free` of the record block. That is correct, because the record holds its fields inline.
- `lower_astrings_from_string` (`src/codegen/builtins/astrings/gen_astrings.rs`) calls
  `lower_empty_collection(List OF AttrSpan)`, which allocates a list block of
  `COLLECTION_HEADER_SIZE` (40 B, rounded to 48 B). It then calls `emit_build_inlined_record`,
  which **byte-copies** that list into the new record. The source list is never freed, and it
  is not a pending temp either, because it was built inside the inline body rather than
  through `lower_value`.
- The four allocations per iteration are `toString(i)`, the `&` concat, the empty list and the
  record. The two statement temps are freed at the end of the statement, and the record when
  its binding drops. The list is the one left over.
- Other builders free their field sources after `emit_build_inlined_record`: the record
  constructor calls `drop_pending_temps_to(arg_temp_watermark)` (`builder_values.rs`),
  `func_partition.rs` calls `free_intermediate_collection` on both lists, and `func_zip.rs`
  frees its items and pair. `fromString` has no such step.
- The drop size cannot be the cause: a missed text or record block would grow with the text
  ("ab0" … "ab1999"), but the leak stays 48 B.

## Goal

- The three reproduction shapes report equal `live_bytes` at N and 2N.
- `astrings` values that carry attributes (`astrings::addAttribute`) are also freed completely.

### Non-goals (must NOT change)

- `AttributedString` contents, rendering, and the `astrings` API.
- **Tempting wrong fix:** building display rows as plain `String` in the browser example.
  That hides the leak in one program and leaves it in the type.

## Blast Radius

- `lower_astrings_from_string` — fixed by this bug.
- Every other inline builder that builds a field value internally (with
  `lower_empty_collection` / `lower_collection_values`) and hands it to
  `emit_build_inlined_record`: the same hazard. `grep -rn "emit_build_inlined_record(" src`
  lists 11 call sites, including `vector/builder_vector_inline.rs` and
  `crypto/func_generate.rs`. Give each a verdict in Phase 1.
- `astrings::writeSpans` — unaffected: its spans list arrives as an argument through
  `lower_value`, so it is a pending temp and gets freed.
- `List OF AttributedString` element drops and record-field drops — unaffected. The drop is
  correct; each element leaked because its constructor did.
- `term` / `app` APIs taking `AttributedString` rows (the browser's screen): consumers, and
  unaffected by the fix.

## Fix Design

In `lower_astrings_from_string`, after the record build: keep the record pointer, free the
source list with `free_intermediate_collection` (as `func_partition.rs` does), then reload the
pointer. Alternatively, register the list as a pending temp so the statement-end drop frees it.

Rejected: changing the `AttributedString` drop to walk `spans`. The record holds the list
inline, so the drop is already right, and walking it would free inline bytes as if they were a
separate block.

## Phases

### Phase 1 — failing test + audit (no behavior change)

- [ ] `rt_scope_drop_leaks.rs`: the three shapes above, plus a value with an attribute, N vs 2N;
      confirm each fails.
- [ ] Confirm the Root Cause by measurement (a throwaway free of the list in
      `lower_astrings_from_string` makes `as2_single` flat), and give each of the 11
      `emit_build_inlined_record` call sites a verdict.

Acceptance: the cases fail for the documented reason; the audit list has a verdict per site.
Commit: —

### Phase 2 — the fix

- [ ] Free the source `spans` list in `lower_astrings_from_string`, and in any audited sibling
      builder found to leak the same way.

Acceptance: Phase 1 cases flat; `double_free_skips 0`; `astrings` suites green.
Commit: —

### Phase 3 — expected outputs + full validation

- [ ] Regenerate shifted goldens; full suite; `scripts/test-accept.sh`; plan-133-A paint stage
      re-run.

Acceptance: full suite green; the paint stage's remainder drops to 0.
Commit: —

## Validation Plan

- Regression tests: Phase 1 cases; plan-133-A's soak paint case, together with bug-620/621.
- Runtime proof: `as2_single` flat.
- Doc sync: none expected.
- Full suite: `cargo test --no-fail-fast`, `scripts/test-accept.sh`.

## Open Decisions

- None.

## Summary

A single missing free in one builtin type's drop. The only care needed is covering every path
that drops the type.

## Phase 1 findings (fix-bug, 2026-09-15)

- Reproduced at main `9b5e5b55f`: `as2_single` N=1000 `live_bytes 61520` (`alloc 4004` /
  `free 3002`), N=2000 `109520` (`8004` / `6002`) — 48 B, one block per value, as documented.
- **Audit of the `emit_build_inlined_record` call sites:**
  - `astrings/gen_astrings.rs:lower_astrings_from_string` — **leaks** (this bug).
  - `astrings/gen_astrings.rs:lower_astrings_write_spans` — safe: the spans list arrives
    through `lower_value` as a pending temp; the text is an alias.
  - `engine/value/builder_values.rs` (record constructor) — safe: `drop_pending_temps_to`.
  - `memory/marshal/construct_helpers.rs` (`construct.T`) — safe: its callers' arguments are
    the constructor's pending temps.
  - `collections/func_partition.rs`, `collections/func_zip.rs` — safe: free their sources.
  - `vector/builder_vector_inline.rs` — safe: float lanes, no blocks.
  - `memory/value/builder_value_semantics.rs`, the `WITH` rebuild — safe: kept fields are
    aliases of the target; replaced fields are the statement's temps.
  - `memory/value/builder_value_semantics.rs:lower_default_value_inner`, record arm —
    **leaks (sub-issue B, new):** it builds each field's default (`lower_empty_collection` for a
    collection) and byte-copies it inline without freeing it. Measured: a loop of
    `MUT r AS Rec` (`name AS String`, `items AS List OF Integer`) N=1000 `live_bytes 48000`
    (`alloc 2002` / `free 1002`), N=2000 `96000` — 48 B per record. A String default is the
    shared empty-string constant (`load_empty_string_constant`), not an arena block.
  - `crypto/func_generate.rs` (3 sites) — a separate native helper of the same name; verdict
    recorded with the fix.
- RED tests (`tests/runtime/rt_debug_soak.rs`, live_bytes): `a_bound_attributed_string_…`,
  `a_list_of_attributed_strings_…`, `a_record_of_attributed_strings_…`,
  `an_attributed_string_with_an_attribute_…`, `a_defaulted_record_…`, and
  `a_paint_loop_keeps_live_bytes_constant` (its `#[ignore]` removed).
