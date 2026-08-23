//! `fs::appendText` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `native::lower_fs_os_seam`.

use super::native::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::appendText` — the shared OS-seam dispatcher
/// [`super::native::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_append_text(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.appendText")
}

const INTRO: &str =
    r#"Append a `String` to the end of a file as UTF-8 text, preserving its existing contents"#;
const DESC: &str = r#"`fs::appendText` opens the file named by `path` in append mode, creating it with
no contents if it does not already exist, writes the complete contents of `value`
as UTF-8 text after whatever the file already held, flushes the file to disk,
closes it, and returns nothing. Any existing contents are preserved and the new
text is added after them; to replace a file's contents instead of extending them,
use `fs::writeText`.

The file is opened with the append flag set, so every write is positioned at the
current end of the file. The text payload is written directly from the `String`'s
packed byte data. A `String` already holds well-formed UTF-8, so the bytes are
written exactly as held, with no re-encoding, decoding, or newline translation,
and no trailing newline is added. The write is retried until every byte has been
written or the host reports an output failure, so a short host write that
transfers only part of the buffer is resumed rather than treated as complete, and
an interrupted (`EINTR`) write is retried from the same cursor before any byte has
moved. An empty `String` leaves the file's length unchanged, creating it as an
empty file if it did not exist.

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
the write is in progress may see only part of the appended text, and a failure
partway through leaves the file extended by only the bytes written so far."#;
const EX: &str = r#"Append a line to a log file:

```
IMPORT fs

SUB main()
  fs::appendText("target/output.txt", "line\n")
END SUB
```

Build up a file across several calls:

```
IMPORT fs
IMPORT io

SUB main()
  fs::appendText("notes.txt", "first\n")
  fs::appendText("notes.txt", "second\n")
  LET text AS String = fs::readText("notes.txt")
  io::print(text)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "appendText",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String"),
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
                    name: "value",
                    desc: "The text to append, taken verbatim as the `String`'s UTF-8 bytes, in \
                           order, after the file's existing contents. An empty `String` leaves \
                           the file's length unchanged.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_append_text),
        }],
    });
}
