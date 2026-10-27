### 1. Mid-grapheme match is a sharp edge the page omits
UNIT:      man-page:strings/startsWithAny
CLAIM:     "Each individual test is the same exact byte comparison `strings::startsWith` performs: the leading bytes of `value` must equal every byte of the candidate, in order, with no normalization and no case folding."
VERDICT:   incomplete
EVIDENCE:  The sentence is true (`src/codegen/builtins/strings/gen_with_any.rs:lower_strings_with_any` compares raw bytes via `emit_string_byte_range_equal_branch`), but §2 requires the Unicode scalar-vs-grapheme edge to be stated, and the page never says a candidate can match part of a grapheme cluster. Probe `/tmp/plan-125-scratch/D-phase4/man-page-strings-startsWithAny/uni.mfb`, built with `mfb build -q .` in `p_uni` and run:
```
nfd graphemes: 1
nfd vs [nfc]: FALSE
nfd vs ["e"] (mid-grapheme): TRUE
nfc vs ["e"]: FALSE
```
where `nfd = strings::fromScalars([toScalar(101), toScalar(769)])` (e + combining acute, one grapheme) and `nfc = strings::fromScalars([toScalar(233)])`. So the one-grapheme string "é" starts with "e" when decomposed and does not when precomposed, and the two spellings of "é" do not match each other. The sibling `strings::startsWith` page at least states the whole-scalar guarantee ("a match can never land mid-scalar"); this page carries neither the guarantee nor the grapheme caveat, and a developer comparing user-typed text against a prefix list will hit it.
SUGGESTED: Append to that paragraph: "A match always ends on a whole Unicode scalar, but it may end inside a grapheme cluster: a decomposed `é` (`e` followed by a combining accent) begins with `"e"`, while the precomposed `é` does not, and the two spellings never match each other. Normalize both sides first if that matters."

No other findings. Verification run: both page examples were copied verbatim into `p_ex1` and `p_ex2` under the scratch directory, built with `mfb build -q .`, and printed `TRUE` and `FALSE` respectively, matching the page. The sharp-edge probe `probe.mfb` (`p_probe`) printed: list-order first match `TRUE`; `[""]` against an empty value `TRUE`; an empty candidate among others `TRUE`; a candidate longer than the value `FALSE` with no error; `"Hello"` vs `["hello"]` `FALSE`; an empty value against `["a"]` `FALSE` and against `[]` `FALSE`; the `MUT` list still held `2 zzz,ab` after the call; and an `AttributedString` built with `astrings::fromString("https://x")` returned `TRUE` against the scheme list. The descriptor in `src/codegen/builtins/strings/func_starts_with_any.rs` registers `errors: vec![]`, consistent with the empty Errors table and the "never fails" claim, and `strings/mod.rs:is_tier_a_query` lists `strings.startsWithAny`, confirming the AttributedString overload. No banned memory vocabulary and no compiler-internal vocabulary appear on the rendered page.
