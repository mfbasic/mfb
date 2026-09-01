//! The built-in `canvas` package (plan-98-B).
//!
//! `canvas` is the 2D drawing surface of `Mode.Canvas` (plan-98-A). Its model is a
//! **retained scene**, not an immediate-mode command stream: a program builds a
//! `List OF DrawItem` and installs it with `canvas::present`, and the runtime keeps
//! rendering that scene on vsync / resize / damage until the next `present`. This is
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
//! through the `ImageRef`/`FontRef` value handles, since a record field cannot hold
//! a resource.

// --- codegen tier imports (migration) ---
use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
    RegistryResource, RegistryUnion, UnionVariant,
};
use crate::types::ParameterType;

mod func_blit_surface;
mod func_create_image;
mod func_destroy_font;
mod func_destroy_image;
mod func_fill;
mod func_fill_stroke;
mod func_font_ref;
mod func_get_bytes;
mod func_get_size;
mod func_graphics;
mod func_image_ref;
mod func_installed_items;
mod func_installed_layers;
pub(crate) mod func_load_font;
mod func_load_image;
mod func_measure_text;
mod func_metal_draw;
mod func_new_surface;
mod func_present;
mod func_present_layers;
mod func_publish_scene;
mod func_rgb;
mod func_rgba;
mod func_scene_hashes;
mod func_set_bytes;
mod func_stroke;
mod gen_font;
mod gen_font_table;
mod gen_image;
mod gen_present;
mod helper_clamp_byte;
mod helper_color;
mod helper_damage;
mod helper_draw;
mod helper_font;
mod helper_geometry;
mod helper_glyph;
mod helper_glyph_cache;
mod helper_inflate;
mod helper_items;
mod helper_paint_defaults;
mod helper_png;
mod helper_render;
mod helper_shapes;
mod helper_surface;
mod scene_base;

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
presents. Like `app`, it is importable **only** in `--app` builds, and every call
that touches the surface requires `app::Mode.Canvas` — outside it they raise the
trappable `ErrWrongMode`. The two colour constructors `canvas::rgb` and
`canvas::rgba` are the exception: they touch no surface, they only build a
`canvas::Color`, so a program can compute its palette before it ever presents anything.

`canvas` is **retained**, not immediate. A program builds a `List OF canvas::DrawItem` and
installs it with `canvas::present`; the runtime keeps rendering that scene on
vsync, resize and damage until the next `present`. `present` is therefore not a
per-frame call — a static picture is presented once and costs nothing thereafter,
and re-presenting an unchanged scene is a no-op. Animated content does call it
every frame, which is why the runtime caches each item's geometry on a content
hash: re-presenting an item that did not change is free.

