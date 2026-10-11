//! `canvas::AutoPaint` (plan-172-K): a shape with up to four alternative paints that the
//! graphics thread chooses between from the pointer.
//!
//! Everything here is read off the software frame a `--debug` build dumps
//! (`MFB_CANVAS_DUMP`).

#[path = "../common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

/// A built `--debug` app and the project it lives in.
struct Built {
    name: &'static str,
    project: PathBuf,
    binary: PathBuf,
}

impl Drop for Built {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.project);
    }
}

/// What one run left behind.
struct Ran {
    /// The last frame drawn, as RGBA rows.
    frame: Vec<u8>,
    stderr: String,
}

fn build(name: &'static str, source: &str) -> Built {
    let project = common::temp_project(name, source);
    std::fs::write(project.join("fixture.ttf"), common::fixture_truetype())
        .expect("write the font fixture");
    let binary = common::build_app_debug(&project, name);
    Built {
        name,
        project,
        binary,
    }
}

/// Run headless with `envs` added. `sync` makes each `present` wait for its frame.
fn run(built: &Built, sync: bool, envs: &[(&str, &str)]) -> Ran {
    let frame_path = built.project.join("frame.rgba");
    let _ = std::fs::remove_file(&frame_path);
    let mut command = Command::new(&built.binary);
    command
        .current_dir(&built.project)
        .env("MFB_MACAPP_HEADLESS", "1")
        .env("MFB_WINAPP_HEADLESS", "1")
        .env("MFB_GTKAPP_HEADLESS", "1")
        .env("MFB_CANVAS_DUMP", &frame_path);
    if sync {
        command.env("MFB_CANVAS_SYNC", "1");
    }
    for (key, value) in envs {
        command.env(key, value);
    }
    let output = command
        .output()
        .unwrap_or_else(|e| panic!("run {}: {e}", built.binary.display()));
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        output.status.success(),
        "{} {}:\n{}\n{stderr}",
        built.name,
        common::exit_description(&output.status),
        String::from_utf8_lossy(&output.stdout),
    );
    Ran {
        frame: read_or_empty(&frame_path),
        stderr,
    }
}

fn read_or_empty(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap_or_default()
}

/// The three shapes every Phase 1 row draws: a rounded rectangle with a stroke, a
/// circle, and a `Text` — the kind whose `AutoPaint` arms are their own (it names a
/// font). `wrap` chooses whether each is presented bare or inside an `AutoPaint` whose
/// four states are all in use, so a state paint reaching the frame would show.
const SHAPES: &str = r#"FUNC auto(s AS canvas::Shape) AS canvas::DrawItem
  LET red AS canvas::Paint = canvas::fill(color::rgb(255, 0, 0))
  LET a AS canvas::AutoPaint = canvas::AutoPaint[shape := s, useEnter := TRUE, useExit := TRUE, useHover := TRUE, usePressed := TRUE, enterPaint := red, exitPaint := red, hoverPaint := red, pressedPaint := red]
  LET item AS canvas::DrawItem = a
  RETURN item
END FUNC

FUNC shapes(face AS canvas::Font, wrap AS Boolean, shift AS Float) AS List OF canvas::DrawItem
  MUT r AS canvas::Rectangle
  r = WITH r { id := "box", position := vector::Float2[x := 40.0 + shift, y := 40.0], size := canvas::Size[width := 200.0, height := 80.0], radiusTL := 12.0, radiusTR := 12.0, radiusBL := 12.0, radiusBR := 12.0, paint := canvas::fillStroke(color::rgb(40, 90, 200), color::rgb(255, 255, 255), 3.0) }
  MUT c AS canvas::Circle
  c = WITH c { id := "dot", position := vector::Float2[x := 400.0, y := 90.0], radius := 45.0, paint := canvas::fill(color::rgb(30, 160, 90)) }
  LET t AS canvas::Text = canvas::Text[id := "label", position := vector::Float2[x := 60.0, y := 240.0], text := "AA", font := face, fontSize := 60.0, paint := canvas::fill(color::rgb(240, 240, 120)), transform := canvas::Transform[0.0, 0.0, 0.0, 0.0, 0.0, 0.0]]
  IF wrap THEN
    RETURN [auto(r), auto(c), auto(t)]
  END IF
  LET ri AS canvas::DrawItem = r
  LET ci AS canvas::DrawItem = c
  LET ti AS canvas::DrawItem = t
  RETURN [ri, ci, ti]
