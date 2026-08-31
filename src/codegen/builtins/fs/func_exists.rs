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
node — and `FALSE` when nothing exists at `path`. The check is implemented with
the host `access` call using the existence mode (`F_OK`, `0`); `access` returning
`0` maps to `TRUE` and any nonzero result maps to `FALSE`.

The final path component is followed when it is a symlink, because `access`
dereferences the last component: a symlink pointing at an existing target reports
`TRUE`, and a symlink whose target is missing reports `FALSE`.

A failed check — for example a missing path or an unreadable parent directory — is
reported as `FALSE` rather than raised as an error. The only failure the call
itself raises is running out of memory while preparing the path of
`path`.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem; it may be
absolute or relative to the current working directory, and may contain Unicode
characters (including emoji) when the host filesystem accepts those names. The
call reads filesystem state only and has no side effects."#;
const EX: &str = r#"Test for any entry at a path before acting on it:

```
IMPORT fs
IMPORT io

SUB main()
  IF fs::exists("data.txt") THEN
    io::print("found")
  END IF
END SUB
```

Unicode paths are accepted:

```
IMPORT fs

SUB main()
  LET present AS Boolean = fs::exists("é日😀.txt")
END SUB
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
                desc: "The filesystem path to test, as UTF-8 bytes; absolute or relative to the \
                       current working directory.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_fs_exists),
        }],
    });
}
