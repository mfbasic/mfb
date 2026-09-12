### 1. Red range endpoint is unstated
UNIT:      man-page:color/rgb
CLAIM:     “The red component, clamped to 0..255.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_byte.rs:BODY` clamps only values `< 0` and `> 255`; probe `/tmp/plan-125-scratch/A-iter2/man-page-color-rgb/src/main.mfb` printed `0 255 42 255` for `color::rgb(-1, 0, 256)` and `color::rgb(0, 255, 42)`.
SUGGESTED: “The red component, clamped to the inclusive range 0..255.”

### 2. Green range endpoint is unstated
UNIT:      man-page:color/rgb
CLAIM:     “The green component, clamped to 0..255.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_byte.rs:BODY` clamps only values `< 0` and `> 255`; probe `/tmp/plan-125-scratch/A-iter2/man-page-color-rgb/src/main.mfb` printed `0 255 42 255` for `color::rgb(-1, 0, 256)` and `color::rgb(0, 255, 42)`.
SUGGESTED: “The green component, clamped to the inclusive range 0..255.”

### 3. Blue range endpoint is unstated
UNIT:      man-page:color/rgb
CLAIM:     “The blue component, clamped to 0..255.”
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/helper_clamp_byte.rs:BODY` clamps only values `< 0` and `> 255`; probe `/tmp/plan-125-scratch/A-iter2/man-page-color-rgb/src/main.mfb` printed `0 255 42 255` for `color::rgb(-1, 0, 256)` and `color::rgb(0, 255, 42)`.
SUGGESTED: “The blue component, clamped to the inclusive range 0..255.”