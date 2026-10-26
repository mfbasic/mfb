//! Damage-limited repaint of groups (plan-116-G Phase 4, plan-172-G, plan-175-A Phase 5).
//!
//! A group node's own geometry is empty and its children are drawn through a name, so
//! the damage contracts `rt_canvas_damage.rs` checks for a plain item are re-checked here
//! for items drawn through a group: replaced contents, a moved node, a turned node, and
//! one `setGroup` changing two children in different lanes. Split out of
//! `rt_canvas_damage.rs` by plan-175-A Phase 5 (the file-size budget).

#[path = "../common/mod.rs"]
mod common;
#[path = "damage_harness/mod.rs"]
mod harness;

use harness::{last, number, pixel, render, render_env};

/// Replacing a group's contents damages the area the group draws, not the window
/// (plan-116-G Phase 4, §4.6).
///
/// This is the §4.6 failure written as a test. A `Group` node's own geometry is empty,
/// so a design that left the node in the draw list and diffed *its* bounds would damage
/// a zero-area rectangle: the frame would be reported as changed and repaint nothing,
/// which reads as "the screen only updates on full redraws".
///
/// **No sleeps, unlike `UNCHANGED` and `MOVED` above.** Those predate this harness
/// setting `MFB_CANVAS_SYNC=1` unconditionally, and with it `present` already waits for
/// the frame it asked for — so the coalescing those sleeps guard against cannot happen
/// here, and a fixed delay would only be a way for a slow or contended machine to make
/// these flaky. Verified rather than assumed: the frame-count assertions below hold
/// without them.
const GROUP_REPLACED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET far AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 20.0, y := 20.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 60.0, height := 60.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET a AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET b AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 120, 220)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("panel", [a])
  LET node AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 500.0, y := 300.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]
  canvas::present([far, node])
  canvas::setGroup("panel", [b])
  canvas::present([far, node])
END FUNC
"#;

#[test]
fn replacing_a_group_damages_where_the_group_draws() {
    let (frame, stats) = render("canvas_damage_group", GROUP_REPLACED, true);

    assert_eq!(
        number(&stats, "frames"),
        2,
        "the second present must produce a frame: the scene list is byte-identical, so \
         only the group's revision can have told it to. stats:\n{}",
        stats.join("\n"),
    );
    assert!(
        number(&stats, "partial") >= 1,
        "the redraw must be PARTIAL. A full redraw would also show the new colour, so \
         the pixel assertion below cannot tell the two apart — this is the assertion \
         that says the damage rectangle was a real one rather than the whole window. \
         stats:\n{}",
        stats.join("\n"),
    );

    // The group's new colour is on screen where the group draws...
    assert_eq!(
        pixel(&frame, 550, 350),
        (40, 120, 220, 255),
        "the group's replaced contents were not painted at its offset",
    );
    // ...and the untouched item elsewhere still is, which is what a partial redraw
    // promises and a damage rectangle that was too large would also satisfy — so it is
    // paired with the `partial` assertion above rather than standing alone.
    assert_eq!(
        pixel(&frame, 50, 50),
        (200, 40, 40, 255),
        "the item outside the group's area was lost",
    );

    let damage = last(&stats, "damage");
    let parts: Vec<i64> = damage.split(',').map(|p| p.parse().unwrap_or(-1)).collect();
    // `damage=` is `x,y,WIDTH,HEIGHT` — `__canvas_damageText` subtracts the origin
    // before printing — not `x0,y0,x1,y1`.
    assert_eq!(parts.len(), 4, "damage= should be x,y,w,h: {damage}");
    let (x, y, w, h) = (parts[0], parts[1], parts[2], parts[3]);
    assert!(
        (498..=500).contains(&x)
            && (298..=300).contains(&y)
            && (100..=106).contains(&w)
            && (100..=106).contains(&h),
        "the damage rectangle {damage} (x,y,w,h) is not the group's drawn area, which is \
         100x100 at (500,300) plus at most a pixel or two of antialias margin. An \
         origin-anchored rectangle means the group node's own empty bounds were diffed \
         instead of its children's; 900x640 means it fell back to a full redraw.",
    );
}

