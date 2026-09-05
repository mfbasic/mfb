//! The `canvas::` and `term::` surfaces, in `-app` mode, on every backend.
//!
//! Both packages branch on the platform family and on the build mode. The
//! corpus lowers its 421 fixtures in CONSOLE mode for one backend, and
//! `canvas.rs` lowers one `-app` program to inspect the scene publish — so the
//! members neither of those touches (`canvas::getSize`, `setBytes`, `getBytes`,
//! `didResize`, `createImage`, and the `term::` shadow-grid helpers) had only
//! their console/Linux arm measured, or none at all.
//!
//! What is asserted is the same property the acceptance matrix spends five
//! runners on: the program lowers, completely, everywhere it is supposed to. An
//! `-app` member that lowers on Linux and not on macOS is a build failure nobody
//! sees until a release runner reaches it.

use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{app_code_cached, code_for_src_cached, fixture_src, CodeTarget};

/// The image surface: create, size, read back, write, and the resize flag.
const CANVAS_SURFACE: &str = "\
IMPORT app
IMPORT canvas
IMPORT collections
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Canvas)
  MUT px AS List OF Byte = []
  FOR i = 0 TO 15
    px = collections::append(px, toByte(i))
  NEXT
  RES img AS canvas::Image = canvas::createImage(2, 2, px) TRAP(e)
    RETURN 1
  END TRAP
  LET size AS canvas::Size = canvas::getSize(img)
  LET back AS List OF Byte = canvas::getBytes(img)
  canvas::setBytes(img, back)
  io::print(toString(len(back)) & toString(canvas::didResize()))
  RETURN 0
END FUNC
";

/// Every `-app`-capable backend lowers the canvas surface completely.
#[test]
fn the_canvas_surface_lowers_on_every_app_capable_backend() {
    let mut agreed: Option<Vec<String>> = None;
    let mut lowered = 0;
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let plan = app_code_cached(CANVAS_SURFACE, target);
        lowered += 1;
        let surface = runtime_members(plan, "runtime.canvas.");
        for member in ["createImage", "imageHandle", "destroyImage"] {
            assert!(
                surface.contains(&format!("runtime.canvas.{member}")),
                "{}: a program that creates an image must emit \
                 `runtime.canvas.{member}`; it emitted {surface:?}",
                target.name()
            );
        }
        match &agreed {
            None => agreed = Some(surface),
            Some(first) => assert_eq!(
                first,
                &surface,
                "{}: the canvas runtime surface differs from the first backend's",
                target.name()
            ),
        }
    }
    assert_eq!(lowered, 4, "four backends have an -app mode");
}

/// A `term::` program lowers in BOTH build modes, on every backend.
///
/// `term::` is where the two modes genuinely differ: in console mode the helpers
/// write escape sequences to a tty, and in `-app` mode they write into the
/// toolkit's shadow grid instead. Only one of those two arms was ever lowered,
/// because every fixture the corpus runs is a console build.
#[test]
fn a_term_program_lowers_in_both_build_modes_on_every_backend() {
    let source = fixture_src("func_term_drawText_valid");
    let mut lowered = 0;
    for target in CodeTarget::ALL {
        let console = code_for_src_cached(&source, target, crate::target::NativeBuildMode::Console);
        assert!(
            !runtime_members(console, "runtime.term.").is_empty(),
            "{}: a term program must emit term runtime members in console mode",
            target.name()
        );
        lowered += 1;
        let Some(_) = target.app_mode() else {
            continue;
        };
        let app = app_code_cached(&source, target);
        assert!(
            app.functions.len() > console.functions.len(),
            "{}: the -app build must emit the toolkit bootstrap on top of the \
             console surface ({} functions against {})",
            target.name(),
            app.functions.len(),
            console.functions.len()
        );
        lowered += 1;
    }
    assert_eq!(
        lowered, 9,
        "five console builds plus four -app builds (rv64 is console-only)"
    );
}

fn runtime_members(plan: &NativeCodePlan, prefix: &str) -> Vec<String> {
    let mut names: Vec<String> = plan
        .functions
        .iter()
        .map(|f| f.name.clone())
        .filter(|name| name.starts_with(prefix))
        .collect();
    names.sort();
    names
}
