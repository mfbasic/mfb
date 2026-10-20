use super::*;

/// plan-176-S: the value types the items are built from — mouse events, sizes, text metrics, blend and
/// gradient modes, the renderer, frame stats and stroke styles. Registered by `canvas::register`, in this
/// position (registration order is render order).
pub(super) fn register(pkg: &mut RegistryPackage) {
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
            RecordProp {
                name: "underlineOffset",
                ty: ParameterType::Float,
                description: "Where the font puts an underline: pixels from the baseline \
                              down to the underline's top edge (negative is above the \
                              baseline). Draw it as a band `underlineThickness` tall \
                              below that edge. A font that gives no underline gets one \
                              0.1 em below the baseline.",
            },
            RecordProp {
                name: "underlineThickness",
                ty: ParameterType::Float,
                description: "The underline's thickness in pixels; 0.05 em for a font \
                              that gives none.",
            },
            RecordProp {
                name: "strikeoutOffset",
                ty: ParameterType::Float,
                description: "Where the font puts a strikeout line: pixels from the \
                              baseline down to its top edge, so it is negative — the line \
                              crosses the letters above the baseline. A font that gives \
                              none gets one 0.26 em above the baseline.",
            },
            RecordProp {
                name: "strikeoutThickness",
                ty: ParameterType::Float,
                description: "The strikeout line's thickness in pixels; the underline's \
                              thickness for a font that gives none.",
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
}
