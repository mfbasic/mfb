//! `canvas::ParticleSystem` (plan-150-A): one scene item drawn as many.
//!
//! The first cases here are the ones the type itself has to pass before anything is
//! drawn from it: a scene holding a system is presented, copied, compared and dropped
//! like any other, and two systems that differ only inside their template are two
//! different scenes.
//!
//! The numbers are read off `MFB_CANVAS_STATS` (one line per frame the graphics thread
//! finished) and the `--debug` report a program prints as it exits, which carries the
//! worker arena's live byte count.

#[path = "../common/mod.rs"]
mod common;

#[path = "rt_canvas_particles/lifecycle_and_model.rs"]
mod lifecycle_and_model;

use common::canvas_image::{compare_exact, compare_within_tolerance, Frame, Tolerance};
use std::process::Command;

// ---- Drawing (plan-150-A Phase 4) ------------------------------------------------------

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;

/// Render a scene headless and synchronously, returning the last frame and the stats
/// line of every frame. `env` adds to the run — `MFB_CANVAS_GPU`, a scripted pointer.
fn draw(name: &str, source: &str, env: &[(&str, &str)]) -> (Frame, Vec<String>) {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
        .expect("write the font fixture");
    let binary = common::build_app_debug(&project, name);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_DUMP", &frame);
    for (key, value) in env {
        command.env(key, value);
    }
    let out = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        out.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&out.status),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    );
    let pixels = std::fs::read(&frame).expect("canvas dump written");
    let lines = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    (Frame::from_rgba(WIDTH, HEIGHT, pixels), lines)
}

fn pixel(frame: &Frame, x: u32, y: u32) -> [u8; 4] {
    let at = ((y * frame.width + x) * 4) as usize;
    [
        frame.pixels[at],
        frame.pixels[at + 1],
        frame.pixels[at + 2],
        frame.pixels[at + 3],
    ]
}

