//! `fs::flush` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Drain an open `File`'s output buffer to its file descriptor"#;
const DESC: &str = r#"`fs::flush` drains any output currently held in `file`'s per-handle buffer,
issuing the pending bytes to the underlying file descriptor at once, then returns
nothing. It matters only when the handle has buffering enabled with
`fs::setBuffered(file, TRUE)`; on an unbuffered handle — the default — nothing is
ever held back, so `fs::flush` is a no-op. It is also a no-op when a buffered
handle has no pending bytes.

Internally the drain issues a `write(fd, buffer, filled)` loop until the buffer is
empty and then resets the fill count to zero; a short write advances the cursor
and continues, and an `EINTR` interruption re-issues the write with the unchanged
cursor. If a write fails, the buffer is left intact so a later `fs::flush` can
retry, and the call raises `ErrOutput`.

Use `fs::flush` at a checkpoint where buffered data must reach the file before the
program continues — for example before another process reads the file, or before a
long pause. Closing the handle with `fs::close`, or letting its `RES` binding
leave scope, also drains the buffer, so an explicit flush is only needed
mid-stream; the final bytes are never lost to a clean close.

Buffering and flushing are per handle: `fs::flush(file)` drains only `file`'s
buffer and affects no other open `File`. Each `File` carries its own buffer and
its own enabled flag."#;
const EX: &str = r#"Force buffered data to disk at a checkpoint, then keep writing:

```
IMPORT fs

SUB main()
  LET header AS String = "id,name\n"
  LET body AS String = "1,alice\n"
  RES out = fs::openFile("report.txt", "write")
  fs::setBuffered(out, TRUE)
  fs::writeAll(out, header)
  fs::flush(out)             ' header reaches disk before the body is written
  fs::writeAll(out, body)
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "flush",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open `File` resource whose output buffer should be drained.",
                aliases: &[],
                ty: ParameterType::Named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
