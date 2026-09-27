### 1. Empty font names undocumented
UNIT:      man-page:astrings/font
CLAIM:     "The font family name."
VERDICT:   incomplete
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase2/man-page-astrings-font/probe.mfb`, built and run with the specified release binary, printed `empty=font=`: `astrings::font("")` is accepted and records an empty name. `src/codegen/builtins/astrings/func_font.rs:__astrings_font` returns `AttrText[AttrTypeText.Font, name]` unchanged.
SUGGESTED: The font-family string to record. It may be empty and is stored unchanged.

### 2. Equal-start overlap rule omitted
UNIT:      man-page:astrings/font
CLAIM:     "Font is a String-valued attribute: overlapping font spans resolve by higher-start-wins at read time."
VERDICT:   incomplete
EVIDENCE:  The same probe printed `tie=font=Later` for two font spans beginning at index 0, and `higher-start=font=Inner` for nested spans. `src/codegen/builtins/astrings/helper_is_winner.rs:__astrings_isWinner` selects the higher `start`, then selects the higher `seq` for equal starts—i.e. the later-added span.
SUGGESTED: Font is a String-valued attribute: at a scalar covered by multiple font spans, the span with the highest start wins; for equal starts, the later-added span wins.