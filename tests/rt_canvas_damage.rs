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

mod common;

use std::process::Command;

const WIDTH: usize = 900;

/// Render headless and return the frame plus every stats line.
fn render(name: &str, source: &str, damage: bool) -> (Vec<u8>, Vec<String>) {
    let project = common::temp_project(name, source);
    let build = Command::new(common::mfb_exe())
        .arg("build")
        .arg("-app")
        .arg(&project)
        .output()
        .expect("run mfb build -app");
    assert!(
        build.status.success(),
        "mfb build -app failed:\n{}\n{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let frame = project.join("frame.rgba");
    let stats = project.join("stats.txt");
    let binary = app_binary(&project, name);
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
        "program exited {:?}:\n{}\n{}",
        run.status.code(),
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

fn app_binary(project: &std::path::Path, name: &str) -> std::path::PathBuf {
    let bundled = project
        .join("build")
        .join(format!("{name}.app"))
        .join("Contents")
        .join("MacOS")
        .join(name);
    if bundled.exists() {
        return bundled;
    }
    let plain = project.join("build").join(name);
    if plain.exists() {
        return plain;
    }
    project.join("build").join(format!("{name}.exe"))
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
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 200.0, h := 120.0, paint := canvas::fill(canvas::rgb(200, 40, 40))]
  LET dot AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 400.0, radius := 60.0, paint := canvas::fill(canvas::rgb(40, 200, 120))]
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
IMPORT os

SUB main()
  app::setMode(app::Mode.Canvas)
  LET box AS canvas::DrawItem = canvas::Rectangle[x := 100.0, y := 100.0, w := 200.0, h := 120.0, paint := canvas::fill(canvas::rgb(200, 40, 40))]
  LET dot AS canvas::DrawItem = canvas::Circle[x := 600.0, y := 400.0, radius := 60.0, paint := canvas::fill(canvas::rgb(40, 200, 120))]
  LET moved AS canvas::DrawItem = canvas::Circle[x := 700.0, y := 420.0, radius := 60.0, paint := canvas::fill(canvas::rgb(40, 200, 120))]
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
