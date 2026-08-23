//! `fs::openFileNoFollow` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `native::lower_fs_os_seam`. Returns a `File`
//! resource; `mode` defaults to `"read"`.

use super::native::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::openFileNoFollow` — the shared OS-seam dispatcher
/// [`super::native::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_open_file_no_follow(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.openFileNoFollow")
}

const INTRO: &str = r#"Open a file, refusing to traverse a symbolic link at any path component"#;
const DESC: &str = r#"`fs::openFileNoFollow` opens the file named by `path` and returns an opaque
`File` resource that later `fs::` calls read from, write to, and close. It
behaves exactly like `fs::openFile` except that it refuses to traverse a
symbolic link at *any* component of `path`, not just the terminal name: if any
component — an intermediate directory or the final name — is a symbolic link,
the open is refused rather than resolving through the link.

The whole-path guarantee is enforced by the host in a single operation. On Linux
the path is resolved with `openat2` carrying `RESOLVE_NO_SYMLINKS`; on macOS the
open uses `O_NOFOLLOW_ANY`, which fails if a symlink is encountered at any
component (bug-260 / OS-04). On a Linux kernel too old for `openat2` (`ENOSYS`,
pre-5.6, or a restrictive seccomp filter) it falls back to a plain `open` with
`O_NOFOLLOW`, which refuses only a symlinked *final* component.

This is useful for safely opening a file whose path you control without being
silently redirected through a symlink that may have been swapped in along the
path.

The `mode` argument is optional: when it is omitted the file is opened for
reading, exactly as if `"read"` had been supplied. The implicit `"read"` is
appended before lowering, matching `fs::openFile`.

`mode` selects how the file is opened. The portable mode names are `"read"` or
`"r"`, `"write"` or `"w"`, `"readWrite"` or `"rw"`, and `"append"` or `"a"`.
`"read"` opens an existing file for reading only and creates nothing. `"write"`
opens the file for writing, creating it when it does not exist and truncating it
to empty when it does. `"readWrite"` opens the file for both reading and writing,
creating it when it does not exist but preserving existing contents. `"append"`
opens the file for writing with every write directed to the end of the file,
creating it when it does not exist. The mode string is matched exactly, byte for
byte, and is case sensitive; any other value is rejected before the file is
touched.

Files created by a `write`, `readWrite`, or `append` open are created with
owner-only `0600` permission bits (subject to the process umask), not
world-readable `0666`, matching `fs::createTempFile` and the atomic writers
(audit-2 OS-01 / bug-184).

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.

The returned `File` is closed by lexical drop when the binding that holds it
leaves scope, or explicitly with `fs::close`. The function reads or writes no
file contents itself; it only opens the descriptor and wraps it in the `File`
resource."#;
const EX: &str = r#"Open a file for reading using the default mode, refusing a symlinked path:

```
IMPORT fs

SUB main()
  RES f AS fs::File = fs::openFileNoFollow("data.txt")
  fs::close(f)
END SUB
```

Open a file for writing; the open fails if any component of the path is a symlink:

```
IMPORT fs

SUB main()
  RES w AS fs::File = fs::openFileNoFollow("out.txt", "write")
  fs::writeAll(w, "hello")
  fs::close(w)
END SUB
```

Open a file for appending so each write lands at the end:

```
IMPORT fs

SUB main()
  RES log AS fs::File = fs::openFileNoFollow("app.log", "a")
  fs::writeAll(log, "started\n")
  fs::close(log)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "openFileNoFollow",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String[, String]"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "path",
                    desc: "The filesystem path of the file to open, as UTF-8 bytes; absolute or \
                           relative to the current working directory. Must be non-empty and free \
                           of embedded NUL bytes. No component of the path may be a symbolic link.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "mode",
                    desc: "The access mode. Optional; defaults to `\"read\"` when omitted. One of \
                           `\"read\"`/`\"r\"` (read existing file), `\"write\"`/`\"w\"` (create \
                           or truncate for writing), `\"readWrite\"`/`\"rw\"` (create-if-absent \
                           for reading and writing, preserving contents), or \
                           `\"append\"`/`\"a\"` (create-if-absent for writing at end of file). \
                           Matched exactly and case sensitively.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::Optional,
                },
            ],
            return_type: ParameterType::Named(super::FILE_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_fs_open_file_no_follow),
        }],
    });
}
