# plan-98 — Canvas API surface (quick reference)

Last updated: 2026-08-30

A flat list of the language-visible API added by plan-98 (consolidated 2D graphics /
canvas mode). This is the reference the sub-plans implement — all resource calls live
under the `canvas::` namespace (`loadImage`/`destroyImage`/`loadFont`/`destroyFont`),
not separate `image::`/`font::` packages.

**Language model:** resources (`Image`, `Font`) are **RES resources** — a plain owned
value holding an integer id (`handle@8` = OS-side texture), with the standard resource
`closed` flag and scope-drop reclaim. No references, no refcounting, no GC in the
language; `close ≠ drop` exactly like a file. `destroy*` (or scope-drop) is safe at any
time — it sets the closed flag; the runtime frees the single backend copy only once it is
no longer in the rendered scene and the GPU has drained the last frame that used it
(`closed AND lastUsedFrame < lastCompletedFrame`). That gate is entirely runtime-side and
invisible to MFB.

**Mode gate:** every `canvas::` call that touches the surface requires `Mode.Canvas`
and traps `ErrWrongMode` elsewhere. **Exception (plan-98-B Correction 12):**
`canvas::rgb`/`rgba` — and the `Paint` constructors `fill`/`stroke`/`fillStroke` —
touch no surface, they only build a value, so they are ungated and a program may
compute its palette before it presents anything. Precedent: `io::readByte` sits
outside the gated read set while its three siblings are in it. `term::*` traps in canvas mode. **`io::*` works fully in canvas mode**:
outputs (`io::print`/`io::write`/error variants) go to stdout/stderr, and inputs
(`io::readByte`/`readChar`/`readLine`/`input`) come **from the window** — the same input
source console mode uses, just delivered from the canvas window's key events. (In
`Mode.None`, which has no window, `io::` outputs still degrade to stdout/stderr but the
gated console reads trap — there is nowhere for input to come from.) The wrong-mode gate
covers exactly three calls today — `io::input`, `io::readLine`, `io::readChar`;
`io::readByte` is ungated (see plan-98-A §2). plan-98-A relaxes those three from
"`Console` only" to "not `None`".

`(fallible)` = returns per the result ABI (tag in x0, value in x1); may trap.

## app:: — presentation mode (existing package, extended)

- `app::setMode(mode AS Mode)`: switch presentation mode; `Mode.Canvas` builds the
  canvas surface, switching away tears it down.
- `app::getMode() AS Mode`: the current presentation mode.

## canvas:: — scene presentation

- `canvas::present(items AS List OF DrawItem)`: **install** a scene as the current
  display content (not a per-frame call). Deep-copies transitively; the runtime renders
  it on vsync/resize/damage until the next `present`. Identical re-present is a no-op.
- `canvas::presentLayers(layers AS List OF DrawLayer)`: install a layered scene; layers
  composite in order. A static layer hashes identically and reuses cached geometry.

## canvas:: — query & metrics

- `canvas::getSize() AS Size` *(fallible)*: the full canvas surface pixel dimensions.
  (Arity overload of the image form below: no arg → canvas; `Image` arg → that image.)
- `canvas::measureText(font AS Font, size AS Float, text AS String) AS TextMetrics`:
  text metrics **without drawing** (shipped from day one; API is shaper-independent).
  (`Real` corrected to `Float` 2026-08-30 — `Real` is not an MFBASIC type;
  `grep -rn "\bReal\b" src/docs/spec/` returns nothing. plan-98-B Correction 8.)

## canvas:: — resources (integer ids, no refs)

- `canvas::loadImage(path AS String) AS Image` *(fallible)*: load an image into the
  backend; returns an opaque `Image` id.
- `canvas::createImage(width AS Integer, height AS Integer, pixels AS List OF Byte) AS Image` *(fallible)*:
  create an image from raw RGBA8 pixels (`width * height * 4` bytes); returns an `Image` id.
- `canvas::destroyImage(img AS Image)`: mark the image for destruction; the runtime
  frees it once no scene or in-flight frame references it. Safe to call anytime.
