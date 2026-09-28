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
| Line_Break ranges after merging adjacent same-class runs | UNMEASURED | Phase 1: generator dry run, `wc -l` |
| `LineBreakTest-16.0.0.txt` cases | UNMEASURED | Phase 1: `grep -c "^[^#]" third_party/unicode/LineBreakTest-16.0.0.txt` |

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

- [ ] Vendor the four UCD files at 16.0.0 into `third_party/unicode/`, with their
      license note beside the existing Scripts file.
- [ ] `tools/unicode-tables/gen_linebreak_table.py`, plus a README entry in
      `tools/unicode-tables/README.md`, plus a `check` line in
      `scripts/check-generated.sh`.
- [ ] Record the range count and the test-case count (§2) with their commands.

Acceptance: the artifact regenerates identically.
  Check: `scripts/check-generated.sh` → 0 diffs (est. 1 min).
Commit: —

### Phase 2 — class lookup

- [ ] `src/codegen/builtins/strings/func_line_break_class.rs`: `internal_only`,
      generated from the artifact following `func_gen_cat.rs`.
- [ ] Test: for every scalar, the class equals the generator's resolved class, via a
      dump from a `--debug` scratch program compared to the artifact.

Acceptance: an exact class for every scalar.
  Check: `cargo test --test <strings rt suite> line_break_class` → pass (est. 3 min).
  Record the suite name when adding it.
Commit: —

### Phase 3 — algorithm, API and docs

- [ ] `func_line_breaks.rs`: the `LineBreak` record, `strings::lineBreaks`, and the
      MFBASIC body implementing LB2–LB31.
- [ ] Conformance test: parse `LineBreakTest-16.0.0.txt` in the Rust test, build one
      MFBASIC program that runs every case and prints the break positions, and compare.
      This is one build, not one per case.
- [ ] Man: `strings::lineBreaks` (with a wrapping example using a fixed-width measure,
      so it runs without canvas, and a cross-reference to `canvas::measureText`) and
      the `strings` package intro. Check with
      `scripts/man-run-examples.sh strings --run` and
      `scripts/man-census.sh --memory-scope` → 0 unclassified.
- [ ] Spec: add a UAX #14 subsection to `src/docs/spec/unicode/01_tables-and-algorithms.md`
      (version, LB1 resolutions, untailored, SA handling), citing the generator and
      the helper.

Acceptance: 100% conformance.
  Check: `cargo test --test <strings rt suite> line_break_conformance` → every case
  passes (est. 4 min).
Commit: —

## Validation Plan

- Tests: a class dump over every scalar, the full conformance file, and edge cases:
  the empty string → `[]` (per Open Decisions, and stated on the man page); a text
  ending in a hard line break; a lone combining mark at position 0.
- Runtime proof: a `/tmp` canvas program wrapping a mixed English/CJK paragraph into a
  column with `lineBreaks` + `measureText([latin, cjk], …)`. Take a screenshot.
- Doc sync: man and spec, as above.
- Final gate: `cargo test` (full), `scripts/check-generated.sh` and
  `scripts/artifact-gate.sh` (est. 35 min).

## Open Decisions

- Report the end of the text: yes, as mandatory (recommended, so a wrap loop needs no
  special case) vs. no.
- The empty string: an empty list (recommended) vs. one stop at 0.

## Corrections

(empty)

## Summary

The algorithm is fiddly but fully specified, and the conformance file is an exact
oracle, so the risk is effort, not ambiguity. It touches no existing member.
