### 1. Incorrect channel-count claim
UNIT:      man-page:color/isDark
CLAIM:     "The threshold is on relative luminance, so it accounts for the eye's uneven channel sensitivity — a saturated blue at full strength is dark, a saturated yellow is not, even though both have two channels at 255."
VERDICT:   wrong
EVIDENCE:  Probe printed `blue   TRUE` and `yellow FALSE`; `color::rgb(0, 0, 255)` has one channel at 255, while `color::rgb(255, 255, 0)` has two. The behavior follows `src/codegen/builtins/color/func_is_dark.rs:BODY`, which tests `color::luminance(base) < 0.5`.
SUGGESTED: "The threshold is on relative luminance, so it accounts for the eye's uneven channel sensitivity — a saturated blue at full strength is dark, while a saturated yellow is not, even though both have a channel at 255."

### 2. Example overpromises readable text
UNIT:      man-page:color/isDark
CLAIM:     "Pick readable text for a background:"
VERDICT:   misleading
EVIDENCE:  The example’s choice is based solely on `color::isDark`. A probe with the same selection rule for `color::gray(187)` printed `near-dark TRUE contrast-with-white 1.92`; white text therefore fails the page’s own cited 4.5 body-text threshold. `src/codegen/builtins/color/func_is_dark.rs:BODY` confirms the predicate is only luminance `< 0.5`.
SUGGESTED: "Pick white text for a dark `#222222` background:"