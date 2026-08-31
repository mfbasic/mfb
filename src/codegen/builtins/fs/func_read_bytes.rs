//! `fs::readBytes` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::readBytes` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_read_bytes(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_atomic_write::lower_fs_read_bytes_path_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Read an entire file into a `List OF Byte`"#;
const DESC: &str = r#"`fs::readBytes` opens the file named by `path` for reading, reads its complete
contents into a single `List OF Byte`, closes the file, and returns the byte
list. The whole file is read in one call — there is no streaming and no partial
result. Bytes are returned exactly as stored on disk, with no encoding, decoding,
or newline translation, so the function is suitable for binary data as well as
text.

Internally the function opens the file read-only, wraps the descriptor in a fresh
`File` handle, and delegates to the same whole-file reader as `fs::readAllBytes`;
the file is always closed before the function returns, on both the success and the
read-failure paths. The returned list's length equals the byte length of the file
at the moment it is read, so an empty file yields an empty `List OF Byte`.

The final path component is followed when it is a symlink, so reading through a
symlink reads the target file. `path` is interpreted as UTF-8 bytes and passed to
the host filesystem; it may be absolute or relative to the current working
directory, and may contain Unicode characters when the host filesystem accepts
those names. The string must not be empty and must not contain an embedded NUL
byte, because the host `open` call requires a NUL-terminated path. Apart from
opening and closing the file descriptor, the call has no side effects."#;
const EX: &str = r#"Read a binary file into a byte list:

```
IMPORT fs

SUB main()
  fs::writeText("data.bin", "first line\nsecond line\n")
  LET bytes AS List OF Byte = fs::readBytes("data.bin")
END SUB
```

Report the size of a file in bytes:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeText("image.png", "first line\nsecond line\n")
  LET bytes AS List OF Byte = fs::readBytes("image.png")
  io::print("size: " & toString(len(bytes)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readBytes",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the file to read, as UTF-8 bytes; absolute or \
                       relative to the current working directory. Must be non-empty and free of \
                       embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::Byte),
            errors: vec![],
            body: Body::abi_function(lower_fs_read_bytes),
        }],
    });
}
