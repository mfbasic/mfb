//! `fs::canonicalPath` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper` (which branches to
//! the relocated `lower_fs_canonical_path_helper`).

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

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
  LET full AS String = fs::canonicalPath("target/output.txt")
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

pub(super) fn register(pkg: &mut RegistryPackage) {
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
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
