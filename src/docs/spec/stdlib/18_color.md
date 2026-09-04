# Colour (color)

The `color` package is the language's single colour value type and the operations
over it. `IMPORT color` needs no manifest dependency. This topic specifies the
*semantic model* — the value model, the clamping rule, the packed byte order, and
the hex grammar. The per-function API — signatures, parameters, return types,
errors — is owned by `./mfb man color`.
[[src/codegen/builtins/color/mod.rs:register]]

Before this package MFBASIC had three unrelated notions of a colour that nothing
converted between: `canvas::Color` (a 4-`Byte` RGBA record), `term::TermColor` (a
3-`Byte` RGB record the runtime allocates) and an `astrings` foreground/background
attribute carrying a packed `0xRRGGBB` `Integer` with no type at all. `color::Color`
replaces all three.

## Value model

```
TYPE Color
  red AS Byte
  green AS Byte
  blue AS Byte
  alpha AS Byte
END TYPE
```

`Color` is an **ordinary value record**. Unlike `term::TermColor`, the runtime
does not allocate it, so it is absent from every read-only-record predicate: a
program may build one with a record literal (`color::Color[r, g, b, a]`) and
produce an updated copy with `WITH`. It obeys the ordinary value semantics of
`./mfb spec language types` §14 — assignment and argument passing produce copies.
[[src/codegen/builtins/term/mod.rs:is_read_only_record]]

`alpha` is **straight, not premultiplied**: `0` is fully transparent, `255` fully
opaque, and `red`/`green`/`blue` are unaffected by it. Two consequences the rest
of the language depends on:

- The all-zero `Color` is fully transparent, which is what makes it the no-op
  default for a `canvas::Paint` channel — an unset colour field is exactly
  `color::rgba(0, 0, 0, 0)`.
- A colour at `alpha` `0` still carries its hue, so
  `color::withAlpha(color::withAlpha(c, 0), 255)` is `c`.

## The clamping rule

Every constructor **clamps** its components to `0`..`255` rather than raising: a
value below `0` becomes `0` and a value above `255` becomes `255`.
[[src/codegen/builtins/color/helper_clamp_byte.rs:register]]

This is a deliberate contract, not an accident of implementation. Colours are
routinely computed — a base plus a delta, a channel scaled by a fraction, an
interpolation between two colours — and a result that lands one past an end is a
rounding artefact rather than a mistake worth stopping the program for.

It is also why every component parameter is declared `Integer` rather than
`Byte`. A `Byte` parameter would push an out-of-range value into a conversion
error at the *call site*, which is precisely the opposite of the promise: the
out-of-range value could never reach the clamp that exists to absorb it.

## Packed integer form

`color::toPacked` and `color::fromPacked` move between a colour and a single
`Integer` in **`0xAARRGGBB`** order — alpha in the highest of the low four bytes,
then red, then green, then blue in the lowest.

`toPacked` always yields a value in `0`..`4294967295` (`0xFFFFFFFF`) and is never
negative, since all four channels are `Byte`s and no term can overflow into its
neighbour.

`fromPacked` reads the **low 32 bits only**. Anything above bit 31 is ignored, so
a value that arrived with high bits set — or a negative one — still yields a
colour rather than failing. `fromPacked(-1)` is opaque white.

The two are exact inverses: `fromPacked(toPacked(c))` is `c` for every colour.

This packed form is not only a convenience: it is what an `astrings`
`Foreground`/`Background` attribute carries, so `color::toPacked` and
`color::fromPacked` are the two ends of storing a colour in styled text
(`./mfb spec stdlib astrings`). A terminal renders such an attribute without its
alpha, but the attribute keeps it.

A 24-bit `0xRRGGBB` value has a zero top byte and therefore unpacks **fully
transparent**. This is the one sharp edge in the packed model, and it is the
opposite of what `color::fromHex` does with the same 24 bits, where a missing
alpha means opaque. The asymmetry is deliberate: a packed integer states all four
bytes, so a zero alpha byte is a statement; a six-digit hex string does not
mention alpha at all, so there is nothing to state.

## Hex text form

`color::fromHex` accepts the four CSS lengths, with or without a leading `#`,
case-insensitively:

| Form | Digits | Expansion |
|---|---|---|
| `rgb` | 3 | each digit doubled — `f0a` is `ff00aa` — alpha `255` |
| `rgba` | 4 | each digit doubled, the fourth is alpha |
| `rrggbb` | 6 | alpha `255` |
| `rrggbbaa` | 8 | as written |

A digit is doubled by `d * 17`, which is exactly `d * 16 + d`, so `f` becomes
`255` and `0` becomes `0` — the short form's endpoints coincide with the long
form's.

