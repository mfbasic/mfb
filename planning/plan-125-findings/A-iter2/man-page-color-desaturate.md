### 1. `amount` boundary behavior is not explicit
UNIT:      man-page:color/desaturate  
CLAIM:     “How far towards grey, clamped to `0.0`..`1.0`.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_fraction.rs:BODY` maps values below `0.0` to `0.0` and above `1.0` to `1.0`. The compiled probe printed `#ff0000` for `amount = -0.01` and `#808080` for `amount = 1.01`. The description never states that the bounds are inclusive or what negative and over-range inputs produce.  
SUGGESTED: “How far towards grey. The inclusive range is `0.0`..`1.0`; values below `0.0` act as `0.0`, and values above `1.0` act as `1.0`.”

### 2. The page omits whether the input colour changes
UNIT:      man-page:color/desaturate  
CLAIM:     “`desaturate` moves the colour's saturation a fraction of the way towards zero, keeping its hue and lightness.”  
VERDICT:   incomplete  
EVIDENCE:  `src/codegen/builtins/color/func_desaturate.rs:BODY` constructs and returns `__color_hslToColor(...)` from `base`; it does not update `base`. The compiled probe desaturated `#3366cc11` at `2.0`, then printed the original value as `#3366cc11`, unchanged. The page does not state this mutation-versus-new-value sharp edge.  
SUGGESTED: “Returns a new colour whose HSL saturation is moved a fraction of the way towards zero; `base` is unchanged.”