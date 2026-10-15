//! Focused codegen contracts for `term::measureText`.

use crate::codegen::error::constants::{UNICODE_PROPERTIES_SYMBOL, UNICODE_STAGE1_SYMBOL};
use crate::target::NativeBuildMode::Console;
use crate::testutil::{app_code_cached, code_for_src_cached, CodeTarget};

const APP_PROGRAM: &str = "\
IMPORT app
IMPORT term

FUNC main() AS Integer
  app::setMode(app::Mode.Console)
  term::on()
  LET width AS Integer = term::measureText(\"e\\u{301}\")
  term::off()
  RETURN width
END FUNC
";

const CONSOLE_PROGRAM: &str = "\
IMPORT term

FUNC main() AS Integer
  term::on()
  LET width AS Integer = term::measureText(\"e\\u{301}\")
  term::off()
  RETURN width
END FUNC
";

fn measure_function(
    plan: &crate::codegen::engine::types::NativeCodePlan,
) -> &crate::codegen::engine::types::CodeFunction {
    plan.functions
        .iter()
        .find(|function| function.name == "runtime.term.measureText")
        .expect("a term::measureText call must emit its runtime helper")
}

#[test]
fn term_measure_text_public_call_retains_unicode_tables_even_for_a_literal() {
    let plan = code_for_src_cached(CONSOLE_PROGRAM, CodeTarget::LinuxX86_64, Console);
    let symbols: Vec<_> = plan
        .data_objects
        .iter()
        .map(|object| object.symbol.as_str())
        .collect();
    for required in [UNICODE_STAGE1_SYMBOL, UNICODE_PROPERTIES_SYMBOL] {
        assert!(
            symbols.contains(&required),
            "public measurement must execute its runtime gates and Unicode walk; missing {required}"
        );
    }
    let _ = measure_function(plan);
}

#[test]
fn term_measure_text_checks_wrong_mode_before_inactive_term_state() {
    let plan = app_code_cached(APP_PROGRAM, CodeTarget::LinuxX86_64);
    let function = measure_function(plan);
    let labels: Vec<(usize, String)> = function
        .instructions
        .iter()
        .enumerate()
        .filter_map(|(index, instruction)| instruction.get("name").map(|name| (index, name)))
        .collect();
    let mode_ok = labels
        .iter()
        .find(|(_, name)| name.ends_with("_mode_ok"))
        .map(|(index, _)| *index)
        .expect("the app helper must contain the Console-mode gate");
    let active = labels
        .iter()
        .find(|(_, name)| name.contains("term_measure_text_active"))
        .map(|(index, _)| *index)
        .expect("the helper must contain the active-TUI gate");
    assert!(
        mode_ok < active,
        "ErrWrongMode must be decided before the active-state ErrUnsupported gate; labels: {labels:?}"
    );
}
