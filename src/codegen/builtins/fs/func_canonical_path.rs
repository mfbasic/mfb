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
as a new `String`; `path` itself is not changed. Resolution collapses `.` and
`..` components, removes redundant separators, and follows every symbolic link
along the way, including one at the final component, so the returned path names
the real file or directory with no indirection left in it. A relative `path` is
resolved against the current working directory. The result is always absolute
and never ends in a separator.

Because resolution walks the real directory tree rather than manipulating the
string alone, every component named by `path`, including the final one, must exist
on the filesystem; a missing component raises `ErrPathNotFound`. To normalize a
path lexically without touching the filesystem, use `fs::pathNormalize` instead.

`path` may contain Unicode characters when the host accepts such names, and the
spelling of each name is preserved in the result. An empty `path`, or one
containing a NUL character, raises `ErrInvalidArgument`.

The function never creates or modifies anything; it only looks the path up.

When `path` cannot be resolved the error says why: `ErrPathNotFound` if a
component does not exist, `ErrAccessDenied` if a directory on the way may not be
searched, and `ErrInvalidPath` if `path` cannot name an entry — a component
after a file, a name the host cannot use or that is too long, a loop of symbolic
links, or a resolved path longer than the host allows. Any other failure raises
`ErrReadFailed`. `ErrOutOfMemory` means the program ran out of memory."#;
const EX: &str = r#"Resolve a path to its real location. The file lives in a scratch directory
under the temporary directory and is removed afterwards:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-canonicalPath-1"])
  fs::createDirectories(dir)
  LET report AS String = fs::pathJoin([dir, "report.txt"])
  fs::writeText(report, "hello")
  io::print(fs::canonicalPath(report))
  fs::deleteFile(report)
  fs::deleteDirectory(dir)
END FUNC
```

Canonicalize a path containing `.` and `..` components:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET top AS String = fs::pathJoin([fs::tempDirectory(), "mfb-canonicalPath-2"])
  fs::createDirectories(fs::pathJoin([top, "a", "b"]))
  LET real AS String = fs::canonicalPath(top & "/a/./b/../b")
  io::print(toString(real = fs::canonicalPath(fs::pathJoin([top, "a", "b"]))))
  fs::deleteDirectory(fs::pathJoin([top, "a", "b"]))
  fs::deleteDirectory(fs::pathJoin([top, "a"]))
  fs::deleteDirectory(top)
END FUNC
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
                desc: "The path to canonicalize; absolute or relative to the current working directory. Every named component, including the last, must exist. Must be non-empty and contain no NUL character.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![
                "ErrOutOfMemory",
                "ErrAccessDenied",
                "ErrInvalidArgument",
                "ErrInvalidPath",
                "ErrPathNotFound",
                "ErrReadFailed",
            ],
            body: Body::abi_function(lower_fs_canonical_path),
        }],
    });
}
