//! `fs::readText` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::readText` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_read_text(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_atomic_write::lower_fs_read_text_path_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Read an entire UTF-8 text file into a `String`"#;
const DESC: &str = r#"`fs::readText` opens the file named by `path` for reading, reads its complete
contents in one call, closes the file, validates that the bytes are well-formed
UTF-8, and returns them as a `String`. The whole file is read at once — there is
no streaming and no partial result. No newline translation or other decoding is
performed beyond the UTF-8 validity check, so the returned `String` holds the
file's bytes exactly as stored on disk, interpreted as UTF-8.

The file is always closed before the function returns, whether it succeeded or
failed — there is no handle to clean up afterwards. The byte length of the
returned `String` equals the byte length of the file at the moment it is read, so
an empty file yields an empty `String`. A partial read caused by the file
shrinking mid-read (an unexpected end of file) is a hard error, not a truncated
result.

The final path component is followed when it is a symlink, so reading through a
symlink reads the target file. `path` is interpreted as UTF-8 bytes and passed to
the host filesystem; it may be absolute or relative to the current working
directory, and may contain Unicode characters when the host filesystem accepts
those names. The string must not be empty and must not contain an embedded NUL
byte, because the host `open` call requires a NUL-terminated path. Apart from
opening and closing the file, the call has no side effects. To read
arbitrary binary data without the UTF-8 requirement, use `fs::readBytes`."#;
const EX: &str = r#"Read a text file into a `String`:

```
IMPORT fs

SUB main()
  fs::writeText("data.txt", "first line\nsecond line\n")
  LET value AS String = fs::readText("data.txt")
END SUB
```

Write a file and read it back:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeText("greeting.txt", "hello")
  LET text AS String = fs::readText("greeting.txt")
  io::print(text)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readText",
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
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_fs_read_text),
        }],
    });
}
