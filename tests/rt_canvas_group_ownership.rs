//! plan-116-J: `canvas::setGroup` takes ownership of the resources its items name, so
//! the caller's bindings may go out of scope without the group losing them.
//!
//! **Why a `Font` and not an `Image`.** The obvious test — install a `Picture`, drop the
//! binding, check the image still draws — cannot discriminate anything: `Picture` draws
//! nothing on any backend yet. `helper_geometry.rs` gives it the `NONE` geometry kind
//! (`CASE Picture(pic) RETURN __canvas_emptyHeader()`), which every renderer skips, and
//! `canvas::imageHandle` has no caller in any renderer while its twin `fontHandle` has
//! six. So a `Picture`-based version of this test renders an all-black frame whether the
//! ownership works or not, and would keep failing after the letter was correct
//! (plan-116-J **J11**). `Text` renders, so a `Font` is the live observable.
//!
//! **Both directions are asserted, and the control is the load-bearing half.** Glyph
//! count zero is equally consistent with "the font was closed too early" and with "group
//! text never renders at all" — which is exactly the trap the `Picture` version falls
//! into. `the_control_draws_with_the_binding_alive` is what makes the ownership
//! assertion mean something.

mod common;

use std::path::Path;
use std::process::Command;

/// The ownership case: `install` opens the font, builds a `Text` item, installs the
/// group, and RETURNS — dropping every binding it made. Without ownership, scope-drop
/// closes the font here and the group's text draws no glyphs.
const DROPS_ITS_BINDING: &str = r#"IMPORT app
IMPORT canvas

SUB install()
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    EXIT SUB
  END TRAP
  LET tag AS canvas::DrawItem = canvas::Text[x := 40.0, y := 120.0, text := "AA", font := face, size := 64.0, paint := canvas::fill(canvas::rgb(230, 60, 170))]
  canvas::setGroup("held", [tag])
END SUB

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  install()
  canvas::present([canvas::Group[name := "held", dx := 0.0, dy := 0.0]])
  RETURN 0
END FUNC
"#;

/// The control: identical scene, but the `Font` binding stays alive in `main`, so the
/// group is drawn while the caller still owns the font either way.
const KEEPS_ITS_BINDING: &str = r#"IMPORT app
IMPORT canvas

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN 1
  END TRAP
  LET tag AS canvas::DrawItem = canvas::Text[x := 40.0, y := 120.0, text := "AA", font := face, size := 64.0, paint := canvas::fill(canvas::rgb(230, 60, 170))]
  canvas::setGroup("held", [tag])
  canvas::present([canvas::Group[name := "held", dx := 0.0, dy := 0.0]])
  RETURN 0
END FUNC
"#;

/// Build and run a canvas scene headless; return its `MFB_CANVAS_STATS` line.
fn stats_for(name: &str, source: &str) -> String {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
        .expect("write the font fixture");
    let stats_path = project.join("stats.txt");
    let binary = common::build_app(&project, name);
    let run = Command::new(&binary)
        // The project directory, so `loadFont("fixture.ttf")` resolves against it.
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        // Wait for the frame. Without it `present` returns at once and the process tears
        // down while the graphics thread is still reading the scene — a font's outlines
        // live in the worker's arena, which is per-thread, so the frame lands with zero
        // text and reads as a reference rather than a truncated one.
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats_path)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let stats = std::fs::read_to_string(&stats_path).unwrap_or_default();
    let _ = std::fs::remove_dir_all(&project);
    stats
}

/// Build and run a canvas scene headless; return `(stdout, every stats line)`.
///
/// `MFB_CANVAS_STATS` is appended to once per present, so a multi-present scene leaves
/// one line per frame and the caller can watch a number move rather than only read its
/// final value.
fn run_for(name: &str, source: &str, with_font: bool) -> (String, Vec<String>) {
    let project = common::temp_project(name, source);
    if with_font {
        std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
            .expect("write the font fixture");
    }
    let stats_path = project.join("stats.txt");
    let binary = common::build_app(&project, name);
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_SYNC", "1")
        .env("MFB_CANVAS_STATS", &stats_path)
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    assert!(
        run.status.success(),
        "program {}:\n{}\n{}",
        common::exit_description(&run.status),
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr),
    );
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    let stats = std::fs::read_to_string(&stats_path).unwrap_or_default();
    let lines: Vec<String> = stats.lines().map(str::to_string).collect();
    let _ = std::fs::remove_dir_all(&project);
    (out, lines)
}

