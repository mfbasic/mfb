//! `fs::createDirectories` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper` (which branches to
//! the relocated `lower_fs_create_directories_helper`).

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Create a directory together with any missing parent directories"#;
const DESC: &str = r#"`fs::createDirectories` creates the directory named by `path` along with any
missing parent directories, like `mkdir -p`, and returns `Nothing` on success.

`path` is scanned left to right and each `/`-separated prefix is created in turn
before the final component is created. A leading `/` is skipped so the filesystem
root is not treated as a component to create. For every prefix, and for the final
component, one host `mkdir` operation is attempted; a component that already
exists (host `EEXIST`, errno `17`) is accepted and the scan continues. As a
result, existing intermediate directories and a final `path` that already exists
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
characters when the host filesystem accepts those names. Internally a
NUL-terminated copy of `path` is allocated for the host calls, and the `/`
separators in that copy are temporarily overwritten with NUL bytes to create each
prefix and restored afterward, so `path` must be non-empty and must not contain an
embedded NUL byte.

When the host refuses to create a prefix or the final component for any reason
other than `EEXIST`, the operation stops at that point and the failure `errno` is
mapped to the matching error below. Only `ENOENT` and `EACCES` are given specific
errors; every other refusal is reported as `ErrOutput`. `errno` values are per-OS;
the same symbolic error is produced on each platform."#;
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
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