`present` **copies the whole scene** — item fields, polygon point lists, text
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
LET glow AS Paint = WITH canvas::fill(red) { blend := BlendMode.Add }
```

`canvas::Paint` is designed so that **each field's zero value is that field's no-op** —
transparent fill and stroke, zero stroke width, `Normal` blend, the identity
transform (which is the *all-zero* `canvas::Transform`, by definition) and a zero-area,
meaning absent, clip. That is what lets `canvas::fill(red)` mean simply "a red
shape": every field the caller did not name is already inert.

An item that draws an image or text names it through a `canvas::ImageRef` / `canvas::FontRef` —
a plain value holding the id the backend knows the resource by. The scene holds
that id and nothing more, which is what lets a published scene outlive the image
it names. `canvas::imageRef` takes that id from an `Image`.

`Image` is a resource, so it is bound with `RES` and named
**package-qualified**, exactly like `fs::File`:

```
RES logo AS canvas::Image = canvas::createImage(w, h, pixels)
```

The value types — `canvas::Color`, `canvas::DrawItem`, `canvas::Paint` and the rest — are referenced bare.
An image closes itself when its binding goes out of scope, or earlier with
`canvas::destroyImage`; destroying one that a presented scene still draws is
safe, because the scene holds only its id."#;

/// Register the `canvas` package on the clean-room registry.
///
/// The type set is declared here as registry data (`add_record` / `add_union` /
/// `add_enum`); there is no `.mfb` companion source.
///
/// **The `DrawItem` variant set is closed.** Adding a variant later is a breaking
/// change — a user's `SELECT CASE` over the union stops being exhaustive — so the
/// full set is frozen here rather than shipped as a subset and extended
/// (plan-98-A invariant 6).
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("canvas", MODULE_INTRO, MODULE_DESC);
    // The companion source needs `collections` (the surface is a `List OF Byte`),
    // `math` (`sqrt`, the one transcendental-free primitive the distance functions
    // use), `os`/`fs` (the headless frame dump), and `canvas` itself — a package
    // reaches its own internal-only members through the qualified spelling, exactly
    // as `astrings` reaches `astrings::readSpans`.
    pkg.add_imports(vec![
        "canvas",
        "collections",
        "math",
        "os",
        "fs",
        "encoding",
    ]);

    // ---- Value types the items are built from -----------------------------

    pkg.add_record(RegistryRecord {
        name: "Color",
        export: true,
        description: "An 8-bit-per-channel RGBA colour. Build one with `canvas::rgb` \
                      or `canvas::rgba`; the all-zero value is fully transparent, \
                      which is what makes it the no-op default for a `canvas::Paint` \
                      channel.",
        props: vec![
            RecordProp {
                name: "red",
                ty: ParameterType::Byte,
                description: "The red channel, `0`..`255`.",
            },
            RecordProp {
                name: "green",
                ty: ParameterType::Byte,
                description: "The green channel, `0`..`255`.",
            },
            RecordProp {
                name: "blue",
                ty: ParameterType::Byte,
                description: "The blue channel, `0`..`255`.",
            },
            RecordProp {
                name: "alpha",
                ty: ParameterType::Byte,
                description: "The alpha channel, `0` fully transparent to `255` fully opaque.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Point",
        export: true,
        description: "A point in canvas pixels, top-left origin, Y increasing downward.",
        props: vec![
            RecordProp {
                name: "x",
                ty: ParameterType::Float,
                description: "The horizontal coordinate in pixels.",
            },
            RecordProp {
                name: "y",
                ty: ParameterType::Float,
                description: "The vertical coordinate in pixels, increasing downward.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Size",
        export: true,
        description: "A pixel extent — the dimensions of the canvas surface or of an image.",
        props: vec![
            RecordProp {
                name: "width",
                ty: ParameterType::Integer,
                description: "The width in pixels.",
            },
            RecordProp {
                name: "height",
                ty: ParameterType::Integer,
                description: "The height in pixels.",
            },
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Bounds",
        export: true,
        description: "An axis-aligned rectangle in canvas pixels. A zero-area \
                      `canvas::Bounds` (either extent `0.0`) means \"no rectangle\", which \
                      is how an unset `canvas::Paint.clip` reads as unclipped.",
        props: vec![
            RecordProp {
                name: "x",
                ty: ParameterType::Float,
                description: "The left edge in pixels.",
            },
            RecordProp {
                name: "y",
                ty: ParameterType::Float,
                description: "The top edge in pixels.",
            },
            RecordProp {
                name: "w",
                ty: ParameterType::Float,
                description: "The width in pixels. `0.0` makes the rectangle empty.",
            },
            RecordProp {
                name: "h",
                ty: ParameterType::Float,
                description: "The height in pixels. `0.0` makes the rectangle empty.",
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
                description: "Multiply source and destination; darkens.",
                advisory: None,
            },
            EnumVariant {
                name: "Screen",
                description: "Inverse-multiply source and destination; lightens.",
                advisory: None,
            },
            EnumVariant {
                name: "Add",
                description: "Add source to destination, clamped; the usual choice for glows.",
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
                      unset `canvas::Paint.transform` leave an item untransformed.",
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

    // A `DrawItem` variant is a record, and **a record field cannot hold a
    // resource** — the language rejects both `font AS Font` (a resource is not a
    // value type) and `font AS RES Font` (`RES` does not parse in a field position).
    // So the scene names a resource through a plain value handle instead, which is
    // exactly the model plan-98 already specified: the backend owns the one real
    // copy and MFB holds only the id. `canvas::imageRef`/`fontRef` read the id out
    // of the owning resource.
    //
    // This is what keeps the scene from retaining anything: a handle is an
    // `Integer`, so a published scene has no opinion about the resource's lifetime.
    // A handle naming a destroyed resource is not a dangling pointer — the runtime
    // simply defers freeing the backing object until the GPU is done with it.
    pkg.add_record(RegistryRecord {
        name: "ImageRef",
        export: true,
        description: "A plain value naming an `Image` — the id the backend knows it \
                      by. Obtain one with `canvas::imageRef`. The zero handle names \
                      no image and draws nothing, which is what an unset \
                      `canvas::Picture.image` is.",
        props: vec![RecordProp {
            name: "id",
            ty: ParameterType::Integer,
            description: "The backend's id for the image. `0` names no image.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "FontRef",
        export: true,
        description: "A plain value naming a `Font` — the id the backend knows it \
                      by. Obtain one with `canvas::fontRef`. The zero handle names \
                      no font, so text carrying it measures and draws as empty.",
        props: vec![RecordProp {
            name: "id",
            ty: ParameterType::Integer,
            description: "The backend's id for the font. `0` names no font.",
        }],
    });

    pkg.add_record(RegistryRecord {
        name: "Paint",
        export: true,
        description: "How an item is filled, stroked, blended, transformed and \
                      clipped. A flat value threaded through each item — there is no \
                      ambient drawing state. Every field's zero value is that \
                      field's no-op, so a partially named `canvas::Paint` does the obvious \
                      thing: `canvas::Paint[fill := c]` is a plain filled shape.",
        props: vec![
            RecordProp {
                name: "fill",
                ty: ParameterType::named("Color"),
                description: "The interior colour. Transparent (the zero `canvas::Color`) \
                              leaves the item unfilled.",
            },
            RecordProp {
                name: "stroke",
                ty: ParameterType::named("Color"),
                description: "The outline colour. Transparent (the zero `canvas::Color`) \
                              leaves the item unstroked.",
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
                              The zero value is `Normal`.",
            },
            RecordProp {
                name: "transform",
                ty: ParameterType::named("Transform"),
                description: "The affine transform applied to the item's geometry. \
                              The all-zero value is the identity.",
            },
            RecordProp {
                name: "clip",
                ty: ParameterType::named("Bounds"),
                description: "Restricts drawing to this rectangle. A zero-area \
                              `canvas::Bounds` — the zero value — means no clipping.",
            },
        ],
    });

    // ---- The eight `DrawItem` variants (a CLOSED set) ---------------------

    pkg.add_record(RegistryRecord {
        name: "Rectangle",
        export: true,
        description: "An axis-aligned rectangle.",
        props: rect_props("The rectangle"),
    });

    pkg.add_record(RegistryRecord {
        name: "RoundedRect",
        export: true,
        description: "An axis-aligned rectangle with rounded corners.",
        props: {
            let mut props = rect_props("The rectangle");
            props.insert(
                4,
                RecordProp {
                    name: "cornerRadius",
                    ty: ParameterType::Float,
                    description: "The corner radius in pixels, clamped to half the \
                                  shorter side.",
                },
            );
            props
        },
    });

    pkg.add_record(RegistryRecord {
        name: "Line",
        export: true,
        description: "A straight segment from one point to another. A line has no \
                      interior, so it is drawn from `paint.stroke`/`paint.strokeWidth` \
                      and ignores `paint.fill`.",
        props: vec![
            RecordProp {
                name: "x1",
                ty: ParameterType::Float,
                description: "The starting point's X coordinate in pixels.",
            },
            RecordProp {
                name: "y1",
                ty: ParameterType::Float,
                description: "The starting point's Y coordinate in pixels.",
            },
            RecordProp {
                name: "x2",
                ty: ParameterType::Float,
                description: "The ending point's X coordinate in pixels.",
            },
            RecordProp {
                name: "y2",
                ty: ParameterType::Float,
                description: "The ending point's Y coordinate in pixels.",
            },
            paint_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Polygon",
        export: true,
        description: "A closed polygon through the given points, in order. Fewer \
                      than three points has no area and draws only its stroke.",
        props: vec![
            RecordProp {
                name: "points",
                ty: ParameterType::list_of(ParameterType::named("Point")),
                description: "The vertices in order. The polygon closes from the \
                              last point back to the first automatically.",
            },
            paint_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Circle",
        export: true,
        description: "A circle given by its centre and radius.",
        props: vec![
            RecordProp {
                name: "x",
                ty: ParameterType::Float,
                description: "The centre's X coordinate in pixels.",
            },
            RecordProp {
                name: "y",
                ty: ParameterType::Float,
                description: "The centre's Y coordinate in pixels.",
            },
            RecordProp {
                name: "radius",
                ty: ParameterType::Float,
                description: "The radius in pixels.",
            },
            paint_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Arc",
        export: true,
        description: "A circular arc — the part of a circle's outline between two \
                      angles. Angles are in **radians, measured clockwise from +X**; \
                      because Y increases downward, a `0.0`..`PI` arc sweeps below \
                      its centre (so that is the smile, not the frown). An arc has \
                      no interior, so it is drawn from `paint.stroke`.",
        props: vec![
            RecordProp {
                name: "x",
                ty: ParameterType::Float,
                description: "The centre's X coordinate in pixels.",
            },
            RecordProp {
                name: "y",
                ty: ParameterType::Float,
                description: "The centre's Y coordinate in pixels.",
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
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Text",
        export: true,
        description: "A run of text drawn in a font at a size. `x`/`y` place the \
                      start of the baseline, not the top-left corner — use \
                      `canvas::measureText` to lay text out without drawing it.",
        props: vec![
            RecordProp {
                name: "x",
                ty: ParameterType::Float,
                description: "The X coordinate of the baseline's start, in pixels.",
            },
            RecordProp {
                name: "y",
                ty: ParameterType::Float,
                description: "The Y coordinate of the baseline, in pixels.",
            },
            RecordProp {
                name: "text",
                ty: ParameterType::String,
                description: "The text to draw.",
            },
            RecordProp {
                name: "font",
                ty: ParameterType::named("FontRef"),
                description: "The font to draw it in, as a handle from \
                              `canvas::fontRef`. The scene holds the id only — it \
                              does not keep the `Font` resource alive.",
            },
            RecordProp {
                name: "size",
                ty: ParameterType::Float,
                description: "The em size in pixels.",
            },
            paint_prop(),
        ],
    });

    pkg.add_record(RegistryRecord {
        name: "Picture",
        export: true,
        description: "An image drawn into a rectangle, scaled to fit it. Named \
                      `canvas::Picture` rather than `Image` because `Image` is the resource \
                      type this variant *names* — the two would collide.",
        props: {
            let mut props = rect_props("The destination rectangle");
            props.insert(
                4,
                RecordProp {
                    name: "image",
                    ty: ParameterType::named("ImageRef"),
                    description: "The image to draw, as a handle from \
                                  `canvas::imageRef`. The scene holds the id only — \
                                  it does not keep the `Image` resource alive.",
                },
            );
            props
        },
    });

    pkg.add_union(RegistryUnion {
        name: "DrawItem",
        export: true,
        variants: vec![
            UnionVariant {
                name: "Picture",
                description: "An image drawn into a rectangle.",
            },
            UnionVariant {
                name: "Rectangle",
                description: "An axis-aligned rectangle.",
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
                name: "RoundedRect",
                description: "A rectangle with rounded corners.",
            },
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
    // Nothing in the frozen `DrawItem` set depends on them: a record field cannot
    // hold a resource, so `Picture`/`Text` name the value handles `ImageRef` /
    // `FontRef` instead — see their declarations above.

    pkg.add_resource(RegistryResource {
        name: IMAGE_TYPE,
        export: true,
        description: "An opaque handle to an image the drawing backend holds, closed \
                      automatically when its binding goes out of scope. A scene names one \
                      through a `canvas::ImageRef`, never directly, so destroying an image a \
                      scene still draws is safe.",
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
        close_may_fail: false,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    pkg.add_resource(RegistryResource {
        name: FONT_TYPE,
        export: true,
        description: "An opaque, owned handle to a loaded font, released \
                      automatically when it leaves scope. A scene names one through a \
                      `FontRef`, never directly, so releasing a font whose text a \
                      scene still draws is safe — that text simply draws as empty.",
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
        close_may_fail: false,
        kind: crate::codegen::resource::ResourceKind::Builtin,
    });

    func_rgb::register(&mut pkg);
    func_rgba::register(&mut pkg);
    func_fill::register(&mut pkg);
    func_stroke::register(&mut pkg);
    func_fill_stroke::register(&mut pkg);
    func_new_surface::register(&mut pkg);
    func_present::register(&mut pkg);
    func_publish_scene::register(&mut pkg);
    func_blit_surface::register(&mut pkg);
    func_metal_draw::register(&mut pkg);
    func_graphics::register(&mut pkg);
    func_installed_items::register(&mut pkg);
    func_installed_layers::register(&mut pkg);
    func_scene_hashes::register(&mut pkg);
    func_present_layers::register(&mut pkg);
    func_create_image::register(&mut pkg);
    func_load_image::register(&mut pkg);
    func_destroy_image::register(&mut pkg);
    func_image_ref::register(&mut pkg);
    gen_font_table::register(&mut pkg);
    func_load_font::register(&mut pkg);
    func_measure_text::register(&mut pkg);
    func_destroy_font::register(&mut pkg);
    func_font_ref::register(&mut pkg);
    func_get_size::register(&mut pkg);
    func_get_bytes::register(&mut pkg);
    func_set_bytes::register(&mut pkg);
    helper_clamp_byte::register(&mut pkg);
    helper_paint_defaults::register(&mut pkg);
    // Order matters only for readability — the helper section renders in call order.
    helper_color::register(&mut pkg);
    helper_shapes::register(&mut pkg);
    helper_draw::register(&mut pkg);
    helper_font::register(&mut pkg);
    helper_glyph::register(&mut pkg);
    helper_damage::register(&mut pkg);
    helper_glyph_cache::register(&mut pkg);
    helper_inflate::register(&mut pkg);
    helper_png::register(&mut pkg);
    helper_geometry::register(&mut pkg);
    helper_items::register(&mut pkg);
    helper_surface::register(&mut pkg);
    helper_render::register(&mut pkg);

    r.add_package(pkg);
}

/// The `x`/`y`/`w`/`h` prefix shared by every rectangle-shaped item, plus the
/// trailing `paint`. Callers that need an extra field (`cornerRadius`, `image`)
/// insert it at index 4, between the extent and the paint.
fn rect_props(what: &'static str) -> Vec<RecordProp> {
    vec![
        RecordProp {
            name: "x",
            ty: ParameterType::Float,
            description: "The left edge in pixels.",
        },
        RecordProp {
            name: "y",
            ty: ParameterType::Float,
            description: "The top edge in pixels.",
        },
        RecordProp {
            name: "w",
            ty: ParameterType::Float,
            description: if what.starts_with("The destination") {
                "The destination width in pixels; the image is scaled to it."
            } else {
                "The width in pixels."
            },
        },
        RecordProp {
            name: "h",
            ty: ParameterType::Float,
            description: if what.starts_with("The destination") {
                "The destination height in pixels; the image is scaled to it."
            } else {
                "The height in pixels."
            },
        },
        paint_prop(),
    ]
}

/// The `paint` field every `DrawItem` variant carries.
fn paint_prop() -> RecordProp {
    RecordProp {
        name: "paint",
        ty: ParameterType::named("Paint"),
        description: "How to fill, stroke, blend, transform and clip the item.",
    }
}

// Man/spec citation anchor: `CANVAS`. The canvas man pages and the app canvas spec
// section ground their package-level and `DrawItem`-set facts here.

#[cfg(test)]
mod tests {
    use crate::codegen::registry::registry;
    use crate::types::ParameterType;

    /// The eight `DrawItem` variants are a **closed set** (plan-98-A invariant 6):
    /// adding one later stops a user's `SELECT CASE` being exhaustive, which is a
    /// breaking change. Pinning the exact list — and its order, which fixes the
    /// tags — makes any addition a deliberate, visible act rather than a silent one.
    #[test]
    fn draw_item_variant_set_is_frozen() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        let union = pkg
            .unions()
            .iter()
            .find(|u| u.name == "DrawItem")
            .expect("DrawItem union");
        let names: Vec<&str> = union.variants.iter().map(|v| v.name).collect();
        assert_eq!(
            names,
            vec![
                "Picture",
                "Rectangle",
                "Line",
                "Polygon",
                "Circle",
                "Arc",
                "Text",
                "RoundedRect",
            ],
            "the DrawItem variant set is frozen; extending it is a breaking change"
        );
    }

    /// Every `DrawItem` variant must name a record the package actually declares,
    /// or the union references a type that does not exist.
    #[test]
    fn every_draw_item_variant_has_a_record() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        let union = pkg
            .unions()
            .iter()
            .find(|u| u.name == "DrawItem")
            .expect("DrawItem union");
        for variant in &union.variants {
            assert!(
                pkg.records().iter().any(|r| r.name == variant.name),
                "DrawItem variant `{}` has no record declaration",
                variant.name
            );
        }
    }

    /// Every variant carries a `paint`, which is what makes `Paint` a threaded value
    /// rather than ambient state. A variant that forgot it would silently draw with
    /// no way to colour it.
    #[test]
    fn every_draw_item_variant_carries_a_paint() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        let union = pkg
            .unions()
            .iter()
            .find(|u| u.name == "DrawItem")
            .expect("DrawItem union");
        for variant in &union.variants {
            let record = pkg
                .records()
                .iter()
                .find(|r| r.name == variant.name)
                .expect("variant record");
            let paint = record
                .props
                .iter()
                .find(|p| p.name == "paint")
                .unwrap_or_else(|| panic!("`{}` has no paint field", variant.name));
            assert_eq!(paint.ty.name(), "Paint", "{}", variant.name);
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

    /// A record field cannot hold a resource, so the two variants that name one
    /// carry a value handle instead. This pins the *shape* of that decision: the
    /// handle is a plain `Integer`, which is what keeps a published scene from
    /// having any opinion about a resource's lifetime.
    #[test]
    fn resource_handles_are_plain_integer_values() {
        let pkg = registry()
            .resolve_package("canvas")
            .expect("canvas package");
        for (handle, owner) in [("ImageRef", "Picture"), ("FontRef", "Text")] {
            let record = pkg
                .records()
                .iter()
                .find(|r| r.name == handle)
                .unwrap_or_else(|| panic!("{handle} record"));
            assert_eq!(record.props.len(), 1, "{handle}");
            assert_eq!(record.props[0].name, "id");
            assert_eq!(record.props[0].ty, ParameterType::Integer, "{handle}");

            let variant = pkg
                .records()
                .iter()
                .find(|r| r.name == owner)
                .unwrap_or_else(|| panic!("{owner} record"));
            let field = variant
                .props
                .iter()
                .find(|p| p.ty.name() == handle)
                .unwrap_or_else(|| panic!("{owner} should name a {handle}"));
            assert!(matches!(field.name, "image" | "font"), "{owner}");
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
        let source = registry()
            .resolve_package("canvas")
            .expect("canvas")
            .get_mfb();
        if crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-canvas>"),
            "builtins/canvas.mfb",
            &source,
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
            "Color",
            "Point",
            "Size",
            "Bounds",
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