- `canvas::loadFont(path AS String) AS Font` *(fallible)*: load a font; returns an
  opaque `Font` id.
- `canvas::destroyFont(font AS Font)`: mark the font for destruction; freed when
  unreferenced. Safe anytime.

## canvas:: — image content (mutate pixels behind an id; orthogonal to the scene)

Content changes here do **not** go through `present` — the scene layout is unchanged,
only the pixels behind the id change. The effect appears on the next rendered frame.

- `canvas::getBytes(image AS Image) AS List OF Byte`: the image's current RGBA8 pixels
  (`width * height * 4` bytes). Cheap — returns the runtime's CPU-side copy, no GPU
  readback.
- `canvas::setBytes(image AS Image, pixels AS List OF Byte)` *(fallible)*: replace the
  image's pixels. Errors (`ErrBadPixelCount`) if `len(pixels) != width * height * 4`.
  **Triggers a redraw only if the id is in the current scene** (otherwise nothing
  visible changed; a later `present` that adds the id will pick up the new content).
- `canvas::getSize(image AS Image) AS Size`: the image's pixel dimensions (the `Image`
  overload of `canvas::getSize`).

## canvas:: — color helpers

- `canvas::rgb(r AS Integer, g AS Integer, b AS Integer) AS Color`: opaque color
  (alpha 255); components clamped to 0..255.
- `canvas::rgba(r AS Integer, g AS Integer, b AS Integer, a AS Integer) AS Color`.

## Types

- `Mode` *(enum)*: `Console = 0`, `None = 1`, `Canvas = 2`.
- `DrawItem` *(union — frozen set; extending it is a breaking change)*:
  `Picture`, `Rectangle`, `Line`, `Polygon`, `Circle`, `Arc`, `Text`, `RoundedRect`.
  Representative fields (all coordinates in pixels, Y-down top-left origin; all carry a
  `paint AS Paint`):
  - `Circle[x, y, radius AS Float, paint]`
  - `Arc[x, y, radius, startAngle, endAngle AS Float, paint]` — angles in **radians**,
    clockwise from +X (Y-down); stroke it via `paint.stroke`/`strokeWidth`.
  - `Rectangle[x, y, w, h AS Float, paint]`, `RoundedRect[…, cornerRadius, paint]`,
    `Line[x1, y1, x2, y2 AS Float, paint]`, `Polygon[points AS List OF Point, paint]`,
    `Picture[x, y, w, h AS Float, image AS ImageRef, paint]`,
    `Text[x, y AS Float, text AS String, font AS FontRef, size AS Float, paint]`.
  - **The image-drawing variant is `Picture`, not `Image`** (corrected 2026-08-30,
    plan-98-B Correction 6): a record and a resource cannot share a name, and the
    resource is `Image`.
- `ImageRef`, `FontRef` *(record)*: a single `id AS Integer` naming a resource the
  backend owns — obtained with `canvas::imageRef`/`canvas::fontRef`. **A record field
  cannot hold a resource** (verified: `handle AS File` is `SYMBOL_UNKNOWN_TYPE`,
  `handle AS RES File` does not parse), so the two variants that reference one carry a
  handle. This is the model already stated above — the backend owns the one real copy
  and MFB holds only the id — so a published scene provably retains nothing. Corrected
  2026-08-30, plan-98-B Correction 5.