/// Moving a group node repaints both where it was and where it went.
///
/// The same contract any moved item has, and it has to be re-tested for a group because
/// what moves is the *node* while the geometry in the cache does not change at all —
/// an implementation keying damage off the geometry would see nothing changed.
const GROUP_MOVED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("panel", [a])
  LET here AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]
  LET there AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 600.0, y := 400.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]
  canvas::present([here])
  canvas::present([there])
END FUNC
"#;

#[test]
fn a_moved_group_repaints_both_positions() {
    let (frame, stats) = render("canvas_damage_group_move", GROUP_MOVED, true);

    assert_eq!(number(&stats, "frames"), 2, "stats:\n{}", stats.join("\n"));
    assert_eq!(
        pixel(&frame, 650, 450),
        (40, 200, 120, 255),
        "the group was not painted at its new position",
    );
    assert_eq!(
        pixel(&frame, 150, 150),
        (0, 0, 0, 255),
        "the group's OLD position was not erased — a moved item has to repaint where it \
         was as well as where it is, and for a group the geometry in the cache is \
         unchanged, so damage keyed off the geometry alone would miss this entirely",
    );
}

/// plan-172-G: changing only a group node's `transform` repaints.
///
/// Nothing else about the frame changes: the same group, at the same position, holding
/// the same geometry in the cache. The node is turned a quarter turn between the two
/// presents. Damage has to see that, erase where the group was and paint where it is
/// — and what it paints has to be the frame a full redraw paints, byte for byte.
const GROUP_TURNED: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET none AS canvas::Transform = canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]
  LET bar AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 40.0, y := 0.0 - 15.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 120.0, height := 30.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := none]
  LET ring AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 100.0, y := 0.0], radius := 30.0, paint := canvas::stroke(color::rgb(255, 255, 255), 6.0), transform := none]
  LET still AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 760.0, y := 120.0], radius := 50.0, paint := canvas::fill(color::rgb(200, 60, 60)), transform := none]
  canvas::setGroup("panel", [bar, ring])
  LET flat AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 400.0, y := 300.0], transform := none, name := "panel"]
  LET turned AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 400.0, y := 300.0], transform := canvas::Transform[a := 0.0, b := 1.0, c := 0.0 - 1.0, d := 0.0, tx := 0.0, ty := 0.0], name := "panel"]
  canvas::present([still, flat])
  canvas::present([still, turned])
END FUNC
"#;

#[test]
fn turning_a_group_repaints_and_matches_a_full_redraw() {
    let (damaged, stats) = render("canvas_damage_group_turn", GROUP_TURNED, true);
    let (full, _) = render("canvas_damage_group_turn_full", GROUP_TURNED, false);

    assert_eq!(number(&stats, "frames"), 2, "stats:\n{}", stats.join("\n"));
    assert_eq!(
        pixel(&damaged, 400, 400),
        (40, 200, 120, 255),
        "the bar was not painted where the quarter turn puts it — (440..560, 285..315) \
         turned about (400, 300) is (385..415, 340..460)",
    );
    assert_eq!(
        pixel(&damaged, 500, 300),
        (0, 0, 0, 255),
        "the group's unturned place was not erased: the geometry and the node's \
         position are unchanged, so only a hash that folds in the matrix sees the turn",
    );
    assert_eq!(
        pixel(&damaged, 760, 120),
        (200, 60, 60, 255),
        "the item outside the group was disturbed",
    );
    // The second frame was a partial redraw, not a fallback to the whole window.
    let damage = last(&stats, "damage");
    assert_ne!(
        damage, "0,0,900,640",
        "turning one group damaged the whole window: {damage}",
    );
    assert!(
        damaged == full,
        "the frame drawn through damage differs from the frame a full redraw draws",
    );
}

