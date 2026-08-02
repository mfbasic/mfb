# plan-70-A: Display-width primitive (Unicode runtime + `strings::displayWidth`)

Last updated: 2026-07-27
Overall Effort (AI): huge (>3d)   — the whole plan-70 feature
Effort (Human): medium (1h–2h)
Effort (AI): medium (1h–2h)
Depends on: nothing
Produces:
- `PackedProperty` width bits (charwidth in `flags` bits 4–5, ambiguous in bit 6)
  and the parser that fills them.
- A codegen helper `emit_unicode_property_charwidth` (reads a scalar's width from
  the property record) in `src/target/shared/code/private/unicode.rs`.
- A codegen routine `emit_grapheme_display_width` that sums the display width of a
  `String` by walking graphemes (each cluster's width = width of its first
  non-zero-width scalar; zero-width clusters = 0) — the reusable primitive B/C/D/E/F consume.
- The `strings::displayWidth(value AS String) AS Integer` builtin.

This is the foundation: the width lookup and the per-grapheme width sum that every
renderer needs. It ships as a standalone, unit-testable builtin so the width-table
plumbing is proven before any grid depends on it.

## Prerequisites

See `plan-70-unicode-wide.md` §Prerequisites (bug-392 + clean baseline). This
sub-plan itself is testable on the host without a terminal.

## Dependency graph

See the umbrella. A ← nothing; B/C/D/E/F ← A.

## 1. Goal

- For any `String`, `strings::displayWidth` returns the sum of its grapheme
  clusters' terminal column widths: `displayWidth("日本語") == 6`,
  `displayWidth("café")` (NFC or NFD) `== 4`, `displayWidth("👨‍👩‍👧‍👦") == 2`,
  `displayWidth("a\u{200B}") == 1` (ZWSP is zero-width), `displayWidth("") == 0`.
- A single codegen helper yields a scalar's width (0/1/2) from the embedded
  property table, with no new table and no record-size change.

### Non-goals

- No change to `mid`/`find`/`graphemes*` semantics. No width-aware `padRight`.
- Ambiguous-width codepoints return **1** (see umbrella Open Decisions); the
  `ambiguous` bit is stored but not acted on.

## 2. Current State

- The width data is vendored but dropped: `utf8proc.h:307-310` has `charwidth:2`
  / `ambiguous_width:1`; `parse_properties` (`src/unicode/runtime_tables.rs:253-295`)
  skips them; `PackedProperty` (`:26-38`) has no width field.
- `flags` u16 uses only bits 0–3 (`runtime_tables.rs:48-51`, assigned at
  `:266-289`). Bits 4–6 free (verified by read). `UNICODE_PROPERTY_OFFSET_FLAGS`
  = 16 (`private/unicode.rs`); the codegen reads flags via
  `emit_unicode_property_u16` already.
- The grapheme walker that A's width-sum reuses is inlined by
  `lower_strings_graphemes` (`src/target/shared/code/builder_strings_builtins.rs:6`)
  and `lower_strings_graphemes_count` (`:2662`), driving
  `emit_grapheme_break_branch`/`emit_grapheme_state_update`
  (`private/unicode.rs:659`/`:797`).
- Builtin registration + signatures live in `src/builtins/strings.rs:3-51`;
  codegen dispatch for each in `builder_strings_builtins.rs`.
- Reference table sizes are asserted in
  `runtime_tables.rs:parses_utf8proc_runtime_tables` (properties = 8385 records);
  A must **not** change any size, only the `flags` byte values.

### Measured populations

| What | Count | Command |
|---|---|---|
| Free `flags` bits | 3 (bits 4,5,6) | read `runtime_tables.rs:48-51` — only 0–3 used |
| utf8proc property field index for `charwidth` / `ambiguous_width` | field 16 / 17 of the record | struct order `utf8proc.h:294-317`; confirm against sample records in `parse_properties` before wiring (task) |

### Verified properties

- Bits 4–6 of `flags` are unused (read, not just located). Packing width there
  keeps the 24-byte record and every asserted table size unchanged.
- UNVERIFIED (task): the exact positional index of `charwidth`/`ambiguous_width`
  in the C `utf8proc_properties[]` initializer rows — must be confirmed by
  reading a few rows in `utf8proc_data.c` against the struct, because
  `parse_properties` addresses fields positionally. This is a property claim; do
  not wire the parser until a spot-check (`日` U+65E5 → width 2, `A` → width 1,
  `U+0301` combining → width 0) passes.

## 3. Design

