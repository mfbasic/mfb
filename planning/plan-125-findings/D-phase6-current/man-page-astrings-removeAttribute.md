All probes done. Every behavioral claim on the page checked out against the binary; I have three minor findings on wording and completeness.

### 1. Boring case not stated: a range with no matching span is not an error
UNIT:      man-page:astrings/removeAttribute
CLAIM:     "removeAttribute returns a new AttributedString with attr removed over the inclusive range [start, endIndex]."
VERDICT:   incomplete
EVIDENCE:  Probe `probe.mfb` (built with `mfb build -q p_probe`), rows `rm disjoint` and `bold rm italic`: bold over 4..6 then `removeAttribute(m, 10, 12, bold())` returned without error and printed `[ ][ ][ ][ ][ F ][ F ][ F ][ ]...`; bold over 0..3 then `removeAttribute(e, 0, 3, italic())` printed `[ F ][ F ][ F ][ F ]`. `func_remove_attribute.rs:BODY` loops the spans and appends non-matches; there is no "nothing matched" branch. The standard (§2 "Return semantics, including what comes back in the boring case") requires the page to say so; today the only hint is the absence of an error row.
SUGGESTED: Append to the first paragraph: "A range that no matching span touches is not an error: the result carries the same attributes as `value`. Every span that matches is affected, not just the first."

### 2. Flank notation names fields of a type the developer cannot see
UNIT:      man-page:astrings/removeAttribute
CLAIM:     "its surviving left flank [s.start, start−1] and/or right flank [endIndex+1, s.endIndex] are kept and the overlap dropped."
VERDICT:   out-of-scope
EVIDENCE:  `s.start` / `s.endIndex` are the fields of the internal `AttrSpan` record (`helper_split_span.rs:BODY`, `AttrSpan[s.start, rs - 1, ...]`). `mfb man astrings types | grep AttrSpan` prints nothing and `mfb man astrings readSpans` reports "unknown astrings function", so no page the developer can open defines `s`. The behaviour itself is correct: probe row `example c after` printed bold on 5..7 and 11..15 after removing 8..10 from a 5..15 span.
SUGGESTED: "A matching span that straddles the range is **split**: the part before `start` and/or the part after `endIndex` survive, and the overlap is dropped."

### 3. "guard construction" names the wrong operation
UNIT:      man-page:astrings/removeAttribute
CLAIM:     "even 0, 0 is out of bounds on an empty AttributedString, so guard construction from text that might be empty."
VERDICT:   misleading
EVIDENCE:  `removeAttribute` constructs nothing; the thing to guard is this call. Probe `err4.mfb` (`fromString("")` then `removeAttribute(a, 0, 0, bold())`) raised `7-705-0001 attribute range out of bounds`, so the fact is right, only the advice is aimed at the wrong place (`fromString("")` itself succeeds).
SUGGESTED: "even `0, 0` is out of bounds on an empty `AttributedString`, so check `scalarLen` before calling when the text might be empty."

Everything else verified true at HEAD, for the audit trail:

- **Example** compiles (`mfb build -q p_ex` wrote `p_ex/build/p_ex.out`, exit 0). Re-run with printing, bold on 5..15 minus 8..10 leaves 5..7 and 11..15, which is what the page implies.
- **Input unchanged**: probe row `b unchanged?` printed the original 5..15 bold after the call, confirming "returns a new AttributedString".
- **Structural match includes the value**: `fontSize(12)` was not removed by `removeAttribute(..., fontSize(14))` but was by `fontSize(12)`; `font("Menlo")` survived `font("Monaco")`; bold survived `italic()`. Matches `helper_attr_equals.rs:BODY`.
- **Winner reveal**: size 10 over 0..7 plus size 20 over 2..5 read as 20 on 2..5; after removing 20 over 2..5, every index read 10.
- **Errors**: `start > endIndex` and `start < 0` both raised `7-705-0002 invalid attribute range`; `endIndex = len` raised `7-705-0001 attribute range out of bounds`; empty text with `0, 0` raised `7-705-0001`. All match `helper_validate_range.rs:BODY`.
- **Scalar units**: on `"héllo"` the range `0..4` succeeded and `0..5` raised out of bounds, so indexes are Unicode scalars, as the page says.
- **Vocabulary**: no word from the §4 banned list appears on the rendered page outside the derived "Allocation failed." message, which is a §4.4 carve-out. No sentence teaches a borrow or ownership model.
