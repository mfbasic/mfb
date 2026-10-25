//! Damage-limited repaint (plan-98-G Phase 3).
//!
//! Three claims, and they are not independent:
//!
//! 1. **It changes no pixels.** The same program rendered with damage on and with it off
//!    produces byte-identical frames. This is the one that has to hold unconditionally;
//!    everything else is an optimisation that is only allowed to exist because of it.
//! 2. **An unchanged scene renders nothing.** Re-presenting the same items skips the
//!    frame entirely — no rasterisation, no blit.
//! 3. **A changed item repaints its own rectangle**, not the window's, and the rectangle
//!    covers where it *was* as well as where it is.
//!
//! Claim 1 is checked against a full-frame render rather than against a stored image, so
//! it cannot go stale: if the rasteriser changes, both sides change together and the
//! comparison still means "damage changed nothing".

#[path = "../common/mod.rs"]
mod common;

#[path = "damage_harness/mod.rs"]
mod harness;

use harness::{last, number, pixel, render, WIDTH};

/// Two static shapes, presented three times with no change between them.
///
/// **The sleeps are load-bearing.** Redraw requests coalesce by design
/// (`.ai/canvas-threading.md` §3): three presents issued back to back can wake the
/// graphics thread once, and then a test counting frames is measuring the scheduler
/// rather than the damage logic. Measured without them — one stats line, `frames=1`,
/// `skipped=0`, which is indistinguishable from damage not working at all.
const UNCHANGED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 400.0], radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([box, dot])
  os::sleep(150)
END FUNC
"#;

/// The same scene, except that the second present moves the circle a long way.
const MOVED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 400.0], radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET moved AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 700.0, y := 420.0], radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([box, moved])
  os::sleep(150)
END FUNC
"#;

#[test]
fn damage_changes_no_pixel() {
    // The claim everything else rests on. Compared against the full-frame render of the
    // same program rather than a stored image, so it cannot go stale.
    for (name, source) in [
        ("canvas_damage_same", UNCHANGED),
        ("canvas_damage_moved", MOVED),
    ] {
        let (with, _) = render(&format!("{name}_on"), source, true);
        let (without, _) = render(&format!("{name}_off"), source, false);
        assert_eq!(with.len(), without.len(), "{name}: frame sizes differ");
        assert!(
            without.iter().any(|&b| b != 0),
            "{name}: the full-frame render is blank, so the comparison would be vacuous",
        );
        if with != without {
            let at = with.iter().zip(&without).position(|(a, b)| a != b).unwrap();
            let pixel = at / 4;
            panic!(
                "{name}: damage changed the picture — first difference at ({}, {}), \
                 got {:?} want {:?}",
                pixel % WIDTH,
                pixel / WIDTH,
                &with[at..(at + 4).min(with.len())],
                &without[at..(at + 4).min(without.len())],
            );
        }
    }
}

/// bug-1089: a square that never changes, and a translucent rectangle whose colour
/// changes on the second present and whose box overlaps one side of the square -- then
/// the same with the square turned 37 degrees, whose box is the hull of its turned
/// outline. The partial redraw clears only the damage rectangle, so an item drawn over
/// its WHOLE box blended its antialiased edge a second time over the kept frame wherever
/// that box ran outside the rectangle (worst 32 levels, at (239, 239)).
const OVERLAPPED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT os

FUNC pt(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET none AS canvas::Transform = canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
  LET turn AS canvas::Transform = canvas::Transform[a := 0.79864, b := 0.60182, c := -0.60182, d := 0.79864, tx := 0.0, ty := 0.0]
  LET a AS canvas::DrawItem = canvas::Polygon[id := "", position := pt(0.0, 0.0), contours := [[pt(300.0 - 60.3, 300.0 - 60.3), pt(300.0 + 60.3, 300.0 - 60.3), pt(300.0 + 60.3, 300.0 + 60.3), pt(300.0 - 60.3, 300.0 + 60.3)]], rule := canvas::FillRule.NonZero, paint := canvas::fill(color::rgb(200, 40, 40)), transform := none]
  LET t AS canvas::DrawItem = canvas::Polygon[id := "", position := pt(0.0, 0.0), contours := [[pt(0.0 - 60.3, 0.0 - 60.3), pt(60.3, 0.0 - 60.3), pt(60.3, 60.3), pt(0.0 - 60.3, 60.3)]], rule := canvas::FillRule.NonZero, paint := canvas::fill(color::rgb(200, 40, 40)), transform := WITH turn { tx := 650.0, ty := 300.0 }]
  LET b1 AS canvas::DrawItem = canvas::Rectangle[id := "", position := pt(340.5, 280.5), bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(40, 200, 120, 120)), transform := none]
  LET b2 AS canvas::DrawItem = canvas::Rectangle[id := "", position := pt(340.5, 280.5), bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(40, 120, 200, 120)), transform := none]
  LET c1 AS canvas::DrawItem = canvas::Rectangle[id := "", position := pt(700.5, 280.5), bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(40, 200, 120, 120)), transform := none]
  LET c2 AS canvas::DrawItem = canvas::Rectangle[id := "", position := pt(700.5, 280.5), bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 40.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgba(40, 120, 200, 120)), transform := none]
  canvas::present([a, t, b1, c1])
  os::sleep(150)
  canvas::present([a, t, b2, c2])
  os::sleep(150)
