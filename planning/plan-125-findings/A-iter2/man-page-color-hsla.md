### 1. Hue range falsely says all values work
UNIT:      man-page:color/hsla
CLAIM:     "The hue in degrees. Wraps, so any value is valid."
VERDICT:   wrong
EVIDENCE:  `/tmp/plan-125-scratch/A-iter2/man-page-color-hsla/run-large/src/main.mfb` calls `color::hsla(1e300, 1.0, 0.5, 255)`; it builds, then prints `Error: 7-705-0010 Arithmetic overflow or numeric conversion outside the destination range.` `src/codegen/builtins/color/helper_hsl.rs:__color_wrapHue` calls `math::floor(hue / 360.0)`, and `mfb man math floor` states that a magnitude too large for `Integer` raises `ErrOverflow`.
SUGGESTED: The hue in degrees. It wraps by full turns; a magnitude whose complete-turn count is too large for an `Integer` raises `ErrOverflow`. Add that error to the descriptor so the rendered Errors table reports it.

### 2. Saturation bounds do not state inclusivity
UNIT:      man-page:color/hsla
CLAIM:     "How colourful, clamped to `0.0` (grey) .. `1.0` (full)."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_fraction.rs:__color_clampFraction` uses strict comparisons (`amount < 0.0`, `amount > 1.0`), so both endpoints are retained. The boundary probe `/tmp/plan-125-scratch/A-iter2/man-page-color-hsla/run-edges/src/main.mfb` prints `#ffffff00` for saturation `-1.0` and lightness `2.0`, demonstrating clamping; the page never says whether `0.0` and `1.0` themselves are included.
SUGGESTED: How colourful, clamped inclusively to `0.0` (grey) through `1.0` (full).

### 3. Lightness bounds do not state inclusivity
UNIT:      man-page:color/hsla
CLAIM:     "How light, clamped to `0.0` (black) .. `1.0` (white)."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_fraction.rs:__color_clampFraction` leaves exactly `0.0` and `1.0` unchanged because it clamps only values below or above them. The same boundary probe prints `#ffffff00` for a lightness input of `2.0`.
SUGGESTED: How light, clamped inclusively to `0.0` (black) through `1.0` (white).

### 4. Alpha bounds do not state inclusivity
UNIT:      man-page:color/hsla
CLAIM:     "The alpha component, clamped to `0`..`255`: `0` fully transparent, `255` fully opaque."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_byte.rs:__color_clampByte` uses strict comparisons (`value < 0`, `value > 255`), retaining `0` and `255`. The boundary probe prints `#ffffff00` for alpha `-1` and `#000000ff` for alpha `999`.
SUGGESTED: The alpha component, clamped inclusively to `0` through `255`: `0` is fully transparent and `255` is fully opaque.