/// The pixels that are not the surface's own opaque black, as (count, min x, min y,
/// max x, max y).
fn lit(frame: &Frame) -> (usize, u32, u32, u32, u32) {
    let (mut count, mut x0, mut y0, mut x1, mut y1) = (0, u32::MAX, u32::MAX, 0, 0);
    for y in 0..frame.height {
        for x in 0..frame.width {
            if pixel(frame, x, y) != [0, 0, 0, 255] {
                count += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    (count, x0, y0, x1, y1)
}

fn stat(line: &str, key: &str) -> String {
    line.split_whitespace()
        .find_map(|field| field.strip_prefix(key))
        .unwrap_or_else(|| panic!("no `{key}` in `{line}`"))
        .to_string()
}

/// What every drawing scene below is written with. `single` is a system of exactly one
/// particle that never moves: one burst of one, from a point, at no speed — so the
/// particle's point is the system's `position`, its turn is `rotation`, its size is
/// `scale` and its tint is `tint`, all known without running the model by hand.
const SCENE_PRELUDE: &str = r#"IMPORT app
IMPORT canvas
IMPORT collections
IMPORT color
IMPORT io
IMPORT math
IMPORT vector

FUNC none() AS canvas::Transform
  RETURN canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
END FUNC

FUNC at(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC white() AS color::Color
  RETURN color::rgb(255, 255, 255)
END FUNC

FUNC point() AS canvas::Shape
  MUT c AS canvas::Circle
  LET s AS canvas::Shape = c
  RETURN s
END FUNC

FUNC still(lifetime AS Float) AS canvas::Emission
  RETURN canvas::Emission[mode := canvas::EmissionMode.Burst, maxParticles := 1, rate := 0.0, burst := 1, burstPeriod := 0.0, lifetime := lifetime, lifetimeVar := 0.0]
END FUNC

FUNC look(first AS color::Color, last AS color::Color, scale AS Float) AS canvas::Appearance
  RETURN canvas::Appearance[startColor := first, endColor := last, startScale := scale, endScale := scale, scaleVar := 0.0]
END FUNC

FUNC single(item AS canvas::AutoShape, x AS Float, y AS Float, rotation AS Float, scale AS Float, tint AS color::Color) AS canvas::DrawItem
  MUT motion AS canvas::Motion
  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(x, y), item := item, emitter := point(), emission := still(10.0), motion := WITH motion { rotation := rotation }, appearance := look(tint, tint, scale), seed := 1, time := 1.0]
  RETURN ps
END FUNC

FUNC box(x AS Float, y AS Float, w AS Float, h AS Float, paint AS canvas::Paint, t AS canvas::Transform) AS canvas::Rectangle
  MUT r AS canvas::Rectangle
  RETURN WITH r { position := at(x, y), size := canvas::Size[width := w, height := h], paint := paint, transform := t }
END FUNC

FUNC disc(radius AS Float, paint AS canvas::Paint) AS canvas::Circle
  MUT c AS canvas::Circle
  RETURN WITH c { radius := radius, paint := paint }
END FUNC

FUNC node(name AS String, x AS Float, y AS Float, t AS canvas::Transform) AS canvas::DrawItem
  MUT g AS canvas::Group
  LET item AS canvas::DrawItem = WITH g { name := name, position := at(x, y), transform := t }
  RETURN item
END FUNC

FUNC scaled(by AS Float) AS canvas::Transform
  RETURN canvas::Transform[a := by, b := 0.0, c := 0.0, d := by, tx := 0.0, ty := 0.0]
END FUNC
"#;

fn scene(body: &str) -> String {
    format!(
        "{SCENE_PRELUDE}\nFUNC main() AS Nothing\n  app::setMode(app::Mode.Canvas)\n{body}  io::print(\"rendered\")\nEND FUNC\n"
    )
}

/// A template that tests the composition: a stroked rectangle that is not at the
/// particle's point and has a transform of its own — a turn of 36.87 degrees (0.8, 0.6)
/// and an offset.
const TILTED: &str = "box(6.0, 4.0, 60.0, 30.0, canvas::fillStroke(color::rgb(40, 110, 255), white(), 3.0), canvas::Transform[a := 0.8, b := 0.6, c := 0.0 - 0.6, d := 0.8, tx := 5.0, ty := 0.0 - 3.0])";

/// **A particle equals its definition.** One live particle renders pixel for pixel as
/// its template inside a group node at the particle's point, with the particle's
/// transform — for a particle that is only moved, and for one that is scaled, with a
/// template that carries a position and a transform of its own.
///
/// The moved particle is at a point that is not a whole pixel on purpose: see
/// `a_moved_particle_is_the_template_moved` for what a whole one shows.
#[test]
fn a_particle_is_its_template_in_a_group_node() {
    for (name, at, rotation_scale, group_transform) in [
        ("moved", "300.25, 200.5", "0.0, 1.0", "none()"),
        ("scaled", "300.0, 200.0", "0.0, 1.5", "scaled(1.5)"),
    ] {
        let (particle, _) = draw(
            &format!("canvas_particle_is_{name}"),
            &scene(&format!(
                "  canvas::present([single({TILTED}, {at}, {rotation_scale}, white())])\n"
            )),
            &[],
        );
        let (grouped, _) = draw(
            &format!("canvas_particle_group_{name}"),
            &scene(&format!(
                "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  canvas::present([node(\"p\", {at}, {group_transform})])\n"
            )),
            &[],
        );
        assert!(lit(&grouped).0 > 1000, "{name}: the reference drew nothing");
        if let Err(diff) = compare_exact(&particle, &grouped) {
            panic!("{name}: a particle is not its template in a group node: {diff}");
        }
    }
}

/// A particle that is neither turned nor scaled IS the template moved: the item the
/// expansion appends is the template with the particle's point added to its
/// `position`, and it renders exactly as that ordinary item does.
///
/// It is NOT always exactly the template in a group moved to that point, and that is
/// the renderer's own property, not the expansion's — the comparison below has no
/// particle in it. A shape with a transform is placed through an inverse held in single
/// precision; at (306, 204) that inverse carries the whole position, inside a group it
/// carries only (6, 4) and the group's (300, 200) is subtracted exactly. The two differ
/// by about 1e-5 px. This template is turned by (0.8, 0.6) on whole coordinates, so its
/// edges cross pixel centres at exact tenths and many coverages sit on a rounding tie:
/// 97 of 576,000 pixels land a level or two apart. Measured, and bounded here so that a
/// real difference between the two paths would not hide behind it.
#[test]
fn a_moved_particle_is_the_template_moved() {
    let (particle, _) = draw(
        "canvas_particle_moved",
        &scene(&format!(
            "  canvas::present([single({TILTED}, 300.0, 200.0, 0.0, 1.0, white())])\n"
        )),
        &[],
    );
    let ordinary_item = TILTED.replacen("box(6.0, 4.0,", "box(306.0, 204.0,", 1);
    let (ordinary, _) = draw(
        "canvas_particle_moved_ordinary",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {ordinary_item}\n  canvas::present([shape])\n"
        )),
        &[],
    );
    assert!(lit(&ordinary).0 > 1000);
    if let Err(diff) = compare_exact(&particle, &ordinary) {
        panic!("a moved particle is not the template moved: {diff}");
    }
    let (grouped, _) = draw(
        "canvas_particle_moved_group",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  canvas::present([node(\"p\", 300.0, 200.0, none())])\n"
        )),
        &[],
    );
    let placement = Tolerance {
        max_channel_delta: 2,
        max_differing_fraction: 0.0002,
    };
    if let Err(diff) = compare_within_tolerance(&ordinary, &grouped, placement) {
        panic!("a placed shape and the same shape in a moved group are further apart than single-precision placement explains: {diff}");
    }
}

/// The same for a particle that is TURNED. The model turns it with the package's own
/// sine and cosine, which a program cannot call, so the reference group is turned with
/// `math::`'s — the same angle to a few parts in ten million. That is far below a
/// coverage step, so the two frames may differ only in the last level of a handful of
/// edge pixels.
#[test]
fn a_turned_particle_is_its_template_in_a_turned_group_node() {
    let (particle, _) = draw(
        "canvas_particle_is_turned",
        &scene(&format!(
            "  canvas::present([single({TILTED}, 300.0, 200.0, 0.5, 1.5, white())])\n"
        )),
        &[],
    );
    let (grouped, _) = draw(
        "canvas_particle_group_turned",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  LET co AS Float = 1.5 * math::cos(0.5)\n  LET si AS Float = 1.5 * math::sin(0.5)\n  canvas::present([node(\"p\", 300.0, 200.0, canvas::Transform[a := co, b := si, c := 0.0 - si, d := co, tx := 0.0, ty := 0.0])])\n"
        )),
        &[],
    );
    assert!(lit(&grouped).0 > 1000);
    let close = Tolerance {
        max_channel_delta: 1,
        max_differing_fraction: 0.0001,
    };
    if let Err(diff) = compare_within_tolerance(&particle, &grouped, close) {
        panic!("a turned particle is not its template in a turned group node: {diff}");
    }
}

/// **A scale that reaches zero draws nothing** — not the shape at full size, which is
/// what the shape rule would make of a transform with no area.
#[test]
fn a_particle_scaled_to_nothing_is_not_drawn() {
    let (frame, _) = draw(
        "canvas_particle_zero_scale",
        &scene(
            "  canvas::present([single(disc(60.0, canvas::fill(white())), 300.0, 200.0, 0.0, 0.0, white())])\n",
        ),
        &[],
    );
    assert_eq!(lit(&frame).0, 0, "a particle of no size was drawn");
    // The same system at a size draws: the frame above is black because of the scale.
    let (frame, _) = draw(
        "canvas_particle_small_scale",
        &scene(
            "  canvas::present([single(disc(60.0, canvas::fill(white())), 300.0, 200.0, 0.0, 0.5, white())])\n",
        ),
        &[],
    );
    assert_eq!(pixel(&frame, 300, 200), [255, 255, 255, 255]);
    assert_eq!(pixel(&frame, 300, 240), [0, 0, 0, 255], "half of 60 is 30");
}

/// **A system inside a turned, scaled group** is turned and scaled with it, like any
/// item: it renders as the nested groups its definition makes of it.
#[test]
fn a_system_in_a_turned_group_is_turned_with_it() {
    let outer =
        "canvas::Transform[a := 0.96, b := 0.56, c := 0.0 - 0.56, d := 0.96, tx := 0.0, ty := 0.0]";
    let (system, _) = draw(
        "canvas_particle_in_group",
        &scene(&format!(
            "  canvas::setGroup(\"g\", [single({TILTED}, 50.25, 20.5, 0.0, 1.5, white())])\n  canvas::present([node(\"g\", 300.0, 200.0, {outer})])\n"
        )),
        &[],
    );
    let (nested, _) = draw(
        "canvas_particle_in_group_reference",
        &scene(&format!(
            "  LET shape AS canvas::DrawItem = {TILTED}\n  canvas::setGroup(\"p\", [shape])\n  canvas::setGroup(\"g\", [node(\"p\", 50.25, 20.5, scaled(1.5))])\n  canvas::present([node(\"g\", 300.0, 200.0, {outer})])\n"
        )),
        &[],
    );
    assert!(lit(&nested).0 > 1000);
    if let Err(diff) = compare_exact(&system, &nested) {
        panic!("a system in a turned group is not its definition there: {diff}");
    }
}

/// **The tint multiplies.** Each channel of the template's fill is multiplied by the
/// particle's, so white leaves it alone, and the change from `startColor` to `endColor`
/// is even in linear light: halfway from white to black is sRGB 188, not 128.
#[test]
fn the_tint_multiplies_and_fades_in_linear_light() {
    let (frame, _) = draw(
        "canvas_particle_tint",
        &scene(
            "  canvas::present([single(disc(40.0, canvas::fill(color::rgb(200, 100, 50))), 300.0, 200.0, 0.0, 1.0, color::rgb(128, 255, 0))])\n",
        ),
        &[],
    );
    // (200 * 128 + 127) / 255, 100 * 255 / 255, 50 * 0.
    assert_eq!(pixel(&frame, 300, 200), [100, 100, 0, 255]);

    let (frame, _) = draw(
        "canvas_particle_fade",
        &scene(
            "  MUT motion AS canvas::Motion\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(300.0, 200.0), item := disc(40.0, canvas::fill(white())), emitter := point(), emission := still(2.0), motion := motion, appearance := look(white(), color::rgb(0, 0, 0), 1.0), seed := 1, time := 1.0]\n  canvas::present([ps])\n",
        ),
        &[],
    );
    assert_eq!(pixel(&frame, 300, 200), [188, 188, 188, 255]);
}

/// **A `Text` template.** A run of text is a particle like any shape: placed, scaled
/// about the particle's point and tinted. The fixture font's one glyph inks the square
/// (100, 0)-(400, 300) of a 1000-unit em, so at size 40 and twice the size `A` is a
/// 24 px square from 8 px right of the point, standing on it.
#[test]
fn a_text_template_is_placed_scaled_and_tinted() {
    let (frame, _) = draw(
        "canvas_particle_text",
        &scene(
            "  RES face AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n    RETURN\n  END TRAP\n  LET label AS canvas::AutoShape = canvas::Text[id := \"\", position := at(0.0, 0.0), text := \"A\", fonts := [face], fontSize := 40.0, letterSpacing := 0.0, paint := canvas::fill(white()), transform := none()]\n  canvas::present([single(label, 300.0, 200.0, 0.0, 2.0, color::rgb(255, 128, 0))])\n",
        ),
        &[],
    );
    assert_eq!(
        pixel(&frame, 320, 188),
        [255, 128, 0, 255],
        "inside the glyph"
    );
    let (count, x0, y0, x1, y1) = lit(&frame);
    assert!(count > 400, "the glyph is 24 px square");
    assert!(
        x0 >= 307 && x1 <= 332 && y0 >= 175 && y1 <= 200,
        "the glyph is outside its scaled box: ({x0}, {y0})-({x1}, {y1})"
    );
}

/// **An `AutoPaint` template draws its shape's own paint**, whatever the mouse is
/// doing: with the pointer resting on the particle and a hover paint in use, the
/// particle is still the shape's own green, and the frame notes no `AutoPaint`.
#[test]
fn an_autopaint_template_draws_its_own_paint() {
    let (frame, stats) = draw(
        "canvas_particle_autopaint",
        &scene(
            "  LET red AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))\n  LET button AS canvas::AutoShape = canvas::AutoPaint[shape := disc(40.0, canvas::fill(color::rgb(0, 255, 0))), useEnter := TRUE, useExit := TRUE, useHover := TRUE, usePressed := TRUE, enterPaint := red, exitPaint := red, hoverPaint := red, pressedPaint := red]\n  canvas::present([single(button, 300.0, 200.0, 0.0, 1.0, white())])\n  canvas::present([single(button, 300.0, 200.0, 0.0, 1.0, white()), single(button, 600.0, 200.0, 0.0, 1.0, white())])\n",
        ),
        &[("MFB_CANVAS_POINTER", "0,300,200,1,1")],
    );
    assert_eq!(pixel(&frame, 300, 200), [0, 255, 0, 255]);
    assert_eq!(pixel(&frame, 600, 200), [0, 255, 0, 255]);
    let last = stats.last().expect("a frame");
    // `auto=` lists the frame's `AutoPaint` instances; `-` is none.
    assert_eq!(
        stat(last, "auto="),
        "-",
        "a particle was noted as an AutoPaint instance: {last}"
    );
}

/// **Curve emitters.** A `PolyCurve` emits along its curve and a `CurvePolygon` over
/// its inside: every particle of each lands within the curve's own extent (the arch
/// from (0, 0) over (20, 30) to (40, 0) peaks at y = 15), and they are spread across
/// it, not piled on a point.
#[test]
fn curve_shapes_are_emitters() {
    for (name, emitter, y_from, y_to) in [
        ("polycurve", "WITH pc { curves := [bend] }", 0, 15),
        (
            "curvepolygon",
            "WITH cp { contours := [[bend, back]] }",
            -15,
            15,
        ),
    ] {
        let (frame, _) = draw(
            &format!("canvas_particle_emit_{name}"),
            &scene(&format!(
                "  LET bend AS canvas::BezierCurve = canvas::BezierCurve[points := [at(0.0, 0.0), at(20.0, 30.0), at(40.0, 0.0)], weights := []]\n  LET back AS canvas::BezierCurve = canvas::BezierCurve[points := [at(40.0, 0.0), at(20.0, 0.0 - 30.0), at(0.0, 0.0)], weights := []]\n  MUT pc AS canvas::PolyCurve\n  MUT cp AS canvas::CurvePolygon\n  LET emitter AS canvas::Shape = {emitter}\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(300.0, 200.0), item := disc(1.0, canvas::fill(white())), emitter := emitter, emission := WITH emission {{ mode := canvas::EmissionMode.Burst, maxParticles := 300, burst := 300, lifetime := 10.0 }}, motion := motion, appearance := look(white(), white(), 1.0), seed := 4, time := 1.0]\n  canvas::present([ps])\n"
            )),
            &[],
        );
        let (count, x0, y0, x1, y1) = lit(&frame);
        // 300 dots: along a 50 px arch they overlap, over the lens they mostly do not.
        assert!(count > 100, "{name}: {count} lit pixels");
        // The emitter's extent, a pixel of slack and the particle's own radius.
        assert!(
            x0 >= 298 && x1 <= 342 && y0 as i64 >= 198 + y_from && y1 as i64 <= 202 + y_to,
            "{name}: particles outside the emitter: ({x0}, {y0})-({x1}, {y1})"
        );
        assert!(x1 - x0 >= 30, "{name}: particles span only {x0}..{x1}");
    }
}

/// A thousand particles between two ordinary items, as one scene index: every one is
/// drawn, the items either side of the system are where they were, and the GPU draws
/// the same picture ON the GPU. This is the path an item of several entries takes
/// (`__canvas_sceneOffsets`), built for a `Picture`'s two and measured here at a
/// thousand.
const THOUSAND: &str = "  LET before AS canvas::DrawItem = box(20.0, 20.0, 60.0, 40.0, canvas::fill(color::rgb(255, 0, 0)), none())\n  LET after AS canvas::DrawItem = box(820.0, 580.0, 60.0, 40.0, canvas::fill(color::rgb(0, 0, 255)), none())\n  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 250.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(450.0, 320.0), item := disc(2.0, canvas::fill(color::rgb(0, 255, 0))), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 1000, burst := 1000, lifetime := 10.0 }, motion := WITH motion { rotation := 0.3 }, appearance := look(white(), white(), 1.5), seed := 2, time := 1.0]\n  canvas::present([before, ps, after])\n";

#[test]
fn a_thousand_particles_are_one_scene_index() {
    let (software, stats) = draw("canvas_particle_thousand", &scene(THOUSAND), &[]);
    let last = stats.last().expect("a frame");
    // The two rectangles and a record per particle.
    assert_eq!(stat(last, "entries="), "1002", "{last}");
    assert_eq!(pixel(&software, 50, 40), [255, 0, 0, 255]);
    assert_eq!(pixel(&software, 850, 600), [0, 0, 255, 255]);
    // 1000 discs of radius 3 over a disc of radius 250: most of ~28,000 px, less overlap.
    let green = (0..HEIGHT)
        .flat_map(|y| (0..WIDTH).map(move |x| (x, y)))
        .filter(|(x, y)| pixel(&software, *x, *y) == [0, 255, 0, 255])
        .count();
    assert!(green > 12_000, "{green} wholly green pixels");

    let (gpu, stats) = draw(
        "canvas_particle_thousand_gpu",
        &scene(THOUSAND),
        &[("MFB_CANVAS_GPU", "1")],
    );
    let last = stats.last().expect("a frame");
    if !last.contains("metalReady=TRUE") && !last.contains("vulkanReady=TRUE") {
        eprintln!("skip the GPU half: this host built no GPU pipeline\n{last}");
        return;
    }
    assert_ne!(stat(last, "gpuFrames="), "0", "the GPU declined: {last}");
    assert_eq!(stat(last, "blocks="), "1002", "{last}");
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("the GPU's thousand particles disagree with the oracle: {diff}");
    }
}

/// A system that is a GROUP's child reaches the GPU as blocks of that group's run
/// (`__canvas_memoGroup`), one per live particle, drawn under the group's matrix — and
/// the group's other items are still either side of them.
#[test]
fn a_system_in_a_group_reaches_the_gpu() {
    let body = "  LET under AS canvas::DrawItem = box(0.0 - 40.0, 0.0 - 40.0, 80.0, 80.0, canvas::fill(color::rgb(90, 0, 0)), none())\n  LET over AS canvas::DrawItem = box(60.0, 0.0 - 10.0, 30.0, 20.0, canvas::fill(color::rgb(0, 0, 200)), none())\n  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 70.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(0.0, 0.0), item := box(0.0 - 3.0, 0.0 - 2.0, 6.0, 4.0, canvas::fillStroke(color::rgb(0, 220, 0), white(), 1.0), none()), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 150, burst := 150, lifetime := 10.0 }, motion := WITH motion { rotationVar := 3.0 }, appearance := look(white(), white(), 1.0), seed := 9, time := 1.0]\n  canvas::setGroup(\"g\", [under, ps, over])\n  LET turn AS canvas::Transform = canvas::Transform[a := 1.2, b := 0.9, c := 0.0 - 0.9, d := 1.2, tx := 0.0, ty := 0.0]\n  canvas::present([node(\"g\", 250.0, 250.0, turn), node(\"g\", 650.0, 400.0, none())])\n";
    let (software, _) = draw("canvas_particle_grouped", &scene(body), &[]);
    assert!(lit(&software).0 > 10_000);
    let (gpu, stats) = draw(
        "canvas_particle_grouped_gpu",
        &scene(body),
        &[("MFB_CANVAS_GPU", "1")],
    );
    let last = stats.last().expect("a frame");
    if !last.contains("metalReady=TRUE") && !last.contains("vulkanReady=TRUE") {
        eprintln!("skip: this host built no GPU pipeline\n{last}");
        return;
    }
    assert_ne!(stat(last, "gpuFrames="), "0", "the GPU declined: {last}");
    // One shared run for both uses of the group: two rectangles and 150 particles.
    assert_eq!(stat(last, "blocks="), "152", "{last}");
    if let Err(diff) = compare_within_tolerance(&gpu, &software, Tolerance::GPU_DEFAULT) {
        panic!("a grouped system on the GPU disagrees with the oracle: {diff}");
    }
}

// ---- Damage (plan-150-B Phase 3) -------------------------------------------------------

/// A steady stream in the lower right quarter of the surface, over a background that
/// never changes, presented thirty times with `time` advanced. The number of live
/// particles differs on nearly every frame — the pool fills over the first 0.9 s — which
/// is what used to make every frame a full redraw.
const STREAM_OVER_BACKGROUND: &str = "  LET background AS canvas::DrawItem = box(0.0, 0.0, 900.0, 640.0, canvas::fill(color::rgb(20, 30, 60)), none())\n  MUT lip AS canvas::Line\n  LET emitter AS canvas::Shape = WITH lip { startAt := at(0.0 - 40.0, 0.0), endAt := at(40.0, 0.0) }\n  MUT emission AS canvas::Emission\n  MUT motion AS canvas::Motion\n  MUT frame AS Integer = 0\n  WHILE frame < 30\n    LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(700.0, 520.0), item := disc(3.0, canvas::fill(color::rgb(255, 200, 80))), emitter := emitter, emission := WITH emission { maxParticles := 60, rate := 40.0, lifetime := 0.9 }, motion := WITH motion { speed := 120.0, angle := 0.0 - 1.5707963267948966, spread := 0.5, gravityY := 200.0 }, appearance := look(white(), color::rgba(255, 255, 255, 60), 1.0), seed := 3, time := toFloat(frame) / 30.0]\n    canvas::present([background, ps])\n    frame = frame + 1\n  END WHILE\n";

/// The damage rectangle of a stats line, as (x, y, width, height); `None` for `none`.
fn damage(line: &str) -> Option<(u32, u32, u32, u32)> {
    let text = stat(line, "damage=");
    if text == "none" {
        return None;
    }
    let parts: Vec<u32> = text
        .split(',')
        .map(|p| p.parse().expect("damage"))
        .collect();
    Some((parts[0], parts[1], parts[2], parts[3]))
}

/// **Under `MFB_CANVAS_DAMAGE`, a moving particle system repaints where it moved.** A
/// system is one unit of the damage diff, whatever its live count, so after the first
/// frame every frame is a partial redraw whose rectangle lies inside the quarter of the
/// surface the particles are in — and the picture it leaves is, pixel for pixel, the
/// picture a full redraw of the same scene makes.
#[test]
fn damage_of_a_system_is_bounded_by_its_particles() {
    let (partial, stats) = draw(
        "canvas_particle_damage",
        &scene(STREAM_OVER_BACKGROUND),
        &[("MFB_CANVAS_DAMAGE", "1")],
    );
    assert_eq!(stats.len(), 30, "one frame per present");
    let entries: Vec<String> = stats.iter().map(|l| stat(l, "blocks=")).collect();
    let mut distinct = entries.clone();
    distinct.sort();
    distinct.dedup();
    assert!(
        distinct.len() > 10,
        "the live count was meant to change from frame to frame: {entries:?}"
    );
    assert_eq!(
        damage(&stats[0]),
        Some((0, 0, 900, 640)),
        "the first frame is full"
    );
    for (frame, line) in stats.iter().enumerate().skip(1) {
        // A frame where nothing is alive yet and nothing was has no damage at all.
        let Some((x, y, w, h)) = damage(line) else {
            continue;
        };
        assert!(
            x >= 450 && y >= 320 && x + w <= 900 && y + h <= 640 && (w < 450 && h < 320),
            "frame {frame} damaged ({x}, {y}) {w}x{h}, outside the particles' quarter: {line}"
        );
    }
    assert_eq!(stat(stats.last().expect("a frame"), "partial="), "29");

    let (full, _) = draw(
        "canvas_particle_damage_off",
        &scene(STREAM_OVER_BACKGROUND),
        &[],
    );
    assert!(lit(&full).0 > 500_000, "the background fills the surface");
    if let Err(diff) = compare_exact(&partial, &full) {
        panic!("thirty partial redraws left a different picture from a full one: {diff}");
    }
}

/// A scene of ordinary items beside a system still diffs item by item: moving one small
/// rectangle far from the system damages that rectangle's two places and nothing of the
/// system, whose particles did not move.
#[test]
fn damage_of_an_item_beside_a_system_is_the_items() {
    let body = "  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 40.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(700.0, 500.0), item := disc(3.0, canvas::fill(white())), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 50, burst := 50, lifetime := 10.0 }, motion := motion, appearance := look(white(), white(), 1.0), seed := 8, time := 1.0]\n  LET first AS canvas::DrawItem = box(40.0, 40.0, 30.0, 20.0, canvas::fill(color::rgb(255, 0, 0)), none())\n  LET moved AS canvas::DrawItem = box(100.0, 60.0, 30.0, 20.0, canvas::fill(color::rgb(255, 0, 0)), none())\n  LET after AS canvas::DrawItem = box(300.0, 300.0, 10.0, 10.0, canvas::fill(color::rgb(0, 0, 255)), none())\n  canvas::present([first, ps, after])\n  canvas::present([moved, ps, after])\n";
    let (_, stats) = draw(
        "canvas_particle_damage_beside",
        &scene(body),
        &[("MFB_CANVAS_DAMAGE", "1")],
    );
    assert_eq!(stats.len(), 2);
    let (x, y, w, h) = damage(&stats[1]).expect("the rectangle moved");
    assert!(
        x >= 38 && y >= 38 && x + w <= 133 && y + h <= 83,
        "moving a rectangle damaged ({x}, {y}) {w}x{h}: {}",
        stats[1]
    );
}

// ---- The uncached record (plan-150-B Phase 2) ------------------------------------------

/// Ten ordinary items in the left half of the surface and, in the right half, a system of
/// 1,000 particles that are neither turned nor scaled, presented `FRAMES` times with
/// `time` advanced. `WITH_SYSTEM` is replaced by TRUE or FALSE.
const MIXED: &str = "  MUT items AS List OF canvas::DrawItem = []\n  MUT i AS Integer = 0\n  WHILE i < 10\n    LET item AS canvas::DrawItem = box(20.0 + toFloat(i) * 40.0, 60.0 + toFloat(i) * 50.0, 30.0, 30.0, canvas::fillStroke(color::rgb(200, 40 + i * 20, 90), white(), 2.0), none())\n    items = collections::append(items, item)\n    i = i + 1\n  END WHILE\n  MUT ring AS canvas::Circle\n  LET emitter AS canvas::Shape = WITH ring { radius := 150.0 }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  MUT frame AS Integer = 0\n  WHILE frame < FRAMES\n    MUT scene AS List OF canvas::DrawItem = items\n    IF WITH_SYSTEM THEN\n      LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(680.0, 320.0), item := disc(2.0, canvas::fill(color::rgb(90, 220, 255))), emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 1000, burst := 1000, lifetime := 100.0 }, motion := WITH motion { speed := 15.0, spread := 3.2 }, appearance := look(white(), white(), 1.0), seed := 12, time := toFloat(frame) / 30.0]\n      scene = collections::append(scene, ps)\n    END IF\n    canvas::present(scene)\n    frame = frame + 1\n  END WHILE\n";

/// **A plain particle's record is not cached.** A thousand particles that are neither
/// turned nor scaled are a thousand blocks of the frame and not one entry of the
/// geometry cache — which holds the ten ordinary items and nothing else — and the
/// floats the cache's arena holds do not grow with the frames: what one frame's
/// particles took is dropped at the next frame boundary with everything else unused.
#[test]
fn a_plain_particles_record_is_not_cached() {
    let body = MIXED.replace("FRAMES", "12").replace("WITH_SYSTEM", "TRUE");
    let (_, stats) = draw(
        "canvas_particle_uncached",
        &scene(&body),
        &[("MFB_CANVAS_GPU", "1")],
    );
    assert_eq!(stats.len(), 12);
    let last = stats.last().expect("a frame");
    assert_eq!(stat(last, "entries="), "10", "{last}");
    if last.contains("metalReady=TRUE") || last.contains("vulkanReady=TRUE") {
        assert_eq!(stat(last, "blocks="), "1010", "{last}");
        assert_eq!(stat(last, "gpuFrames="), "12", "{last}");
    }
    let floats: Vec<u64> = stats
        .iter()
        .map(|line| stat(line, "floats=").parse().expect("floats="))
        .collect();
    // One frame's particles are about 70,000 floats. The arena may hold two frames'
    // worth before a boundary drops the older; it may not hold twelve.
    let most = *floats.iter().max().expect("frames");
    assert!(most < 250_000, "the arena grew with the frames: {floats:?}");
    assert!(
        floats[11] <= floats[5] + 1000,
        "the arena is still growing at the twelfth frame: {floats:?}"
    );
}

/// **The ordinary items are untouched by the system beside them.** The ten items draw
/// the same pixels with a thousand particles in the scene as without: the left half of
/// the surface, where the items are, is identical, and the right half is where every
/// particle is.
#[test]
fn ordinary_items_draw_the_same_beside_a_system() {
    let (with, _) = draw(
        "canvas_particle_mixed_with",
        &scene(&MIXED.replace("FRAMES", "3").replace("WITH_SYSTEM", "TRUE")),
        &[],
    );
    let (without, _) = draw(
        "canvas_particle_mixed_without",
        &scene(&MIXED.replace("FRAMES", "3").replace("WITH_SYSTEM", "FALSE")),
        &[],
    );
    let mut items = 0;
    let mut particles = 0;
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let a = pixel(&with, x, y);
            let b = pixel(&without, x, y);
            if x < 450 {
                assert_eq!(a, b, "an ordinary item's pixel moved at ({x}, {y})");
                if b != [0, 0, 0, 255] {
                    items += 1;
                }
            } else {
                assert_eq!(b, [0, 0, 0, 255], "an item reached the right half");
                if a != [0, 0, 0, 255] {
                    particles += 1;
                }
            }
        }
    }
    assert!(items > 8_000, "{items} item pixels");
    assert!(particles > 5_000, "{particles} particle pixels");
}

