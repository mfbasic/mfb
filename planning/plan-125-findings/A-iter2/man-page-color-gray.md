### 1. Clamp bounds are not explicit

UNIT:      man-page:color/gray  
CLAIM:     "The level every channel takes, clamped to 0..255."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_byte.rs:BODY` returns `0` for values below `0`, `255` above `255`, and preserves the endpoints. Probe `gray(-1)`, `gray(256)`, `gray(0)`, and `gray(255)` printed `0 255 255 255` for low red, high red, low alpha, high alpha. The page never states that both bounds are inclusive or what negative values do.  
SUGGESTED: "The level every channel takes. Values at or below 0 become 0, and values at or above 255 become 255."

### 2. “Midpoint” is not exact for byte channels

UNIT:      man-page:color/gray  
CLAIM:     "`color::gray(128)` is the midpoint by channel value."  
VERDICT:   misleading  
EVIDENCE:  `src/codegen/builtins/color/func_gray.rs:BODY` passes `level` unchanged to all RGB channels; `__color_clampByte` admits the inclusive integer range 0 through 255. Its mathematical midpoint is 127.5, so 127 and 128 are the two central channel values; 128 alone is not the exact midpoint. The example probe printed `128 128 128` for `gray(128)`.  
SUGGESTED: "`color::gray(127)` and `color::gray(128)` are the two central channel values."

### 3. Half-bright recipe omits the required amount

UNIT:      man-page:color/gray  
CLAIM:     "For a grey that is half as bright to the eye, use `color::darken` on white, or pick the level by `color::luminance`."  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/func_darken.rs:BODY` requires an `amount` argument and removes that fraction of linear light; `func_luminance.rs:BODY` only measures a color and cannot choose a level. Probe `color::darken(color::gray(255), 0.5)` printed `188 0.22 0.50`, showing the required `0.5` amount and the contrast with `gray(128)`’s 0.22 luminance.  
SUGGESTED: "For a neutral grey with half of white’s relative luminance, use `color::darken(color::gray(255), 0.5)`; it has channel value 188."