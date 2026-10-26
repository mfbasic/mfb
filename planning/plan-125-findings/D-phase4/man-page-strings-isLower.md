### 1. Titlecase letters are the sharp edge the page skips
UNIT:      man-page:strings/isLower
CLAIM:     "Modifier letters (category Lm) and other letters (Lo, which covers uncased scripts such as Han and Arabic) are not reported as lowercase, and neither are digits, punctuation, or symbols."
VERDICT:   incomplete
EVIDENCE:  `__strings_isLower` in `src/codegen/builtins/strings/helper_scalar_seam.rs` returns `strings::genCat(cp) = "Ll"` and nothing else. Probe (`/tmp/plan-125-scratch/D-phase4/man-page-strings-isLower/probe.bas`, built with `mfb build probe.proj -q`, run `./probe.proj/build/mfb_project.out`): `Dz-title U+01C5 (ǅ) FALSE`, `dz U+01C6 (ǆ) TRUE`. Titlecase digraphs (category Lt) look like cased letters, are neither upper nor lower here, and the sentence that enumerates the excluded categories does not mention them. A developer checking `isUpper OR isLower` will treat `ǅ` as "not a letter of either case" without being told so.
SUGGESTED: "Modifier letters (category Lm), titlecase letters (Lt, such as `ǅ`), and other letters (Lo, which covers uncased scripts such as Han and Arabic) are not reported as lowercase, and neither are digits, punctuation, or symbols."

### 2. The page does not say how it differs from the Unicode "Lowercase" property most developers already know
UNIT:      man-page:strings/isLower
CLAIM:     "isLower is a category test, not a "has-no-uppercase-mapping" test."
VERDICT:   incomplete
EVIDENCE:  Same probe: `ord-a U+00AA (ª) FALSE`, `small-roman-one U+2170 (ⅰ) FALSE`, `circled-a U+24D0 (ⓐ) FALSE`. All three are `Lowercase=Yes` under Unicode's derived property (Ll plus Other_Lowercase), which is what Rust `char::is_lowercase`, Python `str.islower`, and ICU `u_isULowercase` answer. The page rules out one wrong mental model ("no uppercase mapping") but not the one a developer arriving from those languages actually carries. Nothing on the page tells them `ⓐ` is FALSE.
SUGGESTED: Add after that sentence: "It is also narrower than the Unicode `Lowercase` property that other languages' `isLowercase` tests usually implement: scalars such as `ª` (U+00AA), the small roman numerals `ⅰ`–`ⅿ`, and the circled letters `ⓐ`–`ⓩ` are lowercase by that property but are not category Ll, so isLower returns FALSE for them."

### 3. "Deterministic" without a Unicode version is a promise the reader cannot pin
UNIT:      man-page:strings/isLower
CLAIM:     "Classification follows the Unicode general categories, so it covers the whole code-point space rather than just ASCII, and is deterministic and locale-independent."
VERDICT:   incomplete
EVIDENCE:  `src/unicode/range_tables.rs:122` — "The Unicode 16.0.0 general-category runs (4,099 runs, 30 categories)"; and the comment at `range_tables.rs:235` records that U+0295 is Ll in 16.0.0 but Lo in older utf8proc data (probe: `U+0295 (ʕ) TRUE`). Categories change between Unicode versions, so a scalar's answer is only deterministic relative to a stated version. The page names none, so a developer comparing against a different library's answer for `ʕ` has no way to reconcile the disagreement.
SUGGESTED: "Classification follows the Unicode 16.0.0 general categories, so it covers the whole code-point space rather than just ASCII, and is deterministic and locale-independent."

Everything else verified. The intro, both parameter sentences, "returns TRUE when scalar has the Unicode general category Ll and FALSE otherwise", the total/never-fails claim (Errors table is empty and the lowering has no raise path), and the cross-references all hold against `helper_scalar_seam.rs` and the probe. Both page examples built with `mfb build` and printed `TRUE FALSE FALSE` and `4`, matching what the prose implies. The rendered page has 0 banned-vocabulary hits by the man-census pattern, and no sentence teaches a borrow or ownership model.
