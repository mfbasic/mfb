//! `fs::pathExtension` — descriptor + docs.
//!
//! A purely-syntactic `path*` string member: no syscall. It lowers at the call
//! site through the `Body::native` `common` slot (`native::lower_fs_path_extension_nl`).
//! Docs migrated from `src/docs/man/builtins/fs/pathExtension.md`.

use super::native::lower_fs_path_extension_nl;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the extension of a path's final component"#;
const DESC: &str = r#"`fs::pathExtension` returns the extension of `path`'s final component, including
the leading `.`, as a `String`. The operation is purely syntactic: it inspects
the bytes of `path` and never consults the filesystem, resolves `.` or `..`
segments, follows symbolic links, or checks whether any path exists.

Trailing `/` separators are trimmed before the final component is located, so
`"target/output.txt"` and `"target/output.txt/"` both yield `".txt"`. Within that
component the bytes are scanned backward from the end and the scan stops at the
last `.`; the result spans from that `.` through the end of the component, so only
the final extension is returned and `"archive.tar.gz"` yields `".gz"`.

The scan never crosses a `/`, so a `.` in an earlier component is ignored:
`"lib.d/output"` yields an empty `String`. When the final component contains no
`.`, an empty `String` is returned. When the only `.` is the first byte of the
component, that component is treated as a dotfile name and the whole name is
returned, so `".bashrc"` yields `".bashrc"`. An empty `path`, or a `path`
consisting only of `/` separators, returns an empty `String`.

The scan is byte-oriented (the separator is the single byte `47` and the dot is
the single byte `46`), so UTF-8 file names are preserved unchanged and any
embedded bytes are treated literally. A new `String` holding the extension bytes
is allocated for the result. The function reads no external state and has no side
effects other than allocating the returned `String`."#;
const EX: &str = r#"A file name with an extension yields the extension:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("target/output.txt"))
END SUB
```

Only the final extension is returned:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("archive.tar.gz"))
END SUB
```

A component with no `.` yields an empty `String`:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("README"))
END SUB
```

A `.` in an earlier component is ignored:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension("lib.d/output"))
END SUB
```

A dotfile name is returned whole:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathExtension(".bashrc"))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pathExtension",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The path whose extension is wanted, interpreted as raw UTF-8 bytes. \
                       Trailing `/` separators are ignored before the final component is \
                       located. May be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(None, None, Some(lower_fs_path_extension_nl)),
        }],
    });
}
