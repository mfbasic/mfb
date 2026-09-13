### 1. Red range bounds are not explicit
UNIT:      man-page:color/rgba
CLAIM:     "The red component, clamped to `0`..`255`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_rgba.rs:component` supplies this descriptor; `__color_clampByte` clamps `value < 0` to 0 and `value > 255` to 255. Probe `rgba(-1, 256, 128, 0)` printed `0 255 128 0`. The description does not state that both endpoints are inclusive.
SUGGESTED: The red component, clamped to the inclusive range `0` through `255`.

### 2. Green range bounds are not explicit
UNIT:      man-page:color/rgba
CLAIM:     "The green component, clamped to `0`..`255`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_rgba.rs:component` supplies this descriptor; `__color_clampByte` clamps below 0 and above 255, leaving both endpoints valid. The compiled probe `rgba(-1, 256, 128, 0)` printed `0 255 128 0`. The description does not state endpoint inclusivity.
SUGGESTED: The green component, clamped to the inclusive range `0` through `255`.

### 3. Blue range bounds are not explicit
UNIT:      man-page:color/rgba
CLAIM:     "The blue component, clamped to `0`..`255`."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_rgba.rs:component` supplies this descriptor; `src/codegen/builtins/color/helper_clamp_byte.rs:__color_clampByte` implements inclusive endpoints. The rendered parameter description does not say so.
SUGGESTED: The blue component, clamped to the inclusive range `0` through `255`.

### 4. Alpha range bounds are not explicit
UNIT:      man-page:color/rgba
CLAIM:     "The alpha component, clamped to `0`..`255`: `0` fully transparent, `255` fully opaque."
VERDICT:   incomplete
EVIDENCE:  `src/codegen/builtins/color/func_rgba.rs:component` supplies this descriptor; `src/codegen/builtins/color/helper_clamp_byte.rs:__color_clampByte` leaves 0 and 255 valid. The compiled probe `rgba(-1, 256, 128, 0)` printed `0 255 128 0`. The parameter description gives endpoint meanings but never explicitly says both are included in the valid range.
SUGGESTED: The alpha component, clamped to the inclusive range `0` through `255`: `0` is fully transparent and `255` is fully opaque.