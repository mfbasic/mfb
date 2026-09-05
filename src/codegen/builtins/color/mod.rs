//! Package: color
//! Type: Pure MFBasic (colour value type, constructors, packed and hex text forms)
//!
//! One colour type for the whole language. Before plan-122 MFBASIC had three
//! unrelated notions of a colour that nothing converted between: `canvas::Color`
//! (a 4-`Byte` RGBA record), `term::TermColor` (a 3-`Byte` RGB record the runtime
//! allocates) and an `astrings` foreground/background attribute carrying a packed
//! `0xRRGGBB` `Integer` with no type at all. `color::Color` replaces all three.
//!
//! `color` is a **pure-source** package modelled on `encoding`: one `func_*.rs`
//! per public member carrying its `INTRO`/`DESC`/`EX` prose and a
//! [`Body::mfb`](crate::codegen::registry::Body) body, plus `helper_*.rs` chunks
//! for the private `__color_*` FUNCs, assembled by
//! [`RegistryPackage::get_mfb`](crate::codegen::registry::RegistryPackage::get_mfb)
//! in the generic imports → records → helpers → member-bodies order and injected
//! by the generic `registry::augment_project` pass.
//!
//! Two rules govern every member here and are worth stating once:
//!
//! - **Components clamp, they do not raise.** Colours are computed — a base plus a
//!   delta, a channel scaled by a fraction, an interpolation — and a value that
//!   lands one past an end is a rounding artefact, not a program bug. This is the
//!   contract inherited verbatim from `canvas::rgb`/`rgba`; `__color_clampByte` is
//!   the whole implementation of it. It is also why every component parameter is
//!   declared `Integer` rather than `Byte`: a `Byte` parameter would push an
//!   out-of-range value into a conversion error at the call site, which is the
//!   opposite of the promise.
//! - **No transcendentals, ever.** canvas's software rasteriser is the oracle its
//!   GPU backends are compared against and must produce identical bytes on every
//!   target (`crate::codegen::builtins::canvas::helper_color`); from plan-122-B
//!   onward canvas calls into `color`, so the whole package inherits that rule.
//!   IEEE `+ - * /` and `sqrt` only — no `pow`, no `exp`, no trig.
//!
//! Unlike `term::TermColor`, `Color` is an ordinary value record: the runtime does
//! not allocate it, so it is absent from any `is_read_only_record` predicate and a
//! program may build one with a record literal and `WITH`-update it.