/// `<field>=` from a stats line, by NAME.
fn field(stats: &str, name: &str) -> u64 {
    let prefix = format!("{name}=");
    stats
        .split_whitespace()
        .find_map(|f| f.strip_prefix(prefix.as_str()))
        .unwrap_or_else(|| panic!("no `{name}=` field in stats: {stats:?}"))
        .parse()
        .unwrap_or_else(|_| panic!("`{name}=` is not a number in: {stats:?}"))
}

/// **J14**: one long-lived font and a group rebuilt each frame. The retired buffer and
/// the buffer replacing it name the SAME font, so a close-what-the-retired-buffer-named
/// rule closes it and the text vanishes one frame later — silently, because `fontHandle`
/// answers `0` for a closed resource and `0` is "no such object".
///
/// This is not an exotic program. It is the shape every real canvas application has, and
/// it is why the free path closes only what no LIVE buffer names.
const REBUILDS_EACH_FRAME: &str = r#"IMPORT app
IMPORT canvas

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    RETURN 1
  END TRAP
  FOR i = 1 TO 6
    LET tag AS canvas::DrawItem = canvas::Text[x := 40.0, y := 120.0, text := "AA", font := face, size := 64.0, paint := canvas::fill(canvas::rgb(230, 60, 170))]
    canvas::setGroup("panel", [tag])
    canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], canvas::Rectangle[x := 0.0, y := toFloat(i), w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])
  NEXT
  RETURN 0
END FUNC
"#;

/// The other direction: a replacement whose items do NOT name the resource must close it,
/// or the group leaks what it took ownership of.
///
/// `install` takes the image as a **`RES` parameter** — an alias, so `setGroup` moves the
/// callee's local and `main`'s own binding survives to read the verdict. `canvas::getSize`
/// raises `ErrResourceClosed` on a closed image, which is the only direct observable of a
/// close there is: `Picture` draws nothing on any backend (**J11**), and an image holds no
/// file descriptor to watch.
const REPLACED_THEN_DROPPED: &str = r#"IMPORT app
IMPORT canvas
IMPORT io

SUB install(RES img AS canvas::Image)
  LET p AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 8.0, h := 8.0, image := img, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::setGroup("panel", [p])
END SUB

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  LET px AS List OF Byte = [toByte(1), toByte(2), toByte(3), toByte(4)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  install(img)

  LET before AS canvas::Size = canvas::getSize(img) TRAP(e)
    io::print("CLOSED-TOO-EARLY")
    RETURN 2
  END TRAP
  io::print("OPEN-AFTER-INSTALL")

  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0]])
  canvas::setGroup("panel", [canvas::Rectangle[x := 0.0, y := 0.0, w := 20.0, h := 20.0, paint := canvas::fill(canvas::rgb(0, 200, 0))]])
  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], canvas::Rectangle[x := 30.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])
  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], canvas::Rectangle[x := 40.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])
  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], canvas::Rectangle[x := 50.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])

  LET after AS canvas::Size = canvas::getSize(img) TRAP(e)
    io::print("CLOSED-BY-THE-GROUP")
    RETURN 0
  END TRAP
  io::print("STILL-OPEN-LEAKED")
  RETURN 3
END FUNC
"#;

/// **J14's residual case**: a resource named by a group *and* by the live scene.
///
/// `present` does not take ownership (§Non-goals), so a `Picture` built **before** the
/// `setGroup` reaches the scene with nothing for the move checker to object to — the
/// scene's item names `img`, and its construction happened before `img` was moved. If the
/// group's free closed it, the scene's item would start drawing nothing, silently, one
/// frame later.
///
/// This failed when first probed (`CLOSED-OUT-FROM-UNDER-THE-SCENE`), which is why the
/// live set is every group's items **plus the incoming scene** rather than just the
/// replacing slot's.
const GROUP_AND_SCENE_SHARE: &str = r#"IMPORT app
IMPORT canvas
IMPORT io

SUB install(RES img AS canvas::Image)
  LET p AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 8.0, h := 8.0, image := img, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::setGroup("panel", [p])
