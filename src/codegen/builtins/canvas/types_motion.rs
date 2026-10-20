use super::*;

/// plan-176-S: particles and automatic paint — `ParticleSystem`, its emission, motion and appearance, and
/// `AutoPaint`. Registered by `canvas::register`, in this
/// position (registration order is render order).
pub(super) fn register(pkg: &mut RegistryPackage) {
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
                      tenth to the time. A `canvas::Text` shape of two letters \
                      costs about twice a disc, whatever the size of its font: 7 \
                      milliseconds a frame at 1,000 particles and 53 at 10,000. \
                      Polling the mouse \
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
}
