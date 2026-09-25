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

When the host refuses the removal the filesystem is left unchanged and the
error says why: `ErrPathNotFound` if nothing exists at `path`, `ErrResourceBusy`
if the directory is not empty, `ErrAccessDenied` if permission is refused, and
`ErrInvalidPath` if `path` names a file or cannot name a directory — a name
that is too long, or a loop of symbolic links. Windows reports a file as
`ErrWriteFailed`, and most unusable paths as `ErrPathNotFound`. Any other
refusal raises `ErrWriteFailed`. An empty `path`, or one containing a NUL
byte, raises `ErrInvalidArgument`. `ErrOutOfMemory` means the program ran out of memory. The table also lists
`ErrAlreadyExists` because every `fs` function that takes a path reports host
refusals through one shared set of errors; removing a directory never
produces it."#;
const EX: &str = r#"Remove an empty directory:

```
IMPORT fs

SUB main()
  fs::createDirectories("scratch/example")
  fs::deleteDirectory("scratch/example")
END SUB
```

Create a directory and then remove it:

```
IMPORT fs

SUB main()
  fs::createDirectories("scratch/cache")
  fs::deleteDirectory("scratch/cache")
END SUB
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
                "ErrAlreadyExists",
            ],
            body: Body::abi_function(lower_fs_delete_directory),
        }],
    });
}
