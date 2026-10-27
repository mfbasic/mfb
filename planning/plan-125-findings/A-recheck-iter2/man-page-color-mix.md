### 1. Byte-average midpoint is stated as exactly `#808080`, but integer averaging gives `#7f7f7f`
UNIT:      man-page:color/mix
CLAIM:     "Averaging sRGB bytes gives #808080 for black-and-white, which reads as noticeably dark; the linear midpoint is around #bcbcbc."
VERDICT:   misleading
EVIDENCE:  Probe `avg/src/main.mfb` under /tmp/plan-125-scratch/A-recheck-iter2/man-page-color-mix, built with `mfb build -q avg`, printed `byte midpoint 127` for `(toInt(a) + toInt(b)) / 2` with `a = 0`, `b = 255`. That is `#7f7f7f`; `#808080` only appears if the 127.5 is rounded up. The sibling `color::fromLinear` example (its `EX` in `src/codegen/builtins/color/func_from_linear.rs`) prints "byte midpoint 127" for the same computation, so the two pages currently disagree by one step. Meanwhile the linear midpoint from `mix` itself is exactly `#bcbcbc` (example 1 output below), so "around" is hedged on the wrong number: the exact value is hedged and the inexact one is stated flatly.
SUGGESTED: "Averaging sRGB bytes gives `#7f7f7f` or `#808080` for black-and-white, depending on rounding, and either reads as noticeably dark; the linear midpoint is `#bcbcbc`."

No other findings. Every other sentence checked out against the binary:

- **Example 1** (`ex1`): printed `#bcbcbc`, matching "The linear midpoint of black and white is not #808080".
- **Example 2** (`ex2`): printed `#0a141e28` and `#c8d2dce6`, matching "endpoints return their operand exactly, alpha included".
- **Clamp, not raise** (`probe`): `amount = -3.0` printed `#0a141e28` (equals `first`), `amount = 7.5` printed `#c8d2dce6` (equals `second`). The lowering `__color_clampFraction` in `src/codegen/builtins/color/helper_clamp_fraction.rs` is a plain `< 0.0` / `> 1.0` clamp, consistent with "clamped to 0.0..1.0".
- **Alpha on raw value**: `mix(rgba(10,20,30,40), rgba(200,210,220,230), 0.5)` printed alpha `87` = 135 = 40 + (230-40)/2, confirming alpha is interpolated linearly on the raw byte while the colour channels went through `toLinear`/`fromLinear` (`#939ba3`, not `#697580`).
- **Order symmetry**: `mix(black, white, 0.5)` and `mix(white, black, 0.5)` both printed `#bcbcbcff`; `mix(a,b,0.5)` and `mix(b,a,0.5)` both printed `#939ba387`.
- **"22% of the light"**: `color::toLinear(128)` printed `14146` of `65535` = 21.6%, so the claim holds.
- **Errors table**: none rendered, and the descriptor has `errors: vec![]`. With `amount` clamped, the `toInt`/`toByte` steps in `__color_mix` stay inside `0..65535` and `0..255`, so nothing can raise.
- **NaN/infinity**: the probe line `1.0 / 0.0` trapped with `7-705-0015` at the division itself, and `src/codegen/builtins/math/mod.rs` documents that no NaN or Inf operand is constructible, so the clamp's NaN pass-through cannot be reached by a program.
- **§4 vocabulary and §3 scope**: no banned words; "linear light", "gamma-encoded" and "coverage fraction" are colour-science terms a developer needs, not compiler internals. No sentence teaches a borrow or ownership model.
