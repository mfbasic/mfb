//! `os::userDocumentsPath` — descriptor entry + authored docs. The lowering is the shared
//! [`super::gen_host_paths::lower_host_dir`] with [`HostDir::UserDocuments`] (plan-156-C).

use super::gen_host_paths::{lower_host_dir, HostDir};
use crate::codegen::engine::builder::*;
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::types::ParameterType;

/// `os::userDocumentsPath(relative = "")` — see [`lower_host_dir`].
pub(crate) fn lower_user_documents_path(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_host_dir(
        builder,
        ctx,
        HostDir::UserDocuments,
        false,
        "os.userDocumentsPath",
    )
}

/// `os.userDocumentsPathBase(dst, cap) -> length` — internal: the directory `os::userDocumentsPath()`
/// returns, written into a caller buffer without allocating, for the in-place
/// `s = os::userDocumentsPath(s)` arm (plan-156-B §4.4, plan-156-C). See [`lower_host_dir`].
pub(crate) fn lower_user_documents_path_base(
    builder: &mut CodeBuilder,
    _args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_host_dir(
        builder,
        ctx,
        HostDir::UserDocuments,
        true,
        "os.userDocumentsPathBase",
    )
}

const INTRO: &str = r#"The current user's Documents folder"#;
const DESC: &str = r#"`os::userDocumentsPath` returns the absolute path of the current user's
Documents folder — the place a person expects files they saved on purpose to
go. It is not specific to your program.

| OS | Folder |
| --- | --- |
| macOS | `~/Documents` |
| Linux | the desktop's Documents folder from `~/.config/user-dirs.dirs` (which can be translated, such as `~/Dokumente`, or moved); `~/Documents` when that file does not name one |
| Windows | the user's Documents folder, following it when it has been moved, for example into OneDrive |

`~` is the user's home folder, as `os::userHomePath` returns it. On Linux the
file is read the way GTK applications read it, so your program and the desktop
agree on the folder; `XDG_CONFIG_HOME`, when set to an absolute path, is where
`user-dirs.dirs` is looked for.

The folder is not guaranteed to exist — a server or a minimal system may have
none — so check with `fs::directoryExists` before relying on it. On macOS the
first time a program reads or writes there, the system asks the user for
permission. A sandboxed macOS app gets the Documents folder inside its own
container; the user's real Documents folder is not reachable from the sandbox
this way.

Omit `relative` (or pass `""`) to get the folder itself: the result then has no
trailing `/`. Otherwise the result is `<folder>/<relative>`, joined with `/` on
every target — the byte `fs::pathJoin` uses — so on Windows it reads like
`C:\Users\me\\Documents/notes.txt`, which every Windows path API accepts.

The call only works out a path. It does not create anything and does not check
that the path exists — see above.

A `relative` containing a `.` or `..` **path component** raises
`ErrInvalidPath`. A dot inside a name (`notes.txt`, `..cfg`) is fine; only a
whole component that is exactly `.` or `..` is refused. On Windows a component
also ends at `\`, so `..\x` is refused too. If the host cannot say where the
folder is — no `HOME` and no account entry on macOS or Linux, or the folder
lookup failing on Windows — the call raises `ErrUnsupported`."#;
const EX: &str = r#"Save a file to the user's Documents folder when it exists:

```
IMPORT os
IMPORT fs
IMPORT io

SUB main()
  IF fs::directoryExists(os::userDocumentsPath()) THEN
    io::print(os::userDocumentsPath("report.txt"))
  ELSE
    io::print("no Documents folder")
  END IF
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "userDocumentsPath",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: None,
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "relative",
                desc: "A path below the Documents folder (for example `report.txt`); no `.`/`..` path component. Omit it (or pass `\"\"`) for the folder itself.",
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
            body: Body::abi_function(lower_user_documents_path),
        }],
    });
    pkg.add_function(RegistryFunction {
        name: "userDocumentsPathBase",
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
            body: Body::abi_function(lower_user_documents_path_base),
        }],
    });
}
