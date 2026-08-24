//! `fs::writeAllBytes` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::writeAllBytes` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_write_all_bytes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_read_write::lower_fs_write_all_bytes_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Write a `List OF Byte` to an open `File`"#;
const DESC: &str = r#"`fs::writeAllBytes` writes every byte from `bytes` to `file`, starting at the
file's current write position, and returns nothing. The bytes are taken directly
from the byte list's packed data region exactly as held, with no encoding,
decoding, or newline translation, so the function is suitable for binary data as
well as text. An empty byte list writes no bytes and leaves the file unchanged.

The write is retried until every byte has been written or the host reports an
output failure, so a short host write that transfers only part of the buffer is
resumed from the same cursor rather than treated as complete. The file position
advances by the number of bytes written, so consecutive calls write one after
another within the open handle, and a following `fs::writeAllBytes` or
`fs::writeAll` continues from where this call left off.

`file` must be an open `File` resource — such as one returned by `fs::openFile`
or `fs::open` — opened in a mode that permits writing (`"write"`, `"readWrite"`,
or `"append"`). If the handle was previously read with `fs::readLine`, its
buffered read-ahead is first reconciled so the write lands at the true
file-descriptor position rather than the block read-ahead. When per-`File` write
buffering is enabled, the bytes are appended into the handle's buffer instead of
being written straight through; otherwise they go directly to the descriptor. The
function only writes to and repositions `file`; it does not close it and has no
other side effects. Whether the data is forced to disk is governed by the open
handle, not by this call, which does not flush on its own. To write a whole file
by path in a single call rather than through an open handle, use `fs::writeBytes`.

Thread cancellation is cooperative: the runtime does not asynchronously interrupt
a blocking host file write, so a worker that needs prompt cancellation around a
blocking descriptor should check `thread::isCancelled` between operations."#;
const EX: &str = r#"Write raw bytes to an open file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("target/output.bin", "write")
  LET bytes AS List OF Byte = [72, 105]
  fs::writeAllBytes(f, bytes)
  ' f is closed by lexical drop when this scope ends
END SUB
```

Copy the bytes of one open file into another:

```
IMPORT fs

SUB main()
  RES src = fs::openFile("data.bin")
  RES dst = fs::openFile("copy.bin", "write")
  LET bytes AS List OF Byte = fs::readAllBytes(src)
  fs::writeAllBytes(dst, bytes)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeAllBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "file",
                    desc: r#"An open `File` resource to write to, positioned at the point where the bytes should be written. Must not have been closed and must have been opened in a mode that permits writing (`"write"`, `"readWrite"`, or `"append"`)."#,
                    aliases: &[],
                    ty: ParameterType::named(super::FILE_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "bytes",
                    desc: "The bytes to write, in order, taken verbatim from the list's data \
                           region. An empty list writes nothing.",
                    aliases: &["value"],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_write_all_bytes),
        }],
    });
}
