### 1. Dropping alpha breaks the stated round trip
UNIT:      man-page:color/toPacked  
CLAIM:     "To get the 24-bit 0xRRGGBB form with alpha dropped, mask it off: bits::band(color::toPacked(c), 16777215)."  
VERDICT:   incomplete  
EVIDENCE:  The probe at `/tmp/plan-125-scratch/A-iter2/man-page-color-toPacked/probe/src/main.mfb` printed `51 102 204 0` after masking `color::rgba(51, 102, 204, 255)` to 24 bits and passing it to `color::fromPacked`. `src/codegen/builtins/color/func_from_packed.rs:58:__color_fromPacked` reads the absent high byte as alpha `0`; this masked value therefore cannot round-trip through the exact-inverse claim earlier on the page.  
SUGGESTED: To get the 24-bit `0xRRGGBB` form with alpha dropped, mask it off: `bits::band(color::toPacked(c), 16777215)`. Do not pass that result directly to `color::fromPacked`: its missing alpha byte becomes `0` (fully transparent).