### 1. Encoded-byte scaling example states the opposite physical effect

UNIT:      man-page:color/brighten  
CLAIM:     “the same multiplier lifts a dark colour far more than a light one”  
VERDICT:   wrong  
EVIDENCE:  `/tmp/plan-125-scratch/A-iter2/man-page-color-brighten/probe-project/src/main.mfb` printed linear-light increases of `4306` for encoded `64 → 96` and `34067` for `160 → 240`, both a `×1.5` encoded-byte scaling. The lighter colour gains substantially more linear light.  
SUGGESTED: Scaling encoded sRGB bytes does not produce evenly spaced changes in linear light.

### 2. The visual-uniformity promise is not supported by the executable behavior

UNIT:      man-page:color/brighten  
CLAIM:     “so equal amount steps look like equal steps.”  
VERDICT:   misleading  
EVIDENCE:  UNVERIFIED: `src/codegen/builtins/color/func_brighten.rs:BODY` only interpolates integer linear-light channels and converts back. The probe printed `#3366cc`, `#bec6e7`, and `#ffffff` for amounts `0.0`, `0.5`, and `1.0`; this verifies the interpolation, not a perceptual-uniformity claim. Linear light is a physical-light space, not a perceptually uniform lightness space.  
SUGGESTED: “This makes the interpolation linear in light rather than in encoded sRGB channel values.”

### 3. The page omits whether the input colour changes

UNIT:      man-page:color/brighten  
CLAIM:     “`brighten` moves each channel a fraction of the way from where it is to full brightness”  
VERDICT:   incomplete  
EVIDENCE:  `/tmp/plan-125-scratch/A-iter2/man-page-color-brighten/probe-project/src/main.mfb` printed `#3366cc` for `before` after calling `brighten(before, 0.5)`, then `#bec6e7` for the result. `src/codegen/builtins/color/func_brighten.rs:BODY` constructs and returns `Color[...]`; it does not alter `base`.  
SUGGESTED: “Returns a new colour whose channels move a fraction of the way from `base` to full brightness; `base` stays unchanged.”