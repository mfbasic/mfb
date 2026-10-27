### 1. "generally smaller" leaves the ASCII case unstated
UNIT:      man-page:strings/toScalars
CLAIM:     "The element count therefore equals len(value) and is generally smaller than strings::byteLen(value)."
VERDICT:   incomplete
EVIDENCE:  Probe under /tmp/plan-125-scratch/D-phase4/man-page-strings-toScalars/probe: `len(strings::toScalars("abc"))` and `strings::byteLen("abc")` both print `3`; for `"e\u0301中😀"` the counts are `count=4 len=4 byteLen=10`. The count is never larger than the byte length and is equal exactly when every scalar is ASCII. "generally smaller" is true but does not tell the developer when the two agree, which is the case they will hit first.
SUGGESTED: "The element count therefore equals len(value). It never exceeds strings::byteLen(value), and equals it only when every character is ASCII."

Everything else on the page held up. Probes run, all built with the release binary and executed from `build/*.out`:

- Both page examples compile and print exactly what the page implies: example 1 prints `3` then `TRUE`; example 2 prints `a1b2c3`.
- Combining mark and astral character: `"e\u0301中😀"` yields four elements (`101 769 20013 128512`) in left-to-right order, while `strings::graphemes` reports three, confirming scalar-not-grapheme and ordering.
- Round trip: `strings::fromScalars(strings::toScalars(s)) = s` printed `TRUE`.
- Empty string yields a zero-length list (`empty=0`).
- Non-mutation: the source string prints unchanged afterward, and appending to the returned list leaves the original list at its prior length (`m=2 m2=3`).
- The AttributedString overload accepts `astrings::fromString("a中😀")`, returns three scalars, and rebuilds to `a中😀`.
- The descriptor in `src/codegen/builtins/strings/func_to_scalars.rs` declares no errors, matching the page's absence of an Errors table; the seam in `helper_scalar_seam.rs` decodes via `encoding::utf32Encode` with an unreachable TRAP, consistent with "decoding cannot fail".
- No banned memory vocabulary appears, and no sentence teaches a borrow or ownership model. "Walks the UTF-8 once" and "well-formed UTF-8 by construction" are language-level facts a developer can act on, not compiler internals.
