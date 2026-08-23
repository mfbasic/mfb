//! `fs::appendBytes` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::appendBytes` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_append_bytes(
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
            true,
            true,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str =
    r#"Append a `List OF Byte` to the end of a file, preserving its existing contents"#;
const DESC: &str = r#"`fs::appendBytes` opens the file named by `path` in append mode, creating it with
no contents if it does not already exist, writes the complete contents of `bytes`
after whatever the file already held, flushes the file to disk, closes it, and
returns nothing. Any existing contents are preserved and the new bytes are added
after them; to replace a file's contents instead of extending them, use
`fs::writeBytes`.

The file is opened with the append flag set, so every write is positioned at the
current end of the file. The byte payload is written directly from the byte
list's packed data region. The write is retried until every byte has been written
or the host reports an output failure, so a short host write that transfers only
part of the buffer is resumed rather than treated as complete, and an interrupted
(`EINTR`) write is retried from the same cursor before any byte has moved. An
empty byte list leaves the file's length unchanged, creating it as an empty file
if it did not exist. Bytes are written exactly as held in the list, with no
encoding, decoding, or newline translation, so the function is suitable for
binary data as well as text.

When the file is created it is given mode `384` (octal `0600`), owner read/write
only, before the process umask is applied — not the world-readable `0666`. An
existing file keeps its current mode. The file is created and opened only after
`path` has been validated, and the final path component is followed when it is a
symlink, so appending through a symlink appends to the target file.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.

The file is closed before the function returns on both the success and the
write-failure paths. The append is not atomic: a reader observing the file while
the write is in progress may see only part of the appended bytes, and a failure
partway through leaves the file extended by only the bytes written so far."#;
const EX: &str = r#"Append a single newline byte to a log file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = [10]
  fs::appendBytes("target/log.bin", bytes)
END SUB
```

Append the contents of one file to the end of another:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("source.bin")
  fs::appendBytes("combined.bin", bytes)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "appendBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "path",
                    desc: "The filesystem path of the file to append to, as UTF-8 bytes; absolute \
                           or relative to the current working directory. Must be non-empty and \
                           free of embedded NUL bytes. The file is created if it does not exist.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "bytes",
                    desc: "The bytes to append, in order, taken verbatim from the list's data \
                           region after the file's existing contents. An empty list leaves the \
                           file's length unchanged.",
                    aliases: &["value"],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_append_bytes),
        }],
    });
}
