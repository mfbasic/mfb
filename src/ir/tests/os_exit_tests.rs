//! `os::exit` lowering (plan-180-A): a bare statement is the program-exit op
//! (`IrOp::ExitProgram`); every other position is an ordinary call to the
//! `#os_exit` rewrite helper.

use super::helpers::{function, lower_src};
use super::*;

#[test]
fn os_exit_statement_lowers_to_exit_program() {
    let ir = lower_src("IMPORT os\nFUNC main AS Integer\n  os::exit(3)\nEND FUNC\n");
    let body = &function(&ir, "main").body;
    assert!(
        body.iter().any(|op| matches!(op, IrOp::ExitProgram { .. })),
        "a bare os::exit statement must lower to IrOp::ExitProgram"
    );
}

#[test]
fn os_exit_named_code_statement_lowers_to_exit_program() {
    let ir = lower_src("IMPORT os\nFUNC main AS Integer\n  os::exit(code := 3)\nEND FUNC\n");
    let body = &function(&ir, "main").body;
    assert!(body.iter().any(|op| matches!(op, IrOp::ExitProgram { .. })));
}

#[test]
fn os_exit_in_return_lowers_to_rewrite_call() {
    let ir = lower_src(
        "IMPORT os\nFUNC leave() AS Nothing\n  RETURN os::exit(3)\nEND FUNC\n\
         FUNC main AS Integer\n  leave()\n  RETURN 0\nEND FUNC\n",
    );
    let body = &function(&ir, "leave").body;
    assert!(
        !body.iter().any(|op| matches!(op, IrOp::ExitProgram { .. })),
        "RETURN os::exit(...) is not a statement and must not lower to ExitProgram"
    );
    assert!(
        body.iter().any(|op| match op {
            IrOp::Return {
                value: Some(IrValue::Call { target, .. }),
                ..
            } => target == "#os_exit",
            _ => false,
        }),
        "RETURN os::exit(...) must call the #os_exit rewrite helper"
    );
}

#[test]
fn os_exit_completes_a_function_without_return() {
    let ir = lower_src(
        "IMPORT os\nFUNC bail() AS Integer\n  os::exit(1)\nEND FUNC\n\
         FUNC main AS Integer\n  RETURN bail()\nEND FUNC\n",
    );
    assert!(function(&ir, "bail")
        .body
        .iter()
        .any(|op| matches!(op, IrOp::ExitProgram { .. })));
}
