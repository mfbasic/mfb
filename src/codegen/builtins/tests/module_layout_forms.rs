//! Module-layout decisions that depend on what a program uses rather than on
//! what it calls by name.
//!
//! * A program that uses `term::` gets the `term::off` and `term::sync` helpers
//!   even if it never calls them: the exit path restores the terminal through
//!   `off` (§6.5), and `off` presents the final frame through `sync`. The native
//!   plan adds both for any `term::` helper (`target/shared/plan/symbols.rs`), and
//!   the module layout relies on that.
//! * An `--app` program that uses a window member (`app::setTitle`, …) gets the
//!   window's process-global state as data objects — the title, its lock, the
//!   fullscreen word — and a program that does not, does not
//!   (`engine/builder/module_layout.rs`).

use crate::target::NativeBuildMode::Console;
use crate::testutil::{app_code_cached, code_for_src_cached, CodeTarget};

#[test]
fn a_term_program_gets_off_and_sync_without_calling_them() {
    let source = "\
IMPORT term

FUNC main() AS Integer
  term::on()
  term::clear()
  RETURN 0
END FUNC
";
    for target in CodeTarget::ALL {
        let plan = code_for_src_cached(source, target, Console);
        for helper in ["_mfb_rt_term_term_off", "_mfb_rt_term_term_sync"] {
            assert!(
                plan.functions.iter().any(|f| f.symbol == helper),
                "{}: no {helper}",
                target.name()
            );
        }
    }
}

fn data_symbols(source: &str, target: CodeTarget) -> Vec<String> {
    app_code_cached(source, target)
        .data_objects
        .iter()
        .map(|object| object.symbol.clone())
        .collect()
}

#[test]
fn an_app_program_using_a_window_member_carries_the_window_state() {
    let window = "\
IMPORT app
IMPORT io

FUNC main() AS Integer
  app::setTitle(\"demo\")
  io::print(app::getTitle())
  RETURN 0
END FUNC
";
    let plain = "\
IMPORT app
IMPORT io

FUNC main() AS Integer
  app::setMode(app::Mode.Console)
  io::print(\"x\")
  RETURN 0
END FUNC
";
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let with_window = data_symbols(window, target);
        let without = data_symbols(plain, target);
        for symbol in ["_mfb_rt_app_title", "_mfb_rt_app_fullscreen"] {
            assert!(
                with_window.iter().any(|s| s == symbol),
                "{}: no {symbol}",
                target.name()
            );
            assert!(
                !without.iter().any(|s| s == symbol),
                "{}: {symbol} without a window member",
                target.name()
            );
        }
    }
}
