//! `fs::pathBaseName` — descriptor + docs.
//!
//! A purely-syntactic `path*` string member: no syscall. It lowers at the call
//! site through the `Body::abi_inline` (`gen_path_builder::lower_fs_path_base_name_nl`).

use super::gen_path_builder::lower_fs_path_base_name_nl;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the final component of a path"#;
const DESC: &str = r#"`fs::pathBaseName` returns the final component of `path` — the part after the
last `/` separator — as a `String`. The operation is purely syntactic: it
inspects the bytes of `path` and never consults the filesystem, resolves `.` or
`..` segments, follows symbolic links, or checks whether any path exists.

Trailing `/` separators are trimmed before the final component is located, so
`"target/output/"` and `"target/output"` both yield `"output"`. Everything after
the last remaining `/` becomes the result, so the returned `String` carries no
leading separator.

When `path` contains no separator, it is returned unchanged. When `path` is `"/"`
itself, or trims down to a lone `/` because it consists only of separators (for
example `"//"` or `"///"`), `"/"` is returned. An empty `path` returns an empty
`String`.

Only `/` is a separator. A backslash is an ordinary character even on Windows,
so `"a\b"` is returned unchanged. UTF-8 file names are preserved unchanged. The
result is a new value and `path` itself is never changed; the function reads no
external state and has no side effects.

The only error is `ErrOutOfMemory`, raised when the program runs out of memory."#;
const EX: &str = r#"A directory and a file name yield the file name:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathBaseName("target/output.txt"))
END FUNC
```

The final component of an absolute path:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathBaseName("/usr/local/bin"))
END FUNC
```

A trailing separator is ignored before the component is located:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathBaseName("/usr/local/bin/"))
END FUNC
```

A path with no separator is returned unchanged:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathBaseName("output.txt"))
END FUNC
```

The root path yields itself:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathBaseName("/"))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pathBaseName",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The path whose final component is wanted. Trailing `/` separators are ignored. May be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_inline(lower_fs_path_base_name_nl),
        }],
    });
}
