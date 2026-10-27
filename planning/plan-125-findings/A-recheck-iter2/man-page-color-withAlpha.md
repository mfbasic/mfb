Both examples and the probe built and ran. Writing up the findings now.

### 1. "the one member that changes it" is contradicted by `color::mix`
UNIT:      man-page:color/withAlpha
CLAIM:     "color::brighten, color::darken and the HSL adjustments all leave alpha alone, so this is the one member that changes it."
VERDICT:   misleading
EVIDENCE:  `color::mix` lerps alpha along with the colour channels (`src/codegen/builtins/color/func_mix.rs`, `LET aa AS Integer = toInt(first.alpha)` and its description "every channel including alpha takes part"). Probe `p_probe/src/main.mfb` built with `mfb build -q p_probe`: `color::mix(color::rgba(0,0,0,0), color::rgba(0,0,0,255), 0.5).alpha` printed `mix alpha: 127`. So `withAlpha` is not the only non-constructor member that changes alpha. Note: this sentence exists only in the P-125 binary's source; the worktree at `00558e9f2` does not yet carry it.
SUGGESTED: "It is the alpha-only counterpart of building the colour afresh with `color::rgba` or `color::hsla`. `color::brighten`, `color::darken` and the HSL adjustments all carry alpha through unchanged; `withAlpha` is the one member that sets it directly, and `color::mix` is the only other member that changes it, by blending the two alphas."

### 2. No cross-reference to the non-clamping alternative a developer would reach for
UNIT:      man-page:color/withAlpha
CLAIM:     "withAlpha returns a colour with base's red, green and blue and the given alpha, clamped to 0..255."
VERDICT:   incomplete
EVIDENCE:  The package overview (`MODULE_DESC`, `src/codegen/builtins/color/mod.rs`) states that `color::Color[...]` and `WITH` do not clamp because the fields are `Byte`, so an out-of-range value is a conversion error there. This page never mentions that `brand WITH alpha := ...` is the obvious alternative, nor that it behaves differently on out-of-range input. Probe confirmed `withAlpha` clamps on both sides: `neg: 0`, `big: 255`, `huge: 255` (for `-5`, `300`, `4294967551`), and bounds are inclusive: `edge0: 0 edge255: 255`.
SUGGESTED: Add after the first paragraph: "Unlike updating the field yourself with `WITH` or a `color::Color[...]` literal, which take a `Byte` and refuse an out-of-range value, `withAlpha` takes an `Integer` and clamps it, so any alpha is safe to pass."

Everything else on the page verified against the binary. Example 1 printed `255 -> 128` and example 2 printed `0 120 200`, matching what the page implies. The probe showed `base` unchanged after the call (`#0a141e28` before, new value `#0a141ec8`), and the round-trip `withAlpha(withAlpha(o, 0), 255) = o` held for an opaque `o` (`roundtrip equal`). The page uses only permitted memory vocabulary ("copy", "unchanged") and teaches no borrow or ownership model. The Errors table is empty and the body raises nothing (`__color_clampByte` only clamps), which matches.
