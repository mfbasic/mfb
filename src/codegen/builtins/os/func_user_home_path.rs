//! `os::userHomePath` — descriptor entry + authored docs. The lowering is the shared
//! [`super::gen_host_paths::lower_host_dir`] with [`HostDir::UserHome`] (plan-157-C).

use super::gen_host_paths::{lower_host_dir, HostDir};
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `os::userHomePath(relative = "")` — see [`lower_host_dir`].
pub(crate) fn lower_user_home_path(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_host_dir(builder, ctx, HostDir::UserHome, false, "os.userHomePath")
}

/// `os.userHomePathBase(dst, cap) -> length` — internal: the directory `os::userHomePath()`
/// returns, written into a caller buffer without allocating, for the in-place
/// `s = os::userHomePath(s)` arm (plan-157-B §4.4, plan-157-C). See [`lower_host_dir`].
pub(crate) fn lower_user_home_path_base(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_host_dir(builder, ctx, HostDir::UserHome, true, "os.userHomePathBase")
}

const INTRO: &str = r#"The current user's home folder"#;
const DESC: &str = r#"`os::userHomePath` returns the absolute path of the current user's home
folder. It is not specific to your program — for files that belong to your app, use
`os::appDataPath` or `os::appCachePath`, which give it a directory of its own.

| OS | Folder |
| --- | --- |
| macOS | the `HOME` variable, usually `/Users/<you>` |
| Linux | the `HOME` variable, usually `/home/<you>` |
| Windows | the user's profile folder, usually `C:\Users\<you>` |

On macOS and Linux, when `HOME` is unset or empty the account's home directory
is used instead. Windows does not read `HOME`.

A sandboxed macOS app (one built for the Mac App Store) gets its container
folder, because that is what the system reports as its home there.

Omit `relative` (or pass `""`) to get the folder itself: the result then has no
trailing `/`. Otherwise the result is `<folder>/<relative>`, joined with `/` on
every target — the byte `fs::pathJoin` uses — so on Windows it reads like
`C:\Users\me/notes.txt`, which every Windows path API accepts.

The call only works out a path. It does not create anything and does not check
that the path exists.

A `relative` containing a `.` or `..` **path component** raises
`ErrInvalidPath`. A dot inside a name (`notes.txt`, `..cfg`) is fine; only a
whole component that is exactly `.` or `..` is refused. On Windows a component
also ends at `\`, so `..\x` is refused too. If the host cannot say where the
folder is — no `HOME` and no account entry on macOS or Linux, or the folder
lookup failing on Windows — the call raises `ErrUnsupported`."#;
const EX: &str = r#"Show a path in the home folder:

```
IMPORT os
IMPORT io

SUB main()
  io::print(os::userHomePath(".myapprc"))
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "userHomePath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "relative",
                desc: "A path below the home folder (for example `.myapprc`); no `.`/`..` path component. Omit it (or pass `\"\"`) for the folder itself.",
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
            body: Body::abi_function(lower_user_home_path),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "userHomePathBase",
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
            body: Body::abi_function(lower_user_home_path_base),
        }],
    });
}
