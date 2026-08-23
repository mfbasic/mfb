//! `fs::writeBytes` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::writeBytes` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_write_bytes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_atomic_write::lower_fs_write_path_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            false,
            true,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Write a `List OF Byte` to a file, replacing its contents"#;
const DESC: &str = r#"`fs::writeBytes` opens the file named by `path` for writing, truncating it to
empty if it already exists or creating it if it does not, writes the complete
contents of `bytes`, flushes the file to disk, closes it, and returns nothing.
Any previous contents of an existing file are discarded; to add to a file
instead of replacing it, use `fs::appendBytes`.

The byte payload is written directly from the byte list's packed data region.
The write is retried until every byte has been written or the host reports an
output failure, so a short host write that transfers only part of the buffer is
resumed rather than treated as complete, and an interrupted (`EINTR`) write is
retried from the same cursor before any byte has moved. An empty byte list
produces an empty (truncated) file. Bytes are written exactly as held in the
list, with no encoding, decoding, or newline translation, so the function is
suitable for binary data as well as text.

The new file is created with mode `384` (octal `0600`), owner read/write only,
before the process umask is applied — not the world-readable `0666`. The file is
created and truncated only after `path` has been validated, and the final path
component is followed when it is a symlink, so writing through a symlink writes
the target file.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.

The file is closed before the function returns on both the success and the
write-failure paths. The write is not atomic: a reader observing the file while
the write is in progress may see a partially written file, and a failure partway
through leaves the file truncated and partially written. For an all-or-nothing
replacement, use `fs::writeBytesAtomic`."#;
const EX: &str = r#"Write raw bytes to a file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = [72, 105]
  fs::writeBytes("target/output.bin", bytes)
END SUB
```

Replace a file's contents with bytes read from another file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("source.bin")
  fs::writeBytes("copy.bin", bytes)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "path",
                    desc: "The filesystem path of the file to write, as UTF-8 bytes; absolute or \
                           relative to the current working directory. Must be non-empty and free \
                           of embedded NUL bytes.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "bytes",
                    desc: "The bytes to write, in order, taken verbatim from the list's data \
                           region. An empty list truncates the file to zero length.",
                    aliases: &["value"],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_write_bytes),
        }],
    });
}
