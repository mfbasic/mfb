//! `fs::createDirectories` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::createDirectories` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_create_directories(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_directory::lower_fs_create_directories_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Create a directory together with any missing parent directories"#;
const DESC: &str = r#"`fs::createDirectories` creates the directory named by `path` along with any
missing parent directories, like `mkdir -p`, and returns `Nothing` on success.

`path` is scanned left to right and each `/`-separated prefix is created in turn
before the final component is created. A leading `/` is skipped so the filesystem
root is not treated as a component to create. Each prefix, and the final
component, is created in turn; one that already exists is accepted and the walk
continues. As a result, existing intermediate directories and a final `path` that already exists
as a directory all succeed quietly rather than being treated as errors, which
makes `fs::createDirectories` idempotent: re-running it on a path that is already
present succeeds without changing anything.

Unlike `fs::createDirectory`, which creates only the final component and fails
when a parent is missing, `fs::createDirectories` builds the entire chain of
missing parents. Each directory is requested with permission bits `0755`
(`rwxr-xr-x`), which the host masks with the process umask in the usual way, so
each directory's actual mode is `0755` with the umask bits cleared.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. Each prefix is created in turn, so `path` must be non-empty and must not contain an embedded NUL byte.

When the host refuses to create a prefix or the final component for any reason
other than "it already exists", the walk stops there and raises the matching
error below — leaving the directories it had already created. A missing parent
and a permission refusal get their own errors; every other refusal is reported
as `ErrOutput`. The same error is raised on every platform."#;
const EX: &str = r#"Create a nested directory together with its missing parents:

```
IMPORT fs

SUB main()
  fs::createDirectories("target/example/nested")
END SUB
```

Re-running is safe because existing directories are accepted:

```
IMPORT fs

SUB main()
  fs::createDirectories("target/example/nested")
  fs::createDirectories("target/example/nested")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "createDirectories",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the directory to create, including any parents that \
                       must be created first, as UTF-8 bytes; absolute or relative to the current \
                       working directory. Must be non-empty and free of embedded NUL bytes. Every \
                       `/`-separated component is created in order, and components that already \
                       exist as directories are accepted.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::abi_function(lower_fs_create_directories),
        }],
    });
}
