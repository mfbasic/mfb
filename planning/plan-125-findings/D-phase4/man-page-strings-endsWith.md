Every sentence on the page checked out against the binary, including both examples. One sharp edge is missing.

### 1. Scalar boundary stated, grapheme boundary left unsaid
UNIT:      man-page:strings/endsWith
CLAIM:     "Because both operands are well-formed UTF-8 and UTF-8 is self-synchronizing, a matching byte suffix is always also a whole-scalar suffix — a match can never land mid-scalar."
VERDICT:   incomplete
EVIDENCE:  The sentence is true, but it stops at the scalar boundary and never says a match CAN land mid-grapheme. The standard requires "Unicode scalar vs. grapheme" to be stated explicitly (`.ai/man-content.md` §2, "Sharp edges"). Probe `/tmp/plan-125-scratch/D-phase4/man-page-strings-endsWith/probe.bas`, built with `mfb init p_probe && mfb build -q` and run as `./build/p_probe.out`, printed:
```
nfc vs nfd: FALSE          ' endsWith("caf\u{00E9}", "e\u{0301}")
nfd tail: TRUE             ' endsWith("cafe\u{0301}", "\u{0301}")   — matches only the combining mark
grapheme partial: TRUE     ' endsWith("Hi 🇺🇸", "🇸")               — matches half of one flag grapheme
```
A developer who reads "whole-scalar suffix" and knows that `strings::graphemes` exists will reasonably assume a suffix cannot split a visible character. It can. The `nfc vs nfd` line also shows the "No normalization" sentence in practice, which the page states but never illustrates with a Unicode case.
SUGGESTED: After the existing sentence, add: "The boundary is a scalar boundary, not a grapheme boundary: a suffix may match the trailing scalars of a multi-scalar grapheme, so `endsWith("cafe\u{0301}", "\u{0301}")` is `TRUE` even though the visible text ends in `é`. Apply `strings::normalizeNfc` to both operands first if you need composed and decomposed spellings to compare equal; see `strings::graphemes` for grapheme-aware splitting."

Everything else verified. Both page examples were copied verbatim into `p_ex1/src/main.mfb` and `p_ex2/src/main.mfb`, built with `mfb build -q`, and printed `TRUE TRUE FALSE` and `TRUE` respectively, matching the page. The same probe confirmed the case-sensitivity claim (`"Hello","LO"` is FALSE), all four boundary cases in the paragraph (empty/empty TRUE, `"abc",""` TRUE, `"","a"` FALSE, `"bc","abc"` FALSE), that both operands are unchanged after the call, and that an `AttributedString` in the `value` position returns TRUE for `"lo"` on styled `"Hello"`. A second probe passing an `AttributedString` as `suffix` was rejected at compile time with `TYPE_CALL_ARGUMENT_MISMATCH`, consistent with the page saying only `value` accepts the overload. The lowering in `src/codegen/builtins/strings/gen_strings_support.rs:lower_string_prefix_predicate` is a length check followed by a byte-range compare at offset `len(value) - len(suffix)`, and the descriptor's `errors` list is empty, matching "the call never fails" and the absence of an Errors table. No banned memory vocabulary and no compiler-internal vocabulary appear on the page.
