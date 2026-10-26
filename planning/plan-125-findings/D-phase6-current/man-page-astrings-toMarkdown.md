### 1. Font-name `-` escaping claim overstates what is escaped
UNIT:      man-page:astrings/toMarkdown
CLAIM:     "Delimiter characters (`\ * _ ~ ^ :`) in the visible text are backslash-escaped; font names additionally escape `;` (and a literal `-`)."
VERDICT:   misleading
EVIDENCE:  `helper_md_escape_font.rs:__astrings_mdEscapeFont` escapes `-` only when the whole name equals `-`. Probe (`probe/src/main.mfb`, font `"a;b-c*d"` on 0..2 and font `"-"` on 3..4 over `"a*b_c~d^e:f\g;h-i"`) printed `::a\;b-c\*d::a\*b::\-::\_c::-::\~d\^e\:f\\g;h-i`: the `-` inside `a;b-c*d` is not escaped, only the bare `-` name becomes `\-`. The `;` and `-` in visible text are also left alone, which the sentence implies but does not state.
SUGGESTED: "Delimiter characters (`\ * _ ~ ^ :`) in the visible text are backslash-escaped; `;` and `-` in the text are not. In a font name `;` is escaped as well, and a font whose whole name is `-` is written `\-` so it is not read as a reset; a `-` inside a longer name is left as is."

### 2. An empty font name renders as the "unchanged" slot
UNIT:      man-page:astrings/toMarkdown
CLAIM:     "a value sets, `-` resets to default, and an omitted slot leaves it unchanged (`::font::` font-only, `::;size::` size-only, `::-::` font reset)."
VERDICT:   incomplete
EVIDENCE:  `astrings::font` accepts an empty name ("stored as given; it may be empty", `func_font.rs`). Probe: `font("")` on 0..1 of `"abcd"` printed `::::ab::-::cd`. By the page's own grammar `::::` is "both slots omitted, nothing changed", yet it is actually a font switch to the empty name, followed by a real `::-::` reset. The page gives the reader a grammar that cannot distinguish the two.
SUGGESTED: Add: "A font set to the empty name is written with an empty slot (`::::`, or `::;12::` together with a size), which reads the same as no change; avoid empty font names if the output must be read back."

### 3. No closing marker at the end of the text
UNIT:      man-page:astrings/toMarkdown
CLAIM:     "Font/size switch forward via a minimal-delta `::font;size::` marker emitted at run boundaries where the state changes"
VERDICT:   incomplete
EVIDENCE:  `func_to_markdown.rs:BODY` emits a marker only before a run; the end of the text is not a boundary. Probe: `font("Serif")` on 0..3 of `"abcd"` printed `::Serif::abcd`, with no trailing `::-::`. Flags, by contrast, always close (`**hello** world` from the example). A reader concatenating two rendered strings will carry the first string's font into the second.
SUGGESTED: Add after the sentence: "Flags always close at the end of their run, but font and size are never reset at the end of the text; the last marker stays in force."

### 4. Scalar granularity can split a grapheme
UNIT:      man-page:astrings/toMarkdown
CLAIM:     "`toMarkdown` flattens the resolved attribute state … across the scalars into maximal runs"
VERDICT:   incomplete
EVIDENCE:  Probe: `"e\u0301x"` (e plus combining acute) with `bold()` on scalar 0 only printed `**e**́x`, closing `**` between the base letter and its combining mark. The page says "scalars" once in a dense sentence and never states the consequence; §2 of `.ai/man-content.md` requires the scalar-vs-grapheme edge to be stated explicitly.
SUGGESTED: Add: "Runs are cut at Unicode scalar boundaries, the same unit `addAttribute` ranges use, so a marker can land between a base letter and its combining mark."

### 5. First sentence requires the data-model vocabulary to parse
UNIT:      man-page:astrings/toMarkdown
CLAIM:     "`toMarkdown` flattens the resolved attribute state — per member, the covering span with the higher start, or the later-added one where starts are equal — across the scalars into maximal runs and renders each run into a bespoke marker vocabulary."
VERDICT:   misleading
EVIDENCE:  The winner rule is only verified true for font and size (probe 2: `::Arial::ab::Mono::c::;12::de::Arial::f::;-::gh`; probe 3: `::Second::ab::First::cd`). For flags it is irrelevant: `func_bold.rs` DESC says a scalar is bold "when any covering span carries the bold flag", and the probe confirms overlapping bold/italic simply nest. The sentence also duplicates the rule that `addAttribute` already owns ("spans of the same attribute member overlap, the one with the higher start wins", `mfb man astrings addAttribute`), and "resolved attribute state", "projection" and "maximal runs" are not vocabulary the rest of the package uses. "member" is defined on `mfb man astrings types`, so it is not out of scope, but the sentence makes the reader do the resolution themselves.
SUGGESTED: "`toMarkdown` walks the text scalar by scalar, groups neighbouring scalars that carry the same styling into one run, and writes each run with markers. A scalar is bold, italic, underlined, struck or overlined when any covering span says so; its font and size come from the winning span as `astrings::addAttribute` defines it. Foreground and background colours are ignored. `value` is not changed."

### 6. `::;-::` size reset is used by the binary but not named by the page
UNIT:      man-page:astrings/toMarkdown
CLAIM:     "(`::font::` font-only, `::;size::` size-only, `::-::` font reset)"
VERDICT:   incomplete
EVIDENCE:  Probe 2 ended with `::;-::gh` and probe 8 printed `::;-::e` when the size span ended while the font was unchanged. The list names the font reset form but not the size reset form, and a negative size renders as `::;-5::` (probe 8), so the reader needs to know `-` alone is the reset while `-5` is a value.
SUGGESTED: "(`::font::` font-only, `::;size::` size-only, `::-::` font reset, `::;-::` size reset; a negative size such as `::;-5::` is a value, not a reset)."

Example verified: `ex/src/main.mfb` built with `mfb build -q ex` and printed `**hello** world`, matching the page. Empty text printed `[]` as the parameter description says, and a foreground colour span rendered as plain `abc`, confirming "colors are left out". Calling `toMarkdown` twice on the same value gave identical output and `toString` still returned the bare text, confirming "value is not modified". No banned memory vocabulary found on the page.
