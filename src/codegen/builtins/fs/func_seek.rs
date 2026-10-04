//! `fs::seek` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::seek` — calls its per-member `lower_fs_*_helper` emitter and
/// finalizes (crypto/io's clean-room shape).
pub(crate) fn lower_fs_seek(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_read_write::lower_fs_seek_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Move an open `File`'s read and write position to a byte offset"#;
const DESC: &str = r#"`fs::seek` moves `file`'s position to `position` bytes from the start of the
file, so the next read or write on `file` happens there. `fs::seek(f, 0)` goes
back to the beginning. Nothing is returned; the move is the whole effect.

The move is exact whatever the handle was doing before. Output still waiting in
`file`'s write buffer (see `fs::setBuffered`) is written out first, at the
position it was written for, so a buffered write is never relocated by a later
seek. Text that `fs::readLine` had read ahead is discarded, so the next read
starts at `position` and not wherever the read-ahead stopped.

Seeking is how data written to a `File` is read back through the same handle:
open with `"rw"`, write, `fs::seek(f, 0)`, then read. That is the usual way to
use a `File` from `fs::openData`.

`position` may be past the end of the file. Reading there finds no data
(`fs::readLine` raises `ErrEndOfFile`, `fs::eof` returns `TRUE`); writing there
grows the file, and the bytes between the old end and `position` read back as
zero bytes. On a `File` opened in `"append"` mode every write still goes to the
end, wherever the position is.

A negative `position` raises `ErrInvalidArgument` and leaves `file` untouched.
Moving needs a handle the host can reposition: on a pipe or another non-seekable
handle the move fails with `ErrReadFailed`, as `fs::size` and `fs::eof` do on the
same handles. If buffered output cannot be written, `ErrWriteFailed` is raised,
the output stays buffered, and the position does not change. A closed `file`
raises `ErrResourceClosed`, and one handed to another thread with
`thread::transfer` raises `ErrResourceMoved`."#;
const EX: &str = r#"Write to an in-memory file, then go back and read it:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  RES f = fs::openData("", "rw")
  fs::writeAll(f, "first\nsecond\n")
  fs::seek(f, 0)
  io::print(fs::readLine(f))
  io::print(fs::readLine(f))
END FUNC
```

Re-read a file from a remembered offset:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  fs::writeText("notes.txt", "header\nbody line\n")
  RES f = fs::openFile("notes.txt")
  io::print(fs::readLine(f))
  fs::seek(f, 7)
  io::print(fs::readLine(f))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "seek",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("File, Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "file",
                    desc: "An open, seekable `File` resource to reposition. Must not have been \
                           closed.",
                    aliases: &[],
                    ty: ParameterType::named(super::FILE_TYPE_ID),
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "position",
                    desc: "The new position, in bytes from the start of the file. Must be zero \
                           or greater; it may be past the end of the file.",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Nothing,
            errors: vec![
                "ErrInvalidArgument",
                "ErrReadFailed",
                "ErrWriteFailed",
                "ErrResourceClosed",
                "ErrResourceMoved",
            ],
            body: Body::abi_function(lower_fs_seek),
        }],
    });
}
