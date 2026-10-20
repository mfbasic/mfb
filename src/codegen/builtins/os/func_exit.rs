//! `os::exit` — descriptor entry + authored docs, and the `Body::Rewrite` seam
//! for the call positions that are not a plain statement (plan-180-A).
//!
//! A bare `os::exit(code)` statement never reaches this member: IR lowering turns
//! it into `IrOp::ExitProgram` (`src/ir/lower/statement.rs`, the statement-position
//! arm). Every other position (`RETURN
//! os::exit(1)`, a trapped call, a call nested in an expression) is an ordinary
//! call and is rewritten to `__os_exit`, which is the one-statement helper below.
//! Its body is itself a bare statement, so it lowers to the same `ExitProgram`
//! op and exits through the proven result-tag protocol.
//!
//! The helper is a `WhenUsed` chunk so a program that never names `os::exit`
//! keeps its IR byte-identical; a `Body::mfb` body would render into every
//! `IMPORT os` program.

use crate::codegen::registry::{
    Body, DefaultValue, HelperGate, Implementation, Parameter, RegistryFunction, RegistryHelper,
    RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"End the process with an exit code"#;
const DESC: &str = r#"`os::exit(code)` ends the process with status `code` and never returns. It is a
plain builtin call, so it can stand anywhere another `os` call can: as a
statement, as `RETURN os::exit(1)`, or inside a `TRAP` handler.

The exit is not an error. No `TRAP` catches it, including a `TRAP` written
around the `os::exit` call itself, whose handler never runs. Every live scope is
cleaned up from the current function up to the program's entry point before the
process ends. When the call happens on a `thread::` worker, the whole process
ends, not just that worker.

A constant `code` outside `0..255` is a compile error.
A computed value outside that range is passed to the host, which keeps only its
low eight bits.

Because the call never returns, a function whose every path ends in a plain
`os::exit(...)` statement needs no `RETURN`."#;
const EX: &str = r#"Exit with status 3 from a helper, without returning to the caller:

```
IMPORT os
IMPORT io

FUNC fail(reason AS String) AS Nothing
  io::print(reason)
  os::exit(3)
END FUNC

FUNC main AS Integer
  fail("bad configuration")
END FUNC
```"#;

/// The helper chunk injected when a program names `os::exit` in a non-statement
/// position. `os::exit(code)` inside it is a statement, so it lowers to
/// `IrOp::ExitProgram` rather than recursing into the rewrite.
const EXIT_HELPER: &str = r#"REM MFBASIC os::exit call-site companion (plan-180-A). Backs the Body::Rewrite
REM member when os::exit is not a bare statement (RETURN os::exit(...), a trapped
REM call, a call inside an expression).
IMPORT os

FUNC __os_exit(code AS Integer) AS Nothing
  os::exit(code)
END FUNC"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "exit",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "code",
                desc: "the exit status the process ends with",
                aliases: &[],
                ty: ParameterType::Integer,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::Rewrite("__os_exit"),
        }],
    });
    pkg.add_helper(RegistryHelper {
        name: "os",
        gate: HelperGate::WhenUsed(&["exit"]),
        body: Some(EXIT_HELPER),
        import_name: None,
        natively_called: false,
    });
}
