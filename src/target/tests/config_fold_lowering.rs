//! `os::getConfigOr` is folded to a constant while the project is lowered, and a
//! call that cannot fold stops the build.

use crate::manifest::config::ConfigValue;
use crate::target::shared::lower::lower_project;
use crate::target::NativeBuildMode;

const SRC: &str = "\
IMPORT io
IMPORT os

FUNC main() AS Integer
  io::print(os::getConfigOr(\"port\", \"80\"))
  RETURN 0
END FUNC
";

/// A default whose type is not the declared type of the config's key is refused
/// by the lowering, naming how many calls failed and which config was selected.
#[test]
fn a_config_default_of_the_wrong_type_stops_the_lowering() {
    let mut ir = crate::testutil::named_ir_for_src(SRC, "cfgprog");
    ir.selected_config.name = "staging".to_string();
    ir.selected_config
        .values
        .insert("port".to_string(), ConfigValue::Integer(8080));
    let error = lower_project(
        &ir,
        "linux-x86_64".to_string(),
        &[],
        NativeBuildMode::Console,
        None,
        crate::codegen::debug::DebugOptions::OFF,
    )
    .err()
    .expect("a mismatched config default is refused");
    assert_eq!(
        error,
        "1 os::getConfigOr call(s) do not match config `staging`"
    );
}
