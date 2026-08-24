//! `fs::writeAll` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::writeAll` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_write_all(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_read_write::lower_fs_write_all_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Write all of a `String` to an open `File` as UTF-8 text"#;
const DESC: &str = r#"`fs::writeAll` writes the complete contents of `value` to `file` as UTF-8 text,
starting at the file's current write position, and returns nothing. The bytes are
taken directly from the `String`'s packed byte data; because a `String` already
holds well-formed UTF-8, no re-encoding, decoding, or newline translation is
performed. An empty `String` writes no bytes and leaves the file unchanged.

The write is retried until every byte has been written or the host reports an
output failure, so a short host write that transfers only part of the buffer is
resumed from the same cursor rather than treated as complete. The file position
advances by the number of bytes written, so consecutive calls write one after
another within the open handle, and a following `fs::writeAll` or
`fs::writeAllBytes` continues from where this call left off.

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
by path in a single call rather than through an open handle, use `fs::writeText`.

Thread cancellation is cooperative: the runtime does not asynchronously interrupt
a blocking host file write, so a worker that needs prompt cancellation around a
blocking descriptor should check `thread::isCancelled` between operations."#;
const EX: &str = r#"Write text to an open file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("target/output.txt", "write")
  fs::writeAll(f, "Hello")
  ' f is closed by lexical drop when this scope ends
END SUB
```

Write a header line, then the rest of the body:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("target/report.txt", "write")
  fs::writeAll(f, "title\n")
  fs::writeAll(f, "body")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeAll",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "file",
                    desc: r#"An open `File` resource to write to, positioned at the point where the text should be written. Must not have been closed and must have been opened in a mode that permits writing (`"write"`, `"readWrite"`, or `"append"`)."#,
                    aliases: &[],
                    ty: ParameterType::named(super::FILE_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "value",
                    desc: "The text to write, taken verbatim as the `String`'s UTF-8 bytes, in \
                           order. An empty `String` writes nothing.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_write_all),
        }],
    });
}
