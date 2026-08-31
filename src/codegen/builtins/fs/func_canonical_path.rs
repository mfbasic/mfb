//! `fs::canonicalPath` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::canonicalPath` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_canonical_path(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_canonical::lower_fs_canonical_path_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"Resolve a path to its canonical absolute path"#;
const DESC: &str = r#"`fs::canonicalPath` resolves `path` to an absolute, canonical path and returns it
as a `String`. Resolution is performed by the host `realpath` call, which collapses
`.` and `..` components, removes redundant separators, and follows every symbolic
link encountered along the way, so the returned path names the real file or
directory with no indirection left in it. A relative `path` is resolved against the
current working directory; an absolute `path` is canonicalized in place.

Because resolution walks the real directory tree rather than manipulating the
string alone, every component named by `path`, including the final one, must exist
on the filesystem; a missing component raises an error. To normalize a path
lexically without touching the filesystem, use `fs::pathNormalize` instead.

`path` is interpreted as raw UTF-8 bytes and passed to the host filesystem. It may
contain Unicode characters when the host accepts such names, and the byte-oriented
spelling of the name is preserved in the result. The string must not be empty and
must not contain an embedded NUL byte, because the host call requires a
NUL-terminated path; either condition raises `ErrInvalidArgument` before any host
call is made. The result is copied into an arena-backed `String` with the host
resolution buffer sized to hold up to `PATH_MAX` bytes plus the terminating NUL
(`4097`).

The function reads filesystem metadata only; it does not open, create, or modify
any file and has no other side effects."#;
const EX: &str = r#"Resolve a relative path against the working directory:

```
IMPORT fs
IMPORT io

SUB main()
  fs::createDirectories("output")
  fs::writeText("output/report.txt", "hello")
  LET full AS String = fs::canonicalPath("output/report.txt")
  io::print(full)
END SUB
```

Canonicalize a path containing `.` and `..` components:

```
IMPORT fs
IMPORT io

SUB main()
  fs::createDirectories("a/b")
  LET real AS String = fs::canonicalPath("a/./b/../b")
  io::print(real)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "canonicalPath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The path to canonicalize, as UTF-8 bytes; absolute or relative to the \
                       current working directory. Every named component, including the last, must \
                       exist. Must be non-empty and free of embedded NUL bytes.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_function(lower_fs_canonical_path),
        }],
    });
}
