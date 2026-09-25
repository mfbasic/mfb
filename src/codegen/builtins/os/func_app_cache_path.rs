//! `os::appCachePath` — descriptor entry + authored docs. The lowering is the shared
//! [`super::gen_host_paths::lower_host_dir`] with [`HostDir::AppCache`] (plan-157-B).

use super::gen_host_paths::{lower_host_dir, HostDir};
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `os::appCachePath(relative = "")` — see [`lower_host_dir`].
pub(crate) fn lower_app_cache_path(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_host_dir(builder, ctx, HostDir::AppCache, false, "os.appCachePath")
}

/// `os.appCachePathBase(dst, cap) -> length` — internal: the directory `os::appCachePath()`
/// returns, written into a caller buffer without allocating, for the in-place
/// `s = os::appCachePath(s)` arm (plan-157-B §4.4). See [`lower_host_dir`].
pub(crate) fn lower_app_cache_path_base(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_host_dir(builder, ctx, HostDir::AppCache, true, "os.appCachePathBase")
}

const INTRO: &str =
    r#"The directory where this app keeps rebuildable cache files for the current user"#;
const DESC: &str = r#"`os::appCachePath` returns the absolute path of the directory the operating
system sets aside for this program's cache for the current user — downloaded
copies, thumbnails, anything the program can rebuild if it disappears. The
system or the user may clear it at any time, so never keep the only copy of
something there; use `os::appDataPath` for that. The last part of the path is
the project's `name` from `project.json`, the same in console and `--app`
builds.

| OS | Directory |
| --- | --- |
| macOS | `~/Library/Caches/<name>` |
| Linux | `$XDG_CACHE_HOME/<name>`, or `~/.cache/<name>` when `XDG_CACHE_HOME` is unset, empty or not an absolute path |
| Windows | `<Local AppData>\<name>` (usually `C:\Users\<you>\AppData\Local\<name>`) |

`~` is the user's home folder: the `HOME` variable when it is set, otherwise the
account's home directory.

Omit `relative` (or pass `""`) to get the directory itself: the result then has
no trailing `/`. Otherwise the result is `<directory>/<relative>`, joined with
`/` on every target — the byte `fs::pathJoin` uses — so on Windows it reads
`C:\Users\me\AppData\Roaming/MyApp/save.dat`, which every Windows path API
accepts.

The call only works out a path. **It does not create the directory** and does
not check that anything exists there. Before the first write, create the
directory with `fs::createDirectories` — creating one that already exists is
fine.

A `relative` containing a `.` or `..` **path component** raises
`ErrInvalidPath`, so the result cannot point outside the directory. A dot
inside a name (`save.dat`, `..cfg`) is fine; only a whole component that is
exactly `.` or `..` is refused. On Windows a component also ends at `\`, so
`..\x` is refused too. If the host cannot say where the user's directories
are — no `HOME` and no account entry on macOS or Linux, or the folder lookup
failing on Windows — the call raises `ErrUnsupported`.

A sandboxed macOS app (one built for the Mac App Store) gets a directory
inside its own container, because that is what the system reports as its home
there; this is the correct place for it to write."#;
const EX: &str = r#"Keep a downloaded copy in the cache directory:

```
IMPORT os
IMPORT fs
IMPORT io

SUB main()
  fs::createDirectories(os::appCachePath())
  LET copy AS String = os::appCachePath("latest.json")
  fs::writeText(copy, "{}")
  io::print(toString(fs::fileExists(copy)))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "appCachePath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "relative",
                desc: "A path below the app's cache directory (for example `thumbs/a.png`); no `.`/`..` path component. Omit it (or pass `\"\"`) for the directory itself.",
                aliases: &[],
                ty: ParameterType::String,
                default: DefaultValue::Fill {
                    type_name: ParameterType::String,
                    expr: "",
                },
            }],
            return_type: ParameterType::String,
            // Raised through `raise_error_into` (`gen_host_paths.rs`), which the
            // static descriptor scan cannot see — declared here so the page's
            // Errors section lists them (the bug-454 lesson).
            errors: vec!["ErrUnsupported", "ErrInvalidPath"],
            body: Body::abi_function(lower_app_cache_path),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "appCachePathBase",
        intro: "",
        desc: "",
        example: "",
        expected_arguments: None,
        internal_only: true,
        implementations: vec![Implementation {
            params: vec![
                Parameter {
                    name: "dst",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
                Parameter {
                    name: "cap",
                    desc: "",
                    aliases: &[],
                    ty: ParameterType::Integer,
                    default: DefaultValue::None,
                },
            ],
            return_type: ParameterType::Integer,
            errors: vec![],
            body: Body::abi_function(lower_app_cache_path_base),
        }],
    });
}
