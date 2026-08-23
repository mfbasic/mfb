//! `fs::pathNormalize` — descriptor + docs.
//!
//! A purely-syntactic `path*` string member: no syscall. It lowers at the call
//! site through the `Body::abi_inline` (`gen_path_builder::lower_fs_path_normalize_nl`).

use super::gen_path_builder::lower_fs_path_normalize_nl;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Normalize a path string syntactically"#;
const DESC: &str = r#"`fs::pathNormalize` returns a normalized form of `path` as a `String` without ever
consulting the filesystem. The normalization is purely syntactic: repeated `/`
separators are collapsed to a single separator, every `.` component is removed, and
each `..` component removes the preceding normal component when one is available to
remove.

A leading `/` is preserved, so an absolute path stays absolute and a `..`
immediately after the root has nothing to remove and is dropped. In a relative path
a leading `..` (or a run of them) has no earlier component to cancel, so each such
`..` is kept in place. When normalization would leave nothing at all — for example
the inputs `""`, `"."`, or `"a/.."` — the result is `"."` so that the returned path
always names something.

The operation is byte-oriented over the path syntax: only the `/` separator and the
`.` and `..` spellings are interpreted, while all other bytes are copied through
unchanged. UTF-8 file names are therefore preserved exactly, and the function never
resolves symbolic links, accesses any file, or checks whether any path exists. To
resolve a path against the real directory tree instead, use `fs::canonicalPath`. The
normalized output is never longer than the input, and the function has no side
effects other than allocating the returned `String`."#;
const EX: &str = r#"Redundant separators and `.` components are removed:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathNormalize("target//a/./file.txt"))
END SUB
```

A `..` component cancels the preceding component:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathNormalize("/usr/local/../bin"))
END SUB
```

Normalizing to nothing yields `"."`:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathNormalize("a/b/../.."))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pathNormalize",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The path to normalize, interpreted as raw UTF-8 bytes with `/` as the \
                       component separator. May be absolute or relative, and may be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline(lower_fs_path_normalize_nl),
        }],
    });
}
