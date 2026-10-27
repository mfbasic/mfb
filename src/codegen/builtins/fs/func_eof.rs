//! `fs::eof` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::eof` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_eof(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_read_write::lower_fs_eof_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Test whether an open `File` is at end of input"#;
const DESC: &str = r#"`fs::eof` reports whether `file`'s current read position has reached the end of
its contents. It returns `TRUE` when the position is at or beyond the end of the
contents (at or past the file's length in bytes) and `FALSE` while one or more
bytes remain to be read. `file` must be an open `File` resource, such as one
returned by `fs::openFile` or `fs::open`.

Text that `fs::readLine` has read ahead but not yet returned still counts as
remaining input, so `fs::eof` never reports `TRUE` while a line is still to
come. The call reads no contents and has no side effects: the read position is
left exactly where it was, nothing is written, and `file` stays open.

`fs::eof` tests the read position only. A `File` opened with mode `"append"`
starts at the beginning of its existing contents, so `fs::eof` reports `FALSE`
on a non-empty file even though every write still goes to the end. A `File`
opened with `"write"` is empty and reports `TRUE`.

`fs::eof` needs a handle whose position and length can be measured: a regular
file on disk, or a `File` from `fs::openData`. On a pipe, a socket, or another
non-seekable handle the position and length cannot be measured, and the call
raises `ErrReadFailed` instead of returning a `Boolean`.

Naming `file` after `fs::close(file)` in the same function is the build error
`TYPE_USE_AFTER_MOVE`. When the close happened where the compiler cannot see it
— in a function the handle was passed to, or through a second copy of a record
holding it — `fs::eof` raises `ErrResourceClosed`. A handle handed to another
thread with `thread::transfer` raises `ErrResourceMoved`.

Guard a `fs::readLine` loop with `fs::eof`, because `fs::readLine` reports end
of input as `ErrEndOfFile` rather than returning an empty `String`; testing
`fs::eof` first lets the loop stop cleanly. `fs::readAll` and `fs::readAllBytes`
do not need the guard: at end of input they return an empty value."#;
const EX: &str = r#"Read every line until end of input. The file lives in a scratch directory under
the temporary directory, and `fs::close` releases it before it is deleted:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-eof-1"])
  fs::createDirectories(dir)
  LET data AS String = fs::pathJoin([dir, "data.txt"])
  fs::writeText(data, "first line\nsecond line\n")
  RES f = fs::openFile(data)
  WHILE NOT fs::eof(f)
    io::print(fs::readLine(f))
  END WHILE
  fs::close(f)
  fs::deleteFile(data)
  fs::deleteDirectory(dir)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "eof",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "file",
                desc: "An open `File` resource to test, as returned by `fs::open`, `fs::openFile`, `fs::openFileNoFollow`, `fs::openWithin`, `fs::openData`, or `fs::createTempFile`. Must not have been closed.",
                aliases: &[],
                ty: ParameterType::named(super::FILE_TYPE_ID),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrReadFailed", "ErrResourceClosed", "ErrResourceMoved"],
            body: Body::abi_function(lower_fs_eof),
        }],
    });
}
