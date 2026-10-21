//! The capability check looks inside every statement shape for runtime calls.
//!
//! A backend that does not implement a runtime call must refuse the program, and
//! a call hidden in a `DO ... LOOP UNTIL` body or condition, or in the code of
//! `os::exit`, is as much a call as one at the top of `main`. A walker that skips a
//! statement shape lets such a program through to a lowering that aborts.

use super::*;
use crate::target::BackendCapabilities;
use crate::testutil::{nir_for_src, CodeTarget};

const NOTHING_IMPLEMENTED: BackendCapabilities = BackendCapabilities {
    executable: true,
    native_ir: true,
    native_plan: true,
    native_object_plan: true,
    native_code_plan: true,
    runtime_calls: &[],
};

fn refusal_for(statements: &str) -> String {
    let source =
        format!("IMPORT io\nIMPORT os\n\nFUNC main AS Integer\n{statements}  RETURN 0\nEND FUNC\n");
    let module = nir_for_src(
        &source,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers to NIR");
    validate_capabilities(&module, &NOTHING_IMPLEMENTED)
        .err()
        .expect("a backend that implements nothing refuses the program")
}

#[test]
fn a_runtime_call_in_a_do_until_body_is_found() {
    assert_eq!(
        refusal_for("  DO\n    io::print(\"x\")\n  LOOP UNTIL TRUE\n"),
        "native backend does not support runtime call 'io.print'"
    );
}

#[test]
fn a_runtime_call_in_a_do_until_condition_is_found() {
    assert_eq!(
        refusal_for("  DO\n  LOOP UNTIL io::isOutputTerminal()\n"),
        "native backend does not support runtime call 'io.isOutputTerminal'"
    );
}

#[test]
fn a_runtime_call_in_the_exit_code_of_os_exit_is_found() {
    assert_eq!(
        refusal_for("  os::exit(len(io::readLine()))\n"),
        "native backend does not support runtime call 'io.readLine'"
    );
}
