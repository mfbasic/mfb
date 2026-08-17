//! `fs::listDirectory` — descriptor + docs.
//!
//! Native syscall member: its `Body::native` posix/win slots both hold the shared
//! family-generic OS-seam dispatcher `native::lower_fs_helper` (which branches to
//! the relocated `lower_fs_list_directory_helper`).

use super::native::lower_fs_helper;
use super::{Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage};
use crate::types::ParameterType;

const INTRO: &str = r#"List the direct child names of a directory"#;
const DESC: &str = r#"`fs::listDirectory` opens the directory named by `path`, reads every entry it
contains directly, and returns those entry names as a `List OF String`. The list
holds the entry names only, not full paths, and the special `"."` (current
directory) and `".."` (parent directory) entries are always filtered out, so
they never appear in the result.

Only the immediate children of the directory are listed; `fs::listDirectory`
does not descend into subdirectories. Every kind of entry is included regardless
of type, so the result mixes regular files, subdirectories, symlinks, and any
other filesystem objects, each represented by its name with no trailing slash or
type marker.

The names are sorted in ascending byte-wise order, comparing their raw UTF-8
bytes (an ordinary lexicographic ordering for ASCII names), so the result is
deterministic and stable across runs and across hosts. An empty directory, or a
directory that contains only `"."` and `".."`, yields an empty `List`.

Internally the directory is scanned in two passes: the first pass opens, reads,
and closes it to count the entries and their name bytes so the result `List` can
be sized, and the second pass opens, reads, and closes it again to fill the list
before sorting. If a concurrent writer grows the directory between the two
scans, the extra entries are truncated to the sized capacity rather than
overflowing the arena block, and the header is trimmed to what the second pass
actually wrote. The final path component is followed when it is a symlink, so
listing through a symlink that points at a directory lists the target
directory's entries.

`path` is interpreted as UTF-8 bytes and passed to the host filesystem. It may
be absolute or relative to the current working directory and may contain Unicode
characters, including emoji, when the host filesystem accepts those names. The
string must not be empty and must not contain an embedded NUL byte, because the
host call requires a NUL-terminated path; an internal NUL-terminated copy of the
path is allocated for the call. Apart from opening and closing the directory,
the call only reads the filesystem and has no side effects."#;
const EX: &str = r#"Print every entry in a directory in sorted order:

```
IMPORT fs
IMPORT io
IMPORT collections

SUB main()
  LET names AS List OF String = fs::listDirectory("target")
  FOR i = 0 TO len(names) - 1
    io::print(collections::get(names, i))
  NEXT
END SUB
```

An empty directory yields an empty `List`:

```
IMPORT fs
IMPORT io

SUB main()
  fs::createDirectory("target/empty")
  LET names AS List OF String = fs::listDirectory("target/empty")
  io::print(toString(len(names)))
END SUB
```"#;

pub(super) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "listDirectory",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("String"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "path",
                desc: "The filesystem path of the directory to list, as UTF-8 bytes; absolute or \
                       relative to the current working directory. Must be non-empty and free of \
                       embedded NUL bytes, and must name an existing, readable directory.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::list_of(ParameterType::String),
            errors: vec![],
            body: Body::native(Some(lower_fs_helper), Some(lower_fs_helper), None),
        }],
    });
}
