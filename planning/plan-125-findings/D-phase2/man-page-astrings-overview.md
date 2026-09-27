### 1. `replace` loses style on inserted text
UNIT:      man-page:astrings/overview  
CLAIM:     “The ones that change the text — `left`, `right`, `mid`, the `trim` family, `stripPrefix`/`stripSuffix`, `padLeft`/`padRight`, `repeat`, and `replace` — return an `AttributedString` whose styling follows the text it was attached to.”  
VERDICT:   incomplete  
EVIDENCE:  Probe `/tmp/plan-125-scratch/D-phase2/man-page-astrings-overview/src/main.mfb` compiled and ran with the specified release binary. Replacing `b` in bold `"abc"` with `"X"` printed `**a**X**c**`; the inserted `X` has no style. `src/codegen/builtins/astrings/helper_replace.rs:BODY` remaps only retained original segments.  
SUGGESTED:  “These functions return an `AttributedString` whose styling follows retained text. For `replace`, inserted replacement text has no attributes.”

### 2. Overview has no runnable example
UNIT:      man-page:astrings/overview  
CLAIM:     `astrings/overview` renders no `Examples` section.  
VERDICT:   incomplete  
EVIDENCE:  `/Users/justinzaun/Development/mfb/.claude/worktrees/P-125/target/release/mfb man astrings` rendered the overview through “See also” with no example. The verified probe printed `ab`, `[]`, `**a**b`, `ab`, and `**a**X**c**`, confirming construction, defaulting, copying, attribute application, clearing, and replacement behavior are runnable.  
SUGGESTED:  Add a short example that constructs text with `fromString`, applies `bold` over an inclusive scalar range, and prints `astrings::toMarkdown` output.