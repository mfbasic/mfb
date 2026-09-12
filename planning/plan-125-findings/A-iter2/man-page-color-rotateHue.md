### 1. Repeated rotation is not additive

UNIT:      man-page:color/rotateHue  
CLAIM:     “Rotating by `360.0` returns the original colour, and rotating twice is the same as rotating once by the sum.”  
VERDICT:   wrong  
EVIDENCE:  The probe at `/tmp/plan-125-scratch/A-iter2/man-page-color-rotateHue/src/main.mfb`, built and run with the specified release binary, prints `mismatch #000011 #000011 #010011` for two `1.0`-degree rotations versus one `2.0`-degree rotation. `src/codegen/builtins/color/func_rotate_hue.rs:BODY` converts each result back to byte channels, so each separate call rounds before the next rotation.  
SUGGESTED: `Rotating by 360.0 returns the original colour. For the most predictable result, apply the total rotation in one call: separate rotations can differ slightly because each call produces an sRGB colour.`