END FUNC
"#;

#[test]
fn damage_does_not_blend_an_unchanged_overlapping_item_twice() {
    let (with, stats) = render("canvas_damage_overlap_on", OVERLAPPED, true);
    let (without, _) = render("canvas_damage_overlap_off", OVERLAPPED, false);
    assert_eq!(with.len(), without.len(), "frame sizes differ");
    assert!(
        stats.iter().any(|l| l.contains(" partial=1")),
        "the second frame was not a partial redraw, so the comparison proves nothing: {stats:?}"
    );
    let differing: Vec<usize> = with
        .chunks(4)
        .zip(without.chunks(4))
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(i, _)| i)
        .collect();
    if let Some(&first) = differing.first() {
        panic!(
            "damage changed {} pixels; first at ({}, {}), got {:?} want {:?}",
            differing.len(),
            first % WIDTH,
            first / WIDTH,
            &with[first * 4..first * 4 + 4],
            &without[first * 4..first * 4 + 4],
        );
    }
}

#[test]
fn re_presenting_an_unchanged_scene_renders_nothing() {
    // Three presents, one rendered frame, with damage on **or** off — because the skip
    // that makes this true is not the damage union at all. `canvas::publishScene`
    // returns FALSE for a scene identical to the installed one and `__canvas_present`
    // then does not even signal a redraw, so the second and third presents never reach
    // the graphics thread. That is plan-98-A's invariant 2, and it predates this phase.
    //
    // The assertion is here rather than in the damage tests' preamble because it is
    // what makes the damage union's *empty* case rare enough to be worth stating: by
    // the time a wake reaches the renderer, the scene has usually changed. The empty
    // case is a backstop for the wakes that do not come from a present — a resize, an
    // OS damage repaint — and it is covered on the box that has a scripted resize
    // affordance (`scripts/test-canvas-vulkan.sh`), because macOS has none.
    for (name, damage) in [
        ("canvas_damage_skip_on", true),
        ("canvas_damage_skip_off", false),
    ] {
        let (_, lines) = render(name, UNCHANGED, damage);
        assert_eq!(
            number(&lines, "frames"),
            1,
            "{name}: an unchanged scene was re-rendered:\n{}",
            lines.join("\n"),
        );
    }
}

/// The same two shapes, except that the second present renames the rectangle.
const RENAMED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET named AS canvas::DrawItem = canvas::Rectangle[id := "renamed", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 400.0], radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([named, dot])
  os::sleep(150)
END FUNC
"#;

/// plan-175-A decision 1: an `id` is in no item identity, so an item renamed and
/// nothing else damages nothing. The scene IS published -- the frame comes -- and the
/// damage union is empty, so the frame is skipped: nothing is presented, and the picture
/// on screen is the first frame's. (`overID` answering the new name is
/// `rt_canvas_hit.rs:over_id_answers_a_renamed_item`.)
#[test]
fn an_id_only_change_damages_nothing() {
    let (frame, lines) = render("canvas_damage_rename", RENAMED, true);
    // One stats line a frame the graphics thread ran, drawn or skipped.
    assert_eq!(
        lines.len(),
        2,
        "the renamed scene was not published, so no second frame came:\n{}",
        lines.join("\n"),
    );
    assert_eq!(
        (number(&lines, "frames"), number(&lines, "skipped")),
        (1, 1),
        "the renamed scene's frame damaged something; an id is in no identity:\n{}",
        lines.join("\n"),
    );
    assert_eq!(
        number(&lines, "generations"),
        2,
        "renaming an item rebuilt geometry:\n{}",
        lines.join("\n"),
    );
    assert_eq!(
        pixel(&frame, 150, 150),
        (200, 40, 40, 255),
        "the rectangle is drawn"
    );
}

