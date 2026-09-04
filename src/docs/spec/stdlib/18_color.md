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

## Determinism

Nothing in `color` uses a transcendental — no `pow`, no `exp`, no trig — and this
is a hard constraint rather than a coincidence of the current members. canvas's
software rasteriser is the oracle its GPU backends are compared against and must
produce identical bytes on every target; canvas calls into `color`, so the whole
package inherits that rule. Only IEEE `+ - * /` and `sqrt` appear on any `color`
path.
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
