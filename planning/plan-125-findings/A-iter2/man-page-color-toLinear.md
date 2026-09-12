### 1. Parameter bounds are not explicit
UNIT:      man-page:color/toLinear
CLAIM:     “The sRGB channel to convert, `0`..`255`.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_to_linear.rs:register` declares `channel AS Byte`; the scratch probe built with the specified release binary and printed `round-trip mismatches 0` and `linear 128 14146`. The page’s required parameter description still does not say whether either displayed bound is inclusive.
SUGGESTED: The sRGB channel to convert, from `0` through `255`, inclusive.

### 2. “Every perceptual operation” is false
UNIT:      man-page:color/toLinear
CLAIM:     “This pair is the seam every perceptual operation in `color` is built on, and it is the same seam the canvas software rasteriser blends through, so a colour computed here and a pixel drawn there cannot disagree.”
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/color/func_saturate.rs:BODY` and `func_desaturate.rs:BODY` operate through `__color_colorToHsl` / `__color_hslToColor`; their descriptions explicitly say they work on sRGB rather than linear light. These are user-facing colour operations (“more vivid” and “fade … towards grey”), so “every perceptual operation” overstates the implementation.
SUGGESTED: Use `toLinear` before arithmetic that should operate on linear light, then convert the result with `color::fromLinear`.

### 3. Renderer/backend rationale is compiler-facing
UNIT:      man-page:color/toLinear
CLAIM:     “That is deliberate: the software rasteriser is the oracle the GPU backends are compared against, so it must produce identical bytes on every target, and a libm transcendental does not.”
VERDICT:   out-of-scope
EVIDENCE:  This requires the compiler-internal model prohibited by `.ai/man-content.md` §3: rasteriser oracle, GPU backends, cross-target comparison, and library implementation rationale. `src/codegen/builtins/color/helper_srgb.rs:SRGB_TABLE` confirms it is implementation commentary, not terminal-user guidance.
SUGGESTED: Remove this sentence.