/// The same two shapes, except that the second present recolours the rectangle.
const RECOLOURED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET blue AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 100.0, y := 100.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 200.0, height := 120.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 40, 200)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET dot AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 600.0, y := 400.0], radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([blue, dot])
  os::sleep(150)
END FUNC
"#;

/// plan-175-A Phase 2: the geometry cache is keyed by an item's geometry identity, and
/// the record of a recoloured item is refreshed in place -- but damage compares the
/// damage identity, which folds the paint, so the recoloured item still repaints its
/// own box, in its new colour. (With the geometry identity as the damage identity the
/// frame was skipped: plan-175-A §9, spike experiment 1.)
#[test]
fn a_paint_only_change_damages_the_items_box() {
    let (frame, lines) = render("canvas_damage_recolour", RECOLOURED, true);
    assert_eq!(
        number(&lines, "partial"),
        1,
        "the recoloured frame was not a partial repaint:\n{}",
        lines.join("\n"),
    );
    let damage = last(&lines, "damage");
    let rect: Vec<i64> = damage.split(',').map(|v| v.parse().unwrap()).collect();
    assert!(
        rect[0] <= 100
            && rect[1] <= 100
            && rect[2] >= 200
            && rect[3] >= 120
            && rect[0] + rect[2] < 600,
        "the damage `{damage}` is not the rectangle's box (x,y,w,h around 100,100,200,120)"
    );
    assert_eq!(
        number(&lines, "generations"),
        2,
        "a recolour rebuilt geometry:\n{}",
        lines.join("\n"),
    );
    assert_eq!(
        pixel(&frame, 150, 150),
        (40, 40, 200, 255),
        "the rectangle is blue"
    );
}

#[test]
fn a_moved_item_repaints_its_own_rectangle_and_not_the_window() {
    let (_, lines) = render("canvas_damage_rect", MOVED, true);
    assert_eq!(
        number(&lines, "partial"),
        1,
        "the second present was not a partial repaint:\n{}",
        lines.join("\n"),
    );

    let rect = last(&lines, "damage");
    let parts: Vec<i64> = rect
        .split(',')
        .map(|v| v.parse().expect("a number"))
        .collect();
    assert_eq!(parts.len(), 4, "damage should be `x,y,w,h`, got `{rect}`");
    let (x, y, w, h) = (parts[0], parts[1], parts[2], parts[3]);

    // The circle moved from (600,400) to (700,420) with radius 60, so the union of where
    // it was and where it is spans x 540..760 and y 340..480. Asserting the *bounds* of
    // the rectangle rather than its exact value leaves the antialiasing margin free while
    // still failing a rectangle that is the whole window — which is what a damage
    // computation that gave up would produce, and it would produce correct pixels while
    // doing it.
    assert!(
        x <= 540 && y <= 340 && x + w >= 760 && y + h >= 480,
        "the damage rectangle {rect} does not cover both circle positions",
    );
    assert!(
        w <= 300 && h <= 220,
        "the damage rectangle {rect} is far larger than the change that caused it",
    );
    assert!(
        (w as usize) < WIDTH,
        "the damage rectangle {rect} spans the whole window",
    );
}

/// Seven items, each a few tenths of a unit across in its own coordinates and drawn
/// through its own transform, presented eight times: first all red, then with one more
/// item turned green each time. Each present after the first changes exactly one item,
/// so its damage rectangle is that item's box (`__canvas_boundsHeader`, slots 16 to 19).
const TRANSFORMED_BOXES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT collections
IMPORT os

