# Shader functions

A `SHADER FUNC` is an MFB function the canvas runs once per pixel to colour an item. It is
compiled twice: to native code, for the software renderer and for ordinary calls (including
`TESTING`), and to GPU code (Metal and Vulkan), for the GPU renderer. Both run the same source,
so the body is restricted to what a GPU can execute.

## Attaching a shader to an item

```basic
FUNC canvas::shader OF T(fn AS SHADER FUNC(canvas::ShaderPoint, T) AS color::Color, userData AS T) AS canvas::Shader
```

`canvas::Paint` gains a field:

```basic
shader AS canvas::Shader   ' the zero value is no shader
```

```basic
TYPE GlowParams
  centre AS vector::Float2
  radius AS Float
  ink    AS color::Color
END TYPE

SHADER FUNC glow(point AS canvas::ShaderPoint, u AS GlowParams) AS color::Color
  LET d AS Float = vector::distance(point.localPosition, u.centre)
  LET a AS Float = math::clamp(1.0 - d / u.radius, 0.0, 1.0)
  RETURN color::withAlpha(u.ink, toInteger(a * 255.0))
END FUNC

LET p AS GlowParams = GlowParams[centre := vector::Float2[x := 50.0, y := 50.0], radius := 40.0, ink := color::rgb(255, 64, 0)]
LET paint AS canvas::Paint = WITH canvas::fill(color::black) { shader := canvas::shader(glow, p) }
```

- `fn` must name a `SHADER FUNC` directly: no lambda, closure or function variable. The GPU
  code for each named shader is generated at build time.
- A user's shader is concrete and can have any name. It may also be a template
  (`SHADER FUNC tint OF T(...)`); each instantiation is a separate shader.
- `userData` is copied at `canvas::present()` and is part of the item's content: changing it
  means presenting again, and an unchanged one costs nothing. The canvas supplies no time or
  frame counter. An animated shader takes its time from `userData`.

## What the shader paints

- The item's **fill**. The outline stays `Paint.stroke`, as with `Paint.gradient`. If `shader` is
  set, `Paint.gradient` is ignored and `Paint.fill` reaches the shader only as `point.fill`.
- `canvas::Text` ignores `shader`, as it ignores `gradient`.
- The returned colour is straight-alpha sRGB, like every `color::Color`. The engine then applies
  edge coverage, the clip, `transparency` and `blend`, so a shader never antialiases the
  item's edge or composites.

## Types

```basic
' What the engine passes for each pixel it shades.
TYPE canvas::ShaderPoint
  localPosition  AS vector::Float2  ' pixel centre in the item's own coordinates: (0, 0) at the
                                    ' item's position, with its transform and groups undone
  localPerPixelX AS vector::Float2  ' change in localPosition for one screen pixel right
  localPerPixelY AS vector::Float2  ' change in localPosition for one screen pixel down
  screenPosition AS vector::Float2  ' pixel centre on the surface, top-left origin
  uv             AS vector::Float2  ' 0..1 across the item's bounding box (see below)
  fill           AS color::Color    ' the item's Paint.fill
  beneath        AS canvas::ShaderNeighbours  ' what was drawn before this item (see below)
END TYPE

TYPE canvas::ShaderNeighbours
  topLeft     AS color::Color
  top         AS color::Color
  topRight    AS color::Color
  left        AS color::Color
  centre      AS color::Color
  right       AS color::Color
  bottomLeft  AS color::Color
  bottom      AS color::Color
  bottomRight AS color::Color
END TYPE
```

- `localPerPixelX` / `localPerPixelY` tell a shader how large a pixel is in its own coordinates,
  so it can antialias its own detail (stripes, checks) when the item is scaled down.
- `uv` spans the item's fill bounds in its own coordinates, without the stroke. On an
  item with no area in one direction (a horizontal `Line`), that component is 0.