/// plan-175-A Phase 5: one `setGroup` that changes one child's PAINT and another's
/// GEOMETRY redraws both.
///
/// The group walk files its children's identities per slot, keyed by the slot's revision
/// and items block (`__canvas_groupHashes`). A key that missed the replacement would hand
/// the second frame the first frame's `(D, G)`: the recoloured box would resolve as a hit
/// on the old colour and the moved circle would be drawn where it was. `left` keeps its
/// geometry and changes its colour (a refresh); `right` keeps its colour and moves (a
/// build). Rendered with damage on and off under `MFB_CANVAS_GEO_VERIFY=1`, and once on
/// the GPU renderer when the host has one.
const GROUP_PAINT_AND_GEOMETRY: &str = r#"IMPORT app
IMPORT canvas
IMPORT vector
IMPORT color

FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  LET left AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(200, 40, 40)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET recoloured AS canvas::DrawItem = canvas::Rectangle[id := "", position := vector::Float2[x := 0.0, y := 0.0], bind := canvas::BindTo.TopLeft, size := canvas::Size[width := 100.0, height := 100.0], radiusTL := 0.0, radiusTR := 0.0, radiusBL := 0.0, radiusBR := 0.0, paint := canvas::fill(color::rgb(40, 120, 220)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET right AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 250.0, y := 50.0], radius := 40.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  LET moved AS canvas::DrawItem = canvas::Circle[id := "", position := vector::Float2[x := 250.0, y := 250.0], radius := 40.0, paint := canvas::fill(color::rgb(40, 200, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  canvas::setGroup("panel", [left, right])
  LET node AS canvas::DrawItem = canvas::Group[id := "", position := vector::Float2[x := 100.0, y := 100.0], transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0], name := "panel"]
  canvas::present([node])
  canvas::setGroup("panel", [recoloured, moved])
  canvas::present([node])
END FUNC
"#;

#[test]
fn a_set_group_changing_one_childs_paint_and_anothers_geometry_redraws_both() {
    let verify = [("MFB_CANVAS_GEO_VERIFY", "1")];
    let (damaged, stats) = render_env(
        "canvas_damage_group_two_lanes",
        GROUP_PAINT_AND_GEOMETRY,
        true,
        &verify,
    );
    let (full, full_stats) = render_env(
        "canvas_damage_group_two_lanes_full",
        GROUP_PAINT_AND_GEOMETRY,
        false,
        &verify,
    );
    let check = |frame: &[u8], stats: &[String], how: &str| {
        assert_eq!(
            number(stats, "frames"),
            2,
            "{how}: the replacement must draw a second frame:\n{}",
            stats.join("\n")
        );
        assert_eq!(
            number(stats, "geoVerifyMismatches"),
            0,
            "{how}: a resolved record differs from a fresh build:\n{}",
            stats.join("\n")
        );
        assert_eq!(
            pixel(frame, 150, 150),
            (40, 120, 220, 255),
            "{how}: the recoloured child still shows its old paint"
        );
        assert_eq!(
            pixel(frame, 350, 350),
            (40, 200, 120, 255),
            "{how}: the moved child was not drawn where it went"
        );
        assert_eq!(
            pixel(frame, 350, 150),
            (0, 0, 0, 255),
            "{how}: the moved child is still drawn where it was"
        );
    };
    check(&damaged, &stats, "damage on");
    check(&full, &full_stats, "damage off");
    assert!(
        number(&stats, "partial") >= 1,
        "the redraw must be partial:\n{}",
        stats.join("\n")
    );
    assert!(
        damaged == full,
        "damage changed a pixel of the replaced group"
    );

    let gpu = [("MFB_CANVAS_GEO_VERIFY", "1"), ("MFB_CANVAS_GPU", "1")];
    let (frame, stats) = render_env(
        "canvas_damage_group_two_lanes_gpu",
        GROUP_PAINT_AND_GEOMETRY,
        false,
        &gpu,
    );
    if last(&stats, "gpuFrames") != "0" {
        check(&frame, &stats, "GPU");
    }
}
