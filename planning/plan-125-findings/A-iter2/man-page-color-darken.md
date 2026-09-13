### 1. Half-light claim ignores channel quantization
UNIT:      man-page:color/darken
CLAIM:     "`amount` is a fraction: `0.0` returns the colour unchanged, `1.0` returns black, `0.5` removes half the light."
VERDICT:   misleading
EVIDENCE:  Probe `#050505` printed linear input `99`, then `darken(c, 0.5)` as `#020202`, whose channel linear value is `40`, not half of `99` (`49.5`). `src/codegen/builtins/color/func_darken.rs:__color_darkenChannel` truncates the scaled amount, then `color::fromLinear` quantizes back to an sRGB byte.
SUGGESTED: "`0.5` targets half of each channel’s linear-light value. The result is quantized to sRGB bytes, so individual channels can differ from exactly half."

### 2. “Round trip lands lower” is false for the documented expression
UNIT:      man-page:color/darken
CLAIM:     "`darken` and `brighten` are **not** inverses. `darken(brighten(c, 0.5), 0.5)` does not return `c`: each works on a fraction of the *current* value, so the round trip lands lower."
VERDICT:   wrong
EVIDENCE:  Probe with `c = color::fromHex("#3366cc")` printed `#3366cc` followed by `#8b91aa` for the documented expression, which is lighter, not lower. `src/codegen/builtins/color/func_brighten.rs:__color_brightenChannel` followed by `func_darken.rs:__color_darkenChannel` computes a value whose direction relative to `c` depends on the original channel.
SUGGESTED: "`darken` and `brighten` are **not** inverses. `darken(brighten(c, 0.5), 0.5)` does not return `c`, because each works on the current value. Keep the original if you need to restore it."