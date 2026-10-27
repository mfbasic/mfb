//! `fs::currentDirectory` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member `lower_fs_*_helper` emitter (in the `gen_*` backends) and finalizes.

use super::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::currentDirectory` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_current_directory(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_directory::lower_fs_current_directory_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Return the process's current working directory"#;
const DESC: &str = r#"`fs::currentDirectory` returns the absolute current working directory of the
running process as a UTF-8 `String`.

The directory is read from the operating system on every call, never cached, so
the result reflects the working directory at that moment, including any change
made by `fs::setCurrentDirectory`. The returned path is absolute, uses the
host's native separators, and has every symbolic link resolved: a program
started in `/tmp/work` on macOS sees `/private/tmp/work`, so the result may
differ from the shell's `PWD`. Compare paths with `fs::canonicalPath` rather
than by text.

The working directory is the base against which any relative path passed to
other `fs` functions is resolved, so this value names the directory used by
`fs::canonicalPath`, `fs::open`, `fs::readText`, and the rest of the package
when they are given a path that is not absolute. The current directory can be
changed with `fs::setCurrentDirectory`.

The call has no filesystem side effects: it creates, opens, and modifies
nothing.

`ErrReadFailed` is raised when the working directory can no longer be reported:
it has been deleted since the process entered it, or its absolute
path is 4096 bytes or longer. The call never returns a partial or empty path.
`ErrOutOfMemory` means the program ran out of memory."#;
const EX: &str = r#"Read and print the current working directory:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET cwd AS String = fs::currentDirectory()
  io::print(cwd)
END FUNC
```

Resolve a relative path against the working directory:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET cwd AS String = fs::currentDirectory()
  LET full AS String = fs::pathJoin([cwd, "output.txt"])
  io::print(full)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "currentDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec!["ErrOutOfMemory", "ErrReadFailed"],
            body: Body::abi_function(lower_fs_current_directory),
        }],
    });
}
