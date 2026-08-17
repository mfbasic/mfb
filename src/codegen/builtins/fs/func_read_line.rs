//! `fs::readLine` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Read one line of UTF-8 text from an open `File`"#;
const DESC: &str = r#"`fs::readLine` reads a single line from `file` starting at its current read
position, advances the position to just past the line's terminator, and returns
the line as a `String` with the terminator removed. `file` must be an open `File`
resource — such as one returned by `fs::openFile` or `fs::open` — opened in a mode
that permits reading.

A line ends at the first line feed (LF, byte `0x0A`) at or after the current
position. Both LF and CRLF terminators are accepted: when the byte immediately
before the LF is a carriage return (CR, byte `0x0D`) it is treated as part of the
terminator and is also stripped from the returned `String`. A bare CR with no
following LF is not a terminator and is returned as an ordinary character. When
the remaining bytes contain no LF, the entire remainder of the file is returned
as the final line and the position is advanced to end of input; the next call
then fails with end-of-input.

The returned `String` never includes the terminating LF or the CR of a CRLF pair.
An empty line (an LF, or a CRLF, with nothing before it) yields an empty `String`
while still consuming the terminator and advancing the position. The bytes making
up the line are validated as UTF-8 before being returned.

On success the position is left immediately after the consumed terminator (or at
end of input when the last line had no terminator), so repeated calls walk the
file one line at a time. Because end of input is reported as an error rather than
an empty result, use `fs::eof` to test for the end before each call. The function
only reads from and repositions `file`; it does not close it and has no other side
effects.

Reads are served from a transparent per-handle block buffer: internally the file
is read in blocks and lines are handed out from that buffer, so a loop over a
large file runs in linear time rather than re-reading the remainder for every
line. This is invisible — the lines, terminators, EOF point, and errors are
identical to an unbuffered read. A whole-file read (`fs::readAll`,
`fs::readAllBytes`) or a write (`fs::writeAll`) on the same handle transparently
reconciles the buffer first, so mixing them with `fs::readLine` sees the exact
logical position.

Thread cancellation is cooperative. The current runtime does not asynchronously
interrupt arbitrary host file reads; workers that need prompt cancellation around
blocking file descriptors should check `thread::isCancelled` between
cancellation-point operations."#;
const EX: &str = r#"Read the first line of a file:

```
IMPORT fs

SUB main()
  RES f = fs::openFile("data.txt")
  LET line AS String = fs::readLine(f)
  ' f is closed by lexical drop when this scope ends
END SUB
```

Read every line until end of input:

```
IMPORT fs
IMPORT io

SUB main()
  RES f = fs::openFile("data.txt")
  WHILE NOT fs::eof(f)
    io::print(fs::readLine(f))
  END WHILE
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readLine",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open `File` resource to read from, positioned at the start of the line \
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
