### 1. "so" draws the wrong consequence from straight alpha
UNIT:      man-page:color/types
CLAIM:     "alpha is straight, not premultiplied, so the all-zero colour is fully transparent."
VERDICT:   misleading
EVIDENCE:  The all-zero colour is fully transparent under premultiplied alpha too (transparent black is `0,0,0,0` in both models), so the sentence's "so" teaches nothing that distinguishes straight from premultiplied. What straight alpha actually means to a developer is that the colour channels survive at alpha 0. Probe `/tmp/plan-125-scratch/A-recheck-iter2/man-page-color-types/p2/src/main.mfb`: `color::rgba(255, 0, 0, 0)` prints `#ff000000` with `red=255`, `color::withAlpha(ghost, 255)` prints `#ff0000ff`, and `color::toHsl(ghost)` reports `0.00 1.00 0.50` (full-saturation red, not black). Implementation: `func_with_alpha.rs` and `helper_hsl.rs:__color_colorToHsl` read the channels without touching alpha.
SUGGESTED: "`alpha` is straight, not premultiplied: a colour with `alpha` `0` is invisible but still keeps its red, green and blue, so `color::withAlpha` can make it visible again unchanged. The all-zero colour is transparent black."

### 2. "every package" overstates who takes a Color
UNIT:      man-page:color/types
CLAIM:     "the colour value every package speaks"
VERDICT:   misleading
EVIDENCE:  `grep -rl 'color::Color\|COLOR_TYPE\|color\.Color' src/codegen/builtins | grep -v /color/` hits `canvas` (17 files), `term` (6), `astrings` (3), plus `money` and `vector` only in Rust doc comments (`money/func_to_string_rate.rs:4`, `vector/mod.rs:110`). Three packages consume it; `strings`, `fs`, `json`, `http` and the rest do not.
SUGGESTED: "the colour value `canvas`, `term` and `astrings` take wherever they draw"

### 3. Hsl field ranges are what `toHsl` reports, not what the record enforces
UNIT:      man-page:color/types
CLAIM:     "The hue in degrees around the colour wheel, from 0.0 up to but not including 360.0." (and likewise "0.0 (a neutral grey) to 1.0 (fully saturated)" and "0.0 (black) through 0.5 (the vivid hue) to 1.0 (white)")
VERDICT:   incomplete
EVIDENCE:  Probe `p1/src/main.mfb`: `color::Hsl[720.0, 2.0, -1.0]` compiles and prints `720.00 2.00 -1.00`; passing those fields to `color::hsl` prints `#000000ff` (hue wrapped, saturation clamped to 1.0, lightness clamped to 0.0). The record itself accepts any Float; only `toHsl` keeps the fields inside the stated ranges (`helper_hsl.rs:__color_colorToHsl` applies `__color_wrapHue` and `__color_clampFraction`), and `hsl`/`hsla` normalise on the way back in (`__color_hslToColor`). The page states the ranges as properties of the fields, so a reader building an `Hsl` by hand has no way to know an out-of-range value is accepted and what happens to it.
SUGGESTED: Add to the record description: "These ranges are what `color::toHsl` reports. A record you build yourself may hold any value; `color::hsl` wraps `hue` by whole turns and clamps `saturation` and `lightness` to `0.0`..`1.0`."

### 4. Printing a colour is undocumented, and an Hsl cannot be printed at all
UNIT:      man-page:color/types
CLAIM:     (omission) the `color::Color` and `color::Hsl` descriptions say nothing about `toString`
VERDICT:   incomplete
EVIDENCE:  Probe `p1/src/main.mfb`: `toString(color::Color[1, 2, 3, 4])` prints `#01020304` and `toString(color::fromPacked(0))` prints `#00000000` (the override is registered at `color/mod.rs` `pkg.add_override` with `builtin: "toString"`). `toString(h)` for an `Hsl` fails to compile with `TYPE_CALL_ARGUMENT_MISMATCH ... has argument type(s) (color.Hsl)`. A developer debugging a colour will reach for `toString` first, and the types page is where the two record types are introduced; neither behaviour is stated here or on the `color` overview's types section.
SUGGESTED: Color: "`toString` on a colour gives the eight-digit `#rrggbbaa` form, the same text `color::toHexAlpha` returns." Hsl: "`toString` does not accept an `Hsl`; print its three fields individually."

Nothing else on the page failed verification. `./scripts/man-census.sh --memory-scope color` reports 0 unclassified memory-vocabulary hits, and the probes confirmed: `rgb(-5, 300, 128)` clamps to `0 255 128 255` and `rgba(..., 999)` to alpha `255`; `toHsl` of pure green reports `120.00 1.00 0.50`; grey, white and black all report hue `0.00` and saturation `0.00`; `rgb(255, 0, 1)` reports hue `359.76`, confirming the exclusive upper bound; and assigning a `Color` then updating the original leaves the copy unchanged (`copy: 9 1`).
