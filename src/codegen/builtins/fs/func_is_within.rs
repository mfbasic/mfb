//! `fs::isWithin` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `gen_os_seam::lower_fs_os_seam`.

use super::gen_os_seam::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::isWithin` — the shared OS-seam dispatcher
/// [`super::gen_os_seam::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_is_within(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.isWithin")
}

const INTRO: &str = r#"Test whether one path is contained within another"#;
const DESC: &str = r#"`fs::isWithin` canonicalizes both `base` and `child` with the host `realpath`
resolution, then reports whether `child` names the same location as `base` or a
location nested below it. It returns `TRUE` when the canonical `child` path
equals the canonical `base` path, or when it begins with the canonical `base`
path followed by a path separator; it returns `FALSE` otherwise.

Canonicalization collapses `.` and `..` components, removes redundant
separators, and follows every symbolic link in both paths, resolving each
relative argument against the current working directory. The comparison
therefore reflects the real on-disk locations of the two paths rather than the
literal text of either argument, so symlink indirection and `..` traversal
cannot be used to make a path appear contained when it is not, nor to hide
genuine containment.

The containment test is path-boundary aware: it matches only at separator
boundaries, so `base` contains `base/nested/file.txt` and equals `base`, but a
sibling such as `base2` is reported as not within `base` even though its
canonical text shares the `base` prefix. When the canonical `base` is the
filesystem root (`/`), every canonical `child` is within it.

Because canonicalization walks the real directory tree, every component of both
`base` and `child`, including the final one, must exist on the filesystem. Each
argument is interpreted as raw UTF-8 bytes and passed to the host; an argument
may contain Unicode characters when the host accepts such names, but it must not
be empty and must not contain an embedded NUL byte. The function reads
filesystem metadata only; it does not open, create, or modify any file and has
no other side effects.

This check is inherently subject to a time-of-check/time-of-use race: a
component of either path can be swapped for a symbolic link after `isWithin`
returns but before a later `fs::open` acts on the result. When the goal is to
open a caller-supplied name that cannot escape a trusted root, use
`fs::openWithin`, which enforces containment atomically at open time
(bug-259 / OS-03)."#;
const EX: &str = r#"Guard against escaping a root directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET root AS String = fs::canonicalPath("uploads")
  LET candidate AS String = fs::canonicalPath("uploads/report.txt")
  IF fs::isWithin(root, candidate) THEN
    io::print("inside")
  END IF
END SUB
```

A nested file is within its base directory:

```
IMPORT fs

SUB main()
  fs::createDirectories("base/nested")
  fs::writeText("base/nested/file.txt", "hi")
  LET inside AS Boolean = fs::isWithin("base", "base/nested/file.txt")
END SUB
```

A path is within itself, but a sibling is not:

```
IMPORT fs

SUB main()
  LET same AS Boolean = fs::isWithin("base", "base")
  LET sibling AS Boolean = fs::isWithin("base", "base2")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "isWithin",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String, String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "base",
                    desc: "The containing path (also accepted as `path`). Interpreted as UTF-8 \
                           bytes; may be absolute or relative to the current working directory. \
                           Every named component, including the last, must exist. Must be \
                           non-empty and free of embedded NUL bytes.",
                    aliases: &["path"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "child",
                    desc: "The path tested for containment (also accepted as `parent`). \
                           Interpreted as UTF-8 bytes; may be absolute or relative to the current \
                           working directory. Every named component, including the last, must \
                           exist. Must be non-empty and free of embedded NUL bytes.",
                    aliases: &["parent"],
                    ty: ParameterType::String,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Boolean,
            errors: vec![],
            body: Body::abi_function(lower_fs_is_within),
        }],
    });
}
