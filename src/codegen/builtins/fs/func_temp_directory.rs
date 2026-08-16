//! `fs::tempDirectory` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`. Docs migrated from
//! `src/docs/man/builtins/fs/tempDirectory.md`.

use super::native::lower_fs_helper;
use super::{Body, Implementation, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the host temporary directory path"#;
const DESC: &str = r#"`fs::tempDirectory` returns the path of the host's temporary directory as a
UTF-8 `String`. This is the same location `fs::createTempFile` uses when it is
called without a `directory` argument; that zero-argument form is lowered to
supply `fs::tempDirectory()` as the directory automatically.

The directory path is queried from the operating system on every call rather
than cached, so the result reflects the host environment at the moment of the
call. The returned `String` holds only the path bytes, with the trailing NUL
that the host query produces stripped off; no trailing path separator is added.

The source of the path is platform specific:

- On macOS the per-process Darwin user temporary directory is read with
  `confstr(_CS_DARWIN_USER_TEMP_DIR, ...)`, a user-private location under the
  system temporary area. The reported length is the returned size minus one, to
  drop the terminating NUL.
- On Linux the value of the `TMPDIR` environment variable is used when it is set,
  non-empty, and fits within the internal buffer; otherwise the path falls back
  to `/tmp`.

The function takes no arguments and has no filesystem side effects: it neither
creates the directory nor verifies that it exists, it only reports the
configured path. Internally it reads into a fixed 4096-byte buffer before
copying the result into an arena-backed `String`."#;
const EX: &str = r#"Read and print the host temporary directory:

```
IMPORT fs
IMPORT io

SUB main()
  LET dir AS String = fs::tempDirectory()
  io::print(dir)
END SUB
```

Create a temporary file under the host temporary directory:

```
IMPORT fs

SUB main()
  RES f = fs::createTempFile()
  ' f is created under fs::tempDirectory() and closed by lexical drop
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "tempDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("no arguments"),
        implementations: vec![Implementation {
            params: vec![],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
