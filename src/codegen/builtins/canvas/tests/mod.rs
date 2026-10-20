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
///
/// **plan-173 appended `Tile`, last, and moved no tag.** It is paint-free by
/// design: the image texel is its colour.
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
            "Tile",
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

/// Every *painted* variant carries a `paint`, which is what makes `Paint` a
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
///
/// **plan-173 added the first paint-free leaf, `Tile`.** Its sampled texel is the
/// colour, with no tint or stroke surface. It is checked as an explicit singleton
/// so a later paint-free variant cannot bypass this guard accidentally.
#[test]
fn every_draw_item_variant_carries_a_paint() {
    /// Variants that are containers rather than shapes: `Group` (plan-116-G) and
    /// `ParticleSystem` (plan-150-A), whose template carries the paint.
    const CONTAINERS: &[&str] = &["Group", "ParticleSystem"];
    const PAINT_FREE: &[&str] = &["Tile"];
    /// Variants that draw a `shape` they hold, with that shape's paint, and the
    /// union that shape is: `AutoPaint` (plan-172-K) and `Picture` (plan-172-L),
    /// whose shape may itself be an `AutoPaint`.
    const DECORATORS: &[(&str, &str)] = &[("AutoPaint", "Shape"), ("Picture", "AutoShape")];

    let pkg = registry()
        .resolve_package("canvas")
        .expect("canvas package");
    let all = members("DrawItem");
    let decorators: Vec<&str> = DECORATORS.iter().map(|(name, _)| *name).collect();
    for name in CONTAINERS
        .iter()
        .chain(PAINT_FREE.iter())
        .chain(decorators.iter())
    {
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
        if PAINT_FREE.contains(&variant) {
            assert!(
                !record.props.iter().any(|p| p.name == "paint"),
                "`{variant}` is the paint-free image leaf but declares a paint"
            );
            assert!(
                record.props.iter().any(|p| p.name == "image"),
                "`{variant}` is paint-free only because its image supplies colour"
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

    for (owner, field_name, resource, in_list) in [
        ("Picture", "image", "canvas.Image", false),
        // plan-162-C: a `Text` names a LIST of fonts, and each element is still the
        // resource itself, not a copy.
        ("Text", "fonts", "canvas.Font", true),
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
        let held = ParameterType::res(ParameterType::named(resource));
        let expected = if in_list {
            ParameterType::list_of(held)
        } else {
            held
        };
        assert_eq!(
            field.ty, expected,
            "`{owner}.{field_name}` must be `RES {resource}` (in a list for `Text`). A bare \
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
const DEBUG_ONLY_VARIABLES: [&str; 12] = [
    "MFB_CANVAS_STATS",
    "MFB_CANVAS_FACE_DUMP",
    "MFB_CANVAS_FACE_CHECK",
    "MFB_CANVAS_SHAPE_DUMP",
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
            // plan-162-A: the face-index readers are the per-glyph readers too —
            // `__canvas_shapeRun` reads through them — so they are held to the same
            // rule. `__canvas_faceCheck` is the one other caller: the `--debug`
            // oracle that holds each index reader to the byte reader it replaced,
            // for every codepoint. It turns no text into glyphs. plan-162-F:
            // `__canvas_ligaRun` is a stage of the shaped run itself — called only by
            // `__canvas_shapeRun`, it reads the ligature glyph's advance as it
            // substitutes — so it is the run, not a second walk beside it.
            let calls = code.contains("__canvas_glyphIndex(")
                || code.contains("__canvas_glyphAdvance(")
                || code.contains("__canvas_faceGlyph(")
                || code.contains("__canvas_faceAdvance(");
            let allowed = current == "__canvas_shapeRun"
                || current == "__canvas_ligaRun"
                || current == "__canvas_faceCheck";
            if calls && !allowed && !offenders.contains(&current) {
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