- `DrawLayer` *(record)*: an ordered set of `DrawItem`s composited as one layer.
- `Paint` *(record)*: flat value record threaded through items — `fill AS Color`,
  `stroke AS Color`, `strokeWidth AS Float`, `blend AS BlendMode`,
  `transform AS Transform`, `clip AS Bounds`. No ambient state.
  **Every field's zero value is that field's no-op**: transparent fill/stroke, zero
  width, `Normal` blend, the identity transform (which is the *all-zero* `Transform`,
  by definition — see below), and a zero-area `clip` meaning unclipped.
  ~~Named construction defaults the unset fields.~~ **FALSE — corrected 2026-08-30
  (plan-98-B Correction 7).** MFBASIC named construction requires **every** field:
  `Paint[fill := c]` is `TYPE_CONSTRUCTOR_ARITY_MISMATCH` ("has 1 argument(s),
  expected 6"). The spec's `Circle[radius := 10.0]` is a *complete* construction of a
  one-field record, not evidence of defaulting. `Paint` is therefore built with
  `canvas::fill` / `canvas::stroke` / `canvas::fillStroke`, and refined with `WITH`.
- `BlendMode` *(enum)*: `Normal = 0`, `Multiply`, `Screen`, `Add`.
- `Transform` *(record)*: `a, b, c, d, tx, ty AS Float` — a 2×3 affine applied as
  `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`. **The all-zero value means the
  identity**, so an unset transform leaves an item alone rather than collapsing it to
  the origin.
- `Color` *(record)*: `red`, `green`, `blue`, `alpha AS Byte`. Construct via
  `canvas::rgb`/`rgba`.
- `Point` *(record)*: `x`, `y AS Float`.
- `Size` *(record)*: `width`, `height AS Integer` (pixels).
- `Image`, `Font` *(RES resource)*: a plain owned value holding an integer id; the
  backend owns the one real copy, MFB holds only the id. Closed-flag lifetime, no refs.
- `Bounds` *(record, internal)*: `x`, `y`, `w`, `h` — item/damage bounds.
- `TextMetrics` *(record)*: `width`, `height`, `ascent`, `descent`, `lineGap`.

## Error contract (summary)

- Fallible: mode entry, `canvas::loadImage` / `createImage`, `canvas::loadFont`,
  `canvas::setBytes` (wrong pixel count → `ErrBadPixelCount`), `canvas::getSize`.
  `canvas::present` is technically fallible (arena/atlas exhaustion) but device-lost is
  recovered transparently and never surfaced — effectively infallible in practice, like
  `term::sync`.
- `getBytes` / `getSize` / `destroy*` never fail (using a closed id is the universal
  `ErrResourceClosed` trap, per the RES model).

## Example — a yellow smiley face with green eyes and smile

Used to sanity-check the API shape (see "What this surfaced" below).

Exported *types* are referenced bare by importers (like the `app` package's `Mode`);
only *functions* are package-qualified (`app::setMode`, `canvas::present`, `canvas::rgb`).

```basic
IMPORT app
IMPORT canvas
IMPORT io

FUNC main AS Integer
  app::setMode(Mode.Canvas)

  LET yellow AS Color = canvas::rgb(255, 255, 0)
  LET green  AS Color = canvas::rgb(0, 160, 0)

  ' Centre the face on the canvas. getSize() with no arg returns the surface size.
  LET canvasSize AS Size = canvas::getSize()
  LET cx AS Float = toFloat(canvasSize.width) / 2.0
  LET cy AS Float = toFloat(canvasSize.height) / 2.0

  ' Coordinates are pixels, Y-down, top-left origin. Each item is bound first: a
  ' list literal does not span lines (corrected 2026-08-30 — the original spanned
  ' four, which is MFB_PARSE_EXPECTED_EXPRESSION).
  LET face AS DrawItem = Circle[x := cx, y := cy, radius := 150.0, paint := canvas::fill(yellow)]
  LET eyeL AS DrawItem = Circle[x := cx - 50.0, y := cy - 40.0, radius := 22.0, paint := canvas::fill(green)]
  LET eyeR AS DrawItem = Circle[x := cx + 50.0, y := cy - 40.0, radius := 22.0, paint := canvas::fill(green)]
  ' Smile — the lower half of a circle, stroked green (0 → PI sweeps downward, Y-down).
  LET smile AS DrawItem = Arc[x := cx, y := cy + 15.0, radius := 90.0, startAngle := 0.0, endAngle := 3.14159, paint := canvas::stroke(green, 14.0)]

  LET scene AS List OF DrawItem = [face, eyeL, eyeR, smile]

  canvas::present(scene)

  ' When main returns, the program exits — so keep it alive. The runtime keeps rendering
  ' the installed scene while we wait. io input comes from the window in canvas mode.
  ' io::pollInput() with NO argument BLOCKS until input is ready (an optional timeoutMs
  ' bounds the wait), so this is a single blocking wait, not a spin.
  LET keyReady AS Boolean = io::pollInput()

  RETURN 0
END FUNC
```

### What this surfaced (the point of writing it)

- **Colors need a constructor.** Building `Color` from raw `Byte` fields is clumsy in
  source, so `canvas::rgb`/`rgba` were added (real gap — kept).
- ~~**Partial `Paint` construction must default the rest.**~~ **The need was real; the
  mechanism was not.** The example did rely on naming one `Paint` channel and leaving
  the other transparent — but MFB named construction requires **every** field
  (`TYPE_CONSTRUCTOR_ARITY_MISMATCH`), and the spec's `Circle[radius := 10.0]` is a
  complete construction of a one-field record, not evidence of defaulting. So the
  requirement is met by constructors — `canvas::fill(c)`,
  `canvas::stroke(c, width)`, `canvas::fillStroke(f, s, width)` — plus `WITH` for
  blend/transform/clip. Corrected 2026-08-30, plan-98-B Correction 7.
- **A list literal does not span source lines**, and the smiley's did. Measured while
  building this example for real: a `[` … `]` broken over four lines is
  `MFB_PARSE_EXPECTED_EXPRESSION`. Each item is now bound to a `DrawItem` local first,
  which also reads better than a four-line literal did.
- **`Circle`/`Arc` earn their place as first-class variants.** Drawing a circle as a
  `RoundedRect` (radius = min(w,h)/2) works internally but reads badly in source; the
  smile in particular is impossible without `Arc`. Added to the frozen set.
- **`Arc` needs a stated angle convention** (radians, clockwise from +X under Y-down) or
  the smile could render as a frown — documented on the type.
- **Union-variant widening into `List OF DrawItem` is load-bearing** — each bare
  `Circle[…]`/`Arc[…]` must widen to `DrawItem` in the list literal (works per the union
  model; called out so B's type checking covers it).
- ~~**Variant-constructor qualification needs pinning.**~~ **RESOLVED 2026-08-30: bare.**
  `.ai/resources-packages.md:24` states the rule for a new native backend outright —
  "Declare union variants with BARE ids (no `pkg::Type` normalization)". The spec's
  qualified `extras::Circle[radius := 10.0]` form applies to *included* union members,
  not to a directly-exported variant. The example's bare `Circle[…]`/`Paint[…]` is
  correct as written; B's Phase 1 declares them bare.
- **Program lifecycle: when `main` returns, the program exits** — so a canvas program must
  keep `main` alive. The example spins on `io::pollInput()` until any key. This drove the
  decision that **`io::*` works in canvas mode** (outputs to stdout/stderr, inputs from the
  window) — otherwise a canvas program would have no way to read input and would need a
  bespoke event API. Wiring window key events into the `io::` input path (mirroring term
  keyboard input) lands in plan-98-A.
- ~~**A blocking-wait primitive would beat the tight poll loop.**~~ **Corrected
  2026-08-30 — no gap here.** `io::pollInput()` with the argument **omitted already
  blocks** until standard input is ready
  (`src/codegen/builtins/io/func_poll_input.rs:198`: "When it is **omitted, `pollInput`
  blocks** until standard input is ready"), and an optional `timeoutMs` bounds the wait
  per the plan-73 timeout convention (negative → `ErrInvalidArgument`). The example was
  rewritten from a `WHILE NOT` spin to a single blocking call; there is no 100%-CPU busy
  loop and nothing to add. A canvas *frame*-wait is still a possible future convenience,
  but it is not needed for the keep-main-alive case.


## Revision note

**2026-08-30.** Corrected against the tree: the `io::pollInput()` blocking semantics (the
smiley example no longer spins) and the union-variant qualification open item (resolved:
bare ids). No API surface changed. The `Mode` enum this references is now registry data in
`src/codegen/builtins/app/mod.rs`, not a `.mfb` source file — see plan-98-A's Corrections.
