//! `fs::fileExists` — descriptor + docs.
//!
//! Native syscall member: its `Body::abi_function` body delegates to the shared
//! family-generic OS-seam dispatcher `native::lower_fs_os_seam` (which branches to
//! the relocated `lower_fs_kind_exists_helper`).

use super::native::lower_fs_os_seam;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::AbiCtx;
use crate::types::ParameterType;

/// `abi_function` body for `fs::fileExists` — the shared OS-seam dispatcher
/// [`super::native::lower_fs_os_seam`], selected by runtime-call name (crypto/io's
/// clean-room shape); the `abi_function` wrapper finalizes it.
pub(crate) fn lower_fs_file_exists(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_fs_os_seam(builder, ctx, "fs.fileExists")
}

const INTRO: &str = r#"Test whether a path names an existing regular file"#;
const DESC: &str = r#"`fs::fileExists` stats `path` and reports whether it resolves to a regular file.
It returns `TRUE` only when `path` exists and the resolved entry is a regular
file; it returns `FALSE` for a missing path, a directory, or any other
non-regular entry (symlink to a missing target, socket, FIFO, or device node).
The check masks the entry's mode with the file-type bits (`61440`) and compares
against the regular-file type (`32768`), so only regular files qualify.

The final path component is followed when it is a symlink, because the host
`stat` call is used rather than `lstat`: a symlink pointing at a regular file
reports `TRUE`, and a symlink whose target is missing or non-regular reports
`FALSE`.

A failed `stat` — for example a missing path or an unreadable parent directory —
is reported as `FALSE` rather than raised as an error. The only failure the call
itself raises is an allocation failure while preparing the NUL-terminated copy of
`path`.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem; it may be
absolute or relative to the current working directory, and may contain Unicode
characters (including emoji) when the host filesystem accepts those names. The
call reads filesystem state only and has no side effects."#;
const EX: &str = r#"Test for a regular file before reading it:

```
IMPORT fs
IMPORT io

SUB main()
  IF fs::fileExists("data.txt") THEN
    io::print("found")
  END IF
END SUB
```

Unicode paths are accepted:

```
IMPORT fs

SUB main()
  LET present AS Boolean = fs::fileExists("é日😀.txt")
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "fileExists",
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
            body: Body::abi_function(lower_fs_file_exists),
        }],
    });
}
