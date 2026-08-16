//! `fs::pathBaseName` — descriptor + docs.
//!
//! A purely-syntactic `path*` string member: no syscall. It lowers at the call
//! site through the `Body::native` `common` slot (`native::lower_fs_path_base_name_nl`).
//! Docs migrated from `src/docs/man/builtins/fs/pathBaseName.md`.

use super::native::lower_fs_path_base_name_nl;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the final component of a path"#;
const DESC: &str = r#"`fs::pathBaseName` returns the final component of `path` — the part after the
last `/` separator — as a `String`. The operation is purely syntactic: it
inspects the bytes of `path` and never consults the filesystem, resolves `.` or
`..` segments, follows symbolic links, or checks whether any path exists.

Trailing `/` separators are trimmed before the final component is located, so
`"target/output/"` and `"target/output"` both yield `"output"`. Trimming stops
once a single character remains, so it never consumes the whole string. After
trimming, the remaining bytes are scanned backward for the last `/` separator and
everything following it becomes the result, so the returned `String` carries no
leading separator.

When `path` contains no separator, it is returned unchanged. When `path` is `"/"`
itself, or trims down to a lone `/` because it consists only of separators (for
example `"//"` or `"///"`), `"/"` is returned. An empty `path` returns an empty
`String`.

The scan is byte-oriented (the separator is the single byte `47`), so UTF-8 file
names are preserved unchanged and any embedded bytes are treated literally. A new
`String` holding the final-component bytes is allocated for the result. The
function reads no external state and has no side effects other than allocating the
returned `String`."#;
const EX: &str = r#"A directory and a file name yield the file name:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathBaseName("target/output.txt"))
END SUB
```

The final component of an absolute path:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathBaseName("/usr/local/bin"))
END SUB
```

A trailing separator is ignored before the component is located:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathBaseName("/usr/local/bin/"))
END SUB
```

A path with no separator is returned unchanged:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathBaseName("output.txt"))
END SUB
```

The root path yields itself:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathBaseName("/"))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pathBaseName",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The path whose final component is wanted, interpreted as raw UTF-8 bytes. \
                       Trailing `/` separators are ignored. May be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(None, None, Some(lower_fs_path_base_name_nl)),
        }],
    });
}