1. **Parser** (`runtime_tables.rs`): in `parse_properties`, read the charwidth
   field (0–2) and ambiguous flag from each record; OR them into `flags` as
   `charwidth << 4` and `ambiguous << 6`. Add named consts
   `CHARWIDTH_SHIFT = 4`, `CHARWIDTH_MASK = 0b11 << 4`, `AMBIGUOUS = 1 << 6`
   beside the existing flag consts (`:48-51`).
2. **Codegen lookup** (`private/unicode.rs`): add `emit_unicode_property_charwidth`
   mirroring the existing `emit_unicode_property_*` helpers — load the flags u16
   at `UNICODE_PROPERTY_OFFSET_FLAGS`, shift right 4, mask 0b11, into a result
   register. (An ambiguous accessor is not emitted yet — the bit is dormant.)
3. **Display-width sum** (`builder_strings_builtins.rs`): add
   `emit_grapheme_display_width` (or fold into a shared helper) that walks the
   string by grapheme using the same break machinery as `lower_strings_graphemes`,
   and for each cluster adds the width of the cluster's **first non-zero-width
   scalar** (a cluster whose scalars are all zero-width contributes 0; a cluster
   led by a wide scalar contributes 2). This "first non-combining scalar decides
   the cluster width" rule matches wcwidth/wcswidth and notcurses' EGC width.
4. **Builtin**: register `strings.displayWidth` in `strings.rs` (Integer return,
   one String arg) and dispatch it to the codegen routine; add compile-time
   folding for a static argument (evaluate via the existing
   `unicode_segmentation` + a Rust width crate **or** the same utf8proc-derived
   table read in-process — match how the other Unicode builtins fold at
   `builder_strings_package.rs`).
5. **Man + spec** deferred to G, but A adds the `strings::displayWidth` man page
   stub so the builtin registers cleanly (man citations test is strict).

**Risk:** the positional parser field index (§2 UNVERIFIED) — falsify first
(Phase 1). Blast radius is the golden shift on the `flags` column, which G owns;
A's own tests are host-side value checks.

## Phases

### Phase 1 — falsify the field index + width value (spike)

- [x] Read enough `utf8proc_properties[]` rows in
      `third_party/utf8proc/utf8proc_data.c` to fix the positional index of
      `charwidth`/`ambiguous_width`; add a **Rust unit test** in
      `runtime_tables.rs` asserting `property_for_codepoint('日').charwidth == 2`,
      `('A') == 1`, `('\u{0301}') == 0`, `('👍') == 2`, before touching codegen.
      Confirmed index 16 = charwidth, 17 = ambiguous_width (struct order in
      `utf8proc.h:237-318`, matching the existing positional parser). Tests
      `charwidth_field_is_parsed_from_the_utf8proc_table` +
      `ambiguous_width_bit_is_parsed_and_carried` pass. **Gotcha found (see
      Corrections):** field 17 is emitted as `0` in row 0 but `false` elsewhere,
      so it needs `parse_value`, not `parse_bool`.

Acceptance: the unit test passes against the parsed table — the field index and
width semantics are proven. Commit: 112361323 (parser plumbing folded in because
the falsification test cannot read `charwidth()` without it)

### Phase 2 — plumb width into the property record

- [x] Extend `parse_properties` to OR charwidth/ambiguous into `flags`; add the
      shift/mask consts. Landed in the Phase 1 commit (`CHARWIDTH_SHIFT`,
      `CHARWIDTH_MASK`, `AMBIGUOUS`), because the falsification test reads them.
- [x] Confirm every asserted table size in
      `parses_utf8proc_runtime_tables` is **unchanged** (only `flags` bytes move).
      `parses_utf8proc_runtime_tables` (8385 properties, all sizes) and
      `packs_properties_as_fixed_size_records` (24-byte record) both still pass.
- [x] Add `emit_unicode_property_charwidth` in `private/unicode.rs`
      (`(flags >> 4) & 0b11`).

Acceptance: `cargo test` green (3748 passed); the codegen helper's runtime proof
is the Phase 3 dual-path fixture, which drives `charwidth==2` (日), `1` (A/e), and
`0` (combining/ZWJ) through `emit_unicode_property_charwidth` on the dynamic path.
Commit: — (see Phase 3 commit)

### Phase 3 — `strings::displayWidth` builtin

- [x] Add the width-sum walker + dispatch; register `strings.displayWidth` in
      `strings.rs`; add compile-time folding. Named `lower_strings_display_width`
      (the reusable per-scalar primitive is `emit_unicode_property_charwidth`);
      dispatched in `builder_strings_package.rs`; folded via
      `crate::unicode::backend::graphemes` + `property_for_codepoint().charwidth()`
      (same table as the runtime). Wired into `is_native_direct_call`,
      `value_uses_unicode_runtime_tables`, and `unicode_string_call_is_static`.
- [x] Add the man page (`src/docs/man/builtins/strings/displayWidth.md`) — a full
      page following the `strings` template, not a stub.
- [x] Tests: `tests/rt-behavior/lexical/strings-display-width-rt` asserts the §1
      values over BOTH the folded (string-literal) and dynamic (FUNC-parameter)
      paths — they agree exactly; `tests/syntax/strings/displayWidth` proves
      arity/type rejection.

Acceptance: the fixture prints the expected widths (`日本語`=6, `café` NFC/NFD=4,
ZWJ family=2, lone combining=0, empty=0) via **both** paths; `cargo test` green;
`scripts/artifact-gate.sh target/release/mfb all` = 0 diffs after regenerating
**every** shifted native golden (all table-embedding byte-identity fixtures — the
blast radius is far wider than the plan estimated; see Corrections) across all 5
targets — regenerated **here in A**, not deferred to G, so B–F each start from a
clean gate. Commit: —

## Validation Plan

- Tests: the Phase 1 table unit test, the Phase 2 codegen function test, the
  Phase 3 dual-path MFB fixture (folded + runtime).
- Coverage check: the runtime `emit_grapheme_display_width` path must be exercised
  by a **dynamic** (non-constant) argument fixture, not only the folded path.
- Runtime proof: `mfb`-compile and run the Phase 3 fixture; widths match §1.
- Doc sync: `displayWidth.md` man stub (full page in G);
  `01_tables-and-algorithms.md` PackedProperty flags table gains bits 4–6 (G).
- Acceptance: `cargo test` + artifact-gate.

## Open Decisions

See umbrella (Ambiguous = 1; `displayWidth`-only, no `padRight` mode).

## Corrections

- **2026-08-02 — Phase 1 and Phase 2's parser step merged.** The Phase 1
  falsification test asserts `property_for_codepoint(cp).charwidth()`, which
  cannot compile until the parser actually plumbs charwidth into `flags` and the
  accessor exists. So the parser change (Phase 2 bullet 1) landed with the Phase 1
  test in one commit. The Phase 2 boxes are re-marked accordingly. Evidence:
  `cargo test --bin mfb runtime_tables::tests::` → 14 passed.
- **2026-08-02 — `ambiguous_width` (field 17) needs `parse_value`, not
  `parse_bool`.** utf8proc_data.c emits this field inconsistently: the index-0
  record (`utf8proc_data.c:7967`) uses the integer `0`; every other row uses
  `false`/`true`. `parse_bool` panics on `0`, so the field must go through
  `parse_value` (maps `0`/`false`→0, `1`/`true`→1). `charwidth` (field 16) is
  always an integer 0/1/2. Verified by reading rows 0–3 of the table.
- **2026-08-02 — the plan's golden blast-radius estimate was WRONG: the embedded
  `_mfb_unicode_properties` table ships in essentially EVERY binary, so the
  `flags` change shifts the `.ncode` of every table-embedding byte-identity
  fixture, not just "width-using" ones.** Proven by reading a fixture that uses NO
  unicode builtin: `target/release/mfb build -q -ncode tests/byte-identity/crypto`
  then `grep -c _mfb_unicode_properties …` → 100 hits. The umbrella §2 claim
  ("width-using fixtures' sums shift") and A §Design ("blast radius is the golden
  shift on the flags column, which G owns; A's own tests are host-side value
  checks") both under-counted. The table is pulled in far more broadly (any
  `toString`/string path), so crypto/csv/datetime/encoding/http/json/net/… all
  shift. Regenerated the full shifted set here in A (see the gate-driven regen)
  so B–F each begin from a clean `artifact-gate all`.
- **2026-08-02 — the golden gate must use the RELEASE binary, and only the native
  object/code stages shift.** The committed `.ncode`/`.ncodesum` goldens are
  release-generated (baseline `artifact-gate target/release/mfb all` = 0 diffs on
  clean main). A DEBUG mfb and a RELEASE mfb built from the SAME source produce
  the same `.ncode` (verified: both 4e77… for crypto with my changes), so the
  gate binary choice is about matching how the goldens were made, not
  debug-vs-release divergence. The front-end goldens (`.ast`/`.ir`) and the
  pre-object native stages do NOT embed the table bytes, so they do not shift;
  only `nobj`/`ncode` (and their `sum` variants) move.