END SUB

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  LET px AS List OF Byte = [toByte(1), toByte(2), toByte(3), toByte(4)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  LET scenePic AS canvas::DrawItem = canvas::Picture[x := 100.0, y := 0.0, w := 8.0, h := 8.0, image := img, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  install(img)

  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], scenePic])
  canvas::setGroup("panel", [canvas::Rectangle[x := 0.0, y := 0.0, w := 20.0, h := 20.0, paint := canvas::fill(canvas::rgb(0, 200, 0))]])
  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], scenePic, canvas::Rectangle[x := 30.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])
  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], scenePic, canvas::Rectangle[x := 40.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])
  canvas::present([canvas::Group[name := "panel", dx := 0.0, dy := 0.0], scenePic, canvas::Rectangle[x := 50.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])

  LET after AS canvas::Size = canvas::getSize(img) TRAP(e)
    io::print("CLOSED-OUT-FROM-UNDER-THE-SCENE")
    RETURN 0
  END TRAP
  io::print("STILL-OPEN-SCENE-SAFE")
  RETURN 0
END FUNC
"#;

/// A group must not close a resource the live scene still names. **J14.**
#[test]
fn a_group_does_not_close_an_image_the_scene_still_names() {
    let (out, _) = run_for("canvas_group_own_scene_share", GROUP_AND_SCENE_SHARE, false);
    assert!(
        out.contains("STILL-OPEN-SCENE-SAFE"),
        "the scene names this image directly, so the group's free must leave it open — \
         `CLOSED-OUT-FROM-UNDER-THE-SCENE` means the scene's item silently draws nothing \
         from the next frame on\n{out}",
    );
}

/// A group rebuilt each frame from one font keeps drawing. **J14.**
#[test]
fn a_group_rebuilt_each_frame_keeps_its_font() {
    let (_, stats) = run_for("canvas_group_own_rebuild", REBUILDS_EACH_FRAME, true);
    assert!(
        stats.len() >= 4,
        "expected one stats line per present: {stats:?}"
    );
    for (i, line) in stats.iter().enumerate() {
        assert!(
            field(line, "glyphs") > 0,
            "frame {i} lost its glyphs — the free path closed a font the replacing \
             buffer still names, which is what J14 exists to prevent\n{line}",
        );
    }
}

/// …and the buffer it replaces is still freed, so keeping the font is not achieved by
/// keeping the block. Without this the test above passes against a free path that does
/// nothing at all.
#[test]
fn the_rebuild_still_frees_the_buffers_it_retires() {
    let (_, stats) = run_for("canvas_group_own_rebuild_bytes", REBUILDS_EACH_FRAME, true);
    let last = stats.last().expect("at least one present");
    let first = &stats[0];
    assert!(
        field(last, "groupBytes") <= field(first, "groupBytes") * 3,
        "groupBytes grew across 6 rebuilds, so retired buffers are not being freed\n\
         first: {first}\nlast:  {last}",
    );
}

/// A replacement that drops the resource closes it. The whole point of ownership: the
/// caller's binding went out of scope at `install`'s return, so if the group does not
/// close it, nobody does.
#[test]
fn a_replacement_that_drops_the_image_closes_it() {
    let (out, _) = run_for("canvas_group_own_dropped_img", REPLACED_THEN_DROPPED, false);
    assert!(
        out.contains("OPEN-AFTER-INSTALL"),
        "the image must stay open while the group holds it\n{out}",
    );
    assert!(
        out.contains("CLOSED-BY-THE-GROUP"),
        "the group took ownership and then dropped the image from its items, so the \
         reclaim must close it — `STILL-OPEN-LEAKED` means the group owns a resource \
         nothing will ever close\n{out}",
    );
}

/// `glyphs=` from a stats line. Asserted by NAME rather than by field position: the
/// stats line has grown a field per canvas plan and a positional read would silently
/// start reporting its neighbour.
fn glyph_count(stats: &str) -> u32 {
    stats
        .split_whitespace()
        .find_map(|field| field.strip_prefix("glyphs="))
        .unwrap_or_else(|| panic!("no `glyphs=` field in stats: {stats:?}"))
        .parse()
        .expect("glyphs= is a number")
}

/// The control, and it runs first in this file for a reason: if group text does not
/// render at all, the ownership assertion below is vacuous and this is what says so.
#[test]
fn the_control_draws_with_the_binding_alive() {
    let stats = stats_for("canvas_group_own_control", KEEPS_ITS_BINDING);
    assert!(
        glyph_count(&stats) > 0,
        "a group's Text must render while the caller still holds the font — if this \
         fails, group text is broken and `the_group_keeps_the_font_past_its_scope` \
         proves nothing either way\nstats: {stats}",
    );
}

/// plan-116-J Goal bullet 1: the caller's binding goes out of scope and the group's text
/// still draws.
#[test]
fn the_group_keeps_the_font_past_its_scope() {
    let stats = stats_for("canvas_group_own_dropped", DROPS_ITS_BINDING);
    assert!(
        glyph_count(&stats) > 0,
        "`setGroup` must take ownership of the font its items name: `install` returned \
         and dropped every binding it made, so if scope-drop closed the font the \
         group's Text draws no glyphs. `fontHandle` answers 0 for a closed resource and \
         0 is \"no such object\", so this fails silently — nothing raises.\nstats: {stats}",
    );
}

/// The two must agree. A group that owns its font draws the same text as one whose
/// caller still holds it — ownership changes who closes, not what is drawn.
#[test]
fn ownership_does_not_change_what_is_drawn() {
    let owned = stats_for("canvas_group_own_cmp_dropped", DROPS_ITS_BINDING);
    let held = stats_for("canvas_group_own_cmp_control", KEEPS_ITS_BINDING);
    assert_eq!(
        glyph_count(&owned),
        glyph_count(&held),
        "same scene, same glyphs, whoever owns the font\nowned: {owned}\nheld:  {held}",
    );
}

/// Rows 1 and 2 of Phase 3's matrix in one program: a group owning an image,
/// `removeGroup` while a frame is still drawing, then a completed frame.
///
/// `MFB_CANVAS_FRAME_HOLD_MS` slows the graphics thread and the worker's own
/// `os::sleep(120)` is the other half — without it the worker wins the race to the start
/// of the frame and the group is removed before it is ever resolved, which tests the
/// absent-name path instead (plan-116-G Phase 5 measured exactly that on its first run).
///
/// The program prints its verdict at two points, so one run covers both rows:
/// `OPEN-MID-FRAME` while the in-flight frame is still going, and `CLOSED-AFTER` once a
/// frame has completed past the retirement. **"Exactly once" is not asserted by counting
/// closes** — there is nothing to count — but by the state being `open` at the first
/// point and `closed` at the second, which a close that fired early or never would break.
const REMOVE_MID_FRAME: &str = r#"IMPORT app
IMPORT canvas
IMPORT io
IMPORT os

SUB install(RES img AS canvas::Image)
  LET p AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 80.0, h := 80.0, image := img, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::setGroup("panel", [p])
END SUB

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  LET px AS List OF Byte = [toByte(1), toByte(2), toByte(3), toByte(4)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  install(img)

  LET node AS canvas::DrawItem = canvas::Group[dx := 200.0, dy := 200.0, name := "panel"]
  LET mark AS canvas::DrawItem = canvas::Circle[x := 700.0, y := 500.0, radius := 30.0, paint := canvas::fill(canvas::rgb(0, 255, 0))]
  canvas::present([mark, node])

  os::sleep(120)
  canvas::removeGroup("panel")

  ' Still mid-frame: the hold is 600ms and only 120 have passed.
  LET during AS canvas::Size = canvas::getSize(img) TRAP(e)
    io::print("CLOSED-MID-FRAME")
    RETURN 2
  END TRAP
  io::print("OPEN-MID-FRAME")

  os::sleep(1200)
  ' The reclaim runs at the top of `present`, so a present is what drains it.
  canvas::present([mark])
  canvas::present([mark, canvas::Rectangle[x := 0.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])

  LET after AS canvas::Size = canvas::getSize(img) TRAP(e)
    io::print("CLOSED-AFTER")
    RETURN 0
  END TRAP
  io::print("STILL-OPEN-AFTER-REMOVE")
  RETURN 3
END FUNC
"#;

/// Row 5: 200 install/remove cycles of a group owning an `Image` leave `groupBytes=` at
/// its baseline.
///
/// The `Font` half of this row is what would show an fd leak; an image holds no
/// descriptor, because `canvas::createImage` allocates nothing outside MFB's own resource
/// record (**J11**). So this asserts the arena bytes, which is the leak that *is*
/// observable today, and the fd question is recorded in the plan rather than answered by
/// a green run that could not have failed.
const CHURN_200: &str = r#"IMPORT app
IMPORT canvas
IMPORT io

SUB cycle(n AS Integer)
  LET px AS List OF Byte = [toByte(1), toByte(2), toByte(3), toByte(4)]
  RES img AS canvas::Image = canvas::createImage(1, 1, px)
  LET p AS canvas::DrawItem = canvas::Picture[x := 0.0, y := 0.0, w := 8.0, h := 8.0, image := img, paint := canvas::fill(canvas::rgb(255, 255, 255))]
  canvas::setGroup("churn", [p])
END SUB

FUNC main AS Integer
  app::setMode(app::Mode.Canvas)
  MUT i AS Integer = 0
  WHILE i < 200
    cycle(i)
    canvas::removeGroup("churn")
    canvas::present([canvas::Rectangle[x := 0.0, y := toFloat(i - 100 * (i / 100)), w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(1, 1, 1))]])
    i = i + 1
  END WHILE
  ' Two more presents so the last cycle's retirement drains too.
  canvas::present([canvas::Rectangle[x := 8.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(2, 2, 2))]])
  canvas::present([canvas::Rectangle[x := 16.0, y := 0.0, w := 4.0, h := 4.0, paint := canvas::fill(canvas::rgb(3, 3, 3))]])
  io::print("CHURN-DONE")
  RETURN 0
END FUNC
"#;

/// Rows 1 and 2: the in-flight frame keeps its image, and a completed frame closes it.
#[test]
fn removing_a_group_mid_frame_keeps_the_image_until_the_frame_completes() {
    let project = common::temp_project("canvas_group_own_race", REMOVE_MID_FRAME);
    let binary = common::build_app(&project, "canvas_group_own_race");
    let run = Command::new(&binary)
        .current_dir(&project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        // Slows the GRAPHICS thread. The worker's own os::sleep(120) is the other half:
        // without it the worker reaches removeGroup before the frame starts.
        .env("MFB_CANVAS_FRAME_HOLD_MS", "600")
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", binary.display()));
    let out = String::from_utf8_lossy(&run.stdout).into_owned();
    let _ = std::fs::remove_dir_all(&project);
    assert!(
        run.status.success(),
        "program {}:\n{out}",
        common::exit_description(&run.status),
    );
    assert!(
        out.contains("OPEN-MID-FRAME"),
        "the frame that was already drawing when `removeGroup` arrived must still have \
         its image — closing here is the use-after-close the drain gate exists to \
         prevent\n{out}",
    );
    assert!(
        out.contains("CLOSED-AFTER"),
        "once a frame has completed past the retirement the group must close the image \
         it owned; `STILL-OPEN-AFTER-REMOVE` means nothing will ever close it\n{out}",
    );
}

/// Row 5: 200 install/remove cycles do not grow `groupBytes=`.
#[test]
fn two_hundred_owning_cycles_return_group_bytes_to_baseline() {
    let (out, stats) = run_for("canvas_group_own_churn", CHURN_200, false);
    assert!(
        out.contains("CHURN-DONE"),
        "the churn loop did not finish\n{out}"
    );
    let first = &stats[0];
    let last = stats.last().expect("at least one present");
    let baseline = field(first, "groupBytes");
    let end = field(last, "groupBytes");
    assert!(
        end <= baseline.max(1) * 2,
        "groupBytes grew across 200 install/remove cycles, so the table is holding \
         buffers it retired\nfirst: {first}\nlast:  {last}",
    );
}

/// The fixture directory this file assumes exists, so a rename of `common`'s helper is
/// a compile error here rather than a runtime "font failed to load" that would look
/// like an ownership bug.
#[test]
fn the_font_fixture_is_a_real_truetype() {
    let bytes = common::fixture_truetype();
    assert!(bytes.len() > 100, "fixture is {} bytes", bytes.len());
    assert_eq!(
        &bytes[0..4],
        &[0x00, 0x01, 0x00, 0x00],
        "TrueType outlines start with the 0x00010000 version tag; a fixture that fails \
         to parse would make every test in this file report zero glyphs and read as an \
         ownership failure",
    );
    let _ = Path::new(".");
}