use crate::codegen::registry::{
    RecordProp, Registry, RegistryOverride, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

mod constants;
mod func_brighten;
mod func_contrast_ratio;
mod func_darken;
mod func_desaturate;
mod func_from_hex;
mod func_from_linear;
mod func_from_name;
mod func_from_packed;
mod func_gray;
mod func_grayscale;
mod func_hsl;
mod func_hsla;
mod func_invert;
mod func_is_dark;
mod func_is_light;
mod func_luminance;
mod func_mix;
mod func_name_of;
mod func_rgb;
mod func_rgba;
mod func_rotate_hue;
mod func_saturate;
mod func_to_hex;
mod func_to_hex_alpha;
mod func_to_hsl;
mod func_to_linear;
mod func_to_packed;
mod func_with_alpha;
mod helper_clamp_byte;
mod helper_clamp_fraction;
mod helper_hex_byte;
mod helper_hex_value;
mod helper_hsl;
mod helper_name_table;
mod helper_srgb;
mod helper_to_string;

/// The `Color` record type (`red`/`green`/`blue`/`alpha` `Byte`) — the leaf
/// spelling, as it appears inside the package's own injected companion.
pub(crate) const COLOR_TYPE: &str = "Color";
/// `Color`'s package-qualified identity — what a consumer must write, and what the
/// resolver seeds, so a bare `AS Color` from another file is refused (bug-484).
/// Every cross-package descriptor reference uses this constant, never a literal.
pub(crate) const COLOR_TYPE_ID: &str = "color.Color";

/// The `Hsl` record type (`hue`/`saturation`/`lightness` `Float`), returned by
/// `color::toHsl` — the leaf spelling, as it appears inside the package's own
/// injected companion.
pub(crate) const HSL_TYPE: &str = "Hsl";
/// `Hsl`'s package-qualified identity, seeded into `resolver::BUILTIN_TYPES` so a
/// bare `AS Hsl` from another file is refused (bug-484), exactly as `Color` is.
pub(crate) const HSL_TYPE_ID: &str = "color.Hsl";

/// The internal source-companion render target for the `toString(color::Color)`
/// override — routed here by [`RegistryPackage::add_override`], as
/// `toString(net::Url)` is. It is a private helper, not a member: a program
/// reaches it only by calling the general `toString`.
pub(crate) const COLOR_TO_STRING: &str = "__color_toString";

const MODULE_INTRO: &str = r#"One colour type for the whole language: an 8-bit-per-channel RGBA value with constructors, a packed-integer bridge, and hex text forms."#;

const MODULE_DESC: &str = r#"The `color` package defines `color::Color`, the colour value every other package
speaks. It is a built-in package: `IMPORT color` needs no manifest dependency.

A `color::Color` carries four `Byte` channels — `red`, `green`, `blue` and
`alpha`. It is an ordinary value record, so you may build one with
`color::Color[r, g, b, a]` and produce an updated copy with `WITH`, exactly as you
would any record of your own. Most programs use `color::rgb` and `color::rgba`
instead, because those clamp.

**Components clamp rather than fail.** `color::rgba(300, -20, 128, 255)` is the
same colour as `color::rgba(255, 0, 128, 255)`. Colours get computed — a base plus
a delta, a channel scaled by a fraction — and a value one past an end is a
rounding artefact, not a mistake worth stopping the program for. The component
parameters are `Integer` rather than `Byte` precisely so an out-of-range value can
reach that clamp.

**Alpha is straight, not premultiplied.** `0` is fully transparent and `255`
fully opaque, and a colour's `red`/`green`/`blue` are unaffected by it. The
all-zero colour is therefore fully transparent.

**The packed form is alpha-high**, `0xAARRGGBB`. `color::toPacked` and
`color::fromPacked` move between a colour and that single `Integer`, which is how
a colour travels through an API that carries one number.

Text forms use `color::fromHex` and `color::toHex`/`color::toHexAlpha`.
`toString` on a colour renders the lossless `#rrggbbaa` form."#;

/// Register the `color` package on the clean-room registry.
///
/// Injection is the generic path: the package registers its `IMPORT`s, the `Color`
/// record, its shared `__color_*` helpers, and each member's `Body::mfb` body, and
/// `RegistryPackage::get_mfb` assembles the injected source. Nothing about `color`
/// is bespoke.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("color", MODULE_INTRO, MODULE_DESC);

    // `color` imports itself so the assembled companion can call its own public
    // members through the qualified spelling, as `astrings` does. `bits` backs the
    // packed-integer bridge; `strings` and `collections` back the hex text forms.
    // The set grows with the members that need it rather than being declared up
    // front, so an injected `IMPORT` nothing calls never reaches an importer's
    // `.ir`.
    // `math` backs the HSL conversions (`abs`/`floor`/`min`/`max` only — no
    // transcendentals, per the module note above). It costs an importer 0 bytes:
    // its companion is empty (plan-122-A §2).
    pkg.add_imports(vec!["color", "bits", "strings", "collections", "math"]);

    pkg.add_record(RegistryRecord {
        name: COLOR_TYPE,
        export: true,
        description: "An 8-bit-per-channel RGBA colour — the colour value every \
                      package speaks. Build one with `color::rgb` or `color::rgba` \
                      (which clamp), from a packed `Integer` with \
                      `color::fromPacked`, or from text with `color::fromHex`. \
                      `alpha` is straight, not premultiplied, so the all-zero \
                      colour is fully transparent.",
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

    // The HSL view of a colour (plan-122-B §6). Deliberately describes the **sRGB**
    // colour rather than linear light: that is what CSS `hsl()` and every design
    // tool mean, so a round trip agrees with the hex a designer pasted in.
    pkg.add_record(RegistryRecord {
        name: HSL_TYPE,
        export: true,
        description: "A colour as hue, saturation and lightness — the HSL view of a \
                      `color::Color`. `color::toHsl` returns one, and `color::hsl` \
                      builds a colour back from the same three values. It describes \
                      the sRGB colour, not linear light, so it matches what a design \
                      tool shows for the same hex.",
        props: vec![
            RecordProp {
                name: "hue",
                ty: ParameterType::Float,
                description: "The hue in degrees around the colour wheel, `0.0`..`360.0`. \
                              Reported as `0.0` for a colour with no saturation.",
            },
            RecordProp {
                name: "saturation",
                ty: ParameterType::Float,
                description: "How colourful, `0.0` (a neutral grey) to `1.0` (fully saturated).",
            },
            RecordProp {
                name: "lightness",
                ty: ParameterType::Float,
                description: "How light, `0.0` (black) through `0.5` (the vivid hue) to \
                              `1.0` (white).",
            },
        ],
    });

    // `toString(color::Color)` renders the lossless `#rrggbbaa` form — the registry
    // home of what would otherwise be a hand row in
    // `builtins::general_override_target`, exactly as `toString(net::Url)` is.
    pkg.add_override(RegistryOverride {
        builtin: "toString",
        arg_type: COLOR_TYPE,
        helper: COLOR_TO_STRING,
    });

    helper_clamp_byte::register(&mut pkg);
    helper_clamp_fraction::register(&mut pkg);
    helper_hex_value::register(&mut pkg);
    helper_hex_byte::register(&mut pkg);
    helper_to_string::register(&mut pkg);
    helper_srgb::register(&mut pkg);
    // After `helper_clamp_byte`/`helper_clamp_fraction` and `helper_srgb`: the HSL
    // core calls the two clamps, and `get_mfb` renders helpers in this order.
    helper_hsl::register(&mut pkg);
    helper_name_table::register(&mut pkg);

    // Constructors first, then the operations over an existing colour — the order
    // a reader of `mfb man color` meets them in.
    func_rgb::register(&mut pkg);
    func_rgba::register(&mut pkg);
    func_gray::register(&mut pkg);
    func_with_alpha::register(&mut pkg);
    func_invert::register(&mut pkg);

    // The packed-integer bridge.
    func_to_packed::register(&mut pkg);
    func_from_packed::register(&mut pkg);

    // Text forms.
    func_from_hex::register(&mut pkg);
    func_to_hex::register(&mut pkg);
    func_to_hex_alpha::register(&mut pkg);

    // The sRGB <-> linear-light seam (plan-122-B). Public because canvas blends
    // through it: a package cannot reach another's private `__` helpers, and a
    // canvas-private duplicate of the table is exactly the drift plan-122 removes.
    func_to_linear::register(&mut pkg);
    func_from_linear::register(&mut pkg);

    // Perceptual operations, all built on the seam above so they cannot disagree
    // with what canvas renders. `luminance` before its three consumers.
    func_brighten::register(&mut pkg);
    func_darken::register(&mut pkg);
    func_mix::register(&mut pkg);
    func_grayscale::register(&mut pkg);
    func_luminance::register(&mut pkg);
    func_contrast_ratio::register(&mut pkg);
    func_is_dark::register(&mut pkg);
    func_is_light::register(&mut pkg);

    // HSL: the constructors, the decomposition, and the three manipulators.
    func_hsl::register(&mut pkg);
    func_hsla::register(&mut pkg);
    func_to_hsl::register(&mut pkg);
    func_saturate::register(&mut pkg);
    func_desaturate::register(&mut pkg);
    func_rotate_hue::register(&mut pkg);

    // CSS named colours (plan-122-C).
    func_from_name::register(&mut pkg);
    func_name_of::register(&mut pkg);

    // The basic-colour record constants. Independent of the name table above: a
    // constant inlines its four literals rather than consulting it.
    constants::register(&mut pkg);

    r.add_package(pkg);
}

/// Synthetic path/doc labels for the injected `color` source.
/// `parse_source_internal` records the path; `AstProject::to_json` filters this
/// sentinel out of `-ast` output.
const SOURCE_LABEL: &str = "<builtin-color>";
const SOURCE_DOC: &str = "builtins/color.mfb";

/// Inject the `color` package source when a program — **or another injected
/// built-in** — imports it.
///
/// `color` is injected by this dedicated late pass rather than by the generic
/// `registry::augment_project`, for the same transitivity reason as `encoding` and
/// `net`: since plan-122-B, `canvas`'s injected companion carries `IMPORT color`
/// and calls `color::toLinear`/`color::fromLinear` from its blend and gradient
/// helpers. The generic pass examines the **pre-injection** AST, where a canvas
/// program has only written `IMPORT canvas`, so it cannot see that transitive
/// import and never injected `color` — the program then failed at native lowering
/// with `NIR call target '#color_toLinear' does not resolve`, since the canvas
/// companion's call had nothing to bind to.
///
/// The generic pass therefore skips `color` (see `Registry::synthetic_files`),
/// which also prevents a double injection when a program imports `color` directly.
/// The injected *source* is the identical generic `RegistryPackage::get_mfb`
/// assembly the generic pass would produce; only the injection *position* differs.
pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    crate::codegen::registry::inject_late_pass(ast, "color", SOURCE_LABEL, SOURCE_DOC)
}

/// The same injection onto the elaborated project the former source checker consumes.
#[cfg(test)] // the HIR-domain chain serves the in-process tests only (plan-107-D)
pub(crate) fn augmented_hir_project(
    hir: &crate::hir::HirProject,
) -> Result<crate::hir::HirProject, ()> {
    crate::codegen::registry::inject_late_pass_hir(hir, "color", SOURCE_LABEL, SOURCE_DOC)
}
