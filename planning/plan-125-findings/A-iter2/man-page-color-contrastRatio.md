### 1. `mix` does not composite transparency
UNIT:      man-page:color/contrastRatio
CLAIM:     "`alpha` is ignored on both sides, because `luminance` ignores it. A contrast ratio involving a transparent colour is not meaningful until it has been composited over something; do that first, with `color::mix`."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/color/func_mix.rs:BODY` interpolates RGB channels and alpha independently; it does not composite a foreground over a background. The probe at `/tmp/plan-125-scratch/A-iter2/man-page-color-contrastRatio/src/main.mfb` printed `#bcbcbc7f` for `mix(rgba(0,0,0,0), white, 0.5)`, proving the result remains half-transparent rather than becoming a colour composited over white.
SUGGESTED: "`alpha` is ignored on both sides, because `luminance` ignores it. Calculate contrast from the opaque colours actually rendered; `color::mix` interpolates two colours and their alpha, rather than compositing one over the other."

### 2. A 1.0 luminance ratio does not make text invisible
UNIT:      man-page:color/contrastRatio
CLAIM:     "The range is `1.0` (the two colours are equally bright — the text is invisible) to `21.0` (black on white, or white on black)."
VERDICT:   misleading
EVIDENCE:  `src/codegen/builtins/color/func_contrast_ratio.rs:BODY` uses relative luminance only. The probe printed equal luminance (`0.01 0.01`) and ratio `1.00` for visibly different RGB colours `rgb(45,29,0)` and `rgb(1,0,123)`; their exact weighted luminance numerator is equal in `src/codegen/builtins/color/func_luminance.rs:BODY`. A 1.0 result means no luminance contrast, not that differently hued text is necessarily invisible.
SUGGESTED: "The range is `1.0` (the colours have equal relative luminance) to `21.0` (black on white, or white on black). A ratio of `1.0` provides no luminance contrast."

### 3. WCAG UI threshold omits material qualifiers
UNIT:      man-page:color/contrastRatio
CLAIM:     "The WCAG thresholds worth remembering: **4.5** for body text, **3.0** for large text and for user-interface components."
VERDICT:   incomplete
EVIDENCE:  [WCAG 2.2 SC 1.4.3](https://www.w3.org/TR/WCAG22/#contrast-minimum) specifies 4.5:1 for normal text and 3:1 for large text; its non-text criterion applies 3:1 to visual information required to identify a component or state, with exceptions such as inactive components. The rendered sentence presents 3.0 as applying to every UI component.
SUGGESTED: "For WCAG AA, use 4.5 for normal text and 3.0 for large text. Use 3.0 for the visual information needed to identify a user-interface component or its state; WCAG has exceptions, including inactive components."