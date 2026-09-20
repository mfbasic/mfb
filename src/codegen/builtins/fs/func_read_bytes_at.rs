//! `fs::readBytesAt` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::readBytesAt` — calls its per-member `lower_fs_*_helper` emitter and
/// finalizes (crypto/io's clean-room shape).
pub(crate) fn lower_fs_read_bytes_at(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_read_write::lower_fs_read_bytes_at_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Read bytes from a chosen position in an open `File`"#;
const DESC: &str = r#"`fs::readBytesAt` returns up to `count` bytes taken from `file` starting at
`offset`, counted from the start of the file, as a `List OF Byte`. It is the way
to read one piece of a large file without reading the file into memory: only the
bytes asked for are held. `file` must be an open `File` resource opened in a mode
that permits reading.

The read position is left exactly where it was found. `fs::readBytesAt` records
the current position, moves to `offset`, reads, and moves back — on every path,
including the paths that report an error. A `fs::readLine` or `fs::readAllBytes`
before and after a `fs::readBytesAt` call therefore returns the same data it would
have returned without it, which is what makes this call safe on a `File` that
other code is reading.

Fewer than `count` bytes come back only when the file ends first: the amount to
read is measured against the file's length before any byte is read, so a shorter
result always means end of file and never a partial transfer. An `offset` at or
past the end of the file returns an empty `List OF Byte` rather than raising. A
negative `offset` or `count` raises `ErrInvalidArgument`.

Reading at a position requires a handle the host can reposition, so this only
works on a seekable handle — a regular file on disk. On a pipe, a socket, or
another non-seekable handle the call raises `ErrReadFailed`. Calling it on a
`File` that has already been closed raises `ErrResourceClosed`. No decoding or
UTF-8 validation is performed, so the bytes come back exactly as stored, suitable
for binary data. To learn how many bytes a file holds before reading, use
`fs::size`. To read a whole file by path in one call, use `fs::readBytes`."#;
const EX: &str = r#"Read ten bytes from the middle of a file without reading the rest:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeText("data.bin", "abcdefghijklmnopqrstuvwxyz")
  RES f = fs::openFile("data.bin")
  LET chunk AS List OF Byte = fs::readBytesAt(f, 10, 5)
  io::print(toString(len(chunk)))
  ' f closes itself when this scope ends
END SUB
```

Read the last part of a file, letting the end of the file decide how much comes
back:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeText("data.bin", "abcdefghijklmnopqrstuvwxyz")
  RES f = fs::openFile("data.bin")
  LET total AS Integer = fs::size(f)
  ' Asks for 100 bytes but only 6 remain, so 6 come back.
  LET tail AS List OF Byte = fs::readBytesAt(f, total - 6, 100)
  io::print(toString(len(tail)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readBytesAt",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File, Integer, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "file",
                    desc: "An open, seekable `File` resource to read from. Must not have been \
                           closed and must have been opened in a mode that permits reading.",
                    aliases: &[],
                    ty: ParameterType::named(super::FILE_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "offset",
                    desc: "Where to start reading, in bytes from the start of the file. Must not \
                           be negative. An offset at or past the end of the file yields an empty \
                           list.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "count",
                    desc: "How many bytes to read. Must not be negative. Fewer bytes are returned \
                           only when the file ends first.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::abi_function(lower_fs_read_bytes_at),
        }],
    });
}
