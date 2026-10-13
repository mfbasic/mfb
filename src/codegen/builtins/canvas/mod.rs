//! The built-in `canvas` package (plan-98-B).
//!
//! `canvas` is the 2D drawing surface of `Mode.Canvas` (plan-98-A). Its model is a
//! **retained scene**, not an immediate-mode command stream: a program builds a
//! `List OF DrawItem` and installs it with `canvas::present`, and the runtime redraws
//! that scene when it changes, on resize and on OS damage — never on a timer, and
//! without waiting for the display — until the next `present`. This is
//! a deliberate divergence from `term::`, which is ambient mutation plus a
//! present-diff — a retained scene is what lets the runtime cache per-item geometry
//! on a content hash and make a re-`present` of unchanged content free.
//!
//! Two consequences shape every type here:
//!
//! * **A published scene may point at nothing caller-owned.** `present` deep-copies
//!   transitively into runtime-owned storage, because the render thread reads the
//!   scene at arbitrary times after `present` returns.
//! * **`Paint` is a flat value threaded through items, not ambient state.** Ambient
//!   state interacts badly with a retained scene ("which fill was current when item
//!   47 was appended?").
//!
//! `Image` and `Font` are plain **RES resources** — an owned value holding an
//! integer id, with the standard `closed` flag and scope-drop reclaim, exactly like
//! a file. MFB is not refcounted, so a scene does **not** retain the resources it
//! names: it copies the id only. The closed flag alone ends a resource's life.
//! They are declared in plan-98-B Phase 4 alongside the `destroy*` members that
//! close them, because `add_resource` derives a runtime call from its close op.
//! Until then — and in every published scene thereafter — an item names a resource
//! **directly**: `canvas::Picture.image` is a `RES canvas::Image` and
//! `canvas::Text.font` a `RES canvas::Font`. The scene draws through the resource you
//! still own; close it and that item draws nothing.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
    RegistryResource, RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_blit_surface;
mod func_carried_hashes;
mod func_create_image;
mod func_destroy_font;
mod func_destroy_image;
mod func_did_resize;
mod func_enable_mouse;
mod func_fill;
mod func_fill_stroke;
mod func_flatten;
mod func_frame_nanos;
mod func_geo_build;
mod func_geo_cache;
mod func_get_bytes;
mod func_get_frame_stats;
mod func_get_size;
mod func_graphics;
mod func_group_stats;
mod func_handle_bridge;
mod func_installed_items;
mod func_installed_layers;
mod func_item_hash;
pub(crate) mod func_load_font;
mod func_load_image;
mod func_measure_text;
mod func_metal_draw;
mod func_metal_present;
mod func_new_surface;
mod func_nurbs_to_beziers;
mod func_pointer;
mod func_poll_mouse;
mod func_present;
mod func_present_layers;
mod func_publish_scene;
mod func_remove_group;
mod func_scene_hashes;
mod func_set_bytes;
mod func_set_group;
mod func_stroke;
mod func_system_fonts;
mod func_wait_frame;
mod gen_font;
mod gen_font_table;
mod gen_group;
mod gen_image;
mod gen_present;
mod gen_system_fonts_linux;
mod gen_system_fonts_macos;
mod gen_system_fonts_shared;
mod gen_system_fonts_windows;
mod helper_autopaint;
mod helper_clamp_byte;
mod helper_color;
mod helper_curves;
mod helper_damage;
mod helper_dashes;
mod helper_draw;
mod helper_font;
mod helper_font_name;
mod helper_geometry;
mod helper_glyph;
mod helper_glyph_cache;
mod helper_hit;
mod helper_items;
mod helper_joins;
mod helper_paint_defaults;
mod helper_particles;
mod helper_picture;
mod helper_png;
mod helper_render;
mod helper_shapes;
mod helper_surface;
mod scene_base;

/// plan-94-A: the `MouseEvent` value record `canvas::pollMouse` returns.
///
/// `term` owns an independent record of the same name — deliberately, because
/// `term::` is `Mode.Console`-gated and `canvas::` is `Mode.Canvas`-gated, so a
/// canvas program must never have to `IMPORT term` to ask where the mouse is. The
/// two differ in their coordinate prop: `term`'s is `row`/`column` cells, this one's
/// is a `position AS vector::Float2` in surface pixels.
pub(crate) const MOUSE_EVENT_TYPE: &str = "MouseEvent";

/// The `Image` resource's bare type name, and the package-qualified id members
/// spell in their signatures.
pub(crate) const IMAGE_TYPE: &str = "Image";
pub(crate) const IMAGE_TYPE_ID: &str = "canvas.Image";
/// The close op the resource's scope-drop and `destroyImage` both route to.
const DESTROY_IMAGE: &str = "canvas.destroyImage";

/// The `Font` resource's bare type name, and the package-qualified id members use.
pub(crate) const FONT_TYPE: &str = "Font";
pub(crate) const FONT_TYPE_ID: &str = "canvas.Font";
/// The close op the resource's scope-drop and `destroyFont` both route to.
const DESTROY_FONT: &str = "canvas.destroyFont";

const MODULE_INTRO: &str =
    r#"2D drawing for `app::Mode.Canvas` — a retained scene of `canvas::DrawItem`s"#;
const MODULE_DESC: &str = r#"The `canvas` package draws 2D graphics on the surface `app::setMode(app::Mode.Canvas)`
presents. Import it with `IMPORT canvas`; like `app`, it is importable **only** in
`--app` builds, and every call
that touches the surface requires `app::Mode.Canvas` — outside it they raise the
trappable `ErrWrongMode`. The `canvas::Paint` constructors `canvas::fill`,
`canvas::stroke` and `canvas::fillStroke` are exempt: they build a value and touch
no surface.

Colour is not a canvas concept at all. Those three take a `color::Color`, built with
`color::rgb`, `color::fromHex` or any other `color` member — and `color` is gated by
nothing, in any build. So a program still computes its palette before it ever
presents anything; it just does so through `IMPORT color`.

`canvas` is **retained**, not immediate. A program builds a `List OF canvas::DrawItem` and
installs it with `canvas::present`; the runtime draws that scene, and draws it again
only when the window is resized or uncovered, until the next `present`. `present` is therefore not a
per-frame call — a static picture is presented once and costs nothing thereafter,
and re-presenting an unchanged scene publishes nothing, renders nothing and holds
no memory. (It does still copy the scene once to compare it, so it is not free —
it is bounded.) Animated content does call it every frame, which is why the
runtime caches each item's geometry on a content hash: re-presenting an item that
did not change is free.

An item whose content *did* change costs one geometry build, and nothing beyond
it. The cache holds a bounded number of entries and reuses the space an
evicted entry took, so a program presenting items with new coordinates every
frame reaches a steady memory size — set by how much is on screen, not by how
long it has been running — and can animate indefinitely at that size.

`present` **returns at once**; it never waits for the frame to be drawn. An animation
loop that should run at the display's pace ends each iteration with
`canvas::waitFrame`, and reads the time step to advance by from
`canvas::getFrameStats().dt`:

```
DO WHILE running
  LET s AS canvas::FrameStats = canvas::getFrameStats()
  step(math::min(s.dt, MAX_STEP))
  canvas::present(scene())
  canvas::waitFrame()
LOOP
```

`present` **copies the whole scene** — item fields, polygon rings, text
strings, the `canvas::Paint` values. After it returns, the published scene is entirely
its own, so you are free to change or discard whatever you built the list
from.

Coordinates are pixels with a top-left origin and Y increasing downward. Angles
are radians, measured clockwise from +X (which, under Y-down, is the direction
that makes a `0`..`PI` arc sweep *below* its centre).

Every drawn item carries a `canvas::Paint`, a flat value record rather than ambient
state — there is no "current colour" to set. Build one with `canvas::fill`,
`canvas::stroke` or `canvas::fillStroke`, and refine it with `WITH`:

```
LET glow AS canvas::Paint = WITH canvas::fill(color::rgb(255, 64, 0)) { blend := canvas::BlendMode.Add }
```

`canvas::Paint` is designed so that **each field's zero value is that field's no-op** —
transparent fill and stroke, zero stroke width, `Normal` blend and an empty,
meaning absent, clip. A shape's `transform` follows the same rule: the *all-zero*
`canvas::Transform` is the identity, by definition. That is what lets `canvas::fill(red)` mean simply "a red
shape": every field the caller did not name is already inert. The transparent
colour is the all-zero `color::Color`, so this holds for `fill` and `stroke` too.

An item that draws an image or text holds the resource itself — `canvas::Picture.image`
is a `RES canvas::Image` and `canvas::Text.font` a `RES canvas::Font`. A published scene
may still outlive what it names: closing an image or font a scene still draws makes that
item draw nothing, rather than failing the frame.

`Image` is a resource, so it is bound with `RES` and named
**package-qualified**, exactly like `fs::File`:

```
RES logo AS canvas::Image = canvas::createImage(w, h, pixels)
```

The value types — `canvas::DrawItem`, `canvas::Paint` and the rest — are
package-qualified too; a bare `DrawItem` is not a known type.
The one colour type a canvas program names, `color::Color`, belongs to `color` and
needs its own `IMPORT color`.
An image closes itself when its binding goes out of scope, or earlier with
`canvas::destroyImage`; destroying one that a presented scene still draws is
safe. Calling `canvas::destroyImage` again on an image it already closed raises
`ErrResourceClosed`, as does any other use of a closed image. An image
stays on the drawing surface's thread."#;

