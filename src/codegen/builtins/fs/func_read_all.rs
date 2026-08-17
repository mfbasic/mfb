//! `fs::readAll` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`. Docs migrated from
//! `src/docs/man/builtins/fs/readAll.md`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Read all remaining text from an open `File` into a `String`"#;
const DESC: &str = r#"`fs::readAll` reads every remaining byte of `file` as UTF-8 text, starting at the
file's current read position and continuing to end of input, validates that the
bytes are well-formed UTF-8, and returns them as a single `String`. The read
position is advanced to end of input, so a subsequent `fs::eof` reports true.
`file` must be an open `File` resource — such as one returned by `fs::openFile` or
`fs::open` — opened in a mode that permits reading.

The amount to read is measured up front: the function seeks to record the current
position, seeks to the end to find the file's length, seeks back to the start
position, allocates a `String` of exactly that length, and reads the remainder
into it in one or more host reads until the buffer is full. No newline
translation or other decoding is performed beyond the UTF-8 validity check, so the
returned `String` holds the file's remaining bytes exactly as stored on disk,
interpreted as UTF-8. When `file` is already at end of input, no bytes remain and
the empty `String` is returned.

If the file was previously read with `fs::readLine`, the buffered read-ahead is
first reconciled so the measurement and read see the true file-descriptor
position rather than the block read-ahead. The function only reads from and
repositions `file`; it does not close it and has no other side effects. To read
the same data without the UTF-8 requirement, use `fs::readAllBytes`. To read a
whole file by path in a single call rather than from an open handle, use
`fs::readText`.

Thread cancellation is cooperative: the runtime does not asynchronously interrupt
a blocking host file read, so a worker that needs prompt cancellation around a
blocking descriptor should check `thread::isCancelled` between operations."#;
const EX: &str = r#"Read all remaining text from an open file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.txt")
  LET value AS String = fs::readAll(f)
  ' f is closed by lexical drop when this scope ends
END SUB
```

Skip the first line, then read the rest of the file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.txt")
  LET header AS String = fs::readLine(f)
  LET body AS String = fs::readAll(f)
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readAll",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open `File` resource to read from, positioned at the start of the data \
                       to read. Must not have been closed and must have been opened in a mode that \
                       permits reading.",
                aliases: &[],
                ty: ParameterType::Named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