FUNC pt(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC square(cx AS Float, cy AS Float, h AS Float) AS List OF vector::Float2
  RETURN [pt(cx - h, cy - h), pt(cx + h, cy - h), pt(cx + h, cy + h), pt(cx - h, cy + h)]
END FUNC

' Item `k` of the scene, in colour `c`. Every one is a few tenths of a unit across in its
' own space and is drawn through a transform that magnifies it (or, for 6, shrinks it).
FUNC item(k AS Integer, c AS color::Color) AS canvas::DrawItem
  LET big AS canvas::Transform = canvas::Transform[a := 100.0, b := 0.0, c := 0.0, d := 100.0, tx := 0.0, ty := 0.0]
  LET origin AS vector::Float2 = pt(0.0, 0.0)
  IF k = 1 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(5.0, 2.0, 0.3)], rule := canvas::FillRule.NonZero, paint := WITH canvas::stroke(c, 0.1) { join := canvas::JoinStyle.Miter, miterLimit := 4.0 }, transform := big]
  END IF
  IF k = 2 THEN
    RETURN canvas::Line[id := "", position := origin, startAt := pt(1.0, 5.0), endAt := pt(2.0, 5.0), paint := WITH canvas::stroke(c, 0.1) { cap := canvas::CapStyle.Square }, transform := big]
  END IF
  IF k = 3 THEN
    RETURN canvas::Circle[id := "", position := pt(500.0, 500.0), radius := 0.3, paint := canvas::fill(c), transform := canvas::Transform[a := 100.0, b := 0.0, c := 0.0, d := 100.0, tx := 0.0, ty := 0.0]]
  END IF
  IF k = 4 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(0.0, 0.0, 0.4)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := canvas::Transform[a := 80.0, b := 60.0, c := 0.0 - 60.0, d := 80.0, tx := 750.0, ty := 150.0]]
  END IF
  IF k = 5 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(0.0, 0.0, 0.4)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := canvas::Transform[a := 100.0, b := 0.0, c := 50.0, d := 100.0, tx := 750.0, ty := 400.0]]
  END IF
  IF k = 6 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(0.0, 0.0, 400.0)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := canvas::Transform[a := 0.05, b := 0.0, c := 0.0, d := 0.05, tx := 300.0, ty := 560.0]]
  END IF
  RETURN canvas::Polygon[id := "", position := origin, contours := [square(2.0, 2.0, 0.4)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := big]
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT changed AS Integer = 0 - 1
  WHILE changed < 7
    MUT scene AS List OF canvas::DrawItem = []
    MUT k AS Integer = 0
    WHILE k < 7
      MUT c AS color::Color = color::rgb(200, 40, 40)
      IF k <= changed THEN
        c = color::rgb(40, 200, 120)
      END IF
      scene = collections::append(scene, item(k, c))
      k = k + 1
    END WHILE
    canvas::present(scene)
    os::sleep(150)
    changed = changed + 1
  END WHILE
END FUNC
"#;

/// bug-991: a transformed item's box is the mapped hull of its extent and its stroke
/// reach, with a rim of surface pixels, whatever the transform's scale.
///
/// The rim was one UNIT of the item's own space on every side, so under a scale of 100
/// each box here was 200 px too wide and too tall, and every pixel of it ran the item's
/// coverage. The rim a transformed item needs is half the inverse matrix's Frobenius
/// norm in its own units, which is 0.71 px under a uniform scale, and then a pixel.
#[test]
fn a_transformed_items_box_is_its_mapped_extent_and_a_rim_of_pixels() {
    // Each item's extent with its stroke, cap or join reach, in its own units, and the
    // transform it is drawn through as (a, b, c, d, tx, ty).
    type Case = (&'static str, [f64; 4], [f64; 6]);
    let half = 0.05_f64;
    let cases: [Case; 7] = [
        (
            "a filled square, magnified 100 times",
            [1.6, 1.6, 2.4, 2.4],
            [100.0, 0.0, 0.0, 100.0, 0.0, 0.0],
        ),
        (
            // A miter reaches `half * miterLimit` from its vertex at most.
            "a mitered stroked square, magnified",
            [
                4.7 - half * 4.0,
                1.7 - half * 4.0,
                5.3 + half * 4.0,
                2.3 + half * 4.0,
            ],
            [100.0, 0.0, 0.0, 100.0, 0.0, 0.0],
        ),
        (
            // A square cap reaches `half * sqrt(2)` from its end.
            "a square-capped line, magnified",
            [
                1.0 - half * 2.0_f64.sqrt(),
                5.0 - half * 2.0_f64.sqrt(),
                2.0 + half * 2.0_f64.sqrt(),
                5.0 + half * 2.0_f64.sqrt(),
            ],
            [100.0, 0.0, 0.0, 100.0, 0.0, 0.0],
        ),
        (
            // A circle's transform is about its position.
            "a circle, magnified",
            [-0.3, -0.3, 0.3, 0.3],
            [100.0, 0.0, 0.0, 100.0, 500.0, 500.0],
        ),
        (
            "a square, turned and magnified",
            [-0.4, -0.4, 0.4, 0.4],
            [80.0, 60.0, -60.0, 80.0, 750.0, 150.0],
        ),
        (
            "a square, sheared and magnified",
            [-0.4, -0.4, 0.4, 0.4],
            [100.0, 0.0, 50.0, 100.0, 750.0, 400.0],
        ),
        (
            "a square, shrunk to a twentieth",
            [-400.0, -400.0, 400.0, 400.0],
            [0.05, 0.0, 0.0, 0.05, 300.0, 560.0],
        ),
    ];
    let (_, lines) = render("canvas_damage_transformed_boxes", TRANSFORMED_BOXES, true);
    let rects: Vec<&String> = lines.iter().filter(|l| l.contains("damage=")).collect();
    assert_eq!(
        rects.len(),
        cases.len() + 1,
        "one frame for each present:\n{}",
        lines.join("\n"),
    );
    for (k, (what, extent, m)) in cases.iter().enumerate() {
        let field = rects[k + 1]
            .split("damage=")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap();
        let parts: Vec<f64> = field
            .split(',')
            .map(|v| {
                v.parse()
                    .unwrap_or_else(|_| panic!("{what}: damage={field}"))
            })
            .collect();
        assert_eq!(
            parts.len(),
            4,
            "{what}: damage should be `x,y,w,h`, got `{field}`"
        );
        let (x0, y0, x1, y1) = (parts[0], parts[1], parts[0] + parts[2], parts[1] + parts[3]);

        // The hull of the extent's four corners on the surface.
        let corners = [
            (extent[0], extent[1]),
            (extent[2], extent[1]),
            (extent[0], extent[3]),
            (extent[2], extent[3]),
        ];
        let xs = corners.map(|(x, y)| m[0] * x + m[2] * y + m[4]);
        let ys = corners.map(|(x, y)| m[1] * x + m[3] * y + m[5]);
        let hull = (
            xs.iter().cloned().fold(f64::INFINITY, f64::min),
            ys.iter().cloned().fold(f64::INFINITY, f64::min),
            xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        );
        assert!(
            x0 <= hull.0 && y0 <= hull.1 && x1 >= hull.2 && y1 >= hull.3,
            "{what}: the box {field} does not hold the item, whose hull is {hull:?}",
        );
        // The rim is under two and a half pixels a side for every transform here (0.9 px
        // for the uniform scales, 1.4 px in x under the shear, and the pixel), and the
        // damage rectangle is rounded out to whole pixels and a pixel more: five pixels
        // a side leaves that room and fails a rim that is still a unit, which is 100 px
        // a side in six of the seven cases.
        let slack = 5.0;
        assert!(
            x0 >= hull.0 - slack
                && y0 >= hull.1 - slack
                && x1 <= hull.2 + slack
                && y1 <= hull.3 + slack,
            "{what}: the box {field} is more than {slack} px past the item's hull {hull:?} \
             on some side — its rim is in the item's own units, not in pixels",
        );
    }
}

/// Eight groups, each holding one item a few tenths of a unit across, and each drawn
/// through its own transform. They are presented nine times: first all red, then with one
/// more group's item replaced by a green one each time. Each present after the first
/// changes exactly one group, so its damage rectangle is that item's box as the group
/// places it (`__canvas_drawBound`).
const GROUPED_BOXES: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT collections

FUNC pt(x AS Float, y AS Float) AS vector::Float2
  RETURN vector::Float2[x := x, y := y]
END FUNC

FUNC square(cx AS Float, cy AS Float, h AS Float) AS List OF vector::Float2
  RETURN [pt(cx - h, cy - h), pt(cx + h, cy - h), pt(cx + h, cy + h), pt(cx - h, cy + h)]
END FUNC

' Item `k`, in colour `c`. Every one but 7 has no transform of its own: its coordinates
' are the group's.
FUNC item(k AS Integer, c AS color::Color) AS canvas::DrawItem
  LET none AS canvas::Transform = canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
  LET origin AS vector::Float2 = pt(0.0, 0.0)
  IF k = 1 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(5.0, 2.0, 0.3)], rule := canvas::FillRule.NonZero, paint := WITH canvas::stroke(c, 0.1) { join := canvas::JoinStyle.Miter, miterLimit := 4.0 }, transform := none]
  END IF
  IF k = 2 THEN
    RETURN canvas::Line[id := "", position := origin, startAt := pt(1.0, 5.0), endAt := pt(2.0, 5.0), paint := WITH canvas::stroke(c, 0.1) { cap := canvas::CapStyle.Square }, transform := none]
  END IF
  IF k = 3 THEN
    RETURN canvas::Circle[id := "", position := origin, radius := 0.3, paint := canvas::fill(c), transform := none]
  END IF
  IF k = 4 OR k = 5 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(0.0, 0.0, 0.4)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := none]
  END IF
  IF k = 6 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(0.0, 0.0, 400.0)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := none]
  END IF
  IF k = 7 THEN
    RETURN canvas::Polygon[id := "", position := origin, contours := [square(0.0, 0.0, 0.4)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := canvas::Transform[a := 10.0, b := 0.0, c := 0.0, d := 10.0, tx := 0.0, ty := 0.0]]
  END IF
  RETURN canvas::Polygon[id := "", position := origin, contours := [square(2.0, 2.0, 0.4)], rule := canvas::FillRule.NonZero, paint := canvas::fill(c), transform := none]
END FUNC

' The node that draws group `k`: where it is and the matrix it draws through.
FUNC node(k AS Integer) AS canvas::DrawItem
  MUT at AS vector::Float2 = pt(0.0, 0.0)
  MUT m AS canvas::Transform = canvas::Transform[a := 100.0, b := 0.0, c := 0.0, d := 100.0, tx := 0.0, ty := 0.0]
  IF k = 3 THEN
    at = pt(500.0, 500.0)
  END IF
  IF k = 4 THEN
    at = pt(750.0, 150.0)
    m = canvas::Transform[a := 80.0, b := 60.0, c := 0.0 - 60.0, d := 80.0, tx := 0.0, ty := 0.0]
  END IF
  IF k = 5 THEN
    at = pt(750.0, 400.0)
    m = canvas::Transform[a := 100.0, b := 0.0, c := 50.0, d := 100.0, tx := 0.0, ty := 0.0]
  END IF
  IF k = 6 THEN
    at = pt(300.0, 560.0)
    m = canvas::Transform[a := 0.05, b := 0.0, c := 0.0, d := 0.05, tx := 0.0, ty := 0.0]
  END IF
  IF k = 7 THEN
    at = pt(150.0, 380.0)
    m = canvas::Transform[a := 10.0, b := 0.0, c := 0.0, d := 10.0, tx := 0.0, ty := 0.0]
  END IF
  RETURN canvas::Group[id := "", position := at, transform := m, name := "g" & toString(k)]
END FUNC

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  MUT scene AS List OF canvas::DrawItem = []
  MUT k AS Integer = 0
  WHILE k < 8
    canvas::setGroup("g" & toString(k), [item(k, color::rgb(200, 40, 40))])
    scene = collections::append(scene, node(k))
    k = k + 1
  END WHILE
  canvas::present(scene)
  k = 0
  WHILE k < 8
    canvas::setGroup("g" & toString(k), [item(k, color::rgb(40, 200, 120))])
    canvas::present(scene)
    k = k + 1
  END WHILE
END FUNC
"#;

/// bug-1083: the box of an item inside a group is the group-mapped hull of its extent
/// and its stroke reach, with a rim of surface pixels, whatever the group's scale.
///
/// An item's own box carries a rim past its extent: one unit when it has no transform,
/// where a unit is a pixel. A group's matrix carried that finished box, so under a
/// group that scales by 100 the rim was 100 px on every side and every pixel of it ran
/// the item's coverage. The rim a pixel needs is set by the composed inverse, so the
/// group's sites scale the item's rim by the norm of the group's inverse instead: 1.41
/// px under a uniform scale, and then the pixel.
#[test]
fn an_items_box_inside_a_scaling_group_is_its_mapped_extent_and_a_rim_of_pixels() {
    // Each item's extent with its stroke, cap or join reach, in the group's units, and
    // the group's matrix with its position as (a, b, c, d, x, y).
    type Case = (&'static str, [f64; 4], [f64; 6]);
    let half = 0.05_f64;
    let cases: [Case; 8] = [
        (
            "a filled square in a group that magnifies 100 times",
            [1.6, 1.6, 2.4, 2.4],
            [100.0, 0.0, 0.0, 100.0, 0.0, 0.0],
        ),
        (
            "a mitered stroked square in a magnifying group",
            [
                4.7 - half * 4.0,
                1.7 - half * 4.0,
                5.3 + half * 4.0,
                2.3 + half * 4.0,
            ],
            [100.0, 0.0, 0.0, 100.0, 0.0, 0.0],
        ),
        (
            "a square-capped line in a magnifying group",
            [
                1.0 - half * 2.0_f64.sqrt(),
                5.0 - half * 2.0_f64.sqrt(),
                2.0 + half * 2.0_f64.sqrt(),
                5.0 + half * 2.0_f64.sqrt(),
            ],
            [100.0, 0.0, 0.0, 100.0, 0.0, 0.0],
        ),
        (
            "a circle in a magnifying group",
            [-0.3, -0.3, 0.3, 0.3],
            [100.0, 0.0, 0.0, 100.0, 500.0, 500.0],
        ),
        (
            "a square in a group that turns and magnifies",
            [-0.4, -0.4, 0.4, 0.4],
            [80.0, 60.0, -60.0, 80.0, 750.0, 150.0],
        ),
        (
            "a square in a group that shears and magnifies",
            [-0.4, -0.4, 0.4, 0.4],
            [100.0, 0.0, 50.0, 100.0, 750.0, 400.0],
        ),
        (
            "a square in a group that shrinks to a twentieth",
            [-400.0, -400.0, 400.0, 400.0],
            [0.05, 0.0, 0.0, 0.05, 300.0, 560.0],
        ),
        (
            // The item's own transform magnifies ten times, and the group ten more.
            "a magnified square in a magnifying group",
            [-4.0, -4.0, 4.0, 4.0],
            [10.0, 0.0, 0.0, 10.0, 150.0, 380.0],
        ),
    ];
    let (_, lines) = render("canvas_damage_grouped_boxes", GROUPED_BOXES, true);
    let rects: Vec<&String> = lines.iter().filter(|l| l.contains("damage=")).collect();
    assert_eq!(
        rects.len(),
        cases.len() + 1,
        "one frame for each present:\n{}",
        lines.join("\n"),
    );
    for (k, (what, extent, m)) in cases.iter().enumerate() {
        let field = rects[k + 1]
            .split("damage=")
            .nth(1)
            .unwrap()
            .split(' ')
            .next()
            .unwrap();
        let parts: Vec<f64> = field
            .split(',')
            .map(|v| {
                v.parse()
                    .unwrap_or_else(|_| panic!("{what}: damage={field}"))
            })
            .collect();
        assert_eq!(
            parts.len(),
            4,
            "{what}: damage should be `x,y,w,h`, got `{field}`"
        );
        let (x0, y0, x1, y1) = (parts[0], parts[1], parts[0] + parts[2], parts[1] + parts[3]);

        let corners = [
            (extent[0], extent[1]),
            (extent[2], extent[1]),
            (extent[0], extent[3]),
            (extent[2], extent[3]),
        ];
        let xs = corners.map(|(x, y)| m[0] * x + m[2] * y + m[4]);
        let ys = corners.map(|(x, y)| m[1] * x + m[3] * y + m[5]);
        let hull = (
            xs.iter().cloned().fold(f64::INFINITY, f64::min),
            ys.iter().cloned().fold(f64::INFINITY, f64::min),
            xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
            ys.iter().cloned().fold(f64::NEG_INFINITY, f64::max),
        );
        assert!(
            x0 <= hull.0 && y0 <= hull.1 && x1 >= hull.2 && y1 >= hull.3,
            "{what}: the box {field} does not hold the item, whose hull is {hull:?}",
        );
        // The rim is the item's own rim scaled by the norm of the group's inverse: 1.41
        // px for the uniform scales, 1.98 px a side for the turned group's hull, 2.25 px
        // in x for the shear and 2.42 px for the item with its own transform. Then the
        // pixel, and the damage rectangle is rounded out to whole pixels and a pixel
        // more: six and a half pixels a side leaves that room and fails a rim that is
        // still a unit of the group's space, which is 100 px a side in six of the cases
        // and 17 px in the last.
        let slack = 6.5;
        assert!(
            x0 >= hull.0 - slack
                && y0 >= hull.1 - slack
                && x1 <= hull.2 + slack
                && y1 <= hull.3 + slack,
            "{what}: the box {field} is more than {slack} px past the item's hull {hull:?} \
             on some side — its rim is in the group's units, not in pixels",
        );
    }
}

/// A program that reports `didResize` before and after a scripted resize.
///
/// The resize affordance is `MFB_CANVAS_RESIZE_W`/`_H`, which drives the **production**
/// resize path — the same helper the platform's own resize signal calls — and it exists
/// only on the GTK and Windows backends, so this runs headless there. On macOS there is
/// no scripted resize, so the same program exercises only the "no resize yet" half; that
/// is why the
/// assertions below are about the sequence of answers rather than about a fixed count.
const RESIZE_POLL: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color
IMPORT io
IMPORT os

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 10.0, y := 10.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 50.0, height := 50.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::present([box])
  os::sleep(150)
  ' Before anything resizes: the surface has been its size since it existed, so there
  ' is no CHANGE to report.
  io::print("first:" & toString(canvas::didResize()))
  MUT seen AS Integer = 0
  MUT i AS Integer = 0
  WHILE i < 30
    IF canvas::didResize() THEN
      seen = seen + 1
    END IF
    canvas::present([box])
    os::sleep(50)
    i = i + 1
  END WHILE
  io::print("resizes:" & toString(seen))
END FUNC
"#;

#[test]
fn did_resize_is_false_until_the_surface_changes_and_then_true_once() {
    // Without a scripted resize this is the whole assertion available on macOS, and it
    // is the half that would break first: a `didResize` that answered TRUE on its first
    // call — because the counter started at one, or because the platform re-published
    // the size it already had — would make every program lay out twice at startup and
    // look correct while doing it.
    let project = common::temp_project("canvas_did_resize", RESIZE_POLL);
    let binary = common::build_app_debug(&project, "canvas_did_resize");
    let run = std::process::Command::new(&binary)
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    let _ = std::fs::remove_dir_all(&project);

    assert!(
        out.contains("first:FALSE"),
        "didResize was TRUE before anything resized:\n{out}",
    );
    assert!(
        out.contains("resizes:0"),
        "didResize reported a resize in a run where nothing resized:\n{out}",
    );
}

/// `RESIZE_POLL` run headless with a scripted resize to `width` by `height`: whether the
/// first `didResize` answered TRUE, and how many later ones did.
fn scripted_resize(name: &str, width: &str, height: &str) -> (bool, String) {
    let project = common::temp_project(name, RESIZE_POLL);
    // `--debug`: only a debug build reads the scripted resize.
    let binary = common::build_app_debug(&project, name);
    let run = std::process::Command::new(&binary)
        .current_dir(&project)
        .env("MFB_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_RESIZE_W", width)
        .env("MFB_CANVAS_RESIZE_H", height)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let out = String::from_utf8_lossy(&run.stdout).to_string();
    assert!(
        run.status.success(),
        "program {}:\n{out}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stderr),
    );
    let _ = std::fs::remove_dir_all(&project);
    let later = out
        .lines()
        .find_map(|line| line.strip_prefix("resizes:"))
        .unwrap_or_else(|| panic!("no `resizes:` line:\n{out}"))
        .to_string();
    (out.contains("first:TRUE"), later)
}

/// bug-970: a publish of the size the program already sees is not a resize, even when
/// it is the first.
///
/// The surface is 900 by 640 before anything publishes a size (`canvas::getSize` says
/// so), and a window that opens at that size publishes exactly that. The scripted
/// resize calls the platform's own resize handler, so on GTK and Windows this is that
/// first publish without a window; macOS has no scripted resize, and there the
/// publisher is covered by executing its instructions
/// (`publishing_the_size_the_program_already_sees_counts_no_resize`) and by a real
/// window (`scripts/test-macapp.sh` case 3d3).
#[test]
fn a_first_publish_of_the_startup_size_is_not_a_resize() {
    let (first, later) = scripted_resize("canvas_did_resize_same", "900", "640");
    assert!(
        !first && later == "0",
        "a publish of 900 by 640 over a surface that already reads 900 by 640 changed \
         nothing, but didResize answered first={first} then {later} more",
    );
}

/// The other half: a publish of another size is one resize, reported once.
#[test]
fn a_first_publish_of_another_size_is_one_resize() {
    let (first, later) = scripted_resize("canvas_did_resize_other", "640", "480");
    let reported = usize::from(first) + later.parse::<usize>().expect("a count");
    // macOS has no scripted resize, so nothing publishes there.
    let expected = if cfg!(target_os = "macos") { 0 } else { 1 };
    assert_eq!(
        reported, expected,
        "one scripted resize to 640 by 480: first={first}, later={later}",
    );
}
