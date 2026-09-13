### 1. Inversion is presented as a way to find contrast
UNIT:      man-page:color/invert
CLAIM:     "would make `invert` unusable for the thing it is for — finding a contrasting colour for the same mark."
VERDICT:   misleading
EVIDENCE:  The probe in `/tmp/plan-125-scratch/A-iter2/man-page-color-invert/src/main.mfb` printed `127 127 127 alpha 0` for inversion of RGB 128 grey, then `1.01` for `color::contrastRatio(rgb(128,128,128), invert(rgb(128,128,128)))`. `src/codegen/builtins/color/func_invert.rs:52` performs only `255 -` on each RGB channel.
SUGGESTED: Flipping transparency would also change how much of the same mark shows. Use `color::contrastRatio` when you need to measure readable contrast.

### 2. The page omits whether the input is changed
UNIT:      man-page:color/invert
CLAIM:     "`invert` replaces each of red, green and blue with `255 -` that channel and returns `base`'s `alpha` unchanged."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_invert.rs:52` constructs and returns a `Color` from `base`’s fields. The probe printed `245 55 225 alpha 128` after inversion, followed by `original 10 200 30 alpha 128`; the supplied colour was unchanged.
SUGGESTED: `invert` returns a new `color::Color`; `base` is unchanged. It replaces each of red, green and blue with `255 -` that channel and keeps `base`’s alpha.