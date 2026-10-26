### 1. Colour attributes are resolved too, but the page only names font and font size
UNIT:      man-page:astrings/getAttributes
CLAIM:     "flags are present when any covering span carries them; font/font-size take the winning span's value."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/helper_is_winner.rs:__astrings_isWinner` resolves per (class, member) and `mod.rs` registers `AttrTypeNumber` with `FontSize`, `Foreground` and `Background`. Probe `/tmp/plan-125-scratch/D-phase6-current/man-page-astrings-getAttributes/probe/src/main.mfb` adds `foreground(rgb(255,0,0))` over 0..3 then `foreground(rgb(0,255,0))` over 2..3 and calls `getAttributes(b, 2)`; it printed `number Foreground 4278255360` (the green, higher-start span) and at index 0 `number Foreground 4294901760` (the red one). The same run confirmed the rest of the sentence: `fontSize(10)` then `fontSize(20)` both over 0..10 resolved to `FontSize 20` (later insertion wins a tie), and `font("Mono")` from 1 beat `font("Serif")` from 0 at index 2. Foreground and background are simply never mentioned on the page, so a reader does not learn that two overlapping colours resolve the same way.
SUGGESTED: "flags are present when any covering span carries them; font, font size, foreground and background each take the winning span's value."

### 2. The example declares the styled string MUT although nothing mutates it
UNIT:      man-page:astrings/getAttributes
CLAIM:     "MUT b AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())"
VERDICT:   misleading
EVIDENCE:  The example compiles and runs: built with `mfb build -q ex` from `/tmp/plan-125-scratch/D-phase6-current/man-page-astrings-getAttributes/ex` and printed exactly `flag`, exit 0. But `b` is never reassigned, and `addAttribute` returns a new value (`func_add_attribute.rs` DESC: "returns a new `AttributedString`"; the sibling example there uses `LET styled`). A reader learning the package sees `MUT` on the one binding that holds the styled text and is nudged toward reading `addAttribute`/`getAttributes` as in-place operations, which is the mutation-versus-new-value sharp edge the standard asks pages to make explicit. `LET` compiles identically (the `neg`/`past`/`empty` probes use `LET` with the same calls).
SUGGESTED: "LET b AS AttributedString = astrings::addAttribute(a, 0, 4, astrings::bold())"

Everything else on the page checked out against the binary. Scalar indexing is real: "héllo wörld" reports 11 scalars (13 bytes) and index 10 resolves without raising. An uncovered scalar returns an empty list (the `plain at 1` case printed nothing). Winners come back in insertion order of the winning spans, including after a `removeAttribute` split. Negative, past-the-end and empty-text indexes all raised `7-705-0001` with message `attribute index out of bounds`, matching the Errors table and the description. No banned vocabulary and no compiler-model sentences were found.
