//! `os::resourcePath` — descriptor entry + authored docs.
//!
//! Per-member file (planning/migrate.md). **This is the one `os` member that
//! consumes per-compilation build context**: the shared [`crate::codegen::builtins::os::native::lower_os_helper`]
//! dispatcher threads the real `build_mode`/`module_name` (the strip/suffix
//! selection baked into the resource-base offset) into its `os.resourcePath` arm
//! (`native::lower_resource_path`), exactly as the legacy `os::lower_os_helper`
//! did. Every other `os` member accepts and ignores that context. Docs migrated
//! from `src/docs/man/builtins/os/resourcePath.md`.

use crate::codegen::registry::{
    Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

const INTRO: &str = r#"The absolute path of a build resource"#;
const DESC: &str = r#"`os::resourcePath` returns the **absolute** on-disk path of a resource the build
copied out of the project's manifest `resources` section, as an owned `String`.
The `relative` argument is the resource's path below its declared destination
directory (for example `music/song.ogg`), and the result is `<base>/<relative>`.

The base directory is derived at runtime from the running executable's own path
and a build-mode offset baked into the binary, so the same call resolves
correctly for every build shape:

| Build | Executable path | Resource base |
| --- | --- | --- |
| console | `…/build/<name>` | `…/build` |
| macOS `--app` | `…/Contents/MacOS/<name>` | `…/Contents/Resources` |
| Linux `--app` | `…/usr/bin/<name>` | `…/usr/share/<name>` |

The result is absolute and contains no `..` segments, so it opens with `fs::open`
regardless of the working directory — including a macOS `.app` launched from
Finder or a mounted `.AppImage`. Resolution reads only the executable's own path
(`/proc/self/exe` on Linux, `_NSGetExecutablePath` on macOS) and never consults
`$APPDIR` or any other environment variable.

A `relative` containing a `.` or `..` **path component** raises `ErrInvalidPath`
— a resource path must not navigate out of the base. A dot *inside* a filename
(`song.ogg`, `..foo`, `a..b`) is fine; only a whole component that is exactly `.`
or `..` is rejected. A leading `/` is left as-is (it collapses under the base). If
the host cannot determine the executable path, `os::resourcePath` raises
`ErrUnsupported`. It reads host state only and has no side effects."#;
const EX: &str = r#"Open a resource shipped beside the program:

```
IMPORT os
IMPORT fs
IMPORT io

SUB main()
  LET path AS String = os::resourcePath("music/song.ogg")
  io::print(path)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "resourcePath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "relative",
                desc: "The resource path below the build output (for example `music/song.ogg`); no `.`/`..` path component.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::None,
            }],
            return_type: ParameterType::String,
            errors: vec![],
            body: Body::native(
                Some(crate::codegen::builtins::os::native::lower_os_helper),
                Some(crate::codegen::builtins::os::native::lower_os_helper),
                None,
            ),
        }],
    });
}
