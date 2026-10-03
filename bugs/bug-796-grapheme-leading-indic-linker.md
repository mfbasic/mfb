# bug-796: Runtime grapheme segmentation joins a leading Indic linker to a consonant

Last updated: 2026-10-02
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Open
Regression Test: proposed `tests/rt-behavior/strings/grapheme-indic-context-rt`

The correct behavior is that `U+094D U+0915` forms two extended grapheme
clusters, whether segmentation is folded or emitted. Runtime segmentation
currently returns one cluster, losing the required preceding-consonant context
for the Indic conjunct rule.

## Failing Reproduction

Create `/tmp/mfb-unicode-indic-probe/project.json`:

```json
{"name":"indic_probe","version":"0.1.0","mfb":"1.0","kind":"executable","sources":[{"root":"src","role":"main","include":["**/*.mfb"]}],"entry":"main","targets":["native"]}
```

Create `src/main.mfb` in that project:

```basic
IMPORT io
IMPORT strings
FUNC clusters(s AS String) AS Integer
  RETURN len(strings::graphemes(s))
END FUNC
FUNC main AS Integer
  io::print(toString(len(strings::graphemes("\u{94d}\u{915}"))))
  io::print(toString(clusters("\u{94d}\u{915}")))
  RETURN 0
END FUNC
```

```sh
./target/release/mfb build /tmp/mfb-unicode-indic-probe
/tmp/mfb-unicode-indic-probe/build/indic_probe.out
```

Observed on macOS AArch64, default optimization: `2` then `1`.
Expected: `2` then `2`. The initial measurement used these same functions in
`/tmp/mfb-unicode-spec-probe`, after the NFC probe's output.
The folded `unicode_segmentation` path supplies the contrasting correct result.

## Root Cause

`src/codegen/builtins/strings/gen_graphemes.rs:lower_strings_graphemes` seeds
`state_icb` from the first scalar's raw property. For U+094D that is LINKER.
`src/codegen/string/unicode_props.rs:emit_grapheme_break_branch` then suppresses
a boundary whenever `state_icb == LINKER` and the current property is CONSONANT.
No state bit establishes that a consonant preceded the linker. The updater
`emit_grapheme_state_update` likewise uses property values as context states;
a fix must model the complete consonant/extend/linker prefix, not merely
special-case this input.

## Goal and Non-goals

Implement the full Indic conjunct context for GB9c, including a preceding
consonant, intervening Extend/Linker scalars, and a required linker. Preserve
valid consonant-linker-consonant clusters and unrelated grapheme rules.
Do not remove runtime segmentation, change Unicode versions, or force dynamic
inputs through constant folding to hide the discrepancy.

## Blast Radius

Search: `rg -n 'emit_grapheme_break_branch|emit_grapheme_state_update' src/codegen`.

- `gen_graphemes.rs:lower_strings_graphemes`: affected runtime list construction;
  `graphemesCount` and the ordinary `graphemeAt` path consume that list.
- `func_display_width.rs:display_width_of`: affected shared boundary decision;
  a changed boundary does not necessarily change the summed width for this probe.
- `string_self_update.rs`: grapheme-window self-updates use the same state helpers.
- `unicode_props.rs:emit_grapheme_break_branch_free` and
  `emit_grapheme_state_update_free`: same context logic in the standalone emitter
  used by terminal text placement; audit and fix together.
- `unicode/backend.rs:graphemes`: unaffected Rust folding oracle.
- All native targets share these emitters; only macOS AArch64 was executed.

Existing-report search included `bugs/`, `bugs/completed/`, `planning/` and
`planning/completed/` with `GB9c|indic.conjunct|linker.{0,55}consonant`;
matched implementation/history discussions, not an existing report of this failure.

## Fix Design

Represent inactive, consonant-prefix and linker-seen contexts distinctly from raw
Indic property codes. Initialize from the first scalar, preserve a valid prefix
through permitted intervening scalars, and reset it when context is broken.
Apply equivalent transitions to builder and standalone helpers.

## Phases

### Phase 1 — failing tests and audit

- [ ] Add dynamic-versus-folded cases for leading linker, Extend+linker, valid
      consonant+linker, consonant+Extend+linker, repeated linkers, and context resets.
- [ ] Audit callers and test the standalone terminal emitter's state transitions.

Acceptance: leading-linker regression fails today; valid conjunct contrasts pass.
Commit: —

### Phase 2 — correction

- [ ] Correct context initialization, transitions and boundary eligibility in
      both helper families.
- [ ] Verify grapheme list, count, indexing and self-update results.

Acceptance: runtime and folded results agree for the corpus without weakening tests.
Commit: —

### Phase 3 — full validation and documentation

- [ ] Run the full suite before changing any golden; inspect all expected deltas.
- [ ] Run compiler/artifact/runtime gates from `.ai/compiler.md` and
      `.ai/testing-gates.md` across affected targets.
- [ ] Update the Unicode spec's current-behavior limitation once fixed.

Acceptance: full validation passes; the reproduction prints `2` twice.
Commit: —

## Validation Plan

Use the dependency's `unicode_segmentation::UnicodeSegmentation::graphemes`
result as the independent runtime oracle for the repro and contrast corpus.
Keep regression inputs dynamic. Include the pinned Unicode grapheme-break test
corpus when validating the redesigned state machine. Update
`src/docs/spec/unicode/01_tables-and-algorithms.md`. This filing changes no code
or tests.
