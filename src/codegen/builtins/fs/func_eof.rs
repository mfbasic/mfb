//! `fs::eof` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::eof` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_eof(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_read_write::lower_fs_eof_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Test whether an open `File` is at end of input"#;
const DESC: &str = r#"`fs::eof` reports whether `file`'s current read position has reached the end of
its contents. It returns `TRUE` when the position is at or beyond the last byte
and `FALSE` while one or more bytes remain to be read. `file` must be an open
`File` resource, such as one returned by `fs::openFile` or `fs::open`.

The test is buffer-aware (plan-14-C): if the transparent per-handle read buffer
still holds unconsumed bytes (its read cursor is before its fill mark), `fs::eof`
returns `FALSE` immediately without querying the host. Otherwise it asks the host
for the file's current position and total length and compares them — the position
is captured, the handle is seeked to end to read the length, then seeked back to
the captured position, so the read position is left exactly where it was. The
function reads no contents and has no side effects: it does not advance the
position, write anything, or close `file`.

Because determining the length requires seeking, `fs::eof` only works on a
seekable handle — a regular file on disk. On a pipe, a socket, or another
non-seekable handle the host cannot report a position or length, and the call
raises an error instead of returning a `Boolean`.

Use `fs::eof` to guard a read loop so that `fs::readLine` and the other reading
functions are only called while input remains. This is the intended pattern
because end of input is reported by those functions as an error rather than as an
empty result: testing `fs::eof` first lets a loop stop cleanly at the end of the
file."#;
const EX: &str = r#"Read every line until end of input:

```
IMPORT fs
IMPORT io

SUB main()
  RES f = fs::openFile("data.txt")
  WHILE NOT fs::eof(f)
    io::print(fs::readLine(f))
  END WHILE
  ' f is closed by lexical drop when this scope ends
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "eof",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open, seekable `File` resource to test, as returned by `fs::open`, \
                       `fs::openFile`, `fs::openFileNoFollow`, or `fs::createTempFile`. Must not \
                       have been closed.",
                aliases: &[],
                ty: ParameterType::named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_fs_eof),
        }],
    });
}
