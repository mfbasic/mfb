//! `fs::writeBytesAtomic` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::writeBytesAtomic` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_write_bytes_atomic(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_atomic_write::lower_fs_atomic_write_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            super::gen_atomic_write::AtomicWriteValueKind::Bytes,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Atomically replace a file with a `List OF Byte`"#;
const DESC: &str = r#"`fs::writeBytesAtomic` writes the complete contents of `bytes` to a uniquely
named temporary file in the same directory as `path`, flushes that temporary
file to disk, closes it, and then renames it over `path`. A reader observing
`path` during the operation sees either the previous file or the fully written
new file, never a partially written one, so the replacement is all-or-nothing.
The final rename is atomic when the host filesystem supports atomic rename.

The replacement is also crash-durable: after the rename the containing directory
is itself flushed to disk, so once this function returns successfully the new
file survives a crash or power loss and never reverts to the previous contents.
The directory flush is best-effort — if the containing directory cannot be
opened or flushed the write is still reported as successful, because the atomic
rename has already completed.

The temporary file is created next to `path` with a name derived from `path`'s
final component plus a `.mfb-XXXXXX.tmp` suffix, where the host fills in the `X`
markers to make the name unique. Creating the temporary in the same directory as
`path` keeps both files on the same filesystem so the final rename is a
same-filesystem move rather than a copy.

The byte payload is written directly from the byte list's packed data region.
The write is retried until every byte has been written or the host reports an
output failure, so a short host write that transfers only part of the buffer is
resumed rather than treated as complete, and an interrupted (`EINTR`) write is
retried from the same cursor before any byte has moved. An empty byte list
produces an empty file at `path`. Bytes are written exactly as held in the list,
with no encoding, decoding, or newline translation, so the function is suitable
for binary data as well as text.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host calls require a
NUL-terminated path. The containing directory of `path` must already exist and be
writable, since the temporary file is created there.

When any step before the final rename fails, `path` is left unchanged, and the
leftover temporary file is unlinked before the error is reported so a failed
write never litters the target directory with a stray temp. To replace a file in
place without the temporary-and-rename guarantee, use `fs::writeBytes`; for the
text equivalent of this function, use `fs::writeTextAtomic`."#;
const EX: &str = r#"Atomically write raw bytes to a file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = [72, 105]
  fs::writeBytesAtomic("target/output.bin", bytes)
END SUB
```

Atomically replace a file's contents with bytes read from another file:

```
IMPORT fs

SUB main()
  LET bytes AS List OF Byte = fs::readBytes("source.bin")
  fs::writeBytesAtomic("copy.bin", bytes)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeBytesAtomic",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, List OF Byte"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "path",
                    desc: "The filesystem path of the file to replace, as UTF-8 bytes; absolute \
                           or relative to the current working directory. Must be non-empty and \
                           free of embedded NUL bytes. Its containing directory must exist and be \
                           writable.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "bytes",
                    desc: "The bytes to write, in order, taken verbatim from the list's data \
                           region. An empty list produces an empty file at `path`.",
                    aliases: &["value"],
                    ty: ParameterType::list_of(ParameterType::Byte),
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_write_bytes_atomic),
        }],
    });
}
