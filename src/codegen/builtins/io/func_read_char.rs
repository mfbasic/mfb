//! `io::readChar` — descriptor entry + authored docs.
//!
//! Per-member file. `io` lowers through per-function `Body::abi_function`
//! clean-room lowerings (plan-101): this member adapter reproduces its former
//! `lower_io_helper` `match` arm and hatches the finalized OS-seam body back.

use crate::codegen::builtins::io::native::{
    adapter_app_mode, hatch_finalized, lower_io_read_char_helper,
};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{AbiCtx, Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

/// `abi_function` body for `io::readChar` — read one whole Unicode scalar value
/// (one UTF-8 sequence) from stdin, returned as a one-character `String`.
pub(crate) fn lower_read_char(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let body = lower_io_read_char_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        adapter_app_mode(ctx),
    )?;
    hatch_finalized(builder, body, "String", "io.readChar")
}

const INTRO: &str = r#"Read one whole Unicode scalar value from standard input"#;
const DESC: &str = r#"`io::readChar` reads exactly one Unicode scalar value from standard input and
returns it as a one-character `String`. It reads the lead byte, derives the
sequence length from it, and reads the one to three continuation bytes that
complete the scalar. It takes no arguments and does not wait for a newline.

**On a terminal the read is a single keypress.** For the duration of the call,
standard input is switched out of canonical mode and echo is suppressed
(`~ICANON`, `~ECHO`, `VMIN = 1`, `VTIME = 0`), so one key satisfies the read with
no Return and nothing is displayed; the previous line discipline is restored
before the call returns. When standard input is not a terminal the stream is read
as is with no mode change.

Before blocking, any pending standard-output buffer is drained, so a prompt
written with `io::write` appears before the program waits. Decoding is strict
UTF-8, not lenient: an ill-formed sequence raises `ErrEncoding` rather than
yielding a replacement character, and so does a sequence cut short by end of
input. This returns one *scalar value*, not one user-perceived character: a
grapheme cluster made of several scalars takes that many calls. Compare
`io::readByte`, which returns raw bytes with no decoding at all.

End of input is reported as an error, not as an empty result. Use `io::pollInput`
to test for readiness when the program must not block. Standard input is a
per-thread broadcast log; a thread other than the main thread must subscribe with
`thread::openStdIn` before reading, or the call raises `ErrInvalidContext`."#;
const EX: &str = r#"Wait for any keypress to continue:

```
IMPORT io

SUB main()
  io::write("Press any key to continue...")
  LET ignored AS String = io::readChar()
  io::print("")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readChar",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_read_char),
        }],
    });
}
