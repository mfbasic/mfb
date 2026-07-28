# plan-22-C: squircle mask + margin polish for the macOS icon

Last updated: 2026-07-12 (tree-drift refresh; design unchanged, no plan-22 work landed)
Effort: medium (1h–2h)

> **Tree-drift note (2026-07-12).** Not landed and depends entirely on plan-22-B's
> not-yet-created `src/os/macos/icon.rs` + `ImageRenderHook` seam, so its anchors
> are all forward references — nothing in the current tree has drifted. `tiny-skia`
> is still absent from `Cargo.toml`.

Make the generated macOS icon look like a native macOS app icon: inset the
artwork within the standard Big Sur+ margin and clip it to Apple's rounded-
rectangle "squircle" shape, so an arbitrary square source image comes out as a
properly-shaped app icon rather than a full-bleed square. This replaces the
identity render hook plan-22-B left as a seam.

The single behavioral outcome: the `AppIcon.icns` produced by `mfb build -app`
shows the artwork centered inside a squircle with the correct margin — visually
consistent with stock macOS app icons — for both the embedded default and a
user-provided `icon`.

Depends on: **plan-22-B** (the `.icns` pipeline, embedded default, bundle
wiring, and the `ImageRenderHook` seam in `src/os/macos/icon.rs`).

It complements:

- `./mfb spec app macos-runtime` (app icon appearance; canonical source
  `src/docs/spec/app/01_macos-runtime.md`)

## 1. Goal

- Replace the identity render hook in `src/os/macos/icon.rs` with a masking hook
  that, on the 1024×1024 RGBA canvas:
  1. Scales the source artwork to fit the standard macOS icon **content** area
     (the artwork body, inset by the Big Sur margin) centered on the canvas.
  2. Clips it to a **squircle** (continuous-curvature rounded rectangle) matching
     the macOS icon grid corner radius.
  3. Leaves the surrounding margin fully transparent.
- Applies identically to the embedded default and a user `icon`; the downsampled
  entries at every `.icns` size inherit the shape (mask applied once at 1024,
  then resized — plan-22-B already resizes from the single 1024 canvas).

### Non-goals (explicit constraints)

- No language surface change; no `func_<pkg>_<fn>` tests.
- No change to executable bytes / `mfb spec memory`.
- No per-size hand-tuned masks — mask once at 1024 and downsample (the resize
  antialiases the edge). No pixel-perfect match to Apple's exact superellipse is
  required; a high-quality rounded-rect/superellipse approximation is acceptable
  (Open Decision 2).
- No drop shadow / specular highlight by default (optional subtle shadow behind
  Open Decision 3, off unless chosen).

## 2. Current State

- `src/os/macos/icon.rs::build_icns` (from plan-22-B) normalizes the source to a
  1024×1024 RGBA canvas and calls an `ImageRenderHook::identity()` seam before
  downsampling to each `.icns` entry. Everything downstream (icns encode, bundle
  write, plist) is done.
- No 2D rasterizer is present. `tiny-skia` is the requested masking crate and is
  not yet a dependency (plan-22-B added only `image` + `icns`).
- macOS icon grid reference (Big Sur+): on a 1024 canvas the icon body is
  **824×824** centered (100px margin each side); the squircle corner radius is
  ≈ **0.2237 × body** ≈ 184px for a rounded-rect approximation (Apple's true
  shape is a ~n=5 superellipse / continuous corner).

## 3. Design Overview

Add `tiny-skia` and implement the masking hook in three steps, all inside
`src/os/macos/icon.rs`:

1. **Scale to content area** — resize the normalized source to fit an 824×824
   content box (preserving aspect; a square 1024 source → 824×824) and place it
   centered on a 1024 transparent canvas.
2. **Build the squircle path** — a `tiny-skia::Path` for the rounded rectangle /
   superellipse over the 824×824 content box (centered).
3. **Clip** — composite the scaled artwork through the squircle as a mask
   (`tiny_skia::Mask` / clip), producing a 1024 RGBA with transparent margin and
   squircle-clipped artwork. Convert back to `image::RgbaImage` for plan-22-B's
   downsample+encode path.

The correctness risk is premultiplied-alpha conversion between `image`
(straight/unassociated RGBA) and `tiny-skia` (premultiplied) and getting the
squircle geometry (margin + radius) to read as a native icon.

## 4. Detailed Design

### 4.1 Dependency

```toml
tiny-skia = "0.11"
```

Build-time only (compiler), like plan-22-B's crates.

### 4.2 Geometry constants (`src/os/macos/icon.rs`)

```
const CANVAS: u32 = 1024;
const CONTENT: u32 = 824;              // Big Sur icon body on a 1024 grid
const MARGIN: f32 = (1024 - 824) / 2;  // = 100.0
const CORNER_RADIUS: f32 = 184.0;      // ≈ 0.2237 * 824 (rounded-rect approx)
```

### 4.3 Masking hook

Replace `ImageRenderHook::identity()` with `ImageRenderHook::squircle()`:

1. **Fit artwork to content box.** From the normalized 1024 RGBA (plan-22-B),
   resize the *artwork* to fit CONTENT×CONTENT (Lanczos3, preserve aspect), then
   blit centered onto a fresh 1024 transparent `image::RgbaImage`. For the common
   square source this is a straight 1024→824 resize centered with a 100px margin.
