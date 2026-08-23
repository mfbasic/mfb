//! `fs::readText` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `gen_os_seam::lower_fs_os_seam` (which branches to
//! the relocated `lower_fs_read_text_path_helper`).

use super::gen_os_seam::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::readText` — the shared OS-seam dispatcher
/// [`super::gen_os_seam::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_read_text(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.readText")
}

const INTRO: &str = r#"Read an entire UTF-8 text file into a `String`"#;
const DESC: &str = r#"`fs::readText` opens the file named by `path` for reading, reads its complete
contents in one call, closes the file, validates that the bytes are well-formed
UTF-8, and returns them as a `String`. The whole file is read at once — there is
no streaming and no partial result. No newline translation or other decoding is
performed beyond the UTF-8 validity check, so the returned `String` holds the
file's bytes exactly as stored on disk, interpreted as UTF-8.

Internally the function opens the file read-only, seeks to the end and back to
determine the length, allocates the result `String`, reads the bytes in a loop,
and closes the descriptor. The file is always closed before the function returns,
on both the success and the post-open failure paths. The byte length of the
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
opening and closing the file descriptor, the call has no side effects. To read
arbitrary binary data without the UTF-8 requirement, use `fs::readBytes`."#;
const EX: &str = r#"Read a text file into a `String`:

```
IMPORT fs

SUB main()
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
