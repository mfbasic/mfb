//! `fs::directoryExists` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::directoryExists` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_directory_exists(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_exists::lower_fs_kind_exists_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        crate::codegen::error::constants::FS_MODE_DIRECTORY,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Test whether a path names an existing directory"#;
const DESC: &str = r#"`fs::directoryExists` looks `path` up on the filesystem and reports whether it is
a directory. It returns `TRUE` only when `path` exists and the resolved entry is a
directory; it returns `FALSE` for a missing path, a regular file, or any other
non-directory entry (symlink to a missing target, socket, FIFO, or device node).
Use `fs::exists` to accept any kind of entry, or `fs::fileExists` to require a
regular file.

A symlink in the final position is followed: a link to a directory reports
`TRUE`, and a link whose target is missing or is not a directory reports
`FALSE`. No shell expansion is applied: `~` is a literal character, not the
home directory.

When the lookup fails — the path is missing, or a directory on the way to it
cannot be traversed — the result is `FALSE`; no error is raised. A directory
you have no permission to read still reports `TRUE`, because only its existence
is tested. The call itself raises only `ErrInvalidArgument`, when `path`
contains a NUL character, and `ErrOutOfMemory`, when the program runs out of
memory. An empty `path` is not an error: nothing exists there, so the result is
`FALSE`.

`path` may be absolute or relative to the current working directory, and may
contain Unicode characters (including emoji) when the host filesystem accepts
those names. The call reads filesystem state only and has no side effects."#;
const EX: &str = r#"Test for a directory before listing it:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET data AS String = fs::pathJoin([fs::tempDirectory(), "mfb-directoryExists-1"])
  io::print(toString(fs::directoryExists(data)))
  fs::createDirectory(data)
  IF fs::directoryExists(data) THEN
    io::print("found, " & toString(len(fs::listDirectory(data))) & " entries")
  END IF
  fs::deleteDirectory(data)
END FUNC
```

Unicode paths are accepted. This prints `TRUE`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-directoryExists-é日😀"])
  fs::createDirectory(dir)
  io::print(toString(fs::directoryExists(dir)))
  fs::deleteDirectory(dir)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "directoryExists",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path to test; absolute or relative to the current working directory. An empty path is `FALSE`; a path containing a NUL character raises `ErrInvalidArgument`.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec!["ErrOutOfMemory", "ErrInvalidArgument"],
            body: Body::abi_function(lower_fs_directory_exists),
        }],
    });
}
