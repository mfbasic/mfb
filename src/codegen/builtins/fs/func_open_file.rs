//! `fs::openFile` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes. Returns a `File`
//! resource; `mode` defaults to `"read"`.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::openFile` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_open_file(
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

const INTRO: &str = r#"Open a file resource, defaulting to read mode when no mode is given"#;
const DESC: &str = r#"`fs::openFile` opens the file named by `path` and returns an opaque `File`
resource that later `fs::` calls read from, write to, and close. The `mode`
argument is optional: when it is omitted the file is opened for reading, exactly
as if `"read"` had been supplied. This is the only difference from `fs::open`,
which requires `mode` to be given explicitly; otherwise the two functions behave
identically.

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
user-only `0600` permission bits (subject to the process umask), not
world-readable `0666`, matching `fs::createTempFile` and the atomic writers.

The final path component is followed when it is a symlink, so opening through a
symlink opens its target. To refuse a symlinked final component, use
`fs::openFileNoFollow` instead.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory and may contain Unicode
characters when the host filesystem accepts those names. The string must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.

The returned `File` is closed when the binding that holds it goes out of scope, or explicitly with `fs::close`. The function reads or writes no
file contents itself; it only opens the file and gives you the `File`
resource."#;
const EX: &str = r#"Open a file for reading using the default mode:

```
IMPORT fs

SUB main()
  fs::writeText("data.txt", "first line\nsecond line\n")
  RES f AS fs::File = fs::openFile("data.txt")
  fs::close(f)
END SUB
```

Open a file for writing, truncating any previous contents:

```
IMPORT fs

SUB main()
  fs::writeText("out.txt", "first line\nsecond line\n")
  RES w AS fs::File = fs::openFile("out.txt", "write")
  fs::writeAll(w, "hello")
  fs::close(w)
END SUB
```

Open a file for appending so each write lands at the end:

```
IMPORT fs

SUB main()
  fs::writeText("app.log", "first line\nsecond line\n")
  RES log AS fs::File = fs::openFile("app.log", "a")
  fs::writeAll(log, "started\n")
  fs::close(log)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "openFile",
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
                           of embedded NUL bytes.",
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
            return_type: ParameterType::named(super::FILE_TYPE_ID),
            errors: vec![],
            body: Body::abi_function(lower_fs_open_file),
        }],
    });
}
