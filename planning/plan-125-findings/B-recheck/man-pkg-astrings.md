### 1. No page says how to measure an AttributedString, which every range call needs
UNIT:      man-pkg:astrings
PAGE:      package-wide (overview, addAttribute, removeAttribute, clearAttributes, getAttributes)
CATEGORY:  coverage
CLAIM:     Every range/index page requires an index "inside the visible text" (addAttribute: "both ends must fall inside the visible text"; getAttributes: "it must identify a scalar that exists in the visible text"), yet no page in the unit says how to find out how many scalars the text has.
VERDICT:   missing
EVIDENCE:  Probe `len(a)` on an AttributedString fails to compile: `mfb build proj_probe` → `error[2-203-0021 TYPE_CALL_ARGUMENT_MISMATCH]`. `len(strings::toScalars(a))` compiles and prints `11` for "hello world" (p10_overview). `strings::graphemesCount` counts a different unit: for "e\u0301x", `len(strings::toScalars(a))` prints `3` while `strings::graphemesCount(a)` prints `2` (p6_scalars). `grep -n -i 'length\|toScalars\|graphemesCount' all.txt` shows toScalars/graphemesCount only inside the overview's list of readers, never as the way to size a range. The overview itself notes the scalar/grapheme trap ("a character written with a combining mark spans two indices") but gives no counting function.
SUGGESTED: In the overview, right after the strings:: overload paragraph, add: "Ranges and indices count Unicode scalars, not graphemes. The last valid index is `len(strings::toScalars(a)) - 1`; `len(a)` is not accepted, and `strings::graphemesCount` counts graphemes, a different unit." Repeat the one-line formula in addAttribute's `endIndex` description.

### 2. The overview never says where the styling is actually drawn
UNIT:      man-pkg:astrings
PAGE:      overview
CATEGORY:  discoverability
CLAIM:     "Reach the visible text with toString(a); io::print/io::write emit it." — the overview's only statement about output. A developer arriving with "how do I display bold/colored text?" is sent to io::print, which drops the styling, and is never told about term::drawText, the one renderer that honours flags and colours.
VERDICT:   missing
EVIDENCE:  `grep -n 'term' all.txt` — term::drawText appears only on the background and foreground pages ("When such an AttributedString is drawn with term::drawText…"), not on the overview, not in its See also. `mfb man term drawText` line 66: "An overload accepts an AttributedString in the text position." Also the overview says only "Many strings:: functions also accept an AttributedString", but `grep -rl AttributedString src/codegen/builtins | grep -v '/astrings/\|/strings/'` shows regex::count, regex::split and term::measureText also accept one (regex/func_count.rs:76 "value may also be an AttributedString"; term/func_measure_text.rs:30), and probe `regex::count(a, "l")` printed `3` (p5_md).
SUGGESTED: Add to the overview, after the toString sentence: "The styling is drawn by term::drawText (see mfb man term drawText); io::print and toMarkdown do not draw colour. regex::count, regex::split and term::measureText also accept an AttributedString at the text position." This also puts term::drawText and the regex pages into the derived See also.

### 3. toMarkdown promises a reader that does not exist
UNIT:      man-pkg:astrings
PAGE:      toMarkdown
CATEGORY:  overview-mismatch
CLAIM:     "The format is read by the astrings toolchain, not a standard markdown engine; do not treat __/^^/::…:: as CommonMark."
VERDICT:   misleading
EVIDENCE:  `grep -rn -i 'frommarkdown\|parse_markdown\|md_parse' src/` → no matches; the function table lists no reader; `grep -rn -i markdown src/codegen/builtins -l | grep -v astrings` hits only test files. Nothing in the language reads this format back, so "read by the astrings toolchain" sends the developer looking for a parse function the unit does not have. The bullet about empty font names ("if the output must be read back") compounds it.
SUGGESTED: "This is not CommonMark and nothing in MFBASIC reads it back; it is a debugging/display rendering only. Do not feed __/^^/::…:: to a markdown engine." Drop "if the output must be read back" or reword to "if a reader of your own must distinguish the two".

### 4. removeAttribute does not point at clearAttributes
UNIT:      man-pkg:astrings
PAGE:      removeAttribute
CATEGORY:  discoverability
CLAIM:     The page explains structural matching at length but never names clearAttributes, the sibling a developer reaches for by mistake when they want every attribute gone over a range; its page has no See also at all.
VERDICT:   missing
EVIDENCE:  Rendered page in all.txt (REMOVEATTRIBUTE section) ends at Examples with no See also; addAttribute and clearAttributes both cross-reference removeAttribute ("unlike removeAttribute, which clears only the one attribute you name"), so the link is one-way.
SUGGESTED: Append to the first paragraph: "To drop every attribute over a range regardless of which it is, use astrings::clearAttributes; to add one back, astrings::addAttribute." The derived See also then appears.

What I checked and found consistent: the overview's reader/changer lists match the strings:: descriptors exactly (`grep -l AttributedString src/codegen/builtins/strings/func_*.rs` → the same 14 readers and 16 changers); the overview example prints exactly what the page shows, and repeat/upper behave as stated (p10_overview); `a & "y"` is a compile error and `a = b` raises TYPE_REQUIRES_COMPARABLE as the overview says; empty-text getAttributes raises "attribute index out of bounds" as the addAttribute/getAttributes/clearAttributes pages consistently state; readSpans/writeSpans/scalarLen are `internal_only` and correctly absent from the function table; the three "higher start wins, later insertion breaks ties" statements are worded identically across addAttribute, the five flag pages, font, fontSize, foreground and background.
