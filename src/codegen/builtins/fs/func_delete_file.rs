//! `fs::deleteFile` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::deleteFile` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_delete_file(
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
            crate::codegen::engine::types::FsPathOperation::Unlink,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Remove a single file (or symlink) from the filesystem"#;
const DESC: &str = r#"`fs::deleteFile` removes the filesystem entry named by `path` with a single host
`unlink` operation. On success the entry is gone and the function returns
`Nothing`.

When the final component of `path` is a symbolic link, the link itself is removed
rather than the file it points to, because `unlink` does not follow a trailing
symlink. The function removes exactly one non-directory entry; it does not recurse
and it does not remove directories. Use `fs::deleteDirectory` to remove a
directory.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. It must be non-empty and must not contain an embedded NUL byte.

When the host refuses the removal `path` is left unchanged and the error says
why: `ErrPathNotFound` if nothing exists at `path`, `ErrAccessDenied` if
permission is refused, and `ErrInvalidPath` if `path` cannot name a file — it
names a directory, a name in it is too long, a file is used as a directory, or
it is a loop of symbolic links. A directory is refused rather than removed, so
use `fs::deleteDirectory` for one. Any other refusal raises `ErrWriteFailed`. An
empty `path`, or one containing a NUL byte, raises `ErrInvalidArgument`.
`ErrOutOfMemory` means the program ran out of memory."#;
const EX: &str = r#"Remove a generated output file:

```
IMPORT fs

FUNC main() AS Nothing
  fs::createDirectories("output")
  fs::writeText("output/report.txt", "generated")
  fs::deleteFile("output/report.txt")
END FUNC
```

Write a file and then remove it:

```
IMPORT fs

FUNC main() AS Nothing
  fs::writeText("scratch.txt", "temporary")
  fs::deleteFile("scratch.txt")
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "deleteFile",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the entry to remove, as UTF-8 bytes; absolute or \
                       relative to the current working directory. Must be non-empty and free of \
                       embedded NUL bytes. A trailing symlink is removed rather than followed.",
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
                "ErrInvalidArgument",
            ],
            body: Body::abi_function(lower_fs_delete_file),
        }],
    });
}
