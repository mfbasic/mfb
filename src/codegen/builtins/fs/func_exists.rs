//! `fs::exists` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::exists` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_exists(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_exists::lower_fs_exists_helper(&symbol, ctx.platform_imports, ctx.platform)?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Test whether any filesystem entry exists at a path"#;
const DESC: &str = r#"`fs::exists` checks `path` and reports whether any filesystem entry is present
there, regardless of its type. It returns `TRUE` when an entry exists — a regular
file, a directory, a symlink to an existing target, a socket, a FIFO, or a device
node — and `FALSE` when nothing exists at `path`. Use `fs::fileExists` or
`fs::directoryExists` to require one kind of entry.

A symlink is followed: one pointing at an existing target reports `TRUE`, and one
whose target is missing reports `FALSE`. A trailing path separator asks for a
directory: `fs::exists("notes.txt/")` is `FALSE` even when the file `notes.txt`
exists, while a directory reports `TRUE` with or without the separator. No shell
expansion is applied: `~` is a literal character, not the home directory.

A failed check — for example a missing path, or a parent directory you have no
permission to search — is reported as `FALSE` rather than raised as an error.
The call itself raises only `ErrInvalidArgument`, when `path` contains a NUL
character, and `ErrOutOfMemory`, when the program runs out of memory. An empty
`path` is not an error: nothing exists there, so the result is `FALSE`.

`path` may be absolute or relative to the current working directory, and may
contain Unicode characters (including emoji) when the host filesystem accepts
those names. The call reads filesystem state only and has no side effects."#;
const EX: &str = r#"Test for any entry at a path before acting on it:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET data AS String = fs::pathJoin([fs::tempDirectory(), "mfb-exists-1.txt"])
  fs::writeText(data, "hello")
  IF fs::exists(data) THEN
    io::print("found")
  END IF
  fs::deleteFile(data)
END FUNC
```

Unicode paths are accepted. This prints `TRUE`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET name AS String = fs::pathJoin([fs::tempDirectory(), "mfb-exists-é日😀.txt"])
  fs::writeText(name, "")
  io::print(toString(fs::exists(name)))
  fs::deleteFile(name)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "exists",
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
            body: Body::abi_function(lower_fs_exists),
        }],
    });
}
