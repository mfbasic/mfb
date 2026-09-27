### 1. Canvas renderer claim is false
UNIT:      man-page:astrings/foreground
CLAIM:     "The alpha is preserved in the attribute rather than dropped at construction, so a renderer that can model it — a canvas surface, say — still gets the whole colour."
VERDICT:   wrong
EVIDENCE:  `rg -n 'AttributedString' src/codegen/builtins/canvas -g '*.rs'` prints no matches; `src/codegen/builtins/canvas/mod.rs:RegistryRecord Text` declares `text AS String`, so canvas cannot receive an `AttributedString` or its foreground attribute.
SUGGESTED: "The alpha remains in the attribute, so code can recover the whole colour with `color::fromPacked`."

### 2. Equal-start overlap rule omitted
UNIT:      man-page:astrings/foreground
CLAIM:     "overlapping foreground spans resolve by higher-start-wins at read time."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/astrings/helper_is_winner.rs:__astrings_isWinner` selects the higher `seq` (later insertion) when starts are equal. The probe adding red over `0..1`, then blue over `0..0`, printed `#0000ffff` at scalar 0.
SUGGESTED: "Overlapping foreground spans resolve at read time by higher start; when starts are equal, the later-added span wins."