- `beneath`:
  - It is the surface as it was before this item drew: a snapshot, so the shader never
    sees its own output. On the GPU this splits the frame into separate passes at this item.
  - It is free when unused. The compiler knows whether a shader reads `point.beneath`, and an
    item whose shader does not read it takes no snapshot.
  - A neighbour off the surface repeats the nearest edge pixel.
  - The colours are sRGB bytes. Averaging them directly is darker than the canvas's own
    linear-light blending. Use `color::toLinear` / `color::fromLinear` to work in linear light.

## Calls

- The engine calls the shader for the pixels the item may cover. The number of calls is not
  defined: a pixel may be skipped, or evaluated more than once (an item drawn in separate fill
  and stroke passes, a redraw after the window is uncovered). The same inputs always give the
  same colour, which the rules below guarantee.
- Software and GPU agree to a tolerance, not bit for bit (see Numbers).

## What `T` may contain

Float, Integer, Boolean, Byte, enums, `vector::Float2/3/4`, `vector::Integer2/3/4`,
`color::Color`, and records of those, nested. At most 256 bytes. Checked when `canvas::shader`
is instantiated.

## Rules for a `SHADER FUNC` body

The compiler rejects a `SHADER FUNC` that breaks any of these.

**Types.** Only the types `T` may contain, plus `canvas::ShaderPoint` and
`canvas::ShaderNeighbours`. No String, Fixed, Money, List, Map, Set, resources, threads,
unions, or function values.

**Names.** Parameters, locals (`LET` and `MUT`) and constants, including package constants
such as `math::pi`. No top-level binding that is mutable or computed at run time.

**Calls.**
- Other `SHADER FUNC`s, with no cycles: no recursion, direct or indirect.
- These package functions only, scalar and vector overloads (not the `List` ones):
  - `math::`: `abs`, `min`, `max`, `clamp`, `floor`, `ceil`, `round`, `sqrt`, `pow`, `exp`, `log`,
    `log10`, `sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`. Not `rand` or `seed`.
  - `vector::`: every function, on the Float and Integer vectors.
  - `color::`: `rgb`, `rgba`, `withAlpha`, `mix`, `toLinear`, `fromLinear`, `invert`, `grayscale`,
    `luminance`.
  - The conversions `toFloat`, `toInteger` and `toByte`, and record construction and `WITH`.
- No LINK functions, no other packages, no I/O.

**Control flow.** `IF`, `MATCH` on enums and constants, `FOR` loops, `EXIT` and `CONTINUE`. No
`WHILE` or `DO`. A `FOR` loop's bounds must be constants, so every shader finishes. No `FAIL`,
`TRAP`, `RECOVER` or `PROPAGATE`.

**A shader cannot fail.** Operations that raise an error elsewhere have fixed results inside a
`SHADER FUNC`, the same on CPU and GPU:

| Operation | Result |
|---|---|
| Integer overflow | wraps (32-bit) |
| Integer divide or `MOD` by zero | 0 |
| `toInteger(Float)` out of range | saturates to the Integer range; NaN gives 0 |
| A Float result that is NaN or infinite | carried on; NaN becomes 0 when converted |
| `math::` domain errors (`sqrt` of a negative, `log` of 0 or less) | NaN |
| Conversion to `Byte` | saturates to 0..255 |

## Numbers

