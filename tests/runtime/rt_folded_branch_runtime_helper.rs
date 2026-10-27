//! A runtime helper whose only call a constant branch fold removed is no longer
//! declared. `IF FALSE` around a program's only `io::` call used to fail every
//! optimized build with "NIR declares unused runtime helper 'io'": call-graph DCE
//! re-derived the declared helpers only when it removed a function, and here it
//! removes none — branch simplification dropped the call.

#[path = "../common/mod.rs"]
mod common;
use common::*;

const PROGRAM: &str = "IMPORT io\n\nFUNC main() AS Integer\n  IF FALSE THEN\n    \
io::print(\"x\")\n  END IF\n  RETURN 0\nEND FUNC\n";

#[test]
fn a_helper_only_a_folded_branch_called_is_not_declared() {
    for level in ["0", "1", "2", "3"] {
        let project = temp_project(&format!("folded_branch_helper_o{level}"), PROGRAM);
        let exe = build_project_opt(&project, level);
        let status = std::process::Command::new(&exe)
            .status()
            .expect("run the built program");
        assert!(status.success(), "-O{level}: the program must run and exit 0");
    }
}
