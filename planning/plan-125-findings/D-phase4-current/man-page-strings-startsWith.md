### 1. A prefix match cannot "begin" inside a character
UNIT:      man-page:strings/startsWith
CLAIM:     "Matching is by Unicode scalar: a match always covers whole scalars, but not necessarily whole characters: it can begin or end inside a combining sequence, an emoji sequence or a flag, so startsWith("e\u{0301}", "e") is TRUE."
VERDICT:   misleading
EVIDENCE:  `lower_string_prefix_predicate` in `src/codegen/builtins/strings/gen_strings_support.rs` compares `prefix` against the first `len(prefix)` bytes of `value` starting at offset 0, with `suffix == false`. A prefix match therefore always begins at the start of `value`; the only boundary it can land inside is the END of a combining or emoji sequence. The "begin or end" wording is the generic `contains`/`find` sentence and is half impossible here. Probe (`p_probe`): `startsWith("e\u{0301}", "e")` → `TRUE`; `startsWith("\u{1F1FA}\u{1F1F8}x", "\u{1F1FA}")` → `TRUE` (ends inside a flag); `startsWith("\u{00E9}", "e")` → `FALSE`.
SUGGESTED: "Matching is by Unicode scalar: a match always covers whole scalars, but not necessarily whole characters. Because the match starts at the beginning of `value`, it can only *end* inside a combining sequence, an emoji sequence or a flag, so `startsWith("e\u{0301}", "e")` is `TRUE` and `startsWith("🇺🇸x", "🇺")` is `TRUE`."

### 2. Page says both "byte comparison" and "by Unicode scalar" without reconciling them
UNIT:      man-page:strings/startsWith
CLAIM:     "The test is an exact byte comparison of the leading bytes of value against every byte of prefix, in order" and, in the parameter table, "The string whose leading bytes are examined."
VERDICT:   misleading
EVIDENCE:  Both are true of the binary (`emit_string_byte_range_equal_branch` in `gen_strings_support.rs` compares bytes), and the next paragraph says "Matching is by Unicode scalar". The two models are equivalent only because a `String` always holds valid UTF-8, which the page never states. A developer reading "leading bytes" in the parameter table is handed an encoding-level model they do not need, and then told two sentences later the unit is the scalar. Probe: `startsWith("😀", "😁")` → `FALSE` (the two emoji share three leading bytes, so no partial-scalar match is possible).
SUGGESTED: Parameter `value`: "The string whose start is examined. May be empty." Description: "The test compares the start of `value` with `prefix` scalar by scalar, exactly; it succeeds only when every scalar of `prefix` matches." Drop "byte" from the page.

### 3. Only `value` takes an AttributedString; the page does not say `prefix` cannot
UNIT:      man-page:strings/startsWith
CLAIM:     "value may also be an AttributedString: the query runs on its visible text and returns exactly what the String overload returns: the same value and type, and like it, it never fails."
VERDICT:   incomplete
EVIDENCE:  Verified true for `value` (`p_probe`: bold-styled `"Hello"` with `"He"` → `TRUE`, with `"he"` → `FALSE`). But `p_probe_attr_prefix` (both arguments `AttributedString`) and `p_probe_str_attr_prefix` (`String` value, `AttributedString` prefix) both fail to compile with `TYPE_CALL_ARGUMENT_MISMATCH … expected String, String` (the Tier-A resolver in `src/codegen/builtins/strings/mod.rs` only substitutes the leading argument). A developer holding two attributed strings has no way to learn that from the page except by hitting the error.
SUGGESTED: Append: "`prefix` must be a plain `String`; pass `toString(p)` to test against an `AttributedString`'s visible text."

Everything else on the page checks out against the binary. The two Examples sections compiled and ran as separate projects under `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-startsWith/p_ex1` and `p_ex2`, printing `TRUE`, `TRUE`, `FALSE` and `TRUE` respectively. The boundary paragraph was probed in full: a longer prefix → `FALSE`, empty against empty → `TRUE`, non-empty prefix against empty value → `FALSE`, equal strings → `TRUE`, an embedded NUL compares normally, and both operands are unchanged afterwards. The descriptor registers no errors (`errors: vec![]` in `func_starts_with.rs`), matching the absent Errors table and the "never fails" claim. No banned memory vocabulary and no compiler-internal terms appear on the rendered page.
