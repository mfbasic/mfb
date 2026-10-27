//! `fs::listDirectory` — descriptor + docs.
//!
//! Native syscall member: it owns its `Body::abi_function` body, which calls its
//! per-member emitter in the `gen_*` backends and finalizes.

use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::listDirectory` — calls its per-member `lower_fs_*_helper` emitter and finalizes
/// (crypto/io's clean-room shape).
pub(crate) fn lower_fs_list_directory(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    let symbol = builder.current_symbol.clone();
    let (instructions, relocations, stack_size) =
        super::gen_directory::lower_fs_list_directory_helper(
            &symbol,
            ctx.platform_imports,
            ctx.platform,
        )?;
    builder.instructions.extend(instructions);
    builder.relocations.extend(relocations);
    builder.stack_size = stack_size;
    Ok(super::gen_shared::void_result(ctx.call))
}

const INTRO: &str = r#"List the direct child names of a directory"#;
const DESC: &str = r#"`fs::listDirectory` opens the directory named by `path`, reads every entry it
contains directly, and returns those entry names as a `List OF String`. The list
holds the entry names only, not full paths, and the special `"."` (current
directory) and `".."` (parent directory) entries are always filtered out, so
they never appear in the result.

Only the immediate children of the directory are listed; `fs::listDirectory`
does not descend into subdirectories. Every kind of entry is included regardless
of type, so the result mixes regular files, subdirectories, symlinks, and any
other filesystem objects, each represented by its name with no trailing slash or
type marker. Names beginning with a dot (hidden files on Unix hosts) are
included like any other; only `"."` and `".."` are left out.

The names are sorted in ascending byte-wise order of their UTF-8 bytes, not
alphabetically: the sort is case-sensitive, every uppercase ASCII name comes
before every lowercase one (`B`, `Z`, `a`), and names with non-ASCII characters
come after all ASCII names. The order is the same on every run and every host. An empty directory, or a
directory that contains only `"."` and `".."`, yields an empty `List`.

If another program is changing the directory while it is being listed, the
result is a snapshot that may miss entries added during the call; the call does
not fail because of that. A symlink in the final position is followed, so
listing through a symlink that points at a directory lists the target
directory's entries.

`path` may be absolute or relative to the current working directory and may
contain Unicode characters, including emoji, when the host filesystem accepts
those names. It must not be empty and must not contain a NUL character. The call
only reads the filesystem and has no side effects.

An empty `path`, or one containing a NUL character, raises `ErrInvalidArgument`. When
the directory cannot be opened the error says why: `ErrPathNotFound` if `path`
does not exist, `ErrAccessDenied` if it may not be read, and `ErrInvalidPath` if
`path` cannot name a directory — it names a file, a name in it is too long, a file
is used as a directory, or a symlink in it loops back on itself. Any other refusal raises `ErrReadFailed`.
`ErrOutOfMemory` means the program ran out of memory.

Guard the call with `fs::directoryExists`, and build each entry's full path with
`fs::pathJoin`."#;
const EX: &str = r#"Print every entry in a directory in sorted order:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET dir AS String = fs::pathJoin([fs::tempDirectory(), "mfb-listDirectory-1"])
  fs::createDirectories(dir)
  fs::writeText(fs::pathJoin([dir, "b.txt"]), "b")
  fs::writeText(fs::pathJoin([dir, "a.txt"]), "a")
  FOR EACH name IN fs::listDirectory(dir)
    io::print(name)
    fs::deleteFile(fs::pathJoin([dir, name]))
  NEXT
  fs::deleteDirectory(dir)
END FUNC
```

An empty directory yields an empty `List`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  LET empty AS String = fs::pathJoin([fs::tempDirectory(), "mfb-listDirectory-2"])
  fs::createDirectories(empty)
  LET names AS List OF String = fs::listDirectory(empty)
  io::print(toString(len(names)))
  fs::deleteDirectory(empty)
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "listDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the directory to list; absolute or relative to \
                       the current working directory. Must be non-empty and contain no NUL \
                       character, and must name an existing, readable directory.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![
                "ErrOutOfMemory",
                "ErrAccessDenied",
                "ErrInvalidArgument",
                "ErrInvalidPath",
                "ErrPathNotFound",
                "ErrReadFailed",
            ],
            body: Body::abi_function(lower_fs_list_directory),
        }],
    });
}
