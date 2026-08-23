//! `fs::pathDirName` — descriptor + docs.
//!
//! A purely-syntactic `path*` string member: no syscall. It lowers at the call
//! site through the `Body::abi_inline_self` (self-lowering) (`gen_path_builder::lower_fs_path_dir_name_nl`).

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
`"target/output/"` and `"target/output"` both yield `"target"`. Trimming stops
once a single character remains, so it never consumes the whole string. After
trimming, the remaining bytes are scanned backward for the last `/` separator;
the separator that joins the directory to the final component is dropped, so the
result carries no trailing separator unless it is the root itself.

When `path` contains no separator, `"."` is returned. When the last separator
found is at position `0` — the only separator is a leading `/` — or `path` is
`"/"` itself, `"/"` is returned. An empty `path` returns `"."`.

The scan is byte-oriented (the separator is the single byte `47`), so UTF-8 file
names are preserved unchanged and any embedded bytes are treated literally. When
the result is `"."` or `"/"` a shared string constant is returned; otherwise a
new `String` holding the directory bytes is allocated. The function reads no
external state and has no side effects other than allocating the returned
`String`."#;
const EX: &str = r#"A directory and a file name yield the directory:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathDirName("target/output.txt"))
END SUB
```

Leading separators are preserved in the result:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathDirName("/usr/local/bin"))
END SUB
```

A path with no separator yields `"."`:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathDirName("output.txt"))
END SUB
```

The root path yields itself:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathDirName("/"))
END SUB
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
                desc: "The path whose directory portion is wanted, interpreted as raw UTF-8 \
                       bytes. Trailing `/` separators are ignored. May be empty.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::abi_inline_self(lower_fs_path_dir_name_nl),
        }],
    });
}
