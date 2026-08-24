//! `fs::readAllBytes` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::readAllBytes` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_read_all_bytes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_read_write::lower_fs_read_all_bytes_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Read all remaining bytes from an open `File` into a `List OF Byte`"#;
const DESC: &str = r#"`fs::readAllBytes` reads every remaining byte of `file`, starting at the file's
current read position and continuing to end of input, and returns them as a single
`List OF Byte`. The read position is advanced to end of input, so a subsequent
`fs::eof` reports true. `file` must be an open `File` resource — such as one
returned by `fs::openFile` or `fs::open` — opened in a mode that permits reading.

The amount to read is measured up front: the function seeks to record the current
position, seeks to the end to find the file's length, seeks back to the start
position, allocates a `List OF Byte` of exactly that length, and reads the
remainder into it in one or more host reads until the collection is full. No
newline translation, decoding, or UTF-8 validation is performed, so the returned
list holds the file's remaining bytes exactly as stored on disk, making it suitable
for binary data as well as text. When `file` is already at end of input, no bytes
remain and the empty `List OF Byte` is returned.

If the file was previously read with `fs::readLine`, the buffered read-ahead is
first reconciled so the measurement and read see the true file-descriptor position
rather than the block read-ahead. The function only reads from and repositions
`file`; it does not close it and has no other side effects. To read the same data
as validated UTF-8 text, use `fs::readAll`. To read a whole file by path in a
single call rather than from an open handle, use `fs::readBytes`.

Thread cancellation is cooperative: the runtime does not asynchronously interrupt a
blocking host file read, so a worker that needs prompt cancellation around a
blocking descriptor should check `thread::isCancelled` between operations."#;
const EX: &str = r#"Read all remaining bytes from an open file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.bin")
  LET bytes AS List OF Byte = fs::readAllBytes(f)
  ' f is closed by lexical drop when this scope ends
END SUB
```

Skip the first line, then read the remaining bytes of the file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.bin")
  LET header AS String = fs::readLine(f)
  LET body AS List OF Byte = fs::readAllBytes(f)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readAllBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open `File` resource to read from, positioned at the start of the data \
                       to read. Must not have been closed and must have been opened in a mode that \
                       permits reading.",
                aliases: &[],
                ty: ParameterType::named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::abi_function(lower_fs_read_all_bytes),
        }],
    });
}
