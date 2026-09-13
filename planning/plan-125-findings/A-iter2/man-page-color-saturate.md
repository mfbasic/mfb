### 1. CSS comparison is false
UNIT:      man-page:color/saturate
CLAIM:     "This is the HSL model's own answer and it is what CSS and Sass saturate() do."
VERDICT:   wrong
EVIDENCE:  `src/codegen/builtins/color/func_saturate.rs:BODY` uses HSL conversion; the release-binary probe prints `#ff0101` for `color::saturate(color::gray(128), 1.0)`. CSS `saturate()` is an RGB filter matrix, not an HSL conversion, and does not preserve hue or lightness ([MDN](https://developer.mozilla.org/en-US/docs/Web/CSS/Reference/Values/filter-function/saturate)).
SUGGESTED: This is the HSL model's answer: a grey reported with hue `0.0` becomes red when fully saturated.

### 2. Clamp bounds and negative behavior are not explicit
UNIT:      man-page:color/saturate
CLAIM:     "How far towards fully saturated, clamped to `0.0`..`1.0`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_fraction.rs:__color_clampFraction` returns `0.0` when `amount < 0.0` and `1.0` when `amount > 1.0`; the release-binary probe printed `#4d80b3` for `-1.0` and `#0180ff` for `2.0`, matching the `0.0` and `1.0` endpoints.
SUGGESTED: How far towards fully saturated. `0.0` and `1.0` are inclusive; values below `0.0` act as `0.0`, and values above `1.0` act as `1.0`.