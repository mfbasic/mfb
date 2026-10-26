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
//! `canvas::Text.fonts` a `List OF RES canvas::Font` (plan-162-C). The scene draws
//! through the resources you still own; close one and the item draws nothing from it.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
    RegistryResource, RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_blit_surface;
mod func_caret_stops;
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
mod func_group_block;
mod func_item_image;
mod func_group_stats;
mod func_handle_bridge;
mod func_installed_items;
mod func_installed_layers;
mod func_item_hash;
pub(crate) mod func_item_lanes;
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
mod helper_kern;
mod helper_liga;
mod helper_paint_defaults;
mod helper_particles;
mod helper_picture;
mod helper_png;
mod helper_render;
mod helper_shapes;
mod helper_surface;
mod scene_base;
mod types_groups_curves;
mod types_items;
mod types_motion;
mod types_paint;
mod types_values;

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

An item wholly outside the window costs no drawing. Each frame, an item whose
bounds — after every enclosing `canvas::Group`'s position and transform — miss
the surface is skipped, and one that crosses an edge is drawn. It stays in the
scene, so a large map can be one group of `canvas::Tile`s moved under the window:
the tiles that scroll back into view are drawn again with nothing re-presented
but the group.

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
is a `RES canvas::Image` and `canvas::Text.fonts` a list of `RES canvas::Font`, in
fallback order. A published scene may still outlive what it names: closing an image a
scene still draws makes that item draw nothing, and closing a font makes its text draw
from the next font in its list, or nothing when none is left, rather than failing the
frame.

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
/// The type set is declared as registry data by the `types_*` sub-registrars this calls, in order (`add_record` / `add_union` /
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
        // plan-162-D: `__canvas_shapeRun` marks grapheme boundaries with
        // `strings::graphemes`, the segmentation `strings` already ships (utf8proc's
        // properties), rather than a second copy of it here.
        "strings",
        // plan-172-B: every coordinate pair is a `vector::Float2`. Injected by
        // `vector`'s late pass, because a canvas program need not import it. This is
        // the largest import here by far: `vector` has no gated helpers, so its whole
        // companion — nine types and every overload — comes with it. Measured on
        // macOS aarch64 for a program that presents an empty scene: 2,666,000 bytes
        // without it, 2,996,236 with (+330,236), the same size a program that
        // imports `vector` itself already was.
        "vector",
    ]);

    types_values::register(&mut pkg);
    types_paint::register(&mut pkg);
    types_items::register(&mut pkg);
    types_motion::register(&mut pkg);
    types_groups_curves::register(&mut pkg);

    // ---- Resources --------------------------------------------------------

    // Both RES resources are declared with the `destroy*` member that closes them, and
    // that pairing is not stylistic: `add_resource` **derives a runtime call from the
    // close op** (`registry::runtime_specs`), so a resource declared without its close
    // member leaves a call the catalog cannot route
    // (`catalog_is_consistent`: "canvas.destroyImage: None (expected Some(Canvas))").
    //
    // `Picture` and `Text` name these directly: since plan-116-I `Picture.image` is a
    // `RES canvas::Image`, and since plan-162-C `Text.fonts` is a `List OF RES
    // canvas::Font`. The value handles that used to stand in for them are gone.

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
    func_group_block::register(&mut pkg);
    func_item_image::register(&mut pkg);
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
    func_caret_stops::register(&mut pkg);
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
    helper_kern::register(&mut pkg);
    helper_liga::register(&mut pkg);
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
mod tests;
