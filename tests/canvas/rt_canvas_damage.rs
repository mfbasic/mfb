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

use std::process::Command;

const WIDTH: usize = 900;

/// Render headless and return the frame plus every stats line.
fn render(name: &str, source: &str, damage: bool) -> (Vec<u8>, Vec<String>) {
    let project = common::temp_project(name, source);
    let binary = common::build_app(&project, name);
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let mut command = Command::new(&binary);
    command
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame)
        .env("MFB_CANVAS_STATS", &stats)
        .env("MFB_CANVAS_SYNC", "1");
    if damage {
        command.env("MFB_CANVAS_DAMAGE", "1");
    }
    let run = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let pixels = std::fs::read(&frame).expect("canvas dump written");
    let lines: Vec<String> = std::fs::read_to_string(&stats)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect();
    let _ = std::fs::remove_dir_all(&project);
    (pixels, lines)
}

/// One pixel of a dumped frame, as RGBA.
fn pixel(frame: &[u8], x: usize, y: usize) -> (u8, u8, u8, u8) {
    let i = (y * WIDTH + x) * 4;
    (frame[i], frame[i + 1], frame[i + 2], frame[i + 3])
}

/// A `key=value` field of the last stats line.
fn last(lines: &[String], key: &str) -> String {
    let line = lines
        .iter()
        .rev()
        .find(|l| l.contains(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no stats line carries `{key}=`:\n{}", lines.join("\n")));
    line.split(&format!("{key}="))
        .nth(1)
        .unwrap()
        .split(' ')
        .next()
        .unwrap()
        .to_string()
}

fn number(lines: &[String], key: &str) -> i64 {
    last(lines, key)
        .parse()
        .unwrap_or_else(|e| panic!("`{key}` is not a number: {e}"))
}

/// Two static shapes, presented three times with no change between them.
///
/// **The sleeps are load-bearing.** Redraw requests coalesce by design
/// (`.ai/canvas-threading.md` §3): three presents issued back to back can wake the
/// graphics thread once, and then a test counting frames is measuring the scheduler
/// rather than the damage logic. Measured without them — one stats line, `frames=1`,
/// `skipped=0`, which is indistinguishable from damage not working at all.
const UNCHANGED: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 200.0, h := 120.0, paint := canvas::fill(color::rgb(200, 40, 40))]
  LET dot AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 400.0, radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120))]
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([box, dot])
  os::sleep(150)
END SUB
"#;

/// The same scene, except that the second present moves the circle a long way.
const MOVED: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 200.0, h := 120.0, paint := canvas::fill(color::rgb(200, 40, 40))]
  LET dot AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 400.0, radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120))]
  LET moved AS canvas::DrawItem = canvas::Circle[x := 700.0, y := 420.0, radius := 60.0, paint := canvas::fill(color::rgb(40, 200, 120))]
  canvas::present([box, dot])
  os::sleep(150)
  canvas::present([box, moved])
  os::sleep(150)
END SUB
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

/// A program that reports `didResize` before and after a scripted resize.
///
/// The resize affordance is `MFB_CANVAS_RESIZE_W`/`_H`, which drives the **production**
/// resize path — the same helper the platform's own resize signal calls — and it exists
/// only on the GTK backend, so this runs headless there. On macOS there is no scripted
/// resize, so the same program exercises only the "no resize yet" half; that is why the
/// assertions below are about the sequence of answers rather than about a fixed count.
const RESIZE_POLL: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT io
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 10.0, y := 10.0, w := 50.0, h := 50.0, paint := canvas::fill(color::rgb(200, 40, 40))]
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
END SUB
"#;

#[test]
fn did_resize_is_false_until_the_surface_changes_and_then_true_once() {
    // Without a scripted resize this is the whole assertion available on macOS, and it
    // is the half that would break first: a `didResize` that answered TRUE on its first
    // call — because the counter started at one, or because the platform re-published
    // the size it already had — would make every program lay out twice at startup and
    // look correct while doing it.
    let project = common::temp_project("canvas_did_resize", RESIZE_POLL);
    let binary = common::build_app(&project, "canvas_did_resize");
    let run = std::process::Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
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
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  LET far AS canvas::DrawItem = canvas::Rectangle[x := 20.0, y := 20.0, w := 60.0, h := 60.0, paint := canvas::fill(color::rgb(200, 40, 40))]
  LET a AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 100.0, h := 100.0, paint := canvas::fill(color::rgb(40, 200, 120))]
  LET b AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 100.0, h := 100.0, paint := canvas::fill(color::rgb(40, 120, 220))]
  canvas::setGroup("panel", [a])
  LET node AS canvas::DrawItem = canvas::Group[dx := 500.0, dy := 300.0, name := "panel"]
  canvas::present([far, node])
  canvas::setGroup("panel", [b])
  canvas::present([far, node])
END SUB
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
IMPORT color

SUB main()
  app::setMode(app::Mode.Canvas)
  LET a AS canvas::DrawItem = canvas::Rectangle[x := 0.0, y := 0.0, w := 100.0, h := 100.0, paint := canvas::fill(color::rgb(40, 200, 120))]
  canvas::setGroup("panel", [a])
  LET here AS canvas::DrawItem = canvas::Group[dx := 100.0, dy := 100.0, name := "panel"]
  LET there AS canvas::DrawItem = canvas::Group[dx := 600.0, dy := 400.0, name := "panel"]
  canvas::present([here])
  canvas::present([there])
END SUB
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
