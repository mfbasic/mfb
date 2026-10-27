//! `fs::fileExists` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::fileExists` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_file_exists(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) = super::gen_exists::lower_fs_kind_exists_helper(
        &symbol,
        ctx.platform_imports,
        ctx.platform,
        crate::codegen::error::constants::FS_MODE_REGULAR,
    )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Test whether a path names an existing regular file"#;
const DESC: &str = r#"`fs::fileExists` looks `path` up and reports whether it names a regular file.
It returns `TRUE` only when `path` exists and the resolved entry is a regular
file; it returns `FALSE` for a missing path, a directory, or any other
non-regular entry (symlink to a missing target, socket, FIFO, or device node).
Use `fs::exists` to accept any kind of entry, or `fs::directoryExists` to
require a directory.

A symlink as the final path component is followed: a symlink pointing at a
regular file reports `TRUE`, and a symlink whose target is missing or
non-regular reports `FALSE`. A trailing `/` makes the path name a directory, so
`fs::fileExists("notes.txt/")` is `FALSE` even when `notes.txt` is a regular
file. No shell expansion is applied: `~` is a literal character, not the home
directory.

A failed lookup — a missing path, or a parent directory the program is not
permitted to traverse — is reported as `FALSE` rather than raised as an error.
The call itself raises only `ErrInvalidArgument`, when `path` contains a NUL
character, and `ErrOutOfMemory`, when the program runs out of memory. An empty
`path` is not an error: nothing exists there, so the result is `FALSE`.

`path` may be absolute or relative to the current working directory, and may
contain Unicode characters (including emoji) when the host filesystem accepts
those names. The call reads filesystem state only and has no side effects."#;
const EX: &str = r#"Test for a regular file before reading it:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET data AS String = fs::pathJoin([fs::tempDirectory(), "mfb-fileExists-1.txt"])
  fs::writeText(data, "hello")
  IF fs::fileExists(data) THEN
    io::print(fs::readText(data))
  END IF
  fs::deleteFile(data)
END FUNC
```

Unicode paths are accepted. This prints `TRUE`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET name AS String = fs::pathJoin([fs::tempDirectory(), "mfb-fileExists-é日😀.txt"])
  fs::writeText(name, "")
  io::print(toString(fs::fileExists(name)))
  fs::deleteFile(name)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fileExists",
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
            body: Body::abi_function(lower_fs_file_exists),
        }],
    });
}
