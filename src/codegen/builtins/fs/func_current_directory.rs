//! `fs::currentDirectory` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`.

use super::native::lower_fs_helper;
use super::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the process's current working directory"#;
const DESC: &str = r#"`fs::currentDirectory` returns the absolute current working directory of the
running process as a UTF-8 `String`.

The path is queried from the operating system with the host `getcwd` call on
every invocation rather than cached, so the result reflects the process's
working directory at the moment of the call. The returned path is absolute and
is given in the host's native spelling. Internally the path is read into a
fixed 4096-byte arena buffer, its length is measured up to the terminating NUL,
and those bytes are copied into an arena-backed `String`; the terminating NUL is
not included in the returned value.

The working directory is the base against which any relative path passed to
other `fs` functions is resolved, so this value names the directory used by
`fs::canonicalPath`, `fs::open`, `fs::readText`, and the rest of the package
when they are given a path that is not absolute. The current directory can be
changed with `fs::setCurrentDirectory`.

The function takes no arguments, reads process state only, and has no filesystem
side effects: it does not create, open, or modify any file."#;
const EX: &str = r#"Read and print the current working directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET cwd AS String = fs::currentDirectory()
  io::print(cwd)
END SUB
```

Resolve a relative path against the working directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET cwd AS String = fs::currentDirectory()
  LET full AS String = fs::pathJoin([cwd, "output.txt"])
  io::print(full)
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "currentDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