END FUNC
"#;

const HEADER: &str = r#"IMPORT app
IMPORT canvas
IMPORT color
IMPORT io
IMPORT os
IMPORT vector

"#;

fn program(main: &str) -> String {
    format!("{HEADER}{SHAPES}\n{main}")
}

/// With no pointer on the surface an `AutoPaint` is in its default state, and its
/// default state is its shape: the frame is the bare shapes' frame, byte for byte.
#[test]
fn an_autopaint_with_no_pointer_draws_exactly_its_bare_shape() {
    let built = build(
        "canvas_autopaint_bare",
        &program(
            r#"FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("no font")
    RETURN
  END TRAP
  canvas::present(shapes(face, os::getEnvOr("WRAP", "") = "1", 0.0))
END FUNC
"#,
        ),
    );
    let bare = run(&built, true, &[]);
    let wrapped = run(&built, true, &[("WRAP", "1")]);
    assert!(!bare.frame.is_empty(), "the bare scene drew no frame");
    assert!(
        bare.frame.chunks(4).any(|p| p[0] == 40 && p[2] == 200),
        "the bare scene's rectangle is not in its frame"
    );
    assert!(
        bare.frame == wrapped.frame,
        "an AutoPaint with no pointer drew a different frame from its bare shape"
    );
}

/// The same inside a group: the group's walk (`__canvas_appendDraw`'s group arm, and the
/// GPU walk's `__canvas_memoGroup`) reaches an `AutoPaint` child and draws its shape.
#[test]
fn an_autopaint_inside_a_group_draws_exactly_its_bare_shape() {
    let built = build(
        "canvas_autopaint_group",
        &program(
            r#"FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("no font")
    RETURN
  END TRAP
  canvas::setGroup("panel", shapes(face, os::getEnvOr("WRAP", "") = "1", 0.0))
  MUT g AS canvas::Group
  g = WITH g { id := "p", position := vector::Float2[x := 30.0, y := 20.0], name := "panel" }
  LET node AS canvas::DrawItem = g
  canvas::present([node])
END FUNC
"#,
        ),
    );
    let bare = run(&built, true, &[]);
    let wrapped = run(&built, true, &[("WRAP", "1")]);
    assert!(
        bare.frame.chunks(4).any(|p| p[0] == 40 && p[2] == 200),
        "the bare group's rectangle is not in its frame"
    );
    assert!(
        bare.frame == wrapped.frame,
        "an AutoPaint in a group drew a different frame from its bare shape"
    );
}

/// `arena.0.live_bytes` at exit, with `{N}` present / drop cycles.
fn live_bytes(built_name: &'static str, source: &str, cycles: usize) -> u64 {
    let built = build(built_name, &source.replace("{N}", &cycles.to_string()));
    let ran = run(&built, true, &[]);
    let lines = common::debug_report::arena_lines(built_name, &ran.stderr);
    common::debug_report::counter(built_name, &lines, 0, "live_bytes")
}

/// Presenting a scene of `AutoPaint`s and dropping it again reaches a steady state: the
/// worker's arena holds the same bytes after 200 cycles as after 100.
///
/// Each cycle's scene differs from the last (the rectangle moves), so each is a real
/// publish, a real retirement, and — for the `Text` — a font the retired scene named
/// and the next still does. Under `MFB_CANVAS_SYNC` retirement drains on every present,
/// so growth here is a leak and not a backlog (`rt_canvas_present_leak.rs`).
#[test]
fn presenting_and_dropping_autopaints_holds_steady_memory() {
    let source = program(
        r#"FUNC main() AS Nothing
  app::setMode(app::Mode.Canvas)
  RES face AS canvas::Font = canvas::loadFont("fixture.ttf") TRAP(e)
    io::print("no font")
    RETURN
  END TRAP
  MUT i AS Integer = 0
  WHILE i < {N}
    canvas::present(shapes(face, TRUE, toFloat(i MOD 7)))
    canvas::present([])
    i = i + 1
  END WHILE
END FUNC
"#,
    );
    let few = live_bytes("canvas_autopaint_leak_few", &source, 100);
    let many = live_bytes("canvas_autopaint_leak_many", &source, 200);
    let grew = many.saturating_sub(few);
    assert!(
        grew <= 48 * 1024 + 64 * 100,
        "the arena held {few} live bytes after 100 present/drop cycles and {many} after \
         200 — {grew} bytes of growth, {} per cycle",
        grew / 100,
    );
}
