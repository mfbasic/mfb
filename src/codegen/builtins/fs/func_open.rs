//! `fs::open` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes. Returns a `File`
//! resource.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::open` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_open(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_open::lower_fs_open_helper(&symbol, ctx.platform_imports, ctx.platform, false)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str =
    r#"Open a file with an explicitly named access mode and return a `File` resource"#;
const DESC: &str = r#"`fs::open` opens the file named by `path` using the access mode named by `mode`
and returns an opaque `File` resource that later `fs::` calls read from, write
to, and close. Both arguments are required; unlike `fs::openFile`, `mode` has no
default and must be supplied explicitly.

`mode` selects how the file is opened. The mode names are `"read"` or
`"r"`, `"write"` or `"w"`, `"readWrite"` or `"rw"`, and `"append"` or `"a"`; there are no others.
`"read"` opens an existing file for reading only and creates nothing. `"write"`
opens the file for writing, creating it when it does not exist and truncating it
to empty when it does. `"readWrite"` opens the file for both reading and writing,
creating it when it does not exist but preserving existing contents. `"append"`
opens the file for writing with every write directed to the end of the file,
even after `fs::seek`, creating it when it does not exist. The mode string is matched exactly, byte for
byte, and is case sensitive; any other value is rejected before the file is
touched. A `"write"` or `"append"` handle cannot be read; use `"readWrite"`
when you need both.

Files created by a `write`, `readWrite`, or `append` open are created with
user-only `0600` permission bits (subject to the process umask), not
world-readable `0666`, matching `fs::createTempFile` and the atomic writers.

Symbolic links anywhere in `path` are followed, so opening through a symlink
opens its target. To refuse a path that passes through any symbolic link, use
`fs::openFileNoFollow` instead.

`path` may be absolute or relative to the current working directory and may
contain Unicode characters when the host filesystem accepts those names. It must
not be empty and must not contain a NUL character.

The returned `File` is closed when the binding that holds it goes out of scope, or explicitly with `fs::close`. The function reads or writes no
file contents itself; it only opens the file and gives you the `File`
resource. When `fs::open` raises, there is nothing to close.

An empty `path`, one containing a NUL character, or a `mode` that is not one of the
names above raises `ErrInvalidArgument`. When the file cannot be opened the
error says why: `ErrPathNotFound` if it does not exist — or, for a mode that
creates the file, if a directory above it does not — `ErrAccessDenied` if
permission is refused, and `ErrInvalidPath` if `path` cannot name a file — it
names a directory, in any mode, a name in it is too long, a file is used as a
directory, or it is a loop of symbolic links. Any other refusal raises
`ErrReadFailed` for `"r"` and `ErrWriteFailed` for a mode that writes.
`ErrOutOfMemory` means the program ran out of memory."#;
const EX: &str = r#"Open a file for reading and close it explicitly. Each example works in a scratch
directory under the temporary directory and removes it afterwards:

```
IMPORT fs

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-open-1"])
  fs::createDirectories(dir)
  LET data AS String = fs::pathJoin([dir, "data.txt"])
  fs::writeText(data, "first line\nsecond line\n")
  RES f AS fs::File = fs::open(data, "read")
  fs::close(f)
  fs::deleteFile(data)
  fs::deleteDirectory(dir)
END FUNC
```

Open a file for writing, truncating any previous contents:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-open-2"])
  fs::createDirectories(dir)
  LET out AS String = fs::pathJoin([dir, "out.txt"])
  fs::writeText(out, "old contents")
  RES w AS fs::File = fs::open(out, "write")
  fs::writeAll(w, "hello")
  fs::close(w)
  io::print(fs::readText(out))
  fs::deleteFile(out)
  fs::deleteDirectory(dir)
END FUNC
```

Open a file for appending so each write lands at the end. This prints
`started` and then `stopped`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-open-3"])
  fs::createDirectories(dir)
  LET path AS String = fs::pathJoin([dir, "app.log"])
  RES first AS fs::File = fs::open(path, "a")
  fs::writeAll(first, "started\n")
  fs::close(first)
  RES second AS fs::File = fs::open(path, "a")
  fs::writeAll(second, "stopped")
  fs::close(second)
  io::print(fs::readText(path))
  fs::deleteFile(path)
  fs::deleteDirectory(dir)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "open",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "path",
                    desc: "The filesystem path of the file to open; absolute or relative to \
                           the current working directory. Must be non-empty and contain no NUL \
                           character.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "mode",
                    desc: "The access mode. One of `\"read\"`/`\"r\"` (read existing file), \
                           `\"write\"`/`\"w\"` (create or truncate for writing), \
                           `\"readWrite\"`/`\"rw\"` (create-if-absent for reading and writing, \
                           preserving contents), or `\"append\"`/`\"a\"` (create-if-absent for \
                           writing at end of file). Matched exactly and case sensitively.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::named(super::FILE_TYPE_ID),
            errors: vec![
                "ErrOutOfMemory",
                "ErrWriteFailed",
                "ErrPathNotFound",
                "ErrInvalidPath",
                "ErrAccessDenied",
                "ErrInvalidArgument",
                "ErrReadFailed",
            ],
            body: Body::abi_function(lower_fs_open),
        }],
    });
}
