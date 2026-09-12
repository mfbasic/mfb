### 1. Parameter range and saturation are underspecified
UNIT:      man-page:color/fromLinear
CLAIM:     "The linear-light value, `0`..`65535`. Values outside that range saturate to `0` or `255` rather than raising."
VERDICT:   incomplete
EVIDENCE:  Probe built and ran at `/tmp/plan-125-scratch/A-iter2/man-page-color-fromLinear`: `-1=0`, `0=0`, `65535=255`, `65536=255`. The description does not state that the range is inclusive or associate low and high out-of-range values with their respective results.
SUGGESTED: An integer in the inclusive range `0`..`65535`. Values at or below `0` yield `0`; values at or above `65535` yield `255`.

### 2. Nearest-channel tie rule is omitted
UNIT:      man-page:color/fromLinear
CLAIM:     "`fromLinear` is the inverse of `color::toLinear`: it returns the sRGB channel byte whose linear value is nearest to `value`."
VERDICT:   incomplete
EVIDENCE:  The probe printed `10=0` and `11=1`; `src/codegen/builtins/color/helper_srgb.rs:SRGB_TABLE` has adjacent initial values `0` and `20`, making `10` exactly equidistant. `src/codegen/builtins/color/func_from_linear.rs:BODY` uses `>` at the midpoint, selecting the lower channel on ties.
SUGGESTED: `fromLinear` returns the sRGB channel whose linear value is nearest to `value`; when two channels are equally near, it returns the lower channel.

### 3. Canvas implementation detail is out of scope
UNIT:      man-page:color/fromLinear
CLAIM:     "Together they are the seam every perceptual operation in `color` is built on, and the one the canvas software rasteriser blends through."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/color/func_from_linear.rs:DESC` is registry prose. This describes package construction and a renderer implementation, not how a terminal user calls `fromLinear`.
SUGGESTED: Omit this sentence.

### 4. Search/table implementation detail is out of scope
UNIT:      man-page:color/fromLinear
CLAIM:     "The answer is found by binary search over the same 256-entry table `toLinear` reads — eight comparisons, and exactly as deterministic as a lookup. A reverse table would need 65536 entries to say the same thing."
VERDICT:   out-of-scope
EVIDENCE:  `src/codegen/builtins/color/func_from_linear.rs:BODY` implements this search, but none of these details change the call contract or result for an MFBASIC developer.
SUGGESTED: Omit this paragraph.