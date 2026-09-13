### 1. `amount` bounds and out-of-range behavior are not explicit in its parameter description
UNIT:      man-page:color/mix
CLAIM:     “How far from first to second, clamped to 0.0..1.0.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_mix.rs:__color_mix` calls `__color_clampFraction`; probe `behaviour.mfb` printed `#00000000`, `#ffffffff`, `#bcbcbc7f` for amounts `-1.0`, `2.0`, and `0.5`.
SUGGESTED: “How far from `first` to `second`. Bounds are inclusive: amounts at or below `0.0` return `first`; amounts at or above `1.0` return `second`.”

### 2. The page omits that `mix` leaves both input colours unchanged
UNIT:      man-page:color/mix
CLAIM:     “mix interpolates between two colours: amount 0.0 returns first, 1.0 returns second, and 0.5 is the midpoint.”
VERDICT:   incomplete
EVIDENCE:  Probe `fraction-and-inputs.mfb` printed `#bcbcbc7f`, then the unchanged inputs `#00000000` and `#ffffffff`; `src/codegen/builtins/color/func_mix.rs:__color_mix` constructs and returns `Color[...]` from its parameters.
SUGGESTED: “`mix` returns a new colour and leaves both input colours unchanged.”

### 3. Fractional alpha and channel results truncate downward
UNIT:      man-page:color/mix
CLAIM:     “Alpha is interpolated on its raw value, not through the linear transfer — alpha is a coverage fraction, not a light intensity, and is not gamma-encoded.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_mix.rs:__color_mix` uses `toInt(toFloat(ba - aa) * t)`; probe `fraction-and-inputs.mfb` mixed alpha `0` and `255` at `0.5` and printed `#bcbcbc7f` (alpha `127`, not a rounded-up `128`).
SUGGESTED: “Alpha is interpolated on its raw value, then fractional results truncate toward the lower byte value; it is not gamma-encoded.”