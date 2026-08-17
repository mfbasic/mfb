//! `fs::writeTextAtomic` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`. Docs migrated from
//! `src/docs/man/builtins/fs/writeTextAtomic.md`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Atomically replace a file with a `String` written as UTF-8 text"#;
const DESC: &str = r#"`fs::writeTextAtomic` writes the complete contents of `value` as UTF-8 text to a
uniquely named temporary file in the same directory as `path`, flushes that
temporary file to disk, closes it, and then renames it over `path`. A reader
observing `path` during the operation sees either the previous file or the fully
written new file, never a partially written one, so the replacement is
all-or-nothing. The final rename is atomic when the host filesystem supports
atomic rename.

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

The text payload is written directly from the `String`'s packed byte data. A
`String` already holds well-formed UTF-8, so the bytes are written exactly as
held, with no re-encoding, decoding, or newline translation. The write is
retried until every byte has been written or the host reports an output failure,
so a short host write that transfers only part of the buffer is resumed rather
than treated as complete, and an interrupted (`EINTR`) write is retried from the
same cursor before any byte has moved. An empty `String` produces an empty file
at `path`.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host calls require a
NUL-terminated path. The containing directory of `path` must already exist and be
writable, since the temporary file is created there.

When any step before the final rename fails, `path` is left unchanged, and the
leftover temporary file is unlinked before the error is reported so a failed
write never litters the target directory with a stray temp. To replace a file in
place without the temporary-and-rename guarantee, use `fs::writeText`; for the
raw-bytes equivalent of this function, use `fs::writeBytesAtomic`."#;
const EX: &str = r#"Atomically write text to a file:

```
IMPORT fs

SUB main()
  fs::writeTextAtomic("target/output.txt", "done")
END SUB
```

Atomically replace a file's contents and read them back:

```
IMPORT fs
IMPORT io

SUB main()
  fs::writeTextAtomic("greeting.txt", "hello")
  LET text AS String = fs::readText("greeting.txt")
  io::print(text)
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "writeTextAtomic",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String"),
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
                    name: "value",
                    desc: "The text to write, taken verbatim as the `String`'s UTF-8 bytes, in \
                           order. An empty `String` produces an empty file at `path`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
