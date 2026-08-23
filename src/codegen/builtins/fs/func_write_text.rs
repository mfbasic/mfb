//! `fs::writeText` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `native::lower_fs_os_seam`.

use super::native::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::writeText` — the shared OS-seam dispatcher
/// [`super::native::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_write_text(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.writeText")
}

const INTRO: &str = r#"Write a `String` to a file as UTF-8 text, replacing its contents"#;
const DESC: &str = r#"`fs::writeText` opens the file named by `path` for writing, truncating it to
empty if it already exists or creating it if it does not, writes the complete
contents of `value` as UTF-8 text, flushes the file to disk, closes it, and
returns nothing. Any previous contents of an existing file are discarded; to add
to a file instead of replacing it, use `fs::appendText`.

The text payload is written directly from the `String`'s packed byte data. A
`String` already holds well-formed UTF-8, so the bytes are written exactly as
held, with no re-encoding, decoding, or newline translation. The write is
retried until every byte has been written or the host reports an output failure,
so a short host write that transfers only part of the buffer is resumed rather
than treated as complete, and an interrupted (`EINTR`) write is retried from the
same cursor before any byte has moved. An empty `String` produces an empty
(truncated) file.

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
replacement, use `fs::writeTextAtomic`."#;
const EX: &str = r#"Write text to a file:

```
IMPORT fs

SUB main()
  fs::writeText("target/output.txt", "Hello")
END SUB
```

Replace a file's contents and read them back:

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
        name: "writeText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String"),
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
                    name: "value",
                    desc: "The text to write, taken verbatim as the `String`'s UTF-8 bytes, in \
                           order. An empty `String` truncates the file to zero length.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_write_text),
        }],
    });
}
