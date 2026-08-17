//! `fs::createTempFile` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper`. Returns a `File`
//! resource for a freshly created temp file; `directory` is optional. Docs
//! migrated from `src/docs/man/builtins/fs/createTempFile.md`.

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Securely create and open a unique, freshly named temporary file"#;
const DESC: &str = r#"`fs::createTempFile` creates a brand-new file with a unique, unpredictable name
and returns an open `File` resource referring to it. The name has the form
`mfb-<uuid>.tmp`, where `<uuid>` is a freshly generated version 4 UUID rendered
in the canonical 8-4-4-4-12 hexadecimal form. The random bytes that seed the
name are drawn from host entropy before the file is opened, so two calls
effectively never collide and the name cannot be guessed by another process.

The file is opened read/write with exclusive-create semantics and permission
bits `0600` (octal), so the call always yields a freshly created, empty file
readable and writable only by the current user. Exclusive creation means the
call fails rather than reusing or truncating any pre-existing file, which
together with the random name closes the classic temporary-file race and
symlink-redirection attacks. The descriptor is also opened close-on-exec.

Without an argument the file is created inside the host temporary directory, the
same location returned by `fs::tempDirectory`; that directory path is supplied
automatically as the `directory` argument for the zero-argument form. With a
`directory` argument the file is created directly inside that directory. The
argument names the containing directory, not the file — no name component of
your own is added. The directory must already exist and be writable, since the
new file is created there.

`directory` is interpreted as UTF-8 bytes and passed to the host filesystem. It
may be absolute or relative to the current working directory and may contain
Unicode characters when the host filesystem accepts those names. It must not be
empty and must not contain an embedded NUL byte, because the host `open` call
requires a NUL-terminated path.

The returned `File` is positioned at the start of the empty file and is owned by
the caller. It is closed by lexical drop when the binding that holds it leaves
scope, or explicitly with `fs::close`. The file itself is not deleted on close;
removing it is the caller's responsibility, for example with `fs::deleteFile`."#;
const EX: &str = r#"Create a temporary file in the host temporary directory and write to it:

```
IMPORT fs

SUB main()
  RES f = fs::createTempFile()
  fs::writeAll(f, "data")
  ' f is closed by lexical drop when this scope ends
END SUB
```

Create a temporary file in a specific directory:

```
IMPORT fs

SUB main()
  RES g = fs::createTempFile("target")
  fs::writeAll(g, "data")
  ' g is closed by lexical drop when this scope ends
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "createTempFile",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "directory",
                desc: "The path of an existing, writable directory in which to create the \
                       temporary file, as UTF-8 bytes; absolute or relative to the current \
                       working directory. Must be non-empty and free of embedded NUL bytes. When \
                       omitted, the host temporary directory is used.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::Optional,
            }],
            return_type: ParameterType::Named(super::FILE_TYPE_ID),
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
