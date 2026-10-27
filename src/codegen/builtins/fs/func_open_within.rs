//! `fs::openWithin` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes. Returns a `File`
//! resource resolved beneath a trusted root; `mode` defaults to `"read"`.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::openWithin` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_open_within(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_open::lower_fs_open_within_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Open a file resolved beneath a trusted root directory, refusing any escape"#;
const DESC: &str = r#"`fs::openWithin` opens the file named by `relPath` resolved **beneath** the
trusted directory `root`, and returns an opaque `File` resource. Its purpose is
to open a caller-controlled name inside an intended directory with a guarantee
that the result cannot escape that directory — closing the
time-of-check/time-of-use race that a `fs::isWithin(root, path)` check followed
by a separate `fs::open(path)` leaves open.

Containment is enforced at open time. `root` is canonicalized once, the way
`fs::canonicalPath` does (resolving the trusted root's own symbolic links); a
relative `root` is resolved against the working directory. `relPath` is rejected
if it is absolute or contains a `..` component; the canonical root and `relPath`
are joined; and the join is opened with the same whole-path no-symlink rule as
`fs::openFileNoFollow`. Because the canonical root is symlink-free and every
component is re-checked at open time, a component swapped to a symbolic link
*after* canonicalization is **rejected** rather than followed, so the open cannot
be redirected outside `root`. On a Linux kernel older than 5.6, or under a
security filter that reports the newer open call as unimplemented, only a
symbolic link at the final name is refused, exactly as `fs::openFileNoFollow`
does there; a filter that refuses that call with a permission error instead
raises `ErrAccessDenied`.

A `relPath` is therefore refused when it is absolute, contains a `..` component,
or traverses a symbolic link at any component. `relPath` is always interpreted
relative to `root`, never to the process working directory. A `.` component is
passed through unchanged; only an exact `..` component is rejected, so `..b` and
`...` are ordinary names. A `relPath` that names a directory, including `.`,
raises `ErrInvalidPath`.

The `mode` argument is optional: when it is omitted the file is opened for
reading, exactly as if `"read"` had been supplied.

`mode` selects how the file is opened. The mode names are `"read"` or `"r"`,
`"write"` or `"w"`, `"readWrite"` or `"rw"`, and `"append"` or `"a"`; there are
no others. `"read"` opens an existing file for reading only and creates nothing.
`"write"` opens the file for writing, creating it when it does not exist and
truncating it to empty when it does. `"readWrite"` opens the file for both
reading and writing, creating it when it does not exist and leaving existing
contents in place; reading and writing both start at the beginning of the file,
so the first write overwrites what is there. `"append"` opens the file for
writing with every write directed to the end of the file, even after
`fs::seek`, creating it when it does not exist. A `"write"` or `"append"` handle
cannot be read. The mode string is matched exactly, byte for byte, and is case
sensitive; any other value is rejected before the file is touched, though
`root` is resolved first: a `root` that does not exist reports `ErrPathNotFound`
even when `mode` is also invalid.

Files created by a `write`, `readWrite`, or `append` open are created with
user-only `0600` permission bits (subject to the process umask), not
world-readable `0666`, matching `fs::createTempFile` and the atomic writers.

`root` must resolve to an existing directory, and neither `root` nor `relPath`
may be empty or contain a NUL character. The returned `File` is closed when the binding that holds it goes out of scope, or explicitly with `fs::close`.

`ErrInvalidArgument` means `root` or `relPath` is empty or contains a NUL
character, `relPath` is absolute or has a `..` component, `mode` is not one of
the names above, or the canonical root joined with `relPath` is longer than
4096 bytes. When the file cannot be opened the error says why: `ErrPathNotFound`
if `root` or the file does not exist, `ErrAccessDenied` if permission is refused
or the open would pass through a symbolic link or leave `root`, and
`ErrInvalidPath` if the path cannot name a file — it names a directory, in any
mode, a name in it is too long, or a file is used as a directory. Any other
refusal raises `ErrReadFailed` for a read open (`mode` omitted, `"read"` or
`"r"`) and `ErrWriteFailed` for a mode that writes. `ErrOutOfMemory` means the
program ran out of memory."#;
const EX: &str = r#"Open a caller-supplied name beneath a fixed root, for reading. The root is a
scratch directory under the temporary directory, removed afterwards:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET root AS String = fs::pathJoin([fs::tempDirectory(), "mfb-openWithin-1"])
  fs::createDirectories(root)
  fs::writeText(fs::pathJoin([root, "alice.txt"]), "hi")
  LET userName AS String = "alice.txt"
  RES f AS fs::File = fs::openWithin(root, userName)
  io::print(fs::readAll(f))
  fs::close(f)
  fs::deleteFile(fs::pathJoin([root, "alice.txt"]))
  fs::deleteDirectory(root)
END FUNC
```

A `relPath` that tries to escape the root is refused rather than followed:

```
IMPORT fs
IMPORT errorCode
IMPORT io

FUNC main() AS Nothing
  LET root AS String = fs::pathJoin([fs::tempDirectory(), "mfb-openWithin-2"])
  fs::createDirectories(root)
  RES f AS fs::File = fs::openWithin(root, "../../etc/passwd") TRAP(e)
    io::print(toString(e.code = errorCode::ErrInvalidArgument))
    fs::deleteDirectory(root)
    RETURN
  END TRAP
END FUNC
```

Write a new file beneath the root. This prints `hello`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET root AS String = fs::pathJoin([fs::tempDirectory(), "mfb-openWithin-3"])
  LET reports AS String = fs::pathJoin([root, "reports"])
  fs::createDirectories(reports)
  RES w AS fs::File = fs::openWithin(root, "reports/today.txt", "write")
  fs::writeAll(w, "hello")
  fs::close(w)
  io::print(fs::readText(fs::pathJoin([reports, "today.txt"])))
  fs::deleteFile(fs::pathJoin([reports, "today.txt"]))
  fs::deleteDirectory(reports)
  fs::deleteDirectory(root)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "openWithin",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String[, String]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "root",
                    desc: "The trusted base directory; its own symbolic links are resolved, and a relative root is resolved against the working directory. Must resolve to an existing directory and contain no NUL character.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "relPath",
                    desc: "The path to open, relative to `root`. Rejected if it is empty, absolute, contains a `..` component, or traverses a symbolic link at any component. A `.` component is passed through; `..b` and `...` are ordinary names. A relPath that names a directory, including `.`, raises `ErrInvalidPath`.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "mode",
                    desc: "The access mode. Optional; defaults to `\"read\"` when omitted. One of \
                           `\"read\"`/`\"r\"`, `\"write\"`/`\"w\"`, `\"readWrite\"`/`\"rw\"`, or \
                           `\"append\"`/`\"a\"`. Matched exactly and case sensitively.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Optional,
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
            body: Body::abi_function(lower_fs_open_within),
        }],
    });
}