Everything else raises `ErrInvalidFormat` (`77050003`): an unsupported length, a
non-hex character, an empty string, a second `#`. Digits are decoded **before**
the length is branched on, so a bad character inside an otherwise well-formed
length is rejected rather than read as whatever the sentinel arithmetic produces.
[[src/codegen/builtins/color/func_from_hex.rs:register]]

`color::toHex` renders `#rrggbb` and drops alpha; `color::toHexAlpha` renders
`#rrggbbaa` and is lossless. They are two members rather than one alpha-sensitive
member so that the output width is a property of the *call* rather than of the
data — a caller writing a fixed-width field never has to branch. Both zero-pad,
so a channel below `16` keeps its leading zero and cannot shift later channels
left.

Digits are emitted **lowercase**, matching `encoding::hexEncode`, so
`fromHex(toHex(c))` round-trips and two programs' `toHex` output compares equal.

`toString` over a `Color` renders the same `#rrggbbaa` form `toHexAlpha` does —
the lossless one, because `toString` is what a debugging `io::print` reaches for
and must not silently drop a channel.
[[src/codegen/builtins/color/helper_to_string.rs:register]]

## The sRGB seam, and why it is public

`color::toLinear` maps an sRGB channel byte to the light it represents, on
`0`..`65535`; `color::fromLinear` is the inverse. The mapping is a fixed 256-entry
table and a binary search over it, never a computed power function — see
§Determinism.

The table is the **single** source for the whole language. canvas's software
rasteriser blends through this same pair (`./mfb spec app canvas`
§"Rendering conventions"), which is why the seam is public rather than a private
helper: a package cannot reach another package's private members, and a
canvas-private duplicate would be two tables that must agree forever, with
`color::luminance` silently disagreeing with rendered pixels as the failure nobody
could see.

`fromLinear(toLinear(c))` is exact for all 256 channels. The reverse is not and
cannot be: there are 65536 linear values and 256 channels. `fromLinear` saturates
rather than raising — at or below `0` yields `0`, at or past `65535` yields `255`.

**Why the conversion is not a multiplication.** An sRGB channel is *encoded*, not
proportional to light: `128` carries about 22% of the light of `255`
(`toLinear(128)` is `14146`), not half. Blending, brightening or averaging the
encoded bytes directly produces the familiar too-dark midpoint — the linear
midpoint of black and white is `#bcbcbc`, not `#808080`.

## Perceptual operations

`brighten`, `darken`, `mix` and `grayscale` all work on the linear values, so
equal steps look like equal steps. `brighten` and `darken` move a fraction towards
white and black; `mix` interpolates two colours; `grayscale` projects onto relative
luminance.

Two alpha rules, deliberately different:

- `brighten` and `darken` leave `alpha` untouched — lightening a colour must not
  also change how much of it shows.
- `mix` interpolates `alpha`, because it is a blend of two whole colours. It does
  so on the raw value rather than through the transfer, since alpha is a coverage
  fraction, not a light intensity, and is not gamma-encoded.

Endpoint exactness is a contract, not an approximation: `brighten(c, 0.0)` is `c`,
`brighten(c, 1.0)` is white with `c`'s alpha, `darken(c, 1.0)` is black with `c`'s
alpha, `mix(a, b, 0.0)` is `a` and `mix(a, b, 1.0)` is `b`. Every `amount` is
clamped to `0.0`..`1.0`, so these operations interpolate and never extrapolate.

`luminance` is the WCAG relative luminance — `0.2126 R + 0.7152 G + 0.0722 B` over
the **linear** channels, `0.0`..`1.0`. `contrastRatio` is the WCAG
`(hi + 0.05) / (lo + 0.05)`, `1.0`..`21.0`, and sorts its own arguments so order
does not matter. `isDark` is `luminance < 0.5` and `isLight` its exact negation.
All of these ignore `alpha`: luminance is a property of the colour, and what a
translucent colour looks like depends on what is behind it.

## HSL

`color::Hsl` carries `hue` (degrees, `0.0`..`360.0`), `saturation` and `lightness`
(`0.0`..`1.0`). `toHsl` decomposes, `hsl`/`hsla` rebuild, and `saturate`,
`desaturate` and `rotateHue` manipulate.

**HSL describes the sRGB colour, not linear light.** This is a deliberate
asymmetry with the perceptual operations above, and it is the right one: CSS
`hsl()` and every design tool mean the encoded channels, so a round trip agrees
with the hex a designer wrote. The six primary hues land exactly on `#ff0000`,
`#ffff00`, `#00ff00`, `#00ffff`, `#0000ff` and `#ff00ff`, which they would not if
the model were computed in linear light.