/// A `Text` particle is still a cached record: the native builder does not take text,
/// so a text particle goes the way every text item does — through the glyph run, with
/// its glyphs pinned by the slot the cache stamps for this frame. Forty text particles
/// are forty entries, every one drawn.
#[test]
fn a_text_particles_record_is_still_cached_and_its_glyphs_drawn() {
    let (frame, stats) = draw(
        "canvas_particle_text_cached",
        &scene(
            "  RES face AS canvas::Font = canvas::loadFont(\"fixture.ttf\") TRAP(e)\n    RETURN\n  END TRAP\n  LET label AS canvas::AutoShape = canvas::Text[id := \"\", position := at(0.0, 0.0), text := \"AB\", fonts := [face], fontSize := 30.0, letterSpacing := 0.0, paint := canvas::fill(white()), transform := none()]\n  MUT lip AS canvas::Line\n  LET emitter AS canvas::Shape = WITH lip { startAt := at(0.0 - 400.0, 0.0), endAt := at(360.0, 0.0) }\n  MUT motion AS canvas::Motion\n  MUT emission AS canvas::Emission\n  LET ps AS canvas::DrawItem = canvas::ParticleSystem[position := at(450.0, 320.0), item := label, emitter := emitter, emission := WITH emission { mode := canvas::EmissionMode.Burst, maxParticles := 40, burst := 40, lifetime := 10.0 }, motion := WITH motion { speed := 200.0, spread := 3.2 }, appearance := look(white(), white(), 1.0), seed := 5, time := 1.0]\n  canvas::present([ps])\n",
        ),
        &[],
    );
    let last = stats.last().expect("a frame");
    assert_eq!(stat(last, "entries="), "40", "{last}");
    // Two 9 px squares a particle; they overlap, but forty particles are far more ink
    // than a few.
    assert!(lit(&frame).0 > 2_500, "{} lit pixels", lit(&frame).0);
}
