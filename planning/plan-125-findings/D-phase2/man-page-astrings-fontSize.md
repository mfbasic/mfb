### 1. Size range is undocumented
UNIT:      man-page:astrings/fontSize
CLAIM:     “The font size.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/func_font_size.rs:__astrings_fontSize` returns `AttrNumber[AttrTypeNumber.FontSize, size]` without validation. The compiled scratch probe printed `zero=0` and `negative=-7`.
SUGGESTED: The font size in points. Any Integer is accepted, including zero and negative values.

### 2. Equal-start overlap rule is omitted
UNIT:      man-page:astrings/fontSize
CLAIM:     “Font size is an Integer-valued attribute: overlapping font-size spans resolve by higher-start-wins at read time.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/func_get_attributes.rs:__astrings_getAttributes` delegates winner selection to `__astrings_isWinner`; its documented contract states equal starts break to the later insertion. The compiled scratch probe printed `overlap=20` and `tie=20` after adding size 10 and then size 20 over identical `0..4` spans.
SUGGESTED: Font size is an Integer-valued attribute: overlapping font-size spans resolve at read time by higher-start-wins; when starts are equal, the later-added span wins.