- `Float` is 32-bit inside a `SHADER FUNC`, on the CPU as well as the GPU, so the two renderers
  compute the same way. Metal has no 64-bit floats and many GPUs (the Raspberry Pi's included)
  have none either.
- `Integer` is 32-bit inside a `SHADER FUNC`, on both.
- `math::` functions on the GPU are the driver's own, which differ from the CPU's in the
  last bits. Results are expected to agree to within two sRGB step per channel on ordinary
  input.

## Examples

### Antialiased stripes

Diagonal stripes over the item's fill. `localPerPixelX` / `localPerPixelY` give the size of one
pixel in stripe units, so the stripe edges stay smooth at any scale.

```basic
TYPE StripeParams
  width AS Float          ' width of one stripe pair, in the item's own units
  ink   AS color::Color   ' stripe colour; the gaps show Paint.fill
END TYPE

SHADER FUNC stripes(point AS canvas::ShaderPoint, u AS StripeParams) AS color::Color
  ' Position across the stripes, in stripe pairs, and where in its pair the pixel is.
  LET s AS Float = (point.localPosition.x + point.localPosition.y) / u.width
  LET f AS Float = s - math::floor(s)
  ' Signed distance to the nearest stripe edge, in stripe pairs: negative inside a stripe.
  LET d AS Float = math::abs(f - 0.5) - 0.25
  ' How far s moves across one screen pixel.
  LET px AS Float = math::abs(point.localPerPixelX.x + point.localPerPixelX.y) / u.width
  LET py AS Float = math::abs(point.localPerPixelY.x + point.localPerPixelY.y) / u.width
  LET pixel AS Float = math::max(px + py, 0.000001)
  LET cover AS Float = math::clamp(0.5 - d / pixel, 0.0, 1.0)
  RETURN color::mix(point.fill, u.ink, cover)
END FUNC
```

### Animated waves

Three sine waves across the item's bounding box, blended between two colours. The time comes
from `userData`, so the program presents every frame.

```basic
TYPE WaveParams
  time AS Float           ' seconds since the animation started
  low  AS color::Color
  high AS color::Color
END TYPE

SHADER FUNC waves(point AS canvas::ShaderPoint, u AS WaveParams) AS color::Color
  LET x AS Float = point.uv.x
  LET y AS Float = point.uv.y
  LET v AS Float = math::sin(x * 12.0 + u.time) _
                 + math::sin(y * 9.0 - u.time * 1.3) _
                 + math::sin((x + y) * 7.0 + u.time * 0.7)
  ' v is -3..3; mix needs 0..1.
  RETURN color::mix(u.low, u.high, (v + 3.0) / 6.0)
END FUNC

' The loop that drives it:
MUT t AS Float = 0.0
DO WHILE running
  t = t + canvas::getFrameStats().dt
  LET params AS WaveParams = WaveParams[time := t, low := color::navy, high := color::teal]
  canvas::present([canvas::Rectangle[...,
    paint := WITH canvas::fill(color::black) { shader := canvas::shader(waves, params) }]])
  canvas::waitFrame()
LOOP
```

### Frosted glass

A panel that blurs what is behind it and tints it. It reads `beneath`, so this item takes a
snapshot. The average is taken in linear light, as the canvas blends. A 3×3 average is a light
blur.

```basic
TYPE GlassParams
  tint AS color::Color    ' the tint; its alpha is how strong the tint is
END TYPE

' The linear-light sum of three colours, channel by channel.
SHADER FUNC linearSum(a AS color::Color, b AS color::Color, c AS color::Color) AS vector::Integer3
  RETURN vector::Integer3[
    x := color::toLinear(a.red) + color::toLinear(b.red) + color::toLinear(c.red),
    y := color::toLinear(a.green) + color::toLinear(b.green) + color::toLinear(c.green),
    z := color::toLinear(a.blue) + color::toLinear(b.blue) + color::toLinear(c.blue)]
END FUNC

SHADER FUNC glass(point AS canvas::ShaderPoint, u AS GlassParams) AS color::Color
  LET n AS canvas::ShaderNeighbours = point.beneath
  LET top AS vector::Integer3 = linearSum(n.topLeft, n.top, n.topRight)
  LET mid AS vector::Integer3 = linearSum(n.left, n.centre, n.right)
  LET bot AS vector::Integer3 = linearSum(n.bottomLeft, n.bottom, n.bottomRight)
  LET blurred AS color::Color = color::rgb(
    toInteger(color::fromLinear((top.x + mid.x + bot.x) DIV 9)),
    toInteger(color::fromLinear((top.y + mid.y + bot.y) DIV 9)),
    toInteger(color::fromLinear((top.z + mid.z + bot.z) DIV 9)))
  RETURN color::mix(blurred, color::withAlpha(u.tint, 255), toFloat(u.tint.alpha) / 255.0)
END FUNC
```
