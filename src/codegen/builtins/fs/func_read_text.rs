//! `fs::readText` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper` (which branches to
//! the relocated `lower_fs_read_text_path_helper`).

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

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

pub(super) fn register(pkg: &mut RegistryPackage) {
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
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
