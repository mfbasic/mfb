//! `fs::deleteDirectory` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::deleteDirectory` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_delete_directory(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_directory::lower_fs_path_operation_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
            crate::codegen::engine::types::FsPathOperation::Rmdir,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Remove an empty directory from the filesystem"#;
const DESC: &str = r#"`fs::deleteDirectory` removes the empty directory named by `path` with a single
host `rmdir` operation. On success the directory is gone and the function returns
`Nothing`.

The final component of `path` must name an actual directory, and that directory
must be empty. `fs::deleteDirectory` does not recurse and never removes a file or
a symbolic link; use `fs::deleteFile` to remove a non-directory entry. A directory
that still contains entries is left untouched and the call fails with
`ErrResourceBusy`. Only the named directory is removed; parent directories are
left in place.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. It must be non-empty and must not contain an embedded NUL byte.

When the host refuses the removal the filesystem is left unchanged and the error
says why: `ErrPathNotFound` if nothing exists at `path`, `ErrResourceBusy` if
the directory is not empty, `ErrAccessDenied` if permission is refused, and
`ErrInvalidPath` if `path` names a file or cannot name a directory — a name that
is too long, a file used as a directory, or a loop of symbolic links. Any other
refusal raises `ErrWriteFailed`. An empty `path`, or one containing a NUL byte,
raises `ErrInvalidArgument`. `ErrOutOfMemory` means the program ran out of
memory."#;
const EX: &str = r#"Remove an empty directory:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-deleteDirectory-1"])
  fs::createDirectory(dir)
  fs::deleteDirectory(dir)
  io::print(toString(fs::directoryExists(dir)))
END FUNC
```

A directory must be emptied before it can be removed, and its parents are left
in place:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET outer AS String = fs::pathJoin([fs::tempDirectory(), "mfb-deleteDirectory-2"])
  LET inner AS String = fs::pathJoin([outer, "cache"])
  fs::createDirectories(inner)
  fs::writeText(fs::pathJoin([inner, "entry.txt"]), "x")
  fs::deleteFile(fs::pathJoin([inner, "entry.txt"]))
  fs::deleteDirectory(inner)
  io::print(toString(fs::directoryExists(outer)))
  fs::deleteDirectory(outer)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "deleteDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the directory to remove, as UTF-8 bytes; absolute \
                       or relative to the current working directory. Must be non-empty and free \
                       of embedded NUL bytes. The final component must name an existing, empty \
                       directory rather than a file or symbolic link.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![
                "ErrOutOfMemory",
                "ErrWriteFailed",
                "ErrPathNotFound",
                "ErrInvalidPath",
                "ErrAccessDenied",
                "ErrResourceBusy",
                "ErrInvalidArgument",
            ],
            body: Body::abi_function(lower_fs_delete_directory),
        }],
    });
}
