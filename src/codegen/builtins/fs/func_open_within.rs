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
to open a caller-controlled name inside an intended directory with a host-enforced
guarantee that the result cannot escape that directory — closing the
time-of-check/time-of-use race that an `fs::isWithin(root, path)` check followed
by a separate `fs::open(path)` leaves open (bug-259 / OS-03).

Containment is enforced at open time. `root` is canonicalized once with `realpath`
(resolving the trusted root's own symbolic links); `relPath` is rejected if it is
absolute or contains a `..` component; the canonical root and `relPath` are joined;
and the join is opened with the same whole-path no-symlink resolution as
`fs::openFileNoFollow` — on Linux `openat2` carrying `RESOLVE_NO_SYMLINKS`, on
macOS `O_NOFOLLOW_ANY`. Because the canonical root is symlink-free and every
component is re-checked at open time, a component swapped to a symbolic link
*after* canonicalization is **rejected** rather than followed, so the open cannot
be redirected outside `root`.

A `relPath` is therefore refused when it is absolute, contains a `..` component,
or traverses a symbolic link at any component. `relPath` is always interpreted
relative to `root`, never to the process working directory.

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

`root` and `relPath` are interpreted as UTF-8 bytes and passed to the host
filesystem. `root` must resolve to an existing directory (it is canonicalized
with `realpath`), and neither string may be empty or contain an embedded NUL
byte. The returned `File` is closed by lexical drop when the binding that holds
it leaves scope, or explicitly with `fs::close`."#;
const EX: &str = r#"Open a caller-supplied name beneath a fixed root, for reading:

```
IMPORT fs

SUB main()
  LET userName AS String = "alice.txt"
  RES f AS fs::File = fs::openWithin("/srv/data", userName)
  fs::close(f)
END SUB
```

A `relPath` that tries to escape the root is refused rather than followed:

```
IMPORT fs
IMPORT errorCode
IMPORT io

SUB main()
  RES f AS fs::File = fs::openWithin("/srv/data", "../../etc/passwd") TRAP(e)
    io::print(toString(e.code = errorCode::ErrInvalidArgument))
    EXIT SUB
  END TRAP
END SUB
```

Write beneath the root; a symlinked component makes the open fail:

```
IMPORT fs

SUB main()
  RES w AS fs::File = fs::openWithin("/srv/data", "reports/today.txt", "write")
  fs::writeAll(w, "hello")
  fs::close(w)
END SUB
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
                    desc: "The trusted base directory. Canonicalized with `realpath` (its own \
                           symlinks are resolved); must resolve to an existing directory and be \
                           free of embedded NUL bytes.",
                    aliases: &[],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "relPath",
                    desc: "The path to open, relative to `root`. Rejected if it is empty, \
                           absolute, contains a `..` component, or traverses a symbolic link at \
                           any component.",
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
            errors: vec![],
            body: Body::abi_function(lower_fs_open_within),
        }],
    });
}
