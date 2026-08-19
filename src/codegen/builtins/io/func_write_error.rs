//! `io::writeError` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): this member adapter reproduces its former
//! `lower_io_helper` `match` arm and hatches the finalized OS-seam body back.

// --- codegen tier imports (migration) ---
use crate::codegen::builtins::io::native::lower_write_family;
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `abi_function` body for `io::writeError` — write to stderr with no trailing newline.
pub(crate) fn lower_write_error(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_write_family(builder, ctx, true, false, "io.writeError")
}

const INTRO: &str = r#"Write a `String` to standard error with no trailing newline"#;
const DESC: &str = r#"`io::writeError` writes `value` to standard error exactly as stored and adds
nothing. The text is treated as UTF-8 and emitted byte for byte, with no escaping
and no newline translation. An empty `String` writes nothing at all. It is the
newline-free counterpart of `io::printError`.

Only `String` is accepted, and exactly one argument; there is no implicit
conversion, so convert other values first — for example with `toString`.

Standard error is **never buffered**. `io::setBuffered` controls standard output
only, so this call always issues its bytes immediately and there is no flush for
standard error. It is also never retained by `term::` TUI mode — the shadow-grid
routing covers standard output alone.

The underlying write loops until every byte has been transferred: a short write
advances the cursor and re-issues, and an `EINTR` interruption retries with the
cursor unchanged. A zero-byte or failing write raises `ErrOutput`. Output goes to
whatever is bound to standard error: file descriptor 2 in a console program, and
the application transcript in app mode (`mfb build --app`)."#;
const EX: &str = r#"Emit a progress marker on the error stream without breaking the line:

```
IMPORT io

SUB main()
  io::writeError("working")
  io::writeError(".")
  io::printError(" done")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeError",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "value",
                desc:
                    "The text to write. Interpreted as UTF-8 and emitted unchanged; may be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_write_error),
        }],
    });
}
