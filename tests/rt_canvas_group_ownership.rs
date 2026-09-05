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
