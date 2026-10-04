# bug-796: Runtime grapheme segmentation joins a leading Indic linker to a consonant

Last updated: 2026-10-03
Effort: medium (1h–2h)
Severity: MEDIUM
Class: Correctness
Status: Fixed
Regression Test: `tests/rt-behavior/strings/grapheme-indic-context-rt`

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

- [x] Add dynamic-versus-folded cases for leading linker, Extend+linker, valid
      consonant+linker, consonant+Extend+linker, repeated linkers, and context resets.
- [x] Audit callers and test the standalone terminal emitter's state transitions.

Acceptance: leading-linker regression fails today; valid conjunct contrasts pass.
Commit: —

### Phase 2 — correction

- [x] Correct context initialization, transitions and boundary eligibility in
      both helper families.
- [x] Verify grapheme list, count, indexing and self-update results.

Acceptance: runtime and folded results agree for the corpus without weakening tests.
Commit: —

### Phase 3 — full validation and documentation

- [x] Run the full suite before changing any golden; inspect all expected deltas.
- [x] Run compiler/artifact/runtime gates from `.ai/compiler.md` and
      `.ai/testing-gates.md` across affected targets.
- [x] Update the Unicode spec's current-behavior limitation once fixed.

Acceptance: full validation passes; the reproduction prints `2` twice.
Commit: —

## Validation Plan

Use the dependency's `unicode_segmentation::UnicodeSegmentation::graphemes`
result as the independent runtime oracle for the repro and contrast corpus.
Keep regression inputs dynamic. Include the pinned Unicode grapheme-break test
corpus when validating the redesigned state machine. Update
`src/docs/spec/unicode/01_tables-and-algorithms.md`. This filing changes no code
or tests.

## Resolution (2026-10-03)

The transitions in `emit_grapheme_state_update` already reach EXTEND/LINKER only
through a consonant. The defect was the **seed**: each walk stored the first
scalar's raw property as the state. `emit_seed_indic_conjunct_state` (and its
`_free` twin) maps the seed to CONSONANT or "none".

It is applied at all five places a walk seeds the state:

- grapheme count and write (`lower_strings_graphemes`);
- `graphemeAt` (`gen_graphemes.rs`);
- `displayWidth` (`func_display_width.rs`);
- terminal placement (`term_grid.rs`).

A mid-string reseed never sees a linker or an Extend, because GB9 forbids a break
before an Extend-class scalar.

**Proof.** The fixture compares the folded count (`unicode_segmentation`) with
the runtime count, plus `graphemeAt(s, 0)`. The cases are: leading linker,
extend+linker, a valid conjunct, consonant+extend+linker, a double linker, a
context reset, two conjuncts, and linker-only. All agree; on main the first two
are wrong.

The `strings` byte-identity `.ncode` change was localized on `linux-x86_64`:
with label numbering normalized, the only change is six copies of the
4-instruction seed (plus spill offsets). Those goldens are regenerated at the
batch gate. The 21 related fixtures pass. Spec: `mfb spec unicode
tables-and-algorithms` (the limitation is replaced by the context model).
