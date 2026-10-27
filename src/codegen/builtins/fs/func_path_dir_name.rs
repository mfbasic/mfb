//! `fs::pathDirName` — descriptor + docs.
//!
//! A purely-syntactic `path*` string member: no syscall. It lowers at the call
//! site through the `Body::abi_inline` (`gen_path_builder::lower_fs_path_dir_name_nl`).

use super::gen_path_builder::lower_fs_path_dir_name_nl;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Return the directory portion of a path"#;
const DESC: &str = r#"`fs::pathDirName` returns the directory portion of `path` — everything up to but
not including the final component — as a `String`. The operation is purely
syntactic: it inspects the bytes of `path` and never consults the filesystem,
resolves `.` or `..` segments, follows symbolic links, or checks whether any path
exists.

Trailing `/` separators are trimmed before the final component is located, so
`"target/output/"` and `"target/output"` both yield `"target"`. Only the single
separator that joins the directory to the final component is dropped. Repeated
separators before the final component are kept, so `"a//b"` yields `"a/"`; the
result ends in `/` in that case and when it is the root `"/"`.

When `path` contains no separator, `"."` is returned; so does a single name with
a trailing separator, such as `"a/"`. When the directory part is nothing but
leading `/` separators (`"/a"`, `"//a"`, `"/"`, `"///"`), `"/"` is returned. An
empty `path` returns `"."`.

Only `/` is a separator; a backslash is an ordinary character even on Windows,
so `"C:\x\y"` has no directory part and yields `"."`. UTF-8 file names are
preserved unchanged. The result is a new value and `path` itself is never
changed; the function reads no external state and has no side effects.

The only error is `ErrOutOfMemory`, raised when the program runs out of memory."#;
const EX: &str = r#"A directory and a file name yield the directory:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathDirName("target/output.txt"))
END FUNC
```

Leading separators are preserved in the result:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathDirName("/usr/local/bin"))
END FUNC
```

A path with no separator yields `"."`:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathDirName("output.txt"))
END FUNC
```

The root path yields itself:

```
IMPORT fs
IMPORT io

FUNC main() AS Nothing
  io::print(fs::pathDirName("/"))
END FUNC
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pathDirName",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The path whose directory portion is wanted. Trailing `/` separators are ignored. May be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec!["ErrOutOfMemory"],
            body: Body::abi_inline(lower_fs_path_dir_name_nl),
        }],
    });
}
