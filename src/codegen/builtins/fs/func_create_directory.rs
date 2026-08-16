//! `fs::createDirectory` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper` (which branches to
//! the relocated `lower_fs_path_operation_helper`). Docs migrated from
//! `src/docs/man/builtins/fs/createDirectory.md`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Create a single directory whose parent already exists"#;
const DESC: &str = r#"`fs::createDirectory` creates the single directory named by `path` with one host
`mkdir` operation. On success the directory exists and the function returns
`Nothing`.

Only the final component is created; every parent component must already exist.
`fs::createDirectory` does not create intermediate directories, so a `path` whose
parent is missing fails rather than building the chain. Use `fs::createDirectories`
to create a directory together with any missing parents, like `mkdir -p`.

The new directory is requested with permission bits `0755` (`rwxr-xr-x`), which the
host masks with the process umask in the usual way, so the directory's actual mode
is `0755` with the umask bits cleared.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may be
absolute or relative to the current working directory, and may contain Unicode
characters when the host filesystem accepts those names. Internally a
NUL-terminated copy of `path` is allocated for the host call, so `path` must be
non-empty and must not contain an embedded NUL byte.

`fs::createDirectory` never overwrites or reuses an existing entry: if anything
already exists at `path`, including an existing directory, the call fails with
`ErrAlreadyExists` rather than succeeding quietly. When the host refuses the
operation, the failure `errno` is mapped to the matching error below and the
filesystem is left unchanged. `errno` values are per-OS; the same symbolic error is
produced on each platform."#;
const EX: &str = r#"Create a single output directory whose parent already exists:

```
IMPORT fs

SUB main()
  fs::createDirectory("target/example")
END SUB
```

Guard against re-creating a directory that already exists:

```
IMPORT fs

SUB main()
  IF NOT fs::directoryExists("target/cache") THEN
    fs::createDirectory("target/cache")
  END IF
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "createDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the directory to create, as UTF-8 bytes; absolute \
                       or relative to the current working directory. Must be non-empty and free \
                       of embedded NUL bytes. Only the final component is created; every parent \
                       component must already exist as a directory.",
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
