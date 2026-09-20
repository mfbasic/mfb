//! `fs::size` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::size` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_size(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_read_write::lower_fs_size_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Report the length in bytes of an open `File`"#;
const DESC: &str = r#"`fs::size` returns the total number of bytes `file` holds, counted from the
start of the file to its end — not the number of bytes remaining from the current
read position. `file` must be an open `File` resource, such as one returned by
`fs::openFile` or `fs::open`.

The read position is left exactly where it was found. `fs::size` records the
current position, moves to the end to measure the length, and moves back, so a
`fs::readLine` or `fs::readAllBytes` that follows a `fs::size` call returns the
same data it would have returned without it. That makes `fs::size` safe to call on
a `File` that other code is reading, and it is what lets a program decide how much
of a file to read before reading any of it.

Measuring the length requires a handle the host can reposition, so `fs::size` only
works on a seekable handle — a regular file on disk. On a pipe, a socket, or
another non-seekable handle the host cannot report a length, and the call raises
`ErrReadFailed`; `fs::eof` behaves the same way on the same handles. Calling
`fs::size` on a `File` that has already been closed raises `ErrResourceClosed`.

The size reported is the file's length on disk at the moment of the call. Bytes
written through `file` but still held in its write buffer are not counted until
they reach the disk, so call `fs::flush` first when an accurate length is needed
straight after writing. To read bytes at a chosen position rather than measure the
file, use `fs::readBytesAt`."#;
const EX: &str = r#"Measure an open file before reading it:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeText("data.bin", "Hello")
  RES f = fs::openFile("data.bin")
  io::print(toString(fs::size(f)))
  ' f closes itself when this scope ends
END SUB
```

The read position is unchanged, so a read after the measurement still starts at
the beginning:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeText("data.bin", "first line\nsecond line\n")
  RES f = fs::openFile("data.bin")
  LET total AS Integer = fs::size(f)
  io::print(fs::readLine(f))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "size",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open, seekable `File` resource to measure, as returned by `fs::open` or \
                       `fs::openFile`. Must not have been closed.",
                aliases: &[],
                ty: ParameterType::named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_fs_size),
        }],
    });
}
