//! `fs::deleteFile` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper` (which branches to
//! the relocated `lower_fs_path_operation_helper`). Docs migrated from
//! `src/docs/man/builtins/fs/deleteFile.md`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Remove a single file (or symlink) from the filesystem"#;
const DESC: &str = r#"`fs::deleteFile` removes the filesystem entry named by `path` with a single host
`unlink` operation. On success the entry is gone and the function returns
`Nothing`.

When the final component of `path` is a symbolic link, the link itself is removed
rather than the file it points to, because `unlink` does not follow a trailing
symlink. The function removes exactly one non-directory entry; it does not recurse
and it does not remove directories. Use `fs::deleteDirectory` to remove a
directory.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. Internally a
NUL-terminated copy of `path` is allocated for the host call, so `path` must be
non-empty and must not contain an embedded NUL byte.

When the host refuses the removal, the failure `errno` is mapped to the matching
error below and `path` is left unchanged. Attempting to remove a directory is
reported as a host failure (for example `ErrInvalidPath` or `ErrDirectoryNotEmpty`)
rather than as a directory-specific error, since `unlink` does not operate on
directories. `errno` values are per-OS; the same symbolic error is produced on
each platform."#;
const EX: &str = r#"Remove a generated output file:

```
IMPORT fs

SUB main()
  fs::deleteFile("target/output.txt")
END SUB
```

Write a file and then remove it:

```
IMPORT fs

SUB main()
  fs::writeText("scratch.txt", "temporary")
  fs::deleteFile("scratch.txt")
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "deleteFile",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the entry to remove, as UTF-8 bytes; absolute or \
                       relative to the current working directory. Must be non-empty and free of \
                       embedded NUL bytes. A trailing symlink is removed rather than followed.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::Nothing,
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
