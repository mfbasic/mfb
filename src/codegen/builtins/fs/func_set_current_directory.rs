//! `fs::setCurrentDirectory` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper` (which branches to
//! the relocated `lower_fs_path_operation_helper`).

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Change the process's current working directory"#;
const DESC: &str = r#"`fs::setCurrentDirectory` changes the current working directory of the running
process to the directory named by `path`, using a single host change-directory
operation. On success the working directory has been changed and the function
returns `Nothing`.

The change affects the whole process, so every relative path passed to later
`fs` functions — including `fs::open`, `fs::readText`, `fs::canonicalPath`, and
`fs::listDirectory` — resolves against the new directory rather than the old
one. The new value can be read back with `fs::currentDirectory`.

The working directory is process-global, not per-thread: a change made on one
thread is observed by every other thread, and there is no thread-scoped current
directory. Relative-path `fs` operations are therefore not isolated between
concurrently running threads; a program that needs per-thread path resolution
must build absolute paths itself rather than relying on
`fs::setCurrentDirectory`.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may
be absolute or relative to the current working directory; a relative path, such
as `"tests"` or `".."`, is resolved against the existing working directory
before the change takes effect. The path may contain Unicode characters,
including emoji, when the host filesystem accepts those names. The string must
not be empty and must not contain an embedded NUL byte, because the host call
requires a NUL-terminated path; the helper allocates an internal
NUL-terminated copy of the path for the call and rejects an empty or
NUL-containing string before making it.

The named entry must exist and must be a directory the process is allowed to
enter; every component leading to it must itself be a traversable directory.
When the host refuses the operation for any reason the failure is mapped to the
matching error below and the working directory is left unchanged."#;
const EX: &str = r#"Move into a subdirectory and back up to the parent:

```
IMPORT fs

SUB main()
  fs::setCurrentDirectory("tests")
  fs::setCurrentDirectory("..")
END SUB
```

Confirm the move by reading the working directory back:

```
IMPORT fs
IMPORT io

SUB main()
  fs::setCurrentDirectory("target")
  LET here AS String = fs::currentDirectory()
  io::print(here)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "setCurrentDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the directory to become the new working directory. \
                       Interpreted as UTF-8 bytes; may be absolute or relative to the current \
                       working directory. Must be non-empty and free of embedded NUL bytes. The \
                       entry must exist and be a directory the process can enter.",
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
