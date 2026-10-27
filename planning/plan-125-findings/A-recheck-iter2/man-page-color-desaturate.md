### 1. "keeping its hue" is only approximately true; near-grey colours shift hue by up to 10°
UNIT:      man-page:color/desaturate
CLAIM:     "desaturate moves the colour's saturation a fraction of the way towards zero, keeping its hue and lightness."
VERDICT:   misleading
EVIDENCE:  Lowering `__color_desaturate` (src/codegen/builtins/color/func_desaturate.rs:BODY) round-trips through `__color_colorToHsl`/`__color_hslToColor` (helper_hsl.rs), and `__color_fractionChannel` rounds each channel to a Byte, so hue and lightness are re-quantised. Probe `p_drift` (sweep of every colour at step 3 per channel, `desaturate(c, 0.5)`, compare `toHsl` before and after) printed:
           `max hue drift deg 10.00 worst #8d8a90->#8d8c8f 270.00->260.00`
           `max lightness drift 0.00 colours with lightness changed 250044`
           The page's own example colour drifts too: probe `p_edges` printed `hue 220.00 -> 219.74` for `desaturate(rgba(51,102,204,128), 0.5)`. Lightness moves by less than a 1/255 step but does move for 250,044 of 614,125 sampled colours. The claim "returns the colour unchanged" for amount 0.0 IS exact: probe `p_sweep` over all 16,777,216 colours printed `changed-at-0.0 0`.
SUGGESTED: "`desaturate` moves the colour's saturation a fraction of the way towards zero while keeping its hue and lightness. Because the result is rounded back to 8-bit channels, the hue of a nearly-grey colour can shift by several degrees and the lightness by up to one channel step; `amount` `0.0` always returns the colour exactly unchanged and `1.0` returns a neutral grey."

### 2. Mutation versus new value is never stated
UNIT:      man-page:color/desaturate
CLAIM:     "amount 0.0 returns the colour unchanged and 1.0 returns a neutral grey." (the only sentence that hints the result is a return value)
VERDICT:   incomplete
EVIDENCE:  `.ai/man-content.md` §2 lists "mutation vs. new value" as a sharp edge every page must state explicitly. The body returns a freshly built `Color[...]` and never writes to `base` (helper_hsl.rs:`__color_hslToColor`). Probe `p_edges` printed `base after: #3366cc80` after three `desaturate` calls on the same binding, confirming `base` is untouched. The page nowhere says so; a reader of this page alone cannot tell whether `vivid` in the example has been changed after the three calls.
SUGGESTED: Add to the description: "The result is a new colour value; `base` is not changed."

No other findings. Both examples compile and run with the P-125 release `mfb` (projects `p_ex1`, `p_ex2` under the scratch directory). Example 1 printed `#3366cc`, `#5973a6`, `#808080`, matching "unchanged / halfway / neutral grey". Example 2 printed `desaturate #808080` and `grayscale  #4c4c4c`, which confirms "for a pure blue the two differ substantially" and that `desaturate(c, 1.0)` lands on HSL lightness 0.5. Probe `p_edges` also confirmed the clamp claims: `amount -0.5` gave `#3366cc80` (same as 0.0), `amount 7.0` gave `#80808080` (same as 1.0), and alpha 128 was carried through as `80` in every case, matching `toInt(base.alpha)` in the body. A NaN or infinite `amount` cannot reach the function: the language traps non-finite floats at the argument boundary (spec `04_types.md` §Float, errors 7-705-0013 / 7-705-0015 seen in the probe), so the empty Errors table is correct. The page uses no banned memory vocabulary and no compiler-internal terms.
