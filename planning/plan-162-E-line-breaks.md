# plan-162-E: `strings::lineBreaks` — UAX #14 line-break opportunities

Last updated: 2026-09-27
Effort: large (3h–1d)
Depends on: plan-162-D (prerequisites: see plan-162-A §Prerequisites)

Wrapping text needs to know where a line **may** break and where it **must**. Splitting
on spaces gets CJK (break between any two ideographs), hyphens, URLs, closing
punctuation and non-breaking spaces wrong. This letter adds the Unicode Line Breaking
Algorithm (UAX #14, Unicode 16.0.0) to the `strings` package. It is pure text logic
with no font dependency, so it lives in `strings` rather than `canvas`, and it is useful
outside canvas too (terminal wrapping, for example). The wrapping loop itself belongs
to the program's layout code: walk the opportunities, measure each chunk with
`canvas::measureText`, and break at the last opportunity that fits.

**The single behavioral outcome:** `strings::lineBreaks(s)` agrees with every case in
the Unicode 16.0.0 `LineBreakTest.txt` conformance file.

References:

- UAX #14 revision for Unicode 16.0.0, rules LB1–LB31 and the default resolution of
  classes (LB1).
- `src/docs/spec/unicode/01_tables-and-algorithms.md`. The pinned tables use
  **Unicode 16.0.0**, and there is a "two Unicode versions coexist" contract.
  Grapheme segmentation reads utf8proc.
- `tools/unicode-tables/README.md` and `scripts/check-generated.sh` (the generated-table
  gate). `third_party/unicode/` (vendored UCD files).
- `mfb man strings`: positions are zero-based **scalar** indices.

## 1. Goal

- `strings::lineBreaks(text AS String) AS List OF strings::LineBreak`, where
  `LineBreak` is `{index AS Integer, mandatory AS Boolean}`.
  - `index` is the scalar index where a new line may start.
  - Position 0 is never reported. The end of the text is always reported, as
    mandatory (LB3).
- It is 100% conformant with `LineBreakTest-16.0.0.txt`, run as a test.

### Non-goals

- No tailoring: no locale rules, no Korean syllable-block options, and no CSS
  `word-break`/`line-break` modes.
- No dictionary breaking for South-East Asian scripts. Class `SA` resolves to `AL`/`CM`
  per LB1, so Thai text breaks only at spaces and punctuation. This is stated on the
  man page.
- No hyphenation. `lineBreaks` reports existing hyphens as opportunities (`HY`/`BA`); it
  never inserts one.
- No change to any existing `strings` member.

## 2. Current State

- The vendored UCD is `third_party/unicode/Scripts-16.0.0.txt` only
  (`ls third_party/unicode/` → 1 file, 2026-09-27). There is no `LineBreak.txt`,
  `EastAsianWidth.txt` or `emoji-data.txt`.
- Generated range tables live in `src/codegen/string/unicode/`:
  `unicode_gencat_ranges.txt` and `unicode_script_ranges.txt`, included by
  `src/unicode/range_tables.rs` via `include_str!`. Each generator is checked by
  `scripts/check-generated.sh` (`check <generator> <artifact>` lines).
- General-category lookup in emitted programs is `strings::genCat`, an
  `internal_only` native member (`func_gen_cat.rs`, `Body::abi_inline`, "the same
  generated 4,099-arm IF-chain"). This is the precedent for a class lookup.
- Grapheme segmentation is native (`src/codegen/string/unicode_props.rs:emit_grapheme_break_branch`)
  over utf8proc's properties.

### Measured populations

| What | Count | Command |
|---|---|---|
| Line_Break ranges after merging adjacent same-class runs | 2,787 runs (class + flag value) | `python3 -I tools/unicode-tables/gen_linebreak_table.py > …` → stderr `2787 runs, Unicode 16.0.0` |
| `LineBreakTest-16.0.0.txt` cases | 16,672 | `grep -c "^[^#]" third_party/unicode/LineBreakTest-16.0.0.txt` |

## 3. Design Overview

1. **Data.**
   - Vendor `LineBreak-16.0.0.txt`, `EastAsianWidth-16.0.0.txt` (LB30 needs the
     `F`/`W`/`H` exclusion), `emoji-data-16.0.0.txt` (Extended_Pictographic, for LB30b)
     and `LineBreakTest-16.0.0.txt` into `third_party/unicode/`.
   - A new generator `tools/unicode-tables/gen_linebreak_table.py` emits
     `src/codegen/string/unicode/unicode_linebreak_ranges.txt`. Each range gets the
     resolved class, with the LB1 resolutions applied at generation time
     (`AI`/`SG`/`XX` → `AL`, `SA` → `AL` or `CM` by general category, `CJ` → `NS`). It also
     gets two flag bits: East-Asian F/W/H, and Extended_Pictographic-unassigned
     (for LB30b).
   - Using 16.0.0 matches the pinned gencat/script tables, and the spec's two-versions
     contract is extended by one sentence.
2. **Lookup.** `strings::lineBreakClass(cp) AS Integer` is `internal_only`, generated
   from the table the same way `genCat` is.
3. **Algorithm.** A MFBASIC helper `__strings_lineBreaks(text)` implements LB2–LB31 as
   ordered rules over a scalar array, with the look-behind state that LB9 (combining
   marks), LB14–LB17 (with intervening spaces), LB21a, LB25 (numbers, the regex
   form), LB28a (Brahmic) and LB30a (regional-indicator pairs) need.

**Gate:** new API. Correctness is the conformance file.
- **Expected to change:** nothing existing, apart from the new generated artifact and the
  `check-generated.sh` line.
- **Must not change:** any existing strings golden.

**Design uncertainty (first):** the table size and the lookup form. A 4,000-arm
IF-chain is the precedent, but it is measured first.

**Correctness risk:** LB9/LB10 (combining sequences) and LB25 (numeric context), where
most implementations fail conformance. The test file is the gate, not reasoning.

Rejected alternatives:
- **The UAX #14 pair table.** Rejected: it is officially non-normative and misses
  LB25, LB28a, LB30 and later rules, so it fails the conformance file.
- **Placing it in `canvas`.** Rejected: it has no font dependency and is useful in
  `term`/console wrapping.
- **Reading Line_Break from utf8proc.** Rejected: utf8proc doesn't carry it.

## Phases

> **NOTE — keep the checkboxes current as you go** (rules as in plan-162-A).

### Phase 1 — data and measurement

- [x] Vendor the four UCD files at 16.0.0 into `third_party/unicode/`, with their
      license note beside the existing Scripts file. Downloaded 2026-10-17 from
      `https://www.unicode.org/Public/16.0.0/ucd/…`, saved as `LineBreak-16.0.0.txt`,
      `EastAsianWidth-16.0.0.txt`, `emoji-data-16.0.0.txt` and
      `LineBreakTest-16.0.0.txt`. Each file's own header confirms 16.0.0. No licence
      note existed for `Scripts-16.0.0.txt` either, so `third_party/unicode/README.md`
      (sources, SHA-256s) and `LICENSE.txt` (Unicode License v3) now cover all five.
- [x] `tools/unicode-tables/gen_linebreak_table.py`, plus a README entry in
      `tools/unicode-tables/README.md`, plus a `check` line in
      `scripts/check-generated.sh`. It applies LB1 (AI/SG/XX→AL, SA→CM for Mn/Mc else
      AL, CJ→NS) and packs flags into each run's value: +64 East Asian F/W/H, +128
      Extended_Pictographic ∧ Cn, +256/+512 Pi/Pf on QU. General category comes from
      the committed `unicode_gencat_ranges.txt`, so no interpreter table is read
      (Correction 1).
- [x] Record the range count and the test-case count (§2) with their commands.

Acceptance: the artifact regenerates identically.
  Check: `scripts/check-generated.sh` → 0 diffs (est. 1 min).
  Result: exit 0, including `ok: src/codegen/string/unicode/unicode_linebreak_ranges.txt
  matches tools/unicode-tables/gen_linebreak_table.py`.
Commit: 551fbb2bd

### Phase 2 — class lookup

- [x] `src/codegen/builtins/strings/func_line_break_class.rs`: `internal_only`,
      generated from the artifact following `func_gen_cat.rs`. `Body::abi_inline` over
      the new `emit_unicode_range_value` (the bisection `genCat` uses, factored out;
      `genCat` now calls it), reading `_mfb_unicode_linebreak_ranges`, which is emitted
      only when relocated against (`unicode_runtime_data_objects`, test updated to 18
      objects).
- [x] Test: for every scalar, the class equals the generator's resolved class, via a
      dump from a `--debug` scratch program compared to the artifact. Done in Rust
      (Correction 2): `range_tables::tests::linebreak_matches_the_ucd_for_every_scalar`
      re-derives each scalar's value from the vendored UCD files, independently of the
      Python generator, and compares all 1,114,112 with the bisection;
      `binary_search_matches_a_linear_scan_for_every_scalar` covers the line-break table
      too. At run time the emitted lookup is exercised by the conformance file below.
      `cargo test --bin mfb range_tables` → 8 passed.

Acceptance: an exact class for every scalar.
  Check: `cargo test --test <strings rt suite> line_break_class` → pass (est. 3 min).
  Record the suite name when adding it.
  Result: the check is `cargo test --bin mfb range_tables` → 8 passed (Correction 2).
  The runtime suite is `rt_strings_line_breaks`.
Commit: (with Phase 3, Correction 3)

### Phase 3 — algorithm, API and docs

- [x] `func_line_breaks.rs`: the `LineBreak` record, `strings::lineBreaks`, and the
      MFBASIC body implementing LB2–LB31. A `Body::Rewrite` to `__strings_lineBreaks`,
      in a chunk gated `WhenUsed(["lineBreaks"])`, so a program that doesn't call it
      doesn't compile it. Rules run over LB9 units (§3, Correction 4).
- [x] Conformance test: parse `LineBreakTest-16.0.0.txt` in the Rust test, build one
      MFBASIC program that runs every case and prints the break positions, and compare.
      This is one build, not one per case. `tests/runtime/rt_strings_line_breaks.rs`
      (`rt_strings_line_breaks`). The cases go to the program as a data file, and the
      test asserts there are 16,672 of them.
- [x] Fixtures (`.ai/compiler.md`): `tests/rt-behavior/strings/line-breaks-valid`
      (prose, CJK, `$(12.35)`, CR LF, a leading combining mark, NBSP, empty) and
      `tests/syntax/strings/lineBreaks` (no argument, wrong type, too many).
      `scripts/test-accept.sh … line-breaks-valid lineBreaks` → passed.
- [x] Man: `strings::lineBreaks` (with a wrapping example using a fixed-width measure,
      so it runs without canvas, and a cross-reference to `canvas::measureText`) and
      the `strings` package intro. Check with
      `scripts/man-run-examples.sh strings --run` and
      `scripts/man-census.sh --memory-scope` → 0 unclassified. The example wraps a
      sentence to 20 characters with `mid`. `man-run-examples.sh strings --run
      lineBreaks` → 1 built, 1 ran, 0 failed, printing four lines of at most 20
      characters. The package overview names `lineBreaks`. Census → 0.
- [x] Spec: add a UAX #14 subsection to `src/docs/spec/unicode/01_tables-and-algorithms.md`
      (version, LB1 resolutions, untailored, SA handling), citing the generator and
      the helper. "Runtime algorithm: line breaking (UAX #14)", plus Line_Break in the
      version-pinning sentence. `cargo test --bin mfb spec` → 46 passed.

Acceptance: 100% conformance.
  Check: `cargo test --test <strings rt suite> line_break_conformance` → every case
  passes (est. 4 min).
  Result: `cargo test --test rt_strings_line_breaks` → `line_break_conformance` ok, all
  16,672 cases agree, 1.9 s.
Commit: c4a3bf5c4 (Phases 2 and 3)

## Validation Plan

- Tests: a class dump over every scalar, the full conformance file, and edge cases:
  the empty string → `[]` (per Open Decisions, and stated on the man page); a text
  ending in a hard line break; a lone combining mark at position 0.
- Runtime proof: a `/tmp` canvas program wrapping a mixed English/CJK paragraph into a
  column with `lineBreaks` + `measureText([latin, cjk], …)`. Take a screenshot.
  Runtime proof (2026-10-17): `/tmp/p162/proof` wraps a mixed English/Japanese paragraph
  with `strings::lineBreaks` + `canvas::measureText([latin, cjk], …)` (Arial, STHeiti
  Light), draws each line as a `Text` over `[latin, cjk]`, underlines the first line
  from `underlineOffset`/`underlineThickness`, and draws a caret at its sixth
  `caretStops` stop. The dumped frame (`MFB_CANVAS_DUMP`, converted to PNG) shows Latin
  from Arial and kana/kanji from STHeiti, `AVAWATo` kerned, lines broken between CJK
  characters and never inside `“quotes”`/`(brackets)`, the underline exactly the first
  line's measured width (502.62 px), and the caret between `Wrapp` and `ing` (scalar 5,
  x 81.96).
- Doc sync: man and spec, as above.
- Final gate: `cargo test` (full), `scripts/check-generated.sh` and
  `scripts/artifact-gate.sh` (est. 35 min).

## Open Decisions

- Report the end of the text: yes, as mandatory (recommended, so a wrap loop needs no
  special case) vs. no.
- The empty string: an empty list (recommended) vs. one stop at 0.

## Corrections

1. **`genCat` is a rodata range table now, not an IF-chain.** §2 and Phase 2 name
   "the same generated 4,099-arm IF-chain" as the precedent. plan-118-B replaced it:
   `strings::genCat` lowers through `lower_unicode_range_member`, a bisection over a
   `(u32 lastCodepoint, u32 nameIndex)` rodata table (`src/unicode/range_tables.rs`).
   `lineBreakClass` follows that path instead, returning the run's value as an
   `Integer` rather than a name. It bisects the 2,787 runs in 12 steps, where an
   IF-chain would compare up to 2,787 times per character.
2. **The every-scalar class check runs in Rust, not from a `--debug` dump.** A
   `strings` chunk can reach neither `fs` nor `os` (`strings` imports nothing), and an
   `internal_only` member can't be called from user code or from another package's
   chunk. A per-scalar dump would mean giving `strings` new imports that every program
   using it would carry. The check therefore compares, for all 1,114,112 scalars, an
   independent Rust re-derivation from the UCD files with `RangeTable::lookup`, the
   reference implementation of the emitted bisection (the plan-118-B precedent for
   `genCat`). The emitted code is the bisection `genCat` already runs, and the
   conformance program exercises it at run time.
3. **Phases 2 and 3 share a commit.** The class lookup has no runtime consumer but
   `lineBreaks`, so both were verified together: the Rust sweep, then the conformance
   file.
4. **The rules are evaluated over LB9 units.** LB9 says a combining sequence takes its
   base's class "in all of the following rules", with the marks ignored. The text is
   therefore cut into units first, and every rule's look-behind walks units. Positions
   inside a unit cannot break, because LB9 never absorbs after the classes LB4–LB8
   break after. The design is unchanged from §3; the conformance file confirms it.
5. **"Expected to change: nothing existing" was false at first, and is true now.**
   The final artifact gate found the `strings` and `regex` `codegen_cover_rt`
   `.ncodesum` sentinels moved on all 5 targets (10 diffs). Splitting
   `emit_unicode_range_value` out of `emit_unicode_range_lookup` made the lookup
   allocate two temporaries of its own before the bisection's, so every
   `genCat`/`scriptOf` caller got different vreg numbering. Both packages route
   through `lower_unicode_range_member`, and nothing else moved. The fix is a shared
   `emit_unicode_range_search` that hands back its own `addr`/`scratch`, so the lookup
   emits its old instruction sequence. Verified: `scripts/artifact-gate.sh
   target/release/mfb strings` and `… regex` each give 0 diffs on 7 goldens.
   `line_break_conformance` passes 1/1, and `cargo test --bin mfb -- unicode gen_cat
   script_of genCat scriptOf` passes 31/31.

## Summary

The algorithm is fiddly but fully specified, and the conformance file is an exact
oracle, so the risk is effort, not ambiguity. It touches no existing member.
