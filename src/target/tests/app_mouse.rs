//! The terminal mouse members in `-app` mode.
//!
//! In an app build the terminal is a drawn grid inside the toolkit's window, so
//! a mouse program needs the toolkit's own pointer callbacks to feed the event
//! ring -- and a program that never touches the mouse must not carry them (each
//! reads the process-global mode word, which exists only for a mouse program).

use crate::testutil::{app_code_cached, CodeTarget};
use std::collections::BTreeSet;

const PLAIN: &str = "\
IMPORT term

FUNC main() AS Integer
  term::on()
  term::sync()
  term::off()
  RETURN 0
END FUNC
";

const MOUSE: &str = "\
IMPORT term

FUNC main() AS Integer
  term::on()
  term::enableMouse(TRUE)
  LET event AS term::MouseEvent = term::pollMouse()
  term::sync()
  term::off()
  RETURN 0
END FUNC
";

fn symbols(source: &str, target: CodeTarget) -> BTreeSet<String> {
    app_code_cached(source, target)
        .functions
        .iter()
        .map(|function| function.symbol.clone())
        .collect()
}

/// The functions a mouse program adds to the same program without one.
fn added_by_the_mouse(target: CodeTarget) -> Vec<String> {
    let plain = symbols(PLAIN, target);
    symbols(MOUSE, target)
        .into_iter()
        .filter(|symbol| !plain.contains(symbol))
        .collect()
}

/// macOS: the grid view's eleven mouse IMPs (down/up/move/drag/other/right/scroll).
#[test]
fn a_macos_mouse_program_installs_the_grid_views_eleven_mouse_imps() {
    let added = added_by_the_mouse(CodeTarget::MacosAarch64);
    for imp in [
        "mouseDown",
        "mouseUp",
        "mouseMoved",
        "mouseDragged",
        "rightMouseDown",
        "rightMouseUp",
        "rightMouseDragged",
        "otherMouseDown",
        "otherMouseUp",
        "otherMouseDragged",
        "scrollWheel",
    ] {
        let symbol = format!("_mfb_macapp_term_{imp}");
        assert!(added.contains(&symbol), "{symbol} is added; got {added:?}");
    }
    assert!(added.contains(&"_mfb_rt_term_term_pollMouse".to_string()));
}

/// Linux: the GTK press/release/motion/scroll callbacks, on both ISAs.
#[test]
fn a_linux_mouse_program_installs_the_gtk_pointer_callbacks() {
    for target in [CodeTarget::LinuxX86_64, CodeTarget::LinuxAarch64] {
        let added = added_by_the_mouse(target);
        for callback in ["pressed", "released", "motion", "scroll"] {
            let symbol = format!("_mfb_gtkapp_mouse_{callback}");
            assert!(
                added.contains(&symbol),
                "{}: {symbol} is added; got {added:?}",
                target.name()
            );
        }
    }
}

/// A program that never touches the mouse carries no pointer callback on any
/// backend: each would name a mode word that program does not have.
#[test]
fn a_program_without_the_mouse_carries_no_pointer_callbacks() {
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let plain = symbols(PLAIN, target);
        assert!(
            !plain
                .iter()
                .any(|symbol| symbol.contains("_mouse") || symbol.contains("Mouse")),
            "{}: a mouse-free program names {:?}",
            target.name(),
            plain
                .iter()
                .filter(|symbol| symbol.contains("ouse"))
                .collect::<Vec<_>>()
        );
    }
}