2. **Rasterize the squircle mask.** Create a `tiny_skia::Pixmap` 1024×1024. Build
   the path over the content rect `[MARGIN, MARGIN, MARGIN+CONTENT,
   MARGIN+CONTENT]`:
   - Approximation (recommended): `PathBuilder::push_round_rect`-style path — a
     rounded rect with `CORNER_RADIUS`, corners as cubic-Bézier arcs.
   - True squircle (optional, Open Decision 2): sample a superellipse
     `|x/a|^n + |y/a|^n = 1`, `n≈5`, as a poly-line path for continuous corners.
   Fill the path opaque white with antialiasing into the pixmap → this is the
   alpha mask.
3. **Apply the mask.** Multiply the artwork canvas's alpha by the mask alpha
   per pixel (straight-alpha multiply on `image::RgbaImage`), OR draw the artwork
   pixmap through a `tiny_skia::Mask` built from the path. Either way, mind
   premultiplied vs. straight alpha at the `image`↔`tiny-skia` boundary:
   `tiny-skia` stores premultiplied; convert on the way in and out so colors in
   translucent edge pixels don't darken. A per-pixel straight-alpha multiply of
   the `image::RgbaImage` by the mask's alpha channel avoids the round-trip
   entirely and is the recommended, lowest-risk implementation.
4. Return the masked 1024 `image::RgbaImage`; plan-22-B downsamples it to every
   `.icns` size.

### 4.4 Default-icon note

The embedded default (`assets/default_icon.png`, plan-22-B) is authored as
full-bleed artwork; the squircle is applied at render time, so the committed PNG
need not be pre-shaped. If the default already includes its own background, the
mask simply rounds its corners — still correct.

## Layout / ABI Impact

None. Only the *pixels* inside the already-existing `AppIcon.icns` change (shape
+ margin). No new files, no plist change beyond plan-22-B, no executable change.

## Phases

### Phase 1 — squircle mask hook

- [ ] Add `tiny-skia = "0.11"` to `Cargo.toml` (§4.1).
- [ ] Add geometry constants + `ImageRenderHook::squircle()` replacing
      `identity()` in `src/os/macos/icon.rs` (§4.2–4.3).
- [ ] Unit test: render the mask alone and assert corner pixels of the content
      box are transparent (alpha 0), the center is opaque, and pixels outside the
      MARGIN are fully transparent — proving the squircle+margin geometry.
- [ ] Update the icns structure test (plan-22-B) still passes (sizes/OSTypes
      unchanged; only pixels differ).

Acceptance: `cargo test` green; a rendered 1024 icon has transparent margins and
rounded (squircle) corners; visual check on a macOS host shows a native-looking
Dock icon for both the default and a provided square `icon`.
Commit: —

### Phase 2 — docs + fixture refresh

- [ ] Note the squircle/margin shaping in `src/docs/spec/app/01_macos-runtime.md`
      (icon appearance paragraph).
- [ ] Ensure the app-icon fixture / `scripts/test-macapp.sh` assertions from
      plan-22-B still hold (structure unchanged), adding a note that the icon is
      squircle-masked.

Acceptance: `scripts/test-accept.sh` green; docs describe the shaping.
Commit: —

## Validation Plan

- Function tests: N/A (no package function).
- Runtime proof: on a macOS host, the Dock/Finder icon for an `-app` build shows
  the artwork inside a squircle with correct margins (compare side-by-side with a
  stock macOS app icon).
- Doc sync: `src/docs/spec/app/01_macos-runtime.md` (icon appearance).
- Acceptance: `scripts/test-accept.sh target/debug/mfb target/accept-actual`.

## Open Decisions

1. `tiny-skia` for masking vs. a hand-rolled rounded-rect alpha fill (a squircle
   is just an analytic alpha mask; ~40 lines of per-pixel coverage math with no
   new dependency) — **use `tiny-skia`** (recommended; the user requested it and
   it gives antialiased curves for free) vs. hand-roll to hold the minimal-dep
   line. (§4.1)
2. Squircle fidelity — **rounded-rect approximation** (`CORNER_RADIUS ≈ 0.2237 ×
   body`; recommended, visually indistinguishable at icon sizes and simplest) vs.
   a true `n≈5` superellipse (continuous corners, closer to Apple's grid, more
   code). (§4.3 step 2)
3. Drop shadow — **none by default** (recommended; macOS composites its own
   system shadow behind app icons) vs. bake a subtle contact shadow into the
   artwork. (§Non-goals)
4. Content margin — **824/1024 Big Sur body** (recommended, matches current
   macOS grid) vs. full-bleed-minus-small-radius (older Aqua look). (§4.2)

## Non-Goals

- Per-size bespoke masks (mask once at 1024, downsample).
- Pixel-exact match to Apple's proprietary superellipse.
- Linux/GTK icon shaping.

## Summary

The risk is entirely in the alpha handling at the `image`↔`tiny-skia` boundary
and in the squircle geometry reading as native. Because plan-22-B isolated the
render step behind a single hook, this is a localized swap: replace one function,
add one dependency, add one mask-geometry unit test. Nothing outside
`src/os/macos/icon.rs` changes, and no executable bytes or language semantics are
affected.
