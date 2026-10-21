//! The app-mode terminal queries and stderr writers.
//!
//! In an `-app` build `term::isOn`, `term::didResize` and `io::printError` /
//! `io::writeError` do not touch a tty: they read and write the toolkit's own
//! state (the grid's active flag, its resize latch) and append a prefixed chunk
//! to the transcript. The corpus lowers console builds only.

use super::app_window::{function, mnemonics, targets};
use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{app_code_cached, CodeTarget};

const QUERIES: &str = "\
IMPORT io
IMPORT term

FUNC main() AS Integer
  term::on()
  LET on AS Boolean = term::isOn()
  LET resized AS Boolean = term::didResize()
  io::printError(\"warn\")
  io::writeError(\"more\")
  term::off()
  io::print(toString(on) & toString(resized))
  RETURN 0
END FUNC
";

fn plan(target: CodeTarget) -> &'static NativeCodePlan {
    app_code_cached(QUERIES, target)
}

fn count(function: &crate::codegen::engine::types::CodeFunction, mnemonic: &str) -> usize {
    mnemonics(function)
        .iter()
        .filter(|found| **found == mnemonic)
        .count()
}

/// `term::isOn` reads the active flag and nothing else; on the GTK backends it
/// reads it from the process-global toolkit state, elsewhere from the grid state.
#[test]
fn term_is_on_reads_the_active_flag_on_every_app_backend() {
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let is_on = function(plan(target), "_mfb_rt_term_term_isOn");
        assert_eq!(
            count(is_on, "str_u64"),
            0,
            "{}: a query stores nothing",
            target.name()
        );
        assert!(count(is_on, "ldr_u64") >= 1, "{}", target.name());
        let reached = targets(is_on);
        let gtk = matches!(target, CodeTarget::LinuxX86_64 | CodeTarget::LinuxAarch64);
        assert_eq!(
            reached.contains(&"_mfb_gtkapp_state"),
            gtk,
            "{}: only the GTK backends read the toolkit state word; reaches {reached:?}",
            target.name()
        );
    }
}

/// `term::didResize` reports the latch and clears it: it must store, so a second
/// call in the same frame does not report the same resize twice.
#[test]
fn term_did_resize_reports_and_clears_the_latch_on_every_app_backend() {
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        let did_resize = function(plan(target), "_mfb_rt_term_term_didResize");
        assert!(
            count(did_resize, "str_u64") >= 1,
            "{}: a latch that is read must also be cleared",
            target.name()
        );
        let reached = targets(did_resize);
        match target {
            CodeTarget::MacosAarch64 => assert!(
                reached.contains(&"_objc_getAssociatedObject"),
                "the grid state hangs off the app by an associated object: {reached:?}"
            ),
            CodeTarget::WindowsX86_64 => assert!(
                reached.contains(&"_mfb_winapp_tui_resizes")
                    && reached.contains(&"_mfb_winapp_tui_resizes_seen"),
                "windows compares a resize counter with the count already seen: {reached:?}"
            ),
            _ => assert!(reached.contains(&"_mfb_gtkapp_state"), "{reached:?}"),
        }
    }
}

/// `io::printError` / `io::writeError` in an app build append to the transcript
/// with a `[stderr] ` prefix, through the toolkit's own append path, and still
/// reach the process's real stderr.
#[test]
fn stderr_writers_append_a_prefixed_chunk_to_the_transcript_on_every_app_backend() {
    for target in CodeTarget::ALL {
        if target.app_mode().is_none() {
            continue;
        }
        for symbol in ["_mfb_rt_io_io_printError", "_mfb_rt_io_io_writeError"] {
            let writer = function(plan(target), symbol);
            let reached = targets(writer);
            let wanted: &[&str] = match target {
                CodeTarget::MacosAarch64 => {
                    &["_mfb_macapp_str_stderr_prefix", "_mfb_macapp_append"]
                }
                CodeTarget::WindowsX86_64 => &["SendMessageW", "WriteFile"],
                _ => &["_mfb_gtkapp_str_stderr_prefix", "memcpy", "g_idle_add"],
            };
            for symbol_wanted in wanted {
                assert!(
                    reached.contains(symbol_wanted),
                    "{} {symbol}: must reach {symbol_wanted}; reaches {reached:?}",
                    target.name()
                );
            }
        }
    }
}
