### 1. "Never 0" reads as a property of the type, but only holds for what `lineBreaks` returns
UNIT:      man-page:strings/types
CLAIM:     "Where the new line would start, in Unicode scalars from the start of the text, as strings::mid counts. Never 0."
VERDICT:   misleading
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase4-current/man-page-strings-types/proj/src/main.mfb`: `LET custom AS strings::LineBreak = strings::LineBreak[index := 0, mandatory := FALSE]` compiles and `io::print(toString(custom.index))` prints `custom index=0`. The record is an ordinary public record with no constraint on `index`; the "never 0" guarantee is a property of `__strings_lineBreaks` (`src/codegen/builtins/strings/func_line_breaks.rs`, loop starts at `k = 1` and appends `index := n` last), not of the field. The probe confirmed `lineBreaks` itself never reports 0 across six inputs (first reported index was 1, 1, 3, 2, 2; empty text reported nothing). The rest of the sentence is verified: for `"é😀 ab" & LF & "cd" & CR & LF & "ef"` the indexes were 1, 3, 6, 10, 12 (scalar counts, not bytes), and `strings::left(text, b.index)` returned exactly the text before each break.
SUGGESTED: "Where the new line would start, in Unicode scalars from the start of the text, as `strings::mid` counts. `strings::lineBreaks` never reports 0, and its last entry's index is the length of the text."

### 2. The final entry's index is the text length, which the field description does not say
UNIT:      man-page:strings/types
CLAIM:     "`TRUE` where the line must break — after a hard line break, and at the end of the text — and `FALSE` where it only may."
VERDICT:   incomplete
EVIDENCE:  Same probe. For `"x-y z"` (5 scalars) the last entry is `index=5 mandatory=TRUE`; for `"ab" & LF` (3 scalars) the only entry is `index=3 mandatory=TRUE` (the after-LF break and the end-of-text break coincide and are reported once). So the end-of-text entry has an index one past the last scalar, where no line actually "starts"; a reader using `index` as the start of a `strings::mid` slice needs to know that entry is the text's length. The mandatory conditions themselves are verified: LF (index 6), CR+LF reported once after the LF (index 10, nothing at 9), FF (index 2) and U+2029 (index 4) all came back `TRUE`; the hyphen and space breaks came back `FALSE`.
SUGGESTED: "`TRUE` where the line must break — after a line feed, carriage return, form feed, or line or paragraph separator, and at the end of the text, where index equals the text's length — and `FALSE` where it only may. See `mfb man strings lineBreaks` for the full rule set."

Everything else on the page checked out: no banned memory vocabulary, no compiler-internal wording, the record intro's "as `strings::lineBreaks` returns it" is accurate, and the scalar-index unit and the "as `strings::mid` counts" zero-based claim were confirmed by slicing with `strings::left` at each reported index.