/// Register the `canvas` package on the clean-room registry.
///
/// The type set is declared here as registry data (`add_record` / `add_union` /
/// `add_enum`); there is no `.mfb` companion source.
///
/// **The `DrawItem` variant set is closed.** Adding a variant later is a breaking
/// change — a user's `SELECT CASE` over the union stops being exhaustive — so the
/// full set is frozen here rather than shipped as a subset and extended
/// (plan-98-A invariant 6).
///
/// It has been extended **once**, deliberately: plan-116-E appended `Ellipse` as the
/// ninth, last, so no existing variant's tag moved. `draw_item_variant_set_is_frozen`
/// pins the list and its order, and is what makes a tenth exactly as visible as the
/// ninth was.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("canvas", MODULE_INTRO, MODULE_DESC);
    // The companion source needs `collections` (the surface is a `List OF Byte`),
    // `math` (`sqrt`, the one transcendental-free primitive the distance functions
    // use), `os`/`fs` (the headless frame dump), and `canvas` itself — a package
    // reaches its own internal-only members through the qualified spelling, exactly
    // as `astrings` reaches `astrings::readSpans`.
    // `color` (plan-122-B): the blend and gradient helpers convert through
    // `color::toLinear`/`color::fromLinear` rather than a canvas-local sRGB table.
    // canvas already has a non-empty companion and already pays a companion cost, so
    // this adds `color`'s 33,024 bytes but does not change canvas's cost *class*.
    // `compress` (plan-137-C): the PNG decoder inflates IDAT through
    // `compress::zlibDecode`; its gated helpers ride in by `compress`'s late pass.
    pkg.add_imports(vec![
        "canvas",
        "collections",
        "color",
        "compress",
        "math",
        "os",
        "fs",
        "encoding",
        "bits",
        // plan-172-B: every coordinate pair is a `vector::Float2`. Injected by
        // `vector`'s late pass, because a canvas program need not import it. This is
        // the largest import here by far: `vector` has no gated helpers, so its whole
        // companion — nine types and every overload — comes with it. Measured on
        // macOS aarch64 for a program that presents an empty scene: 2,666,000 bytes
        // without it, 2,996,236 with (+330,236), the same size a program that
        // imports `vector` itself already was.
        "vector",
    ]);

    // ---- Value types the items are built from -----------------------------

    // plan-122-D retired `canvas::Color`. The colour type is `color::Color`, an
    // ordinary value record owned by `color` with an identical field set — so the
    // props below reference it by qualified type id, exactly as `tcp` references
    // `net.Address`, and every canvas internal that reads `paint.fill.red` is
    // unchanged. Colour construction lives in `color` and needs no `Mode.Canvas`.
    pkg.add_record(RegistryRecord {
        name: MOUSE_EVENT_TYPE,
        export: true,
        description: "One thing the mouse did, as reported by `canvas::pollMouse`. \
                      Test `kind` against `canvas::MouseKind.None` first: that is \
                      what an idle poll reports, and the rest of the record is \
                      meaningless when it does.",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::named("MouseKind"),
                description: "What happened — a press, a release, motion, a drag, or \
                              a wheel turn. `None` when nothing is pending.",
            },
            RecordProp {
                name: "button",
                ty: ParameterType::named("MouseButton"),
                description: "Which button the event is about. `None` for motion and \
                              wheel events, which belong to no button.",
            },
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the mouse was, in surface pixels — the same \
                              top-left-origin coordinates every `canvas::DrawItem` \
                              uses, so it hands straight to a hit test.",
            },
            RecordProp {
                name: "shift",
                ty: ParameterType::Boolean,
                description: "`TRUE` when Shift was held as the event happened.",
            },
            RecordProp {
                name: "ctrl",
                ty: ParameterType::Boolean,
                description: "`TRUE` when Control was held as the event happened.",
            },
            RecordProp {
                name: "alt",
                ty: ParameterType::Boolean,
                description: "`TRUE` when Alt (Option) was held as the event happened.",
            },
            RecordProp {
                name: "overID",
                ty: ParameterType::String,
                description: "Which of your items the mouse was over: the `id` of the \
                              topmost item under `position` in the scene you last \
                              presented, tested against the item's own outline after \
                              every transform. An item inside a group is reported with \
                              the `id` of each group that holds it in front, outermost \
                              first, joined by `::` — `\"toolbar::save::label\"` — and \
                              a part that is `\"\"` is left out. A \
                              `canvas::Picture`'s `id` goes in front of its shape's in \
                              the same way, and a `canvas::AutoPaint` reports the \
                              `id` of the shape it holds. `\"\"` when nothing is \
                              there, or when nothing on the way to the topmost item \
                              has an `id`.",
            },
        ],
    });

    // `None` MUST stay declared first in both enums: ordinal 0 is what makes the
    // all-zero record read as "nothing pending, no button", which is the sentinel
    // an idle `canvas::pollMouse` returns.
    pkg.add_enum(RegistryEnum {
        name: "MouseKind",
        export: true,
        variants: vec![
            EnumVariant {
                name: "None",
                description: "Nothing is pending. What an idle `canvas::pollMouse` \
                              reports, and what every call reports before \
                              `canvas::enableMouse(TRUE)`. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Down",
                description: "A button was pressed. `button` says which.",
                advisory: None,
            },
            EnumVariant {
                name: "Up",
                description: "A button was released. `button` says which.",
                advisory: None,
            },
            EnumVariant {
                name: "Move",
                description: "The mouse moved with no button held. `button` is \
                              `None`.",
                advisory: None,
            },
            EnumVariant {
                name: "Drag",
                description: "The mouse moved with a button held. `button` says \
                              which one is down.",
                advisory: None,
            },
            EnumVariant {
                name: "ScrollUp",
                description: "The wheel turned away from the user. `button` is \
                              `None`.",
                advisory: None,
            },
            EnumVariant {
                name: "ScrollDown",
                description: "The wheel turned toward the user. `button` is `None`.",
                advisory: None,
            },
        ],
    });
    pkg.add_enum(RegistryEnum {
        name: "MouseButton",
        export: true,
        variants: vec![
            EnumVariant {
                name: "None",
                description: "No button — what motion and wheel events report. The \
                              zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Left",
                description: "The primary button.",
                advisory: None,
            },
            EnumVariant {
                name: "Middle",
                description: "The middle button, usually the wheel pressed down.",
                advisory: None,
            },
            EnumVariant {
                name: "Right",
                description: "The secondary button.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Size",
        export: true,
        description: "A pixel extent — the dimensions of the canvas surface, of an \
                      image, or of a `canvas::Rectangle`. The two are `Float`s because \
                      a shape's extent may be a fraction of a pixel; the surface's and \
                      an image's are always whole numbers.",
        props: vec![
            RecordProp {
                name: "width",
                ty: ParameterType::Float,
                description: "The width in pixels.",
            },
            RecordProp {
                name: "height",
                ty: ParameterType::Float,
                description: "The height in pixels.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "TextMetrics",
        export: true,
        description: "The measured extent of a string in a given font and size, as \
                      returned by `canvas::measureText` — available without drawing \
                      anything.",
        props: vec![
            RecordProp {
                name: "width",
                ty: ParameterType::Float,
                description: "The advance width of the whole string in pixels.",
            },
            RecordProp {
                name: "height",
                ty: ParameterType::Float,
                description: "The line height in pixels (`ascent + descent + lineGap`).",
            },
            RecordProp {
                name: "ascent",
                ty: ParameterType::Float,
                description: "Pixels from the baseline to the top of the tallest glyph.",
            },
            RecordProp {
                name: "descent",
                ty: ParameterType::Float,
                description: "Pixels from the baseline down to the bottom of the \
                              lowest glyph, as a positive number.",
            },
            RecordProp {
                name: "lineGap",
                ty: ParameterType::Float,
                description: "The font's recommended extra leading between lines, in pixels.",
            },
        ],
    });

    pkg.add_enum(RegistryEnum {
        name: "BlendMode",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Normal",
                description: "Source-over compositing — the ordinary case, and the \
                              zero value, so an unset `canvas::Paint.blend` is this.",
                advisory: None,
            },
            EnumVariant {
                name: "Multiply",
                description: "Multiply source and destination; darkens. Multiplying by white leaves the destination alone, and by black gives black.",
                advisory: None,
            },
            EnumVariant {
                name: "Screen",
                description: "Inverse-multiply source and destination; lightens. Screening with black leaves the destination alone, and with white gives white.",
                advisory: None,
            },
            EnumVariant {
                name: "Add",
                description: "Add the source to the destination, clamped; the usual choice for glows. A partly-covered pixel adds proportionally less.",
                advisory: None,
            },
        ],
    });

    pkg.add_enum(RegistryEnum {
        name: "GradientKind",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Linear",
                description: "Interpolate along the line from `startPoint` to \
                              `endPoint`. A point's position on the ramp is its \
                              distance ALONG that line, so the ramp runs in the \
                              direction of the axis and is constant across it. The \
                              zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Radial",
                description: "Interpolate outward from `startPoint`, reaching the last \
                              stop on the circle through `endPoint`. A point's \
                              position on the ramp is its distance FROM the centre \
                              over that radius, so the ramp is circular whatever shape \
                              it fills — an ellipse's radial gradient does not become \
                              elliptical unless the gradient's own `transform` \
                              stretches it. `focalOffset` and `focalRadius` move where \
                              the ramp starts off the centre.",
                advisory: None,
            },
        ],
    });

    pkg.add_enum(RegistryEnum {
        name: "GradientSpread",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Pad",
                description: "Past either end of the ramp, keep that end's colour: the \
                              first stop before the start and the last stop after the \
                              end. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Reflect",
                description: "Past the end, run the ramp backwards to the start, then \
                              forwards again, and so on, so the colours mirror at each \
                              end with no seam.",
                advisory: None,
            },
            EnumVariant {
                name: "Repeat",
                description: "Past the end, start the ramp again from the first stop, \
                              so it repeats as stripes or rings with a hard edge where \
                              the last stop meets the first.",
                advisory: None,
            },
        ],
    });

    pkg.add_enum(RegistryEnum {
        name: "GradientApply",
        export: true,
        variants: vec![
            EnumVariant {
                name: "FillOnly",
                description: "The ramp colours the item's interior and its outline \
                              stays the flat `canvas::Paint.stroke` colour. A \
                              `canvas::Line` or `canvas::Arc`, which has no interior, \
                              stays flat. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "FillAndStroke",
                description: "The ramp colours the outline too, tinted by \
                              `canvas::Paint.stroke` as a `canvas::Picture` is tinted \
                              by its fill: the stroke colour's alpha is how strongly it \
                              tints, so a white stroke, or any stroke colour with alpha \
                              0, shows the plain ramp. A `canvas::Line` or \
                              `canvas::Arc` is drawn in the tinted ramp.",
                advisory: None,
            },
        ],
    });

    // plan-155-B: what drew a frame, and the timing record `canvas::getFrameStats`
    // returns. The variant order is the ordinal `canvas::frameDone` records, so it is
    // frozen: Software first, because it is also what a canvas reports before its first
    // frame.
    pkg.add_enum(RegistryEnum {
        name: "Renderer",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Software",
                description: "Drawn on the CPU. The default for a program with no window, and what a canvas reports before its first frame. On Linux and Windows it also draws a window's first frames, while the GPU is being made ready.",
                advisory: None,
            },
            EnumVariant {
                name: "Metal",
                description: "Drawn on the GPU through Metal (macOS).",
                advisory: None,
            },
            EnumVariant {
                name: "Vulkan",
                description: "Drawn on the GPU through Vulkan (Linux and Windows). A window's frames change from `Software` to `Vulkan` once the GPU is ready, which can be many seconds into the first run of a program on a machine.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "FrameStats",
        export: true,
        description: "The canvas's timing at one moment, as `canvas::getFrameStats` \
                      returns it. Every field is from the same instant, so two of them \
                      can be compared.",
        props: vec![
            RecordProp {
                name: "dt",
                ty: ParameterType::Float,
                description: "Seconds between the last two `canvas::present` calls — the \
                              step to advance an animation by. `0.0` until the second \
                              present.",
            },
            RecordProp {
                name: "pps",
                ty: ParameterType::Float,
                description: "Presents in the last second, identical ones included. Falls \
                              to `0.0` a second after presenting stops.",
            },
            RecordProp {
                name: "fps",
                ty: ParameterType::Float,
                description: "Frames drawn in the last second. A present of an unchanged \
                              scene draws no frame, so a static scene reports `0.0`.",
            },
            RecordProp {
                name: "displayHz",
                ty: ParameterType::Float,
                description: "The refresh rate of the monitor the window is on, in hertz. \
                              `0.0` when it is not known.",
            },
            RecordProp {
                name: "frameSeconds",
                ty: ParameterType::Float,
                description: "How long the last drawn frame took to draw, in seconds. \
                              `0.0` before the first frame.",
            },
            RecordProp {
                name: "presents",
                ty: ParameterType::Integer,
                description: "Every `canvas::present` and `canvas::presentLayers` call \
                              since the program started.",
            },
            RecordProp {
                name: "frames",
                ty: ParameterType::Integer,
                description: "Every frame drawn since the program started.",
            },
            RecordProp {
                name: "renderer",
                ty: ParameterType::named("Renderer"),
                description: "What drew the last frame. `Software` before the first.",
            },
        ],
    });

    pkg.add_enum(RegistryEnum {
        name: "CapStyle",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Butt",
                description: "Cut square at the endpoint, so the stroke stops exactly where the item says it does. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Round",
                description: "Extend past the endpoint by a half-disc of the stroke's half-width, so a thick line ends in a dome rather than a corner.",
                advisory: None,
            },
            EnumVariant {
                name: "Square",
                description: "Extend past the endpoint by half the stroke's width and cut square there, so the stroke ends in a corner a little beyond the point.",
                advisory: None,
            },
        ],
    });

    pkg.add_enum(RegistryEnum {
        name: "JoinStyle",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Round",
                description: "Round the outside of the corner with an arc of the stroke's half-width. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Miter",
                description: "Extend the two edges' outlines until they meet in a point. A corner sharp enough that the point would reach past `canvas::Paint.miterLimit` is drawn as `Bevel` instead.",
                advisory: None,
            },
            EnumVariant {
                name: "Bevel",
                description: "Cut the corner off straight across, joining the two edges' outer outlines with one line.",
                advisory: None,
            },
        ],
    });

    pkg.add_enum(RegistryEnum {
        name: "FillRule",
        export: true,
        variants: vec![
            EnumVariant {
                name: "EvenOdd",
                description: "A point is inside — filled by a `canvas::Polygon`, \
                              kept by a `canvas::Paint.clip` — when a line from it \
                              out to infinity crosses the rings an odd number of \
                              times. A ring inside another is a hole, whichever way \
                              either runs. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "NonZero",
                description: "A point is inside — filled, or kept by a clip — when \
                              the rings wind around it a non-zero number of times. A ring inside another is \
                              more of the fill when both run the same way, and a \
                              hole when they run opposite ways. SVG paths and \
                              TrueType outlines are drawn this way.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Transform",
        export: true,
        description: "A 2×3 affine transform, applied as \
                      `x' = a*x + c*y + tx`, `y' = b*x + d*y + ty`. **The all-zero \
                      value means the identity**, not the degenerate matrix that \
                      collapses every point to the origin — which is what lets an \
                      unset `transform` leave an item untransformed. \
                      A rotation by θ is `a := cos(θ), b := sin(θ), c := -sin(θ), \
                      d := cos(θ)`; a scale is `a := sx, d := sy` with `b` and `c` \
                      zero; an X shear is `c := tan(θ)`. Every shape is turned and scaled \
                      about its own `position` — for a `canvas::Rectangle`, the \
                      point its `bind` names — so a rotation spins it in place, and \
                      `tx` and `ty` move it from there. A `canvas::Group` is turned \
                      about its `position` in the same way, and everything in it \
                      turns with it.",
        props: vec![
            RecordProp {
                name: "a",
                ty: ParameterType::Float,
                description: "Row 0, column 0 — X scale.",
            },
            RecordProp {
                name: "b",
                ty: ParameterType::Float,
                description: "Row 1, column 0 — Y shear.",
            },
            RecordProp {
                name: "c",
                ty: ParameterType::Float,
                description: "Row 0, column 1 — X shear.",
            },
            RecordProp {
                name: "d",
                ty: ParameterType::Float,
                description: "Row 1, column 1 — Y scale.",
            },
            RecordProp {
                name: "tx",
                ty: ParameterType::Float,
                description: "The X translation in pixels.",
            },
            RecordProp {
                name: "ty",
                ty: ParameterType::Float,
                description: "The Y translation in pixels.",
            },
        ],
    });

    // `ImageRef` and `FontRef` used to be declared here, and the comment above them
    // explained why: a record field could not hold a resource, so the scene named one
    // through a plain `Integer` handle instead.
    //
    // plan-114-D lifted that ban and plan-116-I removed the workaround. `Picture.image`
    // and `Text.font` now hold the **resource itself**. What the handle bought — a
    // published scene having no opinion on the resource's lifetime — is preserved
    // rather than given up: the renderer reads the backend id through
    // `canvas::imageHandle`/`fontHandle`, which answer `0` for a destroyed resource
    // instead of raising, so an item naming something the program has since closed
    // draws nothing exactly as a zero handle did.
    //
    // What changed for a caller is that the compiler now knows a scene names a
    // resource. That is the point: `canvas::Picture[id := "", image := imageRef(img), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]` could
    // outlive `img` silently, and `image := img` cannot.

    pkg.add_record(RegistryRecord {
        name: "GradientStop",
        export: true,
        description: "One colour stop of a `canvas::Gradient`. Stops are used **in \
                      the order you give them** — they are not sorted, because \
                      reordering them would silently redraw something other than what \
                      the program asked for. An `offset` outside `0.0`..`1.0`, or one \
                      that goes backwards, is clamped instead: visible and \
                      predictable.",
        props: vec![
            RecordProp {
                name: "offset",
                ty: ParameterType::Float,
                description: "Where along the gradient this colour sits, `0.0` at the \
                              start and `1.0` at the end.",
            },
            RecordProp {
                name: "color",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The colour at that offset. A `color::Color` — the \
                              program needs `IMPORT color` to name the type.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Gradient",
        export: true,
        description: "A colour ramp used as an item's interior instead of a flat \
                      `canvas::Paint.fill`, and with `strokeApply` its outline too. \
                      **Fewer than two stops is not a gradient** — the item falls \
                      back to `fill`, so the all-zero value is inert like every other \
                      `canvas::Paint` field. One stop is a flat colour you should \
                      write as `canvas::fill`, and zero stops name no colour at all. \
                      The ramp is placed in the item's own coordinates, from its own \
                      two points and never from the shape's bounds: the gradient's \
                      `transform` maps it first, then the item's `transform` and \
                      any group's, so a rotated or moved item carries its ramp \
                      with it. Colours between \
                      two stops are mixed in **linear light**, the same space \
                      everything else on the surface is blended in, so a \
                      black-to-white ramp is evenly bright across its width rather \
                      than dark for most of it — see `mfb spec app canvas`.",
        props: vec![
            RecordProp {
                name: "kind",
                ty: ParameterType::named("GradientKind"),
                description: "Linear along an axis, or radial outward from a centre.",
            },
            RecordProp {
                name: "startPoint",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Linear: where the ramp starts. Radial: the centre.",
            },
            RecordProp {
                name: "endPoint",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Linear: where the ramp ends. Radial: a point on the \
                              outer circle. Giving the same point as `startPoint` \
                              leaves the first stop's colour everywhere, rather than \
                              failing.",
            },
            RecordProp {
                name: "stops",
                ty: ParameterType::list_of(ParameterType::named("GradientStop")),
                description: "The colours along the ramp, in the order given.",
            },
            RecordProp {
                name: "spread",
                ty: ParameterType::named("GradientSpread"),
                description: "What the ramp does past its ends. `Pad`, the zero value, \
                              keeps the end colours; `Reflect` mirrors the ramp and \
                              `Repeat` starts it again.",
            },
            RecordProp {
                name: "transform",
                ty: ParameterType::named("Transform"),
                description: "Maps the ramp before the item's own \
                              an item's `transform` does: scale, turn or shear the \
                              gradient without touching the shape. Scaling a radial \
                              gradient by different amounts in x and y makes it \
                              elliptical. The all-zero value is the identity, and a \
                              transform that flattens the ramp to a line is treated as \
                              the identity, as for an item's `transform`.",
            },
            RecordProp {
                name: "focalOffset",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Radial only: where the ramp starts, as an offset from \
                              `startPoint` — an off-centre highlight on a sphere. The \
                              zero value starts it at the centre. An offset that would \
                              put the focus on or outside the outer circle is pulled \
                              just inside it.",
            },
            RecordProp {
                name: "focalRadius",
                ty: ParameterType::Float,
                description: "Radial only: the radius of the circle at the focus that \
                              the ramp starts from, so the first stop's colour fills a \
                              disc rather than a point. The zero value is a point. It \
                              is held between `0.0` and just under the outer radius.",
            },
            RecordProp {
                name: "strokeApply",
                ty: ParameterType::named("GradientApply"),
                description: "Whether the ramp colours the outline as well as the \
                              interior. `FillOnly`, the zero value, leaves the outline \
                              the flat `canvas::Paint.stroke`; `FillAndStroke` paints \
                              it with the ramp tinted by the stroke colour, as a \
                              `canvas::Picture` is tinted by its fill.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Paint",
        export: true,
        description: "How an item is filled, stroked, blended and clipped. \
                      A flat value threaded through each item — there is no \
                      ambient drawing state. Every field's zero value is that \
                      field's no-op — `miterLimit` aside, which only a `Miter` join \
                      reads — and `canvas::fill`, `canvas::stroke` and \
                      `canvas::fillStroke` leave every field they do not name at that \
                      zero, apart from a `miterLimit` of `4.0`, so `canvas::fill(c)` is \
                      a plain filled shape. Building a `canvas::Paint` by name must \
                      list every field, so start from one of those three and refine it \
                      with `WITH`.",
        props: vec![
            RecordProp {
                name: "fill",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The interior colour, a `color::Color`. Transparent (the \
                              zero `color::Color`) leaves the item unfilled.",
            },
            RecordProp {
                name: "stroke",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The outline colour, a `color::Color`. Transparent (the \
                              zero `color::Color`) leaves the item unstroked.",
            },
            RecordProp {
                name: "strokeWidth",
                ty: ParameterType::Float,
                description: "The outline width in pixels. `0.0` draws no outline \
                              regardless of `stroke`.",
            },
            RecordProp {
                name: "blend",
                ty: ParameterType::named("BlendMode"),
                description: "How the item composites onto what is already there. \
                              The zero value is `Normal`, ordinary source-over. \
                              Compositing happens on linear light, so `Multiply` of \
                              mid grey with itself is darker than the sRGB byte \
                              arithmetic would suggest. An item that both fills and \
                              strokes applies the mode twice — the fill onto what is \
                              beneath, then the outline onto that.",
            },
            RecordProp {
                name: "clip",
                ty: ParameterType::list_of(ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                ))),
                description: "Restricts drawing to the area these rings enclose, \
                              under `clipRule` — each ring a closed path of points, \
                              as a `canvas::Polygon`'s `contours` are. The empty list, \
                              the zero value, means no clipping. A clip that encloses \
                              nothing, such as rings of fewer than two points, hides \
                              the whole item. The rings are in the item's own \
                              coordinates, measured from its `position`: the \
                              item's `transform` and any group's apply to the clip \
                              as well as to the item, so a clipped item moves with \
                              its clip. Its edges may \
                              fall between pixels, and a partly-covered pixel is \
                              drawn partly, exactly as a shape's own edge is.",
            },
            RecordProp {
                name: "clipRule",
                ty: ParameterType::named("FillRule"),
                description: "Which points the `clip` rings enclose, as a \
                              `canvas::Polygon`'s `rule` decides its fill: \
                              `EvenOdd`, the zero value, makes a ring inside another \
                              a hole; `NonZero` makes it a hole only when the two \
                              run opposite ways.",
            },
            RecordProp {
                name: "gradient",
                ty: ParameterType::named("Gradient"),
                description: "Fill the item with a colour ramp instead of the flat \
                              `fill`. A gradient with **fewer than two stops is \
                              ignored** and `fill` is used, which is what makes the \
                              zero value inert like every other field here. The ramp \
                              is in the item's own coordinates, so `transform` carries \
                              it: rotating an item turns its ramp with it. The outline \
                              stays the flat `stroke` colour unless the gradient's \
                              `strokeApply` is `FillAndStroke`. A `canvas::Text` item \
                              ignores this field, because a glyph is drawn from a \
                              cached coverage bitmap rather than from a distance \
                              field; a `canvas::PolyLine` takes its colours from \
                              `colors` and a `canvas::Picture` from its image.",
            },
            RecordProp {
                name: "cap",
                ty: ParameterType::named("CapStyle"),
                description: "How each open end of the stroke is shaped: the two ends \
                              of a `canvas::Line`, `canvas::Arc` or `canvas::PolyLine`, \
                              and both ends of every dash. `Butt`, the zero value, \
                              stops the stroke at the end; `Round` adds a half-disc \
                              and `Square` half a square. A closed outline with no \
                              dashes has no ends, so it ignores this.",
            },
            RecordProp {
                name: "join",
                ty: ParameterType::named("JoinStyle"),
                description: "How the stroke turns a corner: where two edges of a \
                              `canvas::Polygon` ring or a `canvas::PolyLine` meet, and \
                              at the four corners of a `canvas::Rectangle`. \
                              `Round`, the zero value, rounds the outside of the \
                              corner. Circles, ellipses, arcs and a rectangle with \
                              rounded corners have no corners and ignore it.",
            },
            RecordProp {
                name: "miterLimit",
                ty: ParameterType::Float,
                description: "How sharp a `Miter` corner may be before it is drawn as \
                              `Bevel` instead, as the longest the corner's point may \
                              reach measured in stroke widths: a corner of interior \
                              angle θ reaches `1 / sin(θ / 2)`. `canvas::fill`, \
                              `canvas::stroke` and `canvas::fillStroke` set `4.0`, \
                              which bevels corners sharper than about 29°. A value \
                              below `1.0`, the zero value included, bevels every \
                              corner.",
            },
            RecordProp {
                name: "dashes",
                ty: ParameterType::list_of(ParameterType::Float),
                description: "Alternating dash and gap lengths along the outline, in \
                              pixels before `transform`, starting with a dash. An odd \
                              number of lengths is repeated once to make it even. An \
                              empty list — the zero value — draws a solid stroke, and \
                              so does a list whose lengths add up to `0.0` or that \
                              holds a negative length. `cap` shapes both ends of \
                              every dash. The pattern runs along the outline from its \
                              start: a `canvas::Line`'s first point, a \
                              `canvas::PolyLine`'s first point, an `canvas::Arc`'s \
                              `startAngle`, each `canvas::Polygon` ring's own first \
                              point, a `canvas::Rectangle`'s top-left corner — or, when \
                              that corner is rounded, its top edge where it leaves the \
                              corner — and a `canvas::Circle`'s or `canvas::Ellipse`'s \
                              rightmost point (before its `transform`) — going clockwise \
                              round every closed shape.",
            },
            RecordProp {
                name: "dashOffset",
                ty: ParameterType::Float,
                description: "How far into the `dashes` pattern the outline starts, in \
                              pixels. It may be negative or longer than the pattern; \
                              the pattern repeats in both directions.",
            },
            RecordProp {
                name: "transparency",
                ty: ParameterType::Float,
                description: "How see-through the whole item is: `0.0`, the zero \
                              value, is opaque and `1.0` invisible, with values \
                              outside that range treated as the nearer end. It fades \
                              the fill, the stroke, a fill gradient, text and a \
                              picture alike. The fill and the stroke fade \
                              separately, so where they overlap the fill shows \
                              through a faded stroke.",
            },
        ],
    });

    // ---- The `DrawItem` variants (a CLOSED set) ---------------------------

    pkg.add_enum(RegistryEnum {
        name: "BindTo",
        export: true,
        variants: vec![
            EnumVariant {
                name: "TopLeft",
                description: "`position` is the rectangle's top-left corner. The zero \
                              value.",
                advisory: None,
            },
            EnumVariant {
                name: "Center",
                description: "`position` is the rectangle's centre, so a rotation \
                              spins it in place.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Rectangle",
        export: true,
        description: "A rectangle, with a radius for each corner. `position` places \
                      it by the point `bind` names — its top-left corner, or its \
                      centre — and its `transform` turns it about that same point: a \
                      `Center` rectangle spins in place, a `TopLeft` one swings about \
                      its corner. Every field has a usable default, so the short way to \
                      make one is to declare it and set what you need: \
                      `MUT box AS canvas::Rectangle`, then `WITH box { position := \
                      vector::Float2[x := 20.0, y := 20.0], size := canvas::Size[width \
                      := 120.0, height := 80.0], paint := canvas::fill(ink) }` — it \
                      binds top-left, has four square corners, no `id` and the \
                      identity transform.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the rectangle is, in pixels: the point `bind` \
                              names. The transform turns the rectangle about it.",
            },
            RecordProp {
                name: "bind",
                ty: ParameterType::named("BindTo"),
                description: "Which point of the rectangle `position` is.",
            },
            RecordProp {
                name: "size",
                ty: ParameterType::named("Size"),
                description: "The width and height in pixels. A rectangle with no \
                              width or no height draws nothing.",
            },
            RecordProp {
                name: "radiusTL",
                ty: ParameterType::Float,
                description: "The top-left corner's radius. `0.0` or less is a square corner. Above `0.0` through \
                              `1.0` it is a fraction of the half-size, so the corner is \
                              an ellipse quadrant: `radiusTL * width / 2` across and \
                              `radiusTL * height / 2` down. Above `1.0` it is pixels, and \
                              the corner is circular, no larger than half the shorter \
                              side. `1.0` is the full pill and `1.01` is a one-pixel \
                              corner, so a radius animated in pixels starts above `1.0`.",
            },
            RecordProp {
                name: "radiusTR",
                ty: ParameterType::Float,
                description: "The top-right corner's radius. See `radiusTL` for how its size chooses its meaning.",
            },
            RecordProp {
                name: "radiusBL",
                ty: ParameterType::Float,
                description: "The bottom-left corner's radius. See `radiusTL` for how its size chooses its meaning.",
            },
            RecordProp {
                name: "radiusBR",
                ty: ParameterType::Float,
                description: "The bottom-right corner's radius. See `radiusTL` for how its size chooses its meaning.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Line",
        export: true,
        description: "A straight segment from one point to another. Its two ends, \
                      `startAt` and `endAt`, are measured from `position`, so moving \
                      the line is changing `position` alone, and its `transform` turns \
                      it about `position` — put `position` at the middle and the line \
                      spins in place. A line has no interior, so it is drawn from \
                      `paint.stroke`/`paint.strokeWidth` and ignores `paint.fill`; \
                      `paint.cap` shapes its two ends. Every field has a usable \
                      default, so the short way to make one is to declare it and set \
                      what you need: `MUT rule AS canvas::Line`, then `WITH rule { \
                      startAt := vector::Float2[x := 20.0, y := 40.0], endAt := \
                      vector::Float2[x := 220.0, y := 40.0], paint := \
                      canvas::stroke(ink, 2.0) }` — `position` stays at the surface's \
                      top-left corner, so the two ends are surface pixels.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point the two ends are measured from, in pixels. \
                              The transform turns the line about it.",
            },
            RecordProp {
                name: "startAt",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the line starts, as an offset from `position`.",
            },
            RecordProp {
                name: "endAt",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the line ends, as an offset from `position`.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Polygon",
        export: true,
        description: "A filled shape bounded by one or more closed rings of \
                      points. Several rings make one shape: a ring inside another \
                      can be a hole (an \"O\", a donut) or more of the fill, as \
                      `rule` decides. The stroke outlines every ring. A ring of \
                      fewer than three points has no area and adds only its \
                      stroke; a ring of fewer than two adds nothing. Every point is \
                      measured from `position`, so moving the shape is changing \
                      `position` alone, and its `transform` turns it about \
                      `position`. Every field has a usable default, so the short way \
                      to make one is to declare it and set what you need: `MUT wedge \
                      AS canvas::Polygon`, then `WITH wedge { position := \
                      vector::Float2[x := 120.0, y := 80.0], contours := [[ \
                      vector::Float2[x := 0.0, y := 0.0 - 40.0], vector::Float2[x := \
                      40.0, y := 40.0], vector::Float2[x := 0.0 - 40.0, y := 40.0]]], \
                      paint := canvas::fill(ink) }` — a triangle around 120, 80.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every vertex is measured from, in pixels. \
                              The transform turns the shape about it.",
            },
            RecordProp {
                name: "contours",
                ty: ParameterType::list_of(ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                ))),
                description: "The rings, each a list of vertices in order, each \
                              vertex an offset from `position`. Every \
                              ring closes from its own last point back to its own \
                              first point automatically. The direction a ring runs \
                              in matters only under `FillRule.NonZero`.",
            },
            RecordProp {
                name: "rule",
                ty: ParameterType::named("FillRule"),
                description: "Which points the rings enclose: `EvenOdd` makes a ring \
                              inside another a hole whichever way it runs, and \
                              `NonZero` makes it a hole only when it runs the \
                              opposite way to the ring around it.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Circle",
        export: true,
        description: "A circle given by its centre and radius. Every field has a usable \
                      default, so the short way to make one is to declare it and set \
                      what you need: `MUT dot AS canvas::Circle`, then \
                      `WITH dot { position := vector::Float2[x := 80.0, y := 60.0], \
                      radius := 20.0, paint := canvas::fill(ink) }`. `id` stays empty \
                      and `transform` stays the identity.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The centre, in pixels. The transform turns the circle about it.",
            },
            RecordProp {
                name: "radius",
                ty: ParameterType::Float,
                description: "The radius in pixels.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Arc",
        export: true,
        description: "A circular arc — the part of a circle's outline between two \
                      angles. Angles are in **radians, measured clockwise from +X**; \
                      because Y increases downward, a `0.0`..`PI` arc sweeps below \
                      its centre (so that is the smile, not the frown). An arc has \
                      no interior, so it is drawn from `paint.stroke`; `paint.cap` \
                      shapes its two ends. It is shortest made from its default: \
                      `MUT bow AS canvas::Arc`, then `WITH bow { position := …, \
                      radius := …, startAngle := …, endAngle := …, paint := … }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description:
                    "The centre of the circle the arc lies on, in pixels. The transform turns \
                              the arc about it.",
            },
            RecordProp {
                name: "radius",
                ty: ParameterType::Float,
                description: "The radius in pixels.",
            },
            RecordProp {
                name: "startAngle",
                ty: ParameterType::Float,
                description: "Where the arc begins, in radians clockwise from +X.",
            },
            RecordProp {
                name: "endAngle",
                ty: ParameterType::Float,
                description: "Where the arc ends, in radians clockwise from +X.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Text",
        export: true,
        description: "A run of text drawn in a font at a size. `position` is the \
                      start of the baseline, not the top-left corner — use \
                      `canvas::measureText` to lay text out without drawing it.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description:
                    "Where the baseline starts, in pixels: its left end. The transform turns \
                              the text about it.",
            },
            RecordProp {
                name: "text",
                ty: ParameterType::String,
                description: "The text to draw.",
            },
            RecordProp {
                name: "font",
                ty: ParameterType::res(ParameterType::named("canvas.Font")),
                description: "The font to draw it in. The item holds the font \
                              itself — you still close it, and closing it while a \
                              scene still names it draws nothing rather than \
                              failing.",
            },
            RecordProp {
                name: "fontSize",
                ty: ParameterType::Float,
                description: "The em size in pixels.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-172-L: how a picture's image is placed in its shape.
    pkg.add_enum(RegistryEnum {
        name: "PictureClip",
        export: true,
        variants: vec![
            EnumVariant {
                name: "ClipOnly",
                description: "The shape only cuts the image out. The image stays \
                              upright however the shape is turned, fitted to the box \
                              around the shape as it sits on the canvas. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "ClipAndTransform",
                description: "The image is fitted to the shape as it was built, and \
                              the shape's transform carries the image with it: turn \
                              the shape and the image turns too.",
                advisory: None,
            },
        ],
    });
    pkg.add_enum(RegistryEnum {
        name: "PictureFit",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Stretch",
                description: "The image is scaled to the box around the shape, each \
                              way separately, so it fills it and may be squashed. The \
                              zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Contain",
                description: "The image keeps its proportions and is as large as fits \
                              inside the box, centred. The rest of the shape shows \
                              nothing.",
                advisory: None,
            },
            EnumVariant {
                name: "Cover",
                description: "The image keeps its proportions and is as small as \
                              covers the whole box, centred. What spills past the \
                              shape is cut off.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Picture",
        export: true,
        description: "An image drawn inside a shape. The shape says where the picture \
                      is and what outline it is cut to: a `canvas::Rectangle`, with \
                      square or rounded corners, a `canvas::Circle`, a \
                      `canvas::Ellipse`, a `canvas::Polygon` or a \
                      `canvas::CurvePolygon`. A shape with no inside — a \
                      `canvas::Line`, `canvas::Arc`, `canvas::PolyLine`, \
                      `canvas::PolyCurve` or `canvas::Text` — draws nothing. The image is visible only inside \
                      the shape. Scaling picks the nearest image pixel, with no \
                      smoothing, so a small image drawn large shows crisp blocks. The \
                      shape's paint is the picture's: its fill colour tints the image \
                      — each channel moves towards the image's channel multiplied by \
                      the fill's, by the fill's alpha, so the zero paint and a white \
                      fill draw it unchanged, an opaque red fill keeps only its red, \
                      and a half-transparent red fill goes halfway — and never fills \
                      the shape itself; `canvas::Paint.transparency` fades the \
                      picture; its blend mode composites it; its stroke outlines the \
                      shape on top of the image; and the shape's `paint.clip`, if it \
                      has one, cuts the image further, so it shows only where the \
                      shape and that clip both cover. A fill gradient is ignored. \
                      The image is fitted to the box around the shape — the smallest \
                      upright rectangle that holds it — as `fit` says, and `clip` \
                      says which box when the shape has a `transform`. A group the \
                      picture is in moves and turns the whole picture, image and \
                      all, whatever `clip` is. When the shape is a \
                      `canvas::AutoPaint`, the paint the mouse selects is the one \
                      used, and nothing else is needed for that to work — not \
                      `canvas::enableMouse`. `canvas::MouseEvent.overID` reports the \
                      picture's own `id` in front of its shape's. A `Picture` has no \
                      default value — it holds an image — so every field is \
                      written out.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "shape",
                ty: ParameterType::named("AutoShape"),
                description: "The shape the image is drawn inside, and whose paint \
                              tints, fades, blends and outlines it.",
            },
            RecordProp {
                name: "clip",
                ty: ParameterType::named("PictureClip"),
                description: "Whether the shape's transform turns the image with it \
                              or only the outline it is cut to.",
            },
            RecordProp {
                name: "fit",
                ty: ParameterType::named("PictureFit"),
                description: "How the image is scaled to the shape: stretched to \
                              fill it, kept whole inside it, or made to cover it.",
            },
            RecordProp {
                name: "image",
                ty: ParameterType::res(ParameterType::named("canvas.Image")),
                description: "The image to draw. The item holds the image \
                              itself — you still close it, and closing it while \
                              a scene still names it draws nothing rather than \
                              failing.",
            },
        ],
    });

    // plan-172-K: three unions, each including the one before it. `Shape` is what can
    // be drawn with a paint of its own; `AutoShape` adds the one thing that DECORATES a
    // shape; `DrawItem` adds the two things that are not shapes at all. An included
    // union's members come first, in its own order, so the three agree on every tag
    // they share: Rectangle 0, Line 1, Polygon 2, Circle 3, Arc 4, Text 5, Ellipse 6,
    // PolyLine 7, PolyCurve 8, CurvePolygon 9, AutoPaint 10, Picture 11, Group 12.
    //
    // plan-150-A appended `ParticleSystem` to `DrawItem`'s own variants, at 13. No tag
    // moved.
    //
    // plan-164-A appended the two curve shapes to `Shape`, which moved `AutoPaint`,
    // `Picture` and `Group` by two (owner-approved, 2026-10-11): a shape has to be a
    // member of `Shape` for an `AutoPaint` to wrap it and a `Picture` to be cut to it.
    //
    // The order is `planning/new_canvas.md`'s and it is load-bearing once, here: a
    // variant's tag is its position, and the native walks ask the type model for a tag
    // by name (`func_geo_build::variant`), never by number.
    pkg.add_union(RegistryUnion {
        includes: vec![],
        name: "Shape",
        export: true,
        variants: vec![
            UnionVariant {
                name: "Rectangle",
                description: "A rectangle, with a radius for each corner.",
            },
            UnionVariant {
                name: "Line",
                description: "A straight segment.",
            },
            UnionVariant {
                name: "Polygon",
                description: "A closed polygon.",
            },
            UnionVariant {
                name: "Circle",
                description: "A circle.",
            },
            UnionVariant {
                name: "Arc",
                description: "A circular arc.",
            },
            UnionVariant {
                name: "Text",
                description: "A run of text.",
            },
            UnionVariant {
                name: "Ellipse",
                description: "An ellipse.",
            },
            UnionVariant {
                name: "PolyLine",
                description: "An open path, stroked, with a colour per point.",
            },
            UnionVariant {
                name: "PolyCurve",
                description: "An open path of curves, stroked.",
            },
            UnionVariant {
                name: "CurvePolygon",
                description: "A closed shape with a curved outline.",
            },
        ],
    });

    pkg.add_union(RegistryUnion {
        includes: vec!["Shape"],
        name: "AutoShape",
        export: true,
        variants: vec![UnionVariant {
            name: "AutoPaint",
            description: "A shape whose paint follows the mouse.",
        }],
    });

    pkg.add_union(RegistryUnion {
        includes: vec!["AutoShape"],
        name: "DrawItem",
        export: true,
        variants: vec![
            UnionVariant {
                name: "Picture",
                description: "An image drawn inside a shape.",
            },
            // `Group` is a CONTAINER, not a shape. It draws nothing itself and carries
            // no `paint` -- see `every_draw_item_variant_carries_a_paint`, which is
            // narrowed rather than weakened to admit it.
            UnionVariant {
                name: "Group",
                description: "A reference to a named group of items, placed and turned.",
            },
            // plan-150-A: the third thing that is not a shape. Appended to `DrawItem`'s
            // own variants -- not to `Shape` or `AutoShape`, whose members come first
            // and would renumber `AutoPaint`, `Picture` and `Group` -- so it takes the
            // next tag and moves no other.
            UnionVariant {
                name: "ParticleSystem",
                description: "Many copies of one shape, placed by a rule and a clock.",
            },
        ],
    });

    // plan-150-A: `item` is an `AutoShape`, not a `DrawItem` (owner's decision,
    // 2026-10-13). A `DrawItem` that held a `DrawItem` would be a type cycle, and a
    // value whose type reaches a cycle is not one flat block (`flatness_walk`): every
    // scene of every canvas program would stop being the single block `present` copies,
    // compares and hands to the graphics thread. So a particle is a shape, plain or
    // wrapped in an `AutoPaint` -- never a `Picture`, a `Group` or another system.
    pkg.add_record(RegistryRecord {
        name: "ParticleSystem",
        export: true,
        description: "Many copies of one shape, drawn from a single item: fire, smoke, \
                      sparks, rain. You give it a shape to copy (`item`), a shape to \
                      scatter the copies over (`emitter`), and three small records that \
                      say how many there are and how long each lasts (`emission`), how \
                      each moves (`motion`) and how each looks as it ages \
                      (`appearance`). Every copy is called a particle. Where a \
                      particle is, how it is turned, how big it is and what colour it \
                      is are all worked out from `seed` and `time` alone, so the same \
                      `seed` and `time` always draw the same picture — on every run \
                      and every machine. Nothing moves by itself: to animate a system, \
                      present it again with a larger `time`, typically once a frame \
                      with `canvas::getFrameStats().dt` added. A particle is the shape \
                      as you built it, placed with its own `position` on the \
                      particle's point, then turned and scaled about that point, and \
                      its fill and stroke colours multiplied by the particle's colour. \
                      A particle is never under the mouse: it is not reported by \
                      `canvas::MouseEvent.overID` and hides nothing beneath it from \
                      the mouse, and a `canvas::AutoPaint` used as `item` is drawn \
                      with its shape's own paint. A particle is a shape: a \
                      `canvas::Picture`, a `canvas::Group` or another \
                      `ParticleSystem` cannot be `item`, and is refused when the \
                      program is built. A system has no `id`, no `paint` \
                      and no `transform` of its own. A group the system is in moves \
                      and turns all of it. A `ParticleSystem` has no default value — \
                      it holds a shape — so every field is written out. What a system \
                      costs is its live particles, every frame it is presented: each \
                      is placed afresh and drawn as an item of its own. Measured on an \
                      Apple M2 Max drawing on the GPU, a system of small discs that \
                      are moved and tinted but not turned or scaled takes 3.5 \
                      milliseconds a frame at 1,000 particles, 21 at 10,000 and 95 at \
                      50,000, so it holds 60 frames a second to about 8,000. A \
                      particle that is turned or scaled costs about twice as much: 44 \
                      milliseconds at 10,000, and 60 frames a second to about 3,000. \
                      There is no number of particles at which a system stops being \
                      drawn or is drawn differently — 140,000 are drawn, at 275 \
                      milliseconds a frame. A `canvas::Polygon` emitter adds about a \
                      tenth to the time. A `canvas::Text` shape costs far more than \
                      any other, and by the size of its font file rather than the \
                      length of its text: 0.4 milliseconds a particle with a 770 KB \
                      font, so keep text particles to a few dozen. Polling the mouse \
                      costs nothing extra however many particles there are.",
        props: vec![
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the system is, in pixels. The emitter's own \
                              `position` is measured from here, so moving this moves \
                              where particles are born — particles already in flight \
                              move with it too, because nothing is remembered between \
                              frames.",
            },
            RecordProp {
                name: "item",
                ty: ParameterType::named("AutoShape"),
                description: "The shape drawn once for each live particle: any \
                              `canvas::Shape`, or a `canvas::AutoPaint`, which is drawn \
                              as its shape with that shape's own paint. Its `position` \
                              and `transform` are offsets inside the particle — leave \
                              `position` at `(0, 0)` to centre the shape on the \
                              particle. Its `id` is not used.",
            },
            RecordProp {
                name: "emitter",
                ty: ParameterType::named("Shape"),
                description: "Where particles are born. A shape with an inside — a \
                              `canvas::Rectangle`, `canvas::Circle`, `canvas::Ellipse`, \
                              `canvas::Polygon`, `canvas::CurvePolygon` or \
                              `canvas::Text` — emits evenly over its area; a \
                              `canvas::Line`, `canvas::Arc`, `canvas::PolyLine` or \
                              `canvas::PolyCurve` emits evenly along its length. A \
                              `canvas::Circle` with `radius := 0.0` is a single point. \
                              The emitter is never drawn: its `position` (measured from \
                              the system's) and its `transform` place and turn it, and \
                              its `paint` and `id` are not used. A rectangle's corner \
                              radii are ignored, and a text emits over the box around \
                              its line, not over its letters.",
            },
            RecordProp {
                name: "emission",
                ty: ParameterType::named("Emission"),
                description: "How many particles there are, how often they are born \
                              and how long each lives.",
            },
            RecordProp {
                name: "motion",
                ty: ParameterType::named("Motion"),
                description: "How each particle moves and turns after it is born.",
            },
            RecordProp {
                name: "appearance",
                ty: ParameterType::named("Appearance"),
                description: "The colour and size of each particle, from the moment it \
                              is born to the moment it dies.",
            },
            RecordProp {
                name: "seed",
                ty: ParameterType::Integer,
                description: "Picks the random part of every particle — where in the \
                              emitter it starts, and its share of each `…Var` field. \
                              Two systems with the same fields and different seeds \
                              look alike without matching particle for particle.",
            },
            RecordProp {
                name: "time",
                ty: ParameterType::Float,
                description: "The system's clock, in seconds. You advance it: the \
                              picture for a given `time` is always the same, and the \
                              window draws a new one only when you present the system \
                              again with a different value. A negative `time` draws \
                              nothing.",
            },
        ],
    });

    // `Continuous` MUST stay declared first: an enum field's zero value is its first
    // variant, so it is what an `Emission` built without naming `mode` gets.
    pkg.add_enum(RegistryEnum {
        name: "EmissionMode",
        export: true,
        variants: vec![
            EnumVariant {
                name: "Continuous",
                description: "Particles are born one after another, `rate` a second, \
                              for as long as `time` advances: a flame, a fountain, \
                              falling rain. Once `maxParticles` have been born the \
                              first is born again, so `maxParticles / rate` seconds \
                              is the longest a particle can be seen. The zero value.",
                advisory: None,
            },
            EnumVariant {
                name: "Burst",
                description: "`burst` particles are born together at `time` zero — \
                              and again every `burstPeriod` seconds when that is \
                              above zero: an explosion, a firework, a puff of dust.",
                advisory: None,
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Emission",
        export: true,
        description: "How many particles a `canvas::ParticleSystem` has, when they are \
                      born and how long they live. `mode` chooses between a steady \
                      stream and bursts; `rate` belongs to the first and `burst` and \
                      `burstPeriod` to the second, and the fields of the mode not in \
                      use are ignored. A system whose `maxParticles` is zero or less \
                      draws nothing, and so does a `Continuous` one whose `rate` is \
                      zero or less and a `Burst` one whose `burst` is zero or less.",
        props: vec![
            RecordProp {
                name: "mode",
                ty: ParameterType::named("EmissionMode"),
                description: "A steady stream (`Continuous`) or all at once (`Burst`).",
            },
            RecordProp {
                name: "maxParticles",
                ty: ParameterType::Integer,
                description: "The most particles the system can show at one moment. \
                              Each one drawn costs what the shape would cost as an \
                              item of its own.",
            },
            RecordProp {
                name: "rate",
                ty: ParameterType::Float,
                description: "`Continuous` only: particles born per second.",
            },
            RecordProp {
                name: "burst",
                ty: ParameterType::Integer,
                description: "`Burst` only: particles born in each burst. More than \
                              `maxParticles` is treated as `maxParticles`.",
            },
            RecordProp {
                name: "burstPeriod",
                ty: ParameterType::Float,
                description: "`Burst` only: seconds from one burst to the next. Zero \
                              or less is a single burst at `time` zero. A particle \
                              that would outlive the period is cut off when the next \
                              burst is born.",
            },
            RecordProp {
                name: "lifetime",
                ty: ParameterType::Float,
                description: "Seconds a particle lives. Must be above zero: \
                              `canvas::present` raises `ErrCanvasParticleLifetime` for \
                              a system whose `lifetime` is zero or less.",
            },
            RecordProp {
                name: "lifetimeVar",
                ty: ParameterType::Float,
                description: "How much lifetimes differ, as a fraction from 0 to 1: \
                              each particle lives `lifetime` plus or minus up to this \
                              share of it. A value outside 0 to 1 is treated as the \
                              nearer end.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Motion",
        export: true,
        description: "How a particle of a `canvas::ParticleSystem` moves and turns. It \
                      leaves the emitter at `speed` in the direction `angle`, is pulled \
                      by gravity, slowed by `drag`, shaken by `turbulence`, and spun by \
                      `spin`. Every field's zero is that effect switched off, so the \
                      all-zero `Motion` leaves each particle where it was born. \
                      Directions are in the window's axes — +X right, +Y down — \
                      whatever the emitter's `transform`: turning the emitter turns \
                      where particles start, not which way they fly.",
        props: vec![
            RecordProp {
                name: "speed",
                ty: ParameterType::Float,
                description: "The speed a particle starts with, in pixels per second.",
            },
            RecordProp {
                name: "speedVar",
                ty: ParameterType::Float,
                description: "How much starting speeds differ, as a fraction from 0 to \
                              1 of `speed`. A value outside 0 to 1 is treated as the \
                              nearer end.",
            },
            RecordProp {
                name: "angle",
                ty: ParameterType::Float,
                description: "The direction a particle starts in, in radians, \
                              measured clockwise from +X as a `canvas::Arc`'s angles \
                              are: `0.0` is right, `math::pi / 2.0` is down, \
                              `0.0 - math::pi / 2.0` is up.",
            },
            RecordProp {
                name: "spread",
                ty: ParameterType::Float,
                description: "How far either side of `angle` a particle may start, in \
                              radians. `0.0` sends every particle the same way; \
                              `math::pi` sends them in every direction.",
            },
            RecordProp {
                name: "gravityX",
                ty: ParameterType::Float,
                description: "A steady pull along X, in pixels per second squared. \
                              Positive is right. Wind is a small value here.",
            },
            RecordProp {
                name: "gravityY",
                ty: ParameterType::Float,
                description: "A steady pull along Y, in pixels per second squared. \
                              Positive is DOWN, as Y is everywhere on the canvas, so \
                              falling sparks want a positive value and rising smoke a \
                              negative one.",
            },
            RecordProp {
                name: "drag",
                ty: ParameterType::Float,
                description: "How quickly a particle loses its starting speed, per \
                              second: at `1.0` it keeps about a third of it after one \
                              second. `0.0` is none. Under drag a falling particle \
                              settles at a steady speed of gravity divided by `drag`.",
            },
            RecordProp {
                name: "rotation",
                ty: ParameterType::Float,
                description: "The angle a particle is turned by when it is born, in \
                              radians, clockwise.",
            },
            RecordProp {
                name: "rotationVar",
                ty: ParameterType::Float,
                description: "How much starting angles differ: each particle starts \
                              turned by `rotation` plus or minus up to this many \
                              radians.",
            },
            RecordProp {
                name: "spin",
                ty: ParameterType::Float,
                description: "How fast a particle turns, in radians per second, \
                              clockwise. Negative turns the other way.",
            },
            RecordProp {
                name: "spinVar",
                ty: ParameterType::Float,
                description: "How much spins differ, as a fraction from 0 to 1 of \
                              `spin`. A value outside 0 to 1 is treated as the nearer \
                              end.",
            },
            RecordProp {
                name: "turbulence",
                ty: ParameterType::Float,
                description: "How far a particle wanders from its path, in pixels: it \
                              sways up to this far along X and along Y, each particle \
                              out of step with the others.",
            },
            RecordProp {
                name: "turbulenceHz",
                ty: ParameterType::Float,
                description: "How fast that wander is, in sways per second.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Appearance",
        export: true,
        description: "The colour and size of a particle of a `canvas::ParticleSystem` \
                      over its life. A particle starts at `startColor` and \
                      `startScale` and changes evenly to `endColor` and `endScale` by \
                      the moment it dies. The colour is a tint: each of the shape's \
                      fill and stroke channels — red, green, blue and alpha — is \
                      multiplied by the particle's, so white leaves the shape's own \
                      colours as they are, an alpha of 0 hides the particle, and \
                      fading `endColor` to transparent makes particles fade out. It \
                      is the same rule a `canvas::Picture`'s fill follows. A shape \
                      filled with a gradient keeps its gradient: the tint does not \
                      reach it. An `Appearance` left at its default is transparent and \
                      has no size, so it draws nothing — set the colours and scales \
                      you want.",
        props: vec![
            RecordProp {
                name: "startColor",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The tint of a particle as it is born.",
            },
            RecordProp {
                name: "endColor",
                ty: ParameterType::named(crate::codegen::builtins::color::COLOR_TYPE_ID),
                description: "The tint of a particle as it dies. The change from \
                              `startColor` is even in light, not in the stored 0 to \
                              255 values, so a fade to transparent does not darken on \
                              the way.",
            },
            RecordProp {
                name: "startScale",
                ty: ParameterType::Float,
                description: "The size of a particle as it is born, as a multiple of \
                              the shape as you built it: `1.0` is that size, `2.0` \
                              twice as wide and as tall. The shape's stroke is scaled \
                              with it.",
            },
            RecordProp {
                name: "endScale",
                ty: ParameterType::Float,
                description: "The size of a particle as it dies. A particle whose \
                              size reaches zero is not drawn.",
            },
            RecordProp {
                name: "scaleVar",
                ty: ParameterType::Float,
                description: "How much sizes differ, as a fraction from 0 to 1: each \
                              particle is larger or smaller by up to this share, for \
                              its whole life. A value outside 0 to 1 is treated as \
                              the nearer end.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "AutoPaint",
        export: true,
        description: "A shape that changes its paint as the mouse moves over it, with no \
                      code of yours running: present it once and the window keeps it \
                      up to date, even while your program is waiting for a key. It \
                      wraps one `canvas::Shape` and holds up to four other paints for \
                      it. The shape is drawn with its own `paint` until the mouse \
                      arrives. When several of the four apply at once the order is \
                      `pressedPaint`, then `enterPaint`, then `hoverPaint`, then `exitPaint`, and a paint \
                      whose `use…` flag is `FALSE` is skipped. Only the TOPMOST item \
                      under the mouse reacts: anything drawn over this shape — an \
                      `AutoPaint` or not — takes the mouse instead. Where the mouse \
                      counts as over the shape is decided by the shape and its own \
                      `paint`, never by one of the four, so a hover paint with a \
                      wider outline does not make the shape easier to hit. Each use of \
                      a group has its own state, so one group drawn twice lights only \
                      the copy the mouse is on. An `AutoPaint` has no `id` of its own: \
                      `canvas::MouseEvent.overID` reports the wrapped shape's. It \
                      carries four whole paints through every `present`, so it is for \
                      the few items in a scene you interact with, not for all of them.",
        props: vec![
            RecordProp {
                name: "shape",
                ty: ParameterType::named("Shape"),
                description: "The shape that is drawn. Its own `paint` is what you see \
                              when the mouse is elsewhere, and what decides where the \
                              mouse counts as over it.",
            },
            RecordProp {
                name: "useEnter",
                ty: ParameterType::Boolean,
                description: "`TRUE` to flash `enterPaint` when the mouse arrives. `FALSE` \
                              goes straight to the hover paint.",
            },
            RecordProp {
                name: "useExit",
                ty: ParameterType::Boolean,
                description: "`TRUE` to flash `exitPaint` when the mouse leaves. `FALSE` \
                              goes straight back to the shape's own paint.",
            },
            RecordProp {
                name: "useHover",
                ty: ParameterType::Boolean,
                description: "`TRUE` to show `hoverPaint` while the mouse rests on the \
                              shape. `FALSE` shows the shape's own paint.",
            },
            RecordProp {
                name: "usePressed",
                ty: ParameterType::Boolean,
                description: "`TRUE` to show `pressedPaint` while a button is held on the \
                              shape. `FALSE` shows whatever hovering would.",
            },
            RecordProp {
                name: "enterPaint",
                ty: ParameterType::named("Paint"),
                description: "Shown for a tenth of a second from the moment the mouse \
                              arrives — or for `options.canvasAutoPaintMS` \
                              milliseconds, 1 to 10000, if your project.json sets \
                              it — then the hover paint takes over — with the mouse still, \
                              too. It is a flash, not a fade: nothing is blended. If \
                              the mouse leaves first, the exit paint starts at once.",
            },
            RecordProp {
                name: "exitPaint",
                ty: ParameterType::named("Paint"),
                description: "Shown for a tenth of a second from the moment the mouse \
                              leaves — the same `options.canvasAutoPaintMS` — then the shape's own paint returns. If the mouse \
                              comes back first, the enter paint starts at once.",
            },
            RecordProp {
                name: "hoverPaint",
                ty: ParameterType::named("Paint"),
                description: "Shown while the mouse is over the shape, once any \
                              enter flash is over — and while a button is held on it \
                              too, when `usePressed` is `FALSE`.",
            },
            RecordProp {
                name: "pressedPaint",
                ty: ParameterType::named("Paint"),
                description: "Shown while any mouse button is held, the press began on \
                              this shape, and the mouse is still over it. Drag off \
                              with the button down and the shape leaves as it would \
                              for any exit; drag back on and this paint returns. A \
                              press that began somewhere else and is dragged onto the \
                              shape shows the hover paint.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Group",
        export: true,
        description: "A reference to a sub-scene installed under a name by \
                      `canvas::setGroup`, drawn with its own origin at `position` and \
                      turned, scaled or sheared about that point by `transform`. A \
                      group inside a group is placed by both, the outer one last, so \
                      a figure built from groups moves and turns as one thing. \
                      Presenting a scene copies this node — an `id`, a position, a \
                      transform and a name — rather than the items it stands for, which is the \
                      point: a large static sub-picture referenced from many scenes \
                      is copied once when you install it, not once per frame. A name \
                      with no group installed draws nothing and does not raise, so a \
                      scene can reference a group that has not been built yet. A \
                      group has no `paint`: it is a container, and the items inside \
                      it carry their own. Every field \
                      has a usable default, so the short way to make one is to \
                      declare it and set what you need: `MUT panel AS canvas::Group`, \
                      then `WITH panel { name := \"panel\", position := \
                      vector::Float2[x := 40.0, y := 40.0] }`. Its `id` names this \
                      USE of the group, not the group: draw one group twice under \
                      two `id`s and `canvas::MouseEvent.overID` says which of the \
                      two the mouse is over, by putting the `id` in front of the \
                      item's own.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "Where the group's own origin lands, in pixels: an item \
                              the group holds at `(0, 0)` is drawn here. The transform \
                              turns the group about it.",
            },
            RecordProp {
                name: "transform",
                ty: ParameterType::named("Transform"),
                description: "Turns, scales or shears everything the group holds, \
                              about `position`; its `tx` and `ty` add to `position`. \
                              It scales the items' strokes with them and carries \
                              their clips and gradients. All zero, or a transform \
                              with no area, leaves the group as it was built.",
            },
            RecordProp {
                name: "name",
                ty: ParameterType::String,
                description: "Which installed group to draw. A name you have not \
                              passed to `canvas::setGroup`, or one you have since \
                              removed, draws nothing.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Ellipse",
        export: true,
        description: "An ellipse given by its centre and two radii, along the X and Y \
                      axes. `radiusX = radiusY` is a circle, and draws identically to \
                      `canvas::Circle` of that radius — so an animation that squashes \
                      a circle can use one item type throughout. To tilt it, give it \
                      a rotation in `transform`. A radius of `0.0` or less draws \
                      nothing, the same rule `canvas::Circle` follows. Like a circle, it is \
                      shortest made from its default: `MUT egg AS canvas::Ellipse`, then \
                      `WITH egg { position := …, radiusX := …, radiusY := …, paint := … }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The centre, in pixels. The transform turns the ellipse about it.",
            },
            RecordProp {
                name: "radiusX",
                ty: ParameterType::Float,
                description: "The radius along the X axis, in pixels, before `transform` \
                              turns it.",
            },
            RecordProp {
                name: "radiusY",
                ty: ParameterType::Float,
                description: "The radius along the Y axis, in pixels.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-154-A. Declared after `Ellipse` and appended LAST to `DrawItem` below, for
    // the tag-order reason given there.
    pkg.add_record(RegistryRecord {
        name: "PolyLine",
        export: true,
        description: "An open path through a list of points, stroked as one item. \
                      `paint.join` shapes every corner where two segments meet, and \
                      `paint.cap` the two ends. Give `colors` a colour per point and the \
                      stroke blends from each point's colour to the next along the \
                      segment between them, so a trail that fades out is one item \
                      whose colours' alpha falls to zero, not one `canvas::Line` per \
                      segment. Colours mix in linear light, as a `canvas::Gradient`'s \
                      do. A path has no interior, so it is drawn from \
                      `paint.strokeWidth` and ignores `paint.fill` and \
                      `paint.gradient`. Fewer than two points draws nothing. Every \
                      point is measured from `position`, so moving the path is \
                      changing `position` alone, and its `transform` turns it about \
                      `position`. Every field has a usable default, so the short way \
                      to make one is to declare it and set what you need: `MUT trail \
                      AS canvas::PolyLine`, then `WITH trail { points := \
                      [vector::Float2[x := 20.0, y := 20.0], vector::Float2[x := \
                      120.0, y := 60.0], vector::Float2[x := 220.0, y := 20.0]], \
                      paint := canvas::stroke(ink, 3.0) }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every point of the path is measured from, in \
                              pixels. The transform turns the path about it.",
            },
            RecordProp {
                name: "points",
                ty: ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                )),
                description: "The points in order, each an offset from \
                              `position`. The path does not close — use \
                              `canvas::Polygon` for a closed shape. A point repeated \
                              in place adds nothing; if every point is the same one, \
                              the path draws what a zero-length `canvas::Line` with \
                              the same `paint.cap` draws.",
            },
            RecordProp {
                name: "colors",
                ty: ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::color::COLOR_TYPE_ID,
                )),
                description: "The stroke colour at each point, in the same order as \
                              `points` — `color::Color` values, so the program needs \
                              `IMPORT color` to name them. An empty list draws the \
                              whole path in `paint.stroke`. A point past the end of \
                              this list takes `paint.stroke`, and colours past the \
                              last point are ignored. With any colours given, the path \
                              draws even when `paint.stroke` is transparent.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-164-A. One curve segment: a record, never a `DrawItem`. The two shapes
    // below hold lists of them.
    pkg.add_record(RegistryRecord {
        name: "BezierCurve",
        export: true,
        description: "One Bézier curve segment: a list of control points, and \
                      optionally a weight for each. It is not drawn by itself — a \
                      `canvas::PolyCurve` strokes a chain of them and a \
                      `canvas::CurvePolygon` fills closed contours of them. The \
                      number of points sets the kind: two is a straight line, three \
                      a quadratic curve, four a cubic, and so on up to seventeen. \
                      The curve starts at its first point and ends at its last; the \
                      points between pull it towards them without being touched. \
                      With no weights it is an ordinary polynomial curve. With one \
                      weight per point it is a rational curve, which is what draws \
                      an exact circle or ellipse arc: a quarter circle is three \
                      points — start, corner, end — with weights `1.0`, `0.7071067811865476` \
                      and `1.0`. A curve that breaks a rule here is skipped: it \
                      draws nothing and the curves either side of it are joined by \
                      a straight segment. `canvas::nurbsToBeziers` makes a list of \
                      these from a NURBS curve, and `canvas::flatten` turns one \
                      into points.",
        props: vec![
            RecordProp {
                name: "points",
                ty: ParameterType::list_of(ParameterType::named(
                    crate::codegen::builtins::vector::FLOAT2_TYPE_ID,
                )),
                description: "The control points in order, each an offset from the \
                              `position` of the shape that holds the curve. Two to \
                              seventeen of them; fewer or more and the curve is \
                              skipped.",
            },
            RecordProp {
                name: "weights",
                ty: ParameterType::list_of(ParameterType::Float),
                description: "Empty for a polynomial curve. Otherwise exactly one \
                              weight per point, in the same order, each greater than \
                              `0.0`: a larger weight pulls the curve harder towards \
                              its point. A list of any other length, or one holding \
                              a weight of `0.0` or less, makes the curve skipped.",
            },
        ],
    });

    // plan-164-A. The curved `PolyLine`. Appended to `Shape` below, after `PolyLine`.
    pkg.add_record(RegistryRecord {
        name: "PolyCurve",
        export: true,
        description: "An open path made of curves joined end to end, stroked as one \
                      item: the curved counterpart of `canvas::PolyLine`. Each \
                      `canvas::BezierCurve` in `curves` starts where the one before \
                      it ended; when it does not, a straight segment joins the two \
                      and is stroked with the rest. `paint.join` shapes each corner \
                      where two curves meet at an angle, `paint.cap` the two ends, \
                      and `paint.dashes` run along the whole path without restarting \
                      at a curve. A path has no interior, so it is drawn from \
                      `paint.strokeWidth` and ignores `paint.fill`. It is never \
                      closed: a path that ends where it began has two caps there, \
                      not a corner — use `canvas::CurvePolygon` for a closed \
                      outline. A curve that breaks a `canvas::BezierCurve` rule is \
                      skipped, and a path with no usable curve draws nothing. The \
                      curves stay smooth however far the shape's own `transform` \
                      enlarges them. Enlarging a `canvas::Group` the path is in \
                      does not redraw the curves at the new size, so a cubic or \
                      higher curve can show slight flats there; scale the shape's \
                      own `transform` to zoom in. Every field has a usable default, \
                      so the short way to make one is to declare it and set what \
                      you need: `MUT wave AS canvas::PolyCurve`, then `WITH wave { \
                      curves := […], paint := canvas::stroke(ink, 3.0) }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every curve point is measured from, in \
                              pixels. The transform turns the path about it.",
            },
            RecordProp {
                name: "curves",
                ty: ParameterType::list_of(ParameterType::named("BezierCurve")),
                description: "The curves in order, joined end to end. An empty list \
                              draws nothing.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    // plan-164-A. The curved `Polygon`: its `contours` and `rule` are `Polygon`'s
    // fields with curves where it has points, on purpose.
    pkg.add_record(RegistryRecord {
        name: "CurvePolygon",
        export: true,
        description: "A closed shape whose outline is made of curves, filled and \
                      stroked as one item: the curved counterpart of \
                      `canvas::Polygon`. Each entry of `contours` is one closed \
                      outline, a list of `canvas::BezierCurve` joined end to end. \
                      Where a curve does not start at the end of the one before, a \
                      straight segment joins them, and a contour whose last curve \
                      does not end at its first curve's start is closed by a \
                      straight segment too; those segments are part of the outline, \
                      filled to and stroked. Several contours make holes and \
                      islands, decided by `rule` exactly as for a polygon. \
                      `paint.join` shapes each corner where two curves meet at an \
                      angle, and `paint.dashes` restart at each contour. A curve \
                      that breaks a `canvas::BezierCurve` rule is skipped, and a \
                      contour with no usable curve adds nothing. The curves stay \
                      smooth however far the shape's own `transform` enlarges \
                      them; enlarging a `canvas::Group` the shape is in does not \
                      redraw them at the new size, so scale the shape's own \
                      `transform` to zoom in. Every field has a usable default, so \
                      the short way to make one is to declare it and set what you \
                      need: `MUT blob AS canvas::CurvePolygon`, then `WITH blob { \
                      contours := [[…]], paint := canvas::fill(ink) }`.",
        props: vec![
            id_prop(),
            RecordProp {
                name: "position",
                ty: ParameterType::named(crate::codegen::builtins::vector::FLOAT2_TYPE_ID),
                description: "The point every curve point is measured from, in \
                              pixels. The transform turns the shape about it.",
            },
            RecordProp {
                name: "contours",
                ty: ParameterType::list_of(ParameterType::list_of(ParameterType::named(
                    "BezierCurve",
                ))),
                description: "The closed outlines, each a list of curves in order. \
                              An empty list draws nothing.",
            },
            RecordProp {
                name: "rule",
                ty: ParameterType::named("FillRule"),
                description: "Which regions count as inside when contours overlap \
                              or nest, as for `canvas::Polygon`.",
            },
            paint_prop(),
            transform_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "DrawLayer",
        export: true,
        description: "An ordered group of items composited as one layer. Layers \
                      given to `canvas::presentLayers` composite in order, and a \
                      layer whose contents did not change reuses its cached \
                      geometry wholesale.",
        props: vec![RecordProp {
            name: "items",
            ty: ParameterType::list_of(ParameterType::named("DrawItem")),
            description: "The layer's items, drawn in order.",
        }],
    });

    // ---- Resources --------------------------------------------------------

    // Both RES resources are declared with the `destroy*` member that closes them, and
    // that pairing is not stylistic: `add_resource` **derives a runtime call from the
    // close op** (`registry::runtime_specs`), so a resource declared without its close
    // member leaves a call the catalog cannot route
    // (`catalog_is_consistent`: "canvas.destroyImage: None (expected Some(Canvas))").
    //
    // `Picture` and `Text` name these directly: since plan-116-I their `image` and
    // `font` fields are `RES canvas::Image` and `RES canvas::Font`. The value handles
    // that used to stand in for them are gone.

    pkg.add_resource(RegistryResource {
        name: IMAGE_TYPE,
        export: true,
        description: "An opaque handle to an image the drawing backend holds, closed \
                      automatically when its binding goes out of scope. A \
                      `canvas::Picture` holds one directly, and destroying an image a \
                      scene still draws is safe — that item draws nothing.",
        close_function: DESTROY_IMAGE,
        // An image belongs to the drawing surface's thread; it does not cross a
        // thread boundary in v1.
        sendable: false,
        // Not audited for transfer (bug-464 left canvas out of scope). Empty
        // here is only consistent with `sendable: false`; opting an image in
        // means auditing its record tail first, not just flipping the bit.
        live_slots: &[],
        // `destroyImage` sets the closed flag and returns; the backend frees the real
        // object later, on its own schedule, so there is nothing here that can fail.
        // Its only error is `ErrResourceClosed` on a re-close, which the drop path
        // treats as benign, so it is not a close failure.
        unsendable_reason: Some("it belongs to the drawing surface's thread"),
        close_may_fail: false,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    pkg.add_resource(RegistryResource {
        name: FONT_TYPE,
        export: true,
        description: "An opaque handle to a loaded font, closed automatically when it \
                      leaves scope. A `canvas::Text` holds one directly, and closing a \
                      font whose text a scene still draws is safe — that text simply \
                      draws as empty.",
        close_function: DESTROY_FONT,
        // A font belongs to the drawing surface's thread, like an image; it does not
        // cross a thread boundary in v1.
        sendable: false,
        // Not audited for transfer, exactly as `Image` is not. Empty here is only
        // consistent with `sendable: false`; opting a font in means auditing its
        // record tail — which holds the whole file — rather than flipping the bit.
        live_slots: &[],
        // `destroyFont` sets the closed flag and returns. The font's bytes are
        // arena-owned, so unlike a file there is no OS handle to hand back and nothing
        // here that can fail.
        unsendable_reason: Some("it belongs to the drawing surface's thread"),
        close_may_fail: false,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    func_fill::register(&mut pkg);
    func_stroke::register(&mut pkg);
    func_fill_stroke::register(&mut pkg);
    func_new_surface::register(&mut pkg);
    func_present::register(&mut pkg);
    // plan-116-G. Registered beside `present` because they are the other two install
    // points: `present` installs a scene, these install what a scene can reference.
    func_set_group::register(&mut pkg);
    func_remove_group::register(&mut pkg);
    func_group_stats::register(&mut pkg);
    func_publish_scene::register(&mut pkg);
    func_blit_surface::register(&mut pkg);
    func_metal_draw::register(&mut pkg);
    func_metal_present::register(&mut pkg);
    func_graphics::register(&mut pkg);
    func_get_frame_stats::register(&mut pkg);
    func_wait_frame::register(&mut pkg);
    func_installed_items::register(&mut pkg);
    func_installed_layers::register(&mut pkg);
    func_scene_hashes::register(&mut pkg);
    func_present_layers::register(&mut pkg);
    func_create_image::register(&mut pkg);
    func_load_image::register(&mut pkg);
    func_destroy_image::register(&mut pkg);
    func_handle_bridge::register(&mut pkg);
    gen_font_table::register(&mut pkg);
    func_load_font::register(&mut pkg);
    func_system_fonts::register(&mut pkg);
    func_measure_text::register(&mut pkg);
    func_destroy_font::register(&mut pkg);
    func_get_size::register(&mut pkg);
    func_nurbs_to_beziers::register(&mut pkg);
    func_flatten::register(&mut pkg);
    func_did_resize::register(&mut pkg);
    func_frame_nanos::register(&mut pkg);
    func_carried_hashes::register(&mut pkg);
    func_geo_build::register(&mut pkg);
    func_geo_cache::register(&mut pkg);
    func_item_hash::register(&mut pkg);
    func_enable_mouse::register(&mut pkg);
    func_poll_mouse::register(&mut pkg);
    func_pointer::register(&mut pkg);
    func_get_bytes::register(&mut pkg);
    func_set_bytes::register(&mut pkg);
    helper_clamp_byte::register(&mut pkg);
    helper_paint_defaults::register(&mut pkg);
    // Order matters only for readability — the helper section renders in call order.
    helper_color::register(&mut pkg);
    helper_shapes::register(&mut pkg);
    helper_draw::register(&mut pkg);
    helper_font::register(&mut pkg);
    helper_font_name::register(&mut pkg);
    helper_glyph::register(&mut pkg);
    helper_damage::register(&mut pkg);
    helper_glyph_cache::register(&mut pkg);
    helper_png::register(&mut pkg);
    helper_curves::register(&mut pkg);
    helper_geometry::register(&mut pkg);
    helper_dashes::register(&mut pkg);
    helper_joins::register(&mut pkg);
    helper_items::register(&mut pkg);
    helper_surface::register(&mut pkg);
    helper_hit::register(&mut pkg);
    helper_autopaint::register(&mut pkg);
    helper_picture::register(&mut pkg);
    helper_particles::register(&mut pkg);
    helper_render::register(&mut pkg);

    r.add_package(pkg);
}

/// The `paint` field every drawn `DrawItem` variant carries.
fn paint_prop() -> RecordProp {
    RecordProp {
        name: "paint",
        ty: ParameterType::named("Paint"),
        description: "How to fill, stroke, blend and clip the item.",
    }
}

/// The `id` field every drawn `DrawItem` variant carries, first (plan-172-C).
fn id_prop() -> RecordProp {
    RecordProp {
        name: "id",
        ty: ParameterType::String,
        description: "A name for this item, of your choosing. `\"\"` is no name. It \
                      draws nothing and changes nothing about how the item looks; it \
                      is how a program tells its own items apart, and what \
                      `canvas::MouseEvent.overID` reports for the item under the \
                      mouse. Two items may share one.",
    }
}

/// The `transform` field every drawn `DrawItem` variant carries, last (plan-172-C).
fn transform_prop() -> RecordProp {
    RecordProp {
        name: "transform",
        ty: ParameterType::named("Transform"),
        description: "The affine transform applied to this item: its geometry, its \
                      stroke, its `paint.clip` and its `paint.gradient` together. The \
                      all-zero value is the identity, so an item you never set this \
                      on draws where its own coordinates put it. Edges stay smooth \
                      after the transform — a rotated square's sides are as clean as \
                      an upright one's. A transform with no area (a zero row, or \
                      `a*d - b*c` within `1e-12` of zero) is treated as the identity \
                      rather than collapsing the item to nothing.",
    }
}

// Man/spec citation anchor: `CANVAS`. The canvas man pages and the app canvas spec
// section ground their package-level and `DrawItem`-set facts here.

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;
    use crate::types::ParameterType;

    /// The members of a canvas union in TAG order: what it `INCLUDES` first, in that
    /// union's own order, then its own variants — the order
    /// `binary_repr::writer::concrete_union_variants` numbers them in.
    fn members(name: &str) -> Vec<&'static str> {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        let union = pkg
            .unions()
            .iter()
            .find(|u| u.name == name)
            .unwrap_or_else(|| panic!("no canvas union `{name}`"));
        let mut out: Vec<&'static str> = union
            .includes
            .iter()
            .flat_map(|included| members(included))
            .collect();
        out.extend(union.variants.iter().map(|v| v.name));
        out
    }

    /// The `DrawItem` variants are a **closed set** (plan-98-A invariant 6): adding
    /// one later stops a user's `SELECT CASE` being exhaustive, which is a breaking
    /// change. Pinning the exact list — and its order, which fixes the tags — makes
    /// any addition a deliberate, visible act rather than a silent one.
    ///
    /// It was extended three times by appending — **plan-116-E `Ellipse`**,
    /// **plan-116-G `Group`**, **plan-154-A `PolyLine`** — appended rather than
    /// inserted where each reads better, because the order fixes the tags.
    ///
    /// **plan-172-K re-derived it, once, and that is the one time the order moved.**
    /// `DrawItem` became three nested unions (`Shape`, `AutoShape INCLUDES Shape`,
    /// `DrawItem INCLUDES AutoShape`) and gained `AutoPaint`. An included union's
    /// members come first, so the eight shapes lead, the decorator follows, and the two
    /// things that are not shapes come last. Every variant's tag changed: nothing may
    /// depend on a tag's number, and the native walks ask the type model for a tag by
    /// NAME.
    ///
    /// The assertion did not become laxer: the list is still exact and still ordered,
    /// and two sibling tests pin the two inner unions the same way.
    ///
    /// **plan-164-A moved three tags, with the owner's approval (2026-10-11).**
    /// `PolyCurve` and `CurvePolygon` are shapes — an `AutoPaint` wraps either and a
    /// `Picture` is cut to the second — so they are members of `Shape`, and an included
    /// union's members come first: they sit after `PolyLine`, and `AutoPaint`,
    /// `Picture` and `Group` each moved by two. Appending them to `DrawItem` alone
    /// would have moved nothing and made them things no decorator could hold.
    ///
    /// **plan-150-A appended `ParticleSystem`, last, and moved no tag.** It is the
    /// fourteenth variant, after `Group`, among `DrawItem`'s own. The list grew by one
    /// visible entry and the assertion is as exact as it was.
    #[test]
    fn draw_item_variant_set_is_frozen() {
        assert_eq!(
            members("DrawItem"),
            vec![
                "Rectangle",
                "Line",
                "Polygon",
                "Circle",
                "Arc",
                "Text",
                "Ellipse",
                "PolyLine",
                "PolyCurve",
                "CurvePolygon",
                "AutoPaint",
                "Picture",
                "Group",
                "ParticleSystem",
            ],
            "the DrawItem variant set is frozen; extending it is a breaking change"
        );
    }

    /// plan-150-A: a particle is a shape -- `ParticleSystem.item` is an `AutoShape`, so
    /// every member of it carries a paint or decorates something that does, and none
    /// is a container or a `ParticleSystem`.
    #[test]
    fn the_particle_template_is_a_shape() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        let system = pkg
            .records()
            .iter()
            .find(|r| r.name == "ParticleSystem")
            .expect("ParticleSystem record");
        let item = system
            .props
            .iter()
            .find(|p| p.name == "item")
            .expect("ParticleSystem.item");
        assert_eq!(item.ty.name(), "AutoShape");
        for inner in members("AutoShape") {
            assert!(!["Picture", "Group", "ParticleSystem"].contains(&inner));
        }
    }

    /// plan-150-A: no canvas value type reaches itself. Every scene is one flat block
    /// -- `present` copies it with one `copy_flat_block`, compares it by content, and
    /// the native walks step through it by offset -- and a record that held, at any
    /// depth, the union it is a variant of would make that union a pointer-linked graph
    /// for every program, not only the ones that use the field. It was tried:
    /// `ParticleSystem.item AS DrawItem` failed the build of a program that constructs
    /// no particle system at all ("native content comparison does not support type
    /// 'List OF canvas.DrawItem'").
    #[test]
    fn no_canvas_type_reaches_itself() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        // What a named canvas type holds: a record's field types, a union's members.
        let holds = |name: &str| -> Vec<String> {
            let mut out = Vec::new();
            if let Some(record) = pkg.records().iter().find(|r| r.name == name) {
                for prop in &record.props {
                    let mut ty = &prop.ty;
                    loop {
                        match ty {
                            ParameterType::ListOf(inner) | ParameterType::Res(inner) => ty = inner,
                            _ => break,
                        }
                    }
                    out.push(ty.name().trim_start_matches("canvas.").to_string());
                }
            }
            if pkg.unions().iter().any(|u| u.name == name) {
                out.extend(members(name).into_iter().map(str::to_string));
            }
            out
        };
        let names: Vec<&str> = pkg
            .records()
            .iter()
            .map(|r| r.name)
            .chain(pkg.unions().iter().map(|u| u.name))
            .collect();
        for start in names {
            let mut stack = holds(start);
            let mut seen = std::collections::HashSet::new();
            while let Some(next) = stack.pop() {
                assert_ne!(next, start, "canvas::{start} reaches itself");
                if seen.insert(next.clone()) {
                    stack.extend(holds(&next));
                }
            }
        }
    }

    /// plan-172-K: `Shape` is what an `AutoPaint` may wrap and (plan-172-L) what a
    /// `Picture` is cut to — exactly the things drawn with a paint of their own.
    /// `AutoPaint` is not in it, which is what makes an `AutoPaint` of an `AutoPaint`
    /// a type error rather than a rule.
    ///
    /// plan-164-A appended `PolyCurve` and `CurvePolygon` (owner-approved,
    /// 2026-10-11): the eight became ten, in the order their straight counterparts
    /// were added.
    #[test]
    fn shape_variant_set_is_frozen() {
        assert_eq!(
            members("Shape"),
            vec![
                "Rectangle",
                "Line",
                "Polygon",
                "Circle",
                "Arc",
                "Text",
                "Ellipse",
                "PolyLine",
                "PolyCurve",
                "CurvePolygon",
            ],
            "the Shape variant set is frozen; extending it is a breaking change"
        );
    }

    /// plan-172-K: `AutoShape` is every shape, plain or decorated — `Shape`'s members and
    /// `AutoPaint`. It shares every tag with `Shape`, and `DrawItem` every tag with it.
    #[test]
    fn auto_shape_variant_set_is_frozen() {
        let auto = members("AutoShape");
        assert_eq!(auto.last(), Some(&"AutoPaint"));
        assert_eq!(auto[..auto.len() - 1], members("Shape")[..]);
        assert_eq!(members("DrawItem")[..auto.len()], auto[..]);
    }

    /// Every `DrawItem` variant must name a record the package actually declares,
    /// or the union references a type that does not exist.
    #[test]
    fn every_draw_item_variant_has_a_record() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        for variant in members("DrawItem") {
            assert!(
                pkg.records().iter().any(|r| r.name == variant),
                "DrawItem variant `{variant}` has no record declaration",
            );
        }
    }

    /// Every *drawable* variant carries a `paint`, which is what makes `Paint` a
    /// threaded value rather than ambient state. A variant that forgot it would
    /// silently draw with no way to colour it.
    ///
    /// **plan-116-G narrowed this to its actual subject; it did not weaken it.** The
    /// union gained its first CONTAINER variant, `Group`, which draws nothing itself —
    /// its children carry their own paint. The test's premise ("a variant that forgot
    /// it would silently draw") does not describe a container: a `Group` with a `paint`
    /// would be a field no renderer could honour, so requiring one would be requiring a
    /// lie.
    ///
    /// **plan-172-K added the second kind of exemption, a DECORATOR.** `AutoPaint` has
    /// no `paint` of its own either — but for the opposite reason: it draws its `shape`,
    /// and the shape carries one. So it is not excused, it is checked differently: its
    /// `shape` must be a `Shape`, and every member of `Shape` is in the checked set
    /// below.
    ///
    /// **plan-150-A added a second container, `ParticleSystem`.** It draws its `item`
    /// once per particle and has no paint for the reason a `Group` has none: what it
    /// holds carries one. It is listed here and not as a decorator because that list
    /// checks a field named `shape`; `the_particle_template_is_a_shape` checks `item`.
    ///
    /// Both exemptions are explicit lists rather than a property anything could drift
    /// into, so a future variant is not silently excused, and each list is itself
    /// checked against the union so a typo cannot quietly exempt nothing.
    #[test]
    fn every_draw_item_variant_carries_a_paint() {
        /// Variants that are containers rather than shapes: `Group` (plan-116-G) and
        /// `ParticleSystem` (plan-150-A), whose template carries the paint.
        const CONTAINERS: &[&str] = &["Group", "ParticleSystem"];
        /// Variants that draw a `shape` they hold, with that shape's paint, and the
        /// union that shape is: `AutoPaint` (plan-172-K) and `Picture` (plan-172-L),
        /// whose shape may itself be an `AutoPaint`.
        const DECORATORS: &[(&str, &str)] = &[("AutoPaint", "Shape"), ("Picture", "AutoShape")];

        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        let all = members("DrawItem");
        let decorators: Vec<&str> = DECORATORS.iter().map(|(name, _)| *name).collect();
        for name in CONTAINERS.iter().chain(decorators.iter()) {
            assert!(
                all.contains(name),
                "`{name}` is exempted from the paint requirement but is not a \
                 DrawItem variant — a renamed or removed variant leaves an \
                 exemption that silently covers nothing",
            );
        }
        for variant in all {
            let record = pkg
                .records()
                .iter()
                .find(|r| r.name == variant)
                .expect("variant record");
            if CONTAINERS.contains(&variant) {
                assert!(
                    !record.props.iter().any(|p| p.name == "paint"),
                    "`{variant}` is listed as a container but declares a `paint` field, \
                     which no renderer can honour — either it is a shape and belongs in \
                     the checked set, or the field should go",
                );
                continue;
            }
            if let Some((_, holds)) = DECORATORS.iter().find(|(name, _)| *name == variant) {
                assert!(
                    !record.props.iter().any(|p| p.name == "paint"),
                    "`{variant}` is listed as a decorator but declares a `paint` of its \
                     own, beside the one its shape carries",
                );
                let shape = record
                    .props
                    .iter()
                    .find(|p| p.name == "shape")
                    .unwrap_or_else(|| panic!("`{variant}` has no shape field"));
                assert_eq!(shape.ty.name(), *holds, "{variant}");
                // Every member of what it holds either carries a paint or is itself a
                // decorator of things that do — never a container, and never `variant`.
                for inner in members(holds) {
                    assert!(
                        !CONTAINERS.contains(&inner) && inner != variant,
                        "`{inner}` is a member of {holds} and has no paint to give",
                    );
                }
                continue;
            }
            let paint = record
                .props
                .iter()
                .find(|p| p.name == "paint")
                .unwrap_or_else(|| panic!("`{variant}` has no paint field"));
            assert_eq!(paint.ty.name(), "Paint", "{variant}");
        }
    }

    /// A record and a resource sharing a name would make the type unresolvable —
    /// which is exactly why the image-drawing variant is `Picture`, not `Image`.
    #[test]
    fn no_record_shares_a_name_with_a_resource() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        for resource in pkg.resources() {
            assert!(
                !pkg.records().iter().any(|r| r.name == resource.name),
                "record and resource both named `{}`",
                resource.name
            );
        }
    }

    /// The two `DrawItem` variants that name a resource hold the **resource itself**,
    /// not a handle to it.
    ///
    /// This assertion has been inverted, deliberately. It used to read
    /// `resource_handles_are_plain_integer_values` and pin the opposite: an `ImageRef`/
    /// `FontRef` record with a single `Integer` `id`. That shape existed for one reason
    /// — a record field could not hold a resource — and plan-114-D removed the reason.
    /// plan-116-I removed the workaround.
    ///
    /// What is pinned now is the part that could regress silently. A field typed
    /// `Named("canvas.Image")` renders *identically* to one typed
    /// `Res(Named("canvas.Image"))` — `mfb man` shows the same text for both — but the
    /// first is a value field that copies the resource record, and the second aliases
    /// the live one. So this asserts the **variant**, not the spelling.
    ///
    /// The lifetime property the old handle bought is not given up, it moved: the
    /// renderer reads the backend id through `canvas::imageHandle`/`fontHandle`, which
    /// answer `0` rather than raising once the resource is closed, so a scene naming a
    /// destroyed resource still draws nothing instead of failing.
    #[test]
    fn the_resource_naming_variants_hold_the_resource_itself() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");

        for (owner, field_name, resource) in [
            ("Picture", "image", "canvas.Image"),
            ("Text", "font", "canvas.Font"),
        ] {
            let variant = pkg
                .records()
                .iter()
                .find(|r| r.name == owner)
                .unwrap_or_else(|| panic!("{owner} record"));
            let field = variant
                .props
                .iter()
                .find(|p| p.name == field_name)
                .unwrap_or_else(|| panic!("{owner} should have a `{field_name}` field"));
            assert_eq!(
                field.ty,
                ParameterType::res(ParameterType::named(resource)),
                "`{owner}.{field_name}` must be `RES {resource}`. A bare \
                 `Named(\"{resource}\")` renders the same and is a VALUE field — it \
                 would copy the resource record instead of aliasing the live one",
            );
        }

        // And the workaround is gone, not merely unused: a lingering `ImageRef` would
        // still be exported, still be constructible, and still be the thing an example
        // reached for.
        for gone in ["ImageRef", "FontRef"] {
            assert!(
                !pkg.records().iter().any(|r| r.name == gone),
                "the `{gone}` record is still registered; plan-116-I deletes it rather \
                 than leaving it as a second way to name a resource",
            );
        }
        for gone in ["imageRef", "fontRef"] {
            assert!(
                !pkg.functions().iter().any(|f| f.name == gone),
                "`canvas::{gone}` is still registered",
            );
        }
    }

    /// The assembled companion source must parse.
    ///
    /// `canvas` carries the software rasteriser as MFBASIC source, so this is a large
    /// body of code whose only other compile check is building a program that imports
    /// the package — which reports errors against a virtual `<builtin-canvas>` file
    /// the developer cannot open. This fails in milliseconds and names the line.
    #[test]
    fn reassembled_source_parses() {
        let canvas = registry().resolve_package("canvas").expect("canvas");
        assert_parses(&canvas.get_mfb_for(false));
        // plan-130-E: the `--debug` build's source, with the reporting hooks.
        let debug = canvas.get_mfb_for(true);
        assert!(debug.contains("\"MFB_CANVAS_STATS\"") && debug.contains("\"MFB_CANVAS_DUMP\""));
        assert!(debug.contains("\"MFB_CANVAS_POINTER\""));
        for switch in DEBUG_ONLY_VARIABLES {
            assert!(
                debug.contains(&format!("\"{switch}\"")),
                "the --debug canvas source does not read {switch}"
            );
        }
        assert_parses(&debug);
    }

    /// Every variable only a `--debug` build's canvas source reads: the reporting hooks
    /// and the switches a test sets to drive or observe a headless run.
    const DEBUG_ONLY_VARIABLES: [&str; 9] = [
        "MFB_CANVAS_STATS",
        "MFB_CANVAS_DUMP",
        "MFB_CANVAS_POINTER",
        "MFB_CANVAS_AUTO_WINDOW_MS",
        "MFB_CANVAS_GEO_VERIFY",
        "MFB_CANVAS_DAMAGE",
        "MFB_CANVAS_DISPLAY_HZ",
        "MFB_CANVAS_FRAME_HOLD_MS",
        "MFB_CANVAS_GLYPH_BUDGET",
    ];

    /// A normal build's canvas source reads exactly three variables: the renderer
    /// choice, the headless switch, and `MFB_CANVAS_SYNC`, which stays in every build
    /// while canvas is in development (`__canvas_readSyncMode`). Everything else is a
    /// `--debug` build's.
    #[test]
    fn the_normal_source_reads_only_the_renderer_headless_and_sync_variables() {
        let normal = registry()
            .resolve_package("canvas")
            .expect("canvas")
            .get_mfb_for(false);
        for switch in DEBUG_ONLY_VARIABLES {
            assert!(
                !normal.contains(&format!("\"{switch}\"")),
                "normal canvas source reads {switch}"
            );
        }
        let mut read: Vec<&str> = normal
            .split("os::getEnvOr(\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .collect();
        read.sort_unstable();
        read.dedup();
        assert_eq!(read, ["MFB_CANVAS_GPU", "MFB_CANVAS_SYNC", "MFB_HEADLESS"]);
    }

    /// plan-130-E: a normal build's canvas source carries no reporting hook — nothing
    /// reads the stats or dump variable, and `__canvas_writeStats` does not exist.
    /// (Comments in the source may still name the variables; the check is for code.)
    #[test]
    fn the_normal_source_has_no_reporting_hook() {
        let normal = registry()
            .resolve_package("canvas")
            .expect("canvas")
            .get_mfb_for(false);
        for hook in [
            "\"MFB_CANVAS_STATS\"",
            "\"MFB_CANVAS_DUMP\"",
            "__canvas_writeStats",
            // plan-172-J: the scripted pointer, and the one MFBASIC caller of the
            // pointer's writer.
            "\"MFB_CANVAS_POINTER\"",
            "canvas::publishPointer",
            // plan-172-K: the stretched AutoPaint window.
            "\"MFB_CANVAS_AUTO_WINDOW_MS\"",
        ] {
            assert!(
                !normal.contains(hook),
                "normal canvas source contains {hook}"
            );
        }
    }

    /// bug-756: text becomes glyphs in exactly one function, `__canvas_shapeRun`.
    ///
    /// `measureText`'s DESC promises that "measuring and drawing use the same glyph
    /// walk". That holds only if no other function turns a codepoint into a glyph id
    /// or an advance itself: a second walk is where kerning, fallback or spacing gets
    /// added to one path and not the other, and the measured width stops matching the
    /// drawn one without any build failing. So the per-glyph readers may be called
    /// from the shaping helper and nowhere else, in either build's assembled source.
    #[test]
    fn canvas_text_has_one_glyph_walk() {
        // Both builds: `__canvas_geoRunReference`, the verify-mode reference for the
        // glyph run, exists only in the `--debug` source.
        let canvas = registry().resolve_package("canvas").expect("canvas");
        let mut offenders: Vec<String> = Vec::new();
        for debug in [false, true] {
            let source = canvas.get_mfb_for(debug);
            let mut current = String::new();
            for line in source.lines() {
                let code = line.trim_start();
                if let Some(rest) = code.strip_prefix("FUNC ") {
                    current = rest.split('(').next().unwrap_or("").trim().to_string();
                    continue;
                }
                if code.starts_with('\'') {
                    continue;
                }
                let calls = code.contains("__canvas_glyphIndex(")
                    || code.contains("__canvas_glyphAdvance(");
                if calls && current != "__canvas_shapeRun" && !offenders.contains(&current) {
                    offenders.push(current.clone());
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "the per-glyph readers are called outside `__canvas_shapeRun`: \
             {offenders:?}. Every text consumer must read the one shaped run, or \
             measuring and drawing can drift apart",
        );
    }

    fn assert_parses(source: &str) {
        if crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-canvas>"),
            "builtins/canvas.mfb",
            source,
        )
        .is_err()
        {
            // The parser reports its own diagnostics against `<builtin-canvas>`, a
            // file that exists only in memory — so echo the numbered source, or the
            // line numbers it just printed name nothing a developer can open.
            let mut report = String::new();
            for (index, line) in source.lines().enumerate() {
                report.push_str(&format!("\n{:5} | {line}", index + 1));
            }
            panic!("reassembled canvas source does not parse (diagnostics above):{report}");
        }
    }

    /// The package registers and its types are visible as builtin types, which is
    /// what lets a program write `Circle[…]` and `List OF DrawItem` bare.
    #[test]
    fn canvas_types_are_builtin_types() {
        for name in [
            "DrawItem",
            "DrawLayer",
            "Paint",
            "Size",
            "FillRule",
            "JoinStyle",
            "TextMetrics",
            "Transform",
            "BlendMode",
            "Circle",
            "Picture",
        ] {
            assert!(
                registry().is_builtin_type(name),
                "`{name}` should be a builtin type"
            );
        }
    }
}
