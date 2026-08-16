//! `fs::pathJoin` — descriptor + docs.
//!
//! A `path*` string member: purely syntactic, no syscall. It lowers at the call
//! site through the `Body::native` `common` slot (`native::lower_fs_path_join_nl`,
//! delegating to the relocated `impl CodeBuilder` path emitters), which itself
//! calls the standalone `lower_fs_path_join_helper` runtime helper so
//! imported-package binary_repr joins identically. Docs migrated from
//! `src/docs/man/builtins/fs/pathJoin.md`.

use super::native::lower_fs_path_join_nl;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"Join path components into a single path"#;
const DESC: &str = r#"`fs::pathJoin` concatenates the path components in `parts` with the POSIX `/`
separator and returns the combined path as a `String`. The components are joined
in list order, inserting exactly one separator where one is needed so that no
duplicate slashes appear between components: a separator is added before a
component only when the result so far is non-empty and does not already end in
`/`.

Empty components are skipped entirely; they contribute neither text nor a
separator. If a component is absolute — its first byte is `/` — it discards
everything accumulated before it and the result restarts from that component, so
the last absolute component in the list determines the prefix of the result.

The join is purely syntactic: it operates on the bytes of each component and
never consults the filesystem, resolves `.` or `..` segments, follows symbolic
links, or checks whether any path exists. Each component is interpreted as raw
UTF-8 bytes, so Unicode file names are preserved unchanged, and embedded NUL
bytes are copied verbatim rather than treated as terminators. An empty list, or
a list containing only empty components, yields the empty `String`. The function
reads no external state and has no side effects other than allocating the
returned `String`."#;
const EX: &str = r#"Join a directory and a file name:

```
IMPORT fs
IMPORT io

SUB main()
  LET path AS String = fs::pathJoin(["target", "output.txt"])
  io::print(path)
END SUB
```

A trailing separator is not duplicated by the join:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathJoin(["target/", "output.txt"]))
END SUB
```

An absolute component discards everything joined before it:

```
IMPORT fs
IMPORT io

SUB main()
  io::print(fs::pathJoin(["ignored", "/etc", "hosts"]))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "pathJoin",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("List OF String"),
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "parts",
                desc: "The path components to join, in order, interpreted as raw UTF-8 bytes. \
                       Empty components are skipped; any component beginning with `/` is treated \
                       as absolute and resets the accumulated result. May be an empty list.",
                aliases: &[],
                ty: ParameterType::Named("List OF String"),
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(None, None, Some(lower_fs_path_join_nl)),
        }],
    });
}
