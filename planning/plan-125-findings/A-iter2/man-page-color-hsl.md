### 1. Saturation bound is not explicitly inclusive
UNIT:      man-page:color/hsl  
CLAIM:     “How colourful, clamped to `0.0` (grey) .. `1.0` (full).”  
VERDICT:   incomplete  
EVIDENCE:  `__color_clampFraction` in `src/codegen/builtins/color/helper_clamp_fraction.rs` clamps only values `< 0.0` or `> 1.0`, so both bounds are inclusive. The scratch probe built and ran with the specified binary; saturation `-1.0` printed `#808080` and `2.0` printed `#ff8000`.  
SUGGESTED: “How colourful. Clamped inclusively: values at or below `0.0` are grey; values at or above `1.0` have full saturation.”

### 2. Lightness bound is not explicitly inclusive
UNIT:      man-page:color/hsl  
CLAIM:     “How light, clamped to `0.0` (black) .. `1.0` (white).”  
VERDICT:   incomplete  
EVIDENCE:  `__color_clampFraction` in `src/codegen/builtins/color/helper_clamp_fraction.rs` preserves exactly `0.0` and `1.0` and clamps only outside them. The scratch probe built and ran with the specified binary; lightness `-1.0` printed `#000000` and `2.0` printed `#ffffff`.  
SUGGESTED: “How light. Clamped inclusively: values at or below `0.0` are black; values at or above `1.0` are white.”

### 3. The 8-bit rounding edge is omitted
UNIT:      man-page:color/hsl  
CLAIM:     “`hsl` builds a `color::Color` from the HSL model: `hue` in degrees around the colour wheel, `saturation` from grey to full colour, and `lightness` from black through the pure hue to white.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/mod.rs:COLOR_TYPE` defines `Color` as Byte channels; `__color_fractionChannel` in `src/codegen/builtins/color/helper_hsl.rs` computes `toInt(value * 255.0 + 0.5)`, rounding components to 8-bit channels. The scratch probe built and ran with the specified binary; `color::toHex(color::hsl(0.0, 1.0, 0.501))` printed `#ff0101`.  
SUGGESTED: “The returned `color::Color` has 8-bit channels, so the converted red, green and blue components are rounded to the nearest channel value.”