`hue` **wraps**; `saturation` and `lightness` **clamp**. An angle is periodic and
a fraction is not, so `rotateHue(c, 400.0)` is `rotateHue(c, 40.0)` while a
saturation of `9.0` is simply `1.0`.

Two consequences of a grey having no meaningful hue, which differ and are both
deliberate:

- `toHsl` reports `hue` as `0.0` for any grey, and the round trip is still exact,
  because `hsl` ignores hue entirely when saturation is `0.0`.
- `saturate` on a grey therefore produces **red**, not the grey — `0.0` degrees is
  red, and that is the HSL model's own answer. It is not special-cased, because
  doing so would make the function discontinuous at saturation `0`.

`desaturate(c, 1.0)` and `grayscale(c)` are both "remove the colour" and they
**disagree**: the first preserves HSL lightness, the second preserves perceived
brightness. For a pure blue they differ substantially.

## Named colours

`color::fromName` resolves a CSS Color Level 4 `<named-color>` — case-insensitively,
with surrounding whitespace trimmed — and `color::nameOf` is the reverse. Both raise
`ErrNotFound` (`77050004`) rather than guessing. The table is transcribed from the
CSS Color 4 specification and is not restated here; that document is the authority
for both its membership and its values.

Two properties a caller has to know:

- **CSS `green` is `#008000`**, a dark green. The vivid `#00ff00` most people
  picture is `lime`. `color::green` follows CSS for the same reason, so the
  constant and the lookup agree.
- **`nameOf` matches exactly.** A colour one step off a named one has no name, and
  a colour whose alpha is not `255` has no name at all, because every entry in the
  table is opaque. There is no nearest-colour search: that is a different function
  with a contestable metric.

Six colours have two CSS spellings (`gray`/`grey`, `darkgray`/`darkgrey`,
`lightgray`/`lightgrey`, `slategray`/`slategrey`, `aqua`/`cyan`,
`fuchsia`/`magenta`). Both resolve through `fromName`; `nameOf` returns the
alphabetically first, which is a stable rule rather than an artefact of how the
table is stored.

There is deliberately no `transparent`. CSS's `transparent` is `#00000000`, which
`color::rgba(0, 0, 0, 0)` already spells, and a name whose alpha is not `255` would
contradict `nameOf`'s exact-match rule.

A set of record constants covers the basic colours — `black`, `white`, `red`,
`green`, `blue`, `yellow`, `cyan`, `magenta`, `gray`, `silver`, `maroon`, `olive`,
`navy`, `teal`, `purple`, `orange` — all fully opaque. A record constant inlines
its four field literals at the call site rather than being rendered into the
package's injected source, so an unused one costs a program nothing.

## Terminals ignore alpha

`term::setForeground`/`setBackground` take a `color::Color` and
`term::getForeground`/`getBackground` return one, but a terminal cell has no alpha
channel. The setters read only red, green and blue — a half-transparent colour
draws exactly the cells an opaque one draws — and the getters always report `alpha`
`255`.

This is a rendering limitation, not a conversion: the alpha is not clamped away or
rejected, it simply has nowhere to go. Synthesizing a blend against whatever is
already in the cell would disagree with what a canvas surface draws for the same
colour, so the terminal does not attempt one. An `astrings` attribute, by contrast,
*keeps* the alpha (`./mfb spec stdlib astrings`) — it is only the terminal renderer
that drops it.

## Determinism

Nothing in `color` uses a transcendental — no `pow`, no `exp`, no trig — and this
is a hard constraint rather than a coincidence of the current members. canvas's
software rasteriser is the oracle its GPU backends are compared against and must
produce identical bytes on every target; canvas calls into `color`, so the whole
package inherits that rule. Only IEEE `+ - * /` and `sqrt` appear on any `color`
path.

The sRGB table is 256 pasted literals for exactly this reason: evaluating
`((c + 0.055) / 1.055) ^ 2.4` at run time would put a libm `pow` on the
rasteriser's path and make the oracle platform-dependent. The HSL conversions are
likewise pure arithmetic — `+ - * /`, comparisons and `math::abs`/`floor`/`min`/
`max` — despite hue being an angle; no trigonometric function appears.
[[src/codegen/builtins/canvas/helper_color.rs:register]]

## Cross-package use

Imports are not transitive and a package cannot re-export another's types, so
**a program that names a `color::Color` must `IMPORT color`** — no matter which
package handed the value over. This is the same rule `net::Address` follows for
the transports.

## See Also

* ./mfb man color — the per-function API reference
* ./mfb spec language types — the value/copy semantics `Color` obeys
* ./mfb spec stdlib astrings — the attributed-string colours
* ./mfb spec app canvas — the rendering conventions colours are drawn under
