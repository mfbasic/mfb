### 1. AttrNumber description omits two supported uses
UNIT:      man-page:astrings/types
CLAIM:     “AttrNumber — An Integer-valued attribute (font size).”
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/astrings/mod.rs:RegistryUnion Attribute` describes `AttrNumber` only as font size, while `RegistryEnum AttrTypeNumber` registers `FontSize`, `Foreground`, and `Background`. The probe at `/tmp/plan-125-scratch/D-phase2/man-page-astrings-types/probe-project/src/main.mfb` compiled and ran; its `foreground(color::rgba(18, 52, 86, 120))` attribute produced `2014458966` (`0x78123456`), confirming the colour-valued use.
SUGGESTED: “AttrNumber — An Integer-valued attribute (font size or packed foreground/background colour).”

### 2. Font-size value range is undocumented
UNIT:      man-page:astrings/types
CLAIM:     “FontSize — The font size in points.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/func_font_size.rs:__astrings_fontSize` directly constructs `AttrNumber[AttrTypeNumber.FontSize, size]` with no range check. The probe compiled and ran with `astrings::fontSize(0)` and `astrings::fontSize(-7)`; it printed `::;0::x`, `::;-7::x`, `0`, and `-7`, proving zero and negative values are accepted and preserved.
SUGGESTED: “The font size in points. Any Integer value, including zero and negative values, is accepted and preserved.”