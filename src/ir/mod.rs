use crate::ast::{
    AstProject, CallArg, ConstructorArg, EnumMember, ExitTarget, Expression, Function,
    FunctionKind, Item, LoopKind, MatchCase, MatchPattern, Param, Statement, TypeDecl,
    TypeDeclKind, TypeField, UnionVariant, Visibility,
};
use crate::builtins;
use crate::json_string;
use crate::numeric;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone)]

pub struct IrProject {
    pub(crate) name: String,
    pub(crate) entry: Option<EntryPoint>,
    pub(crate) bindings: Vec<IrBinding>,
    pub(crate) types: Vec<IrType>,
    pub(crate) functions: Vec<IrFunction>,
    /// Native `LINK` resources declared in this project, surfaced to package
    /// metadata (`RESOURCE_TABLE`) since they carry no executable IR
    /// (plan-link-update.md §10).
    pub(crate) native_resources: Vec<IrNativeResource>,
    /// Native `LINK` functions declared in this project, carried to the backend
    /// so it can emit marshaling thunks + dlopen/dlsym initializers
    /// (plan-linker.md §12).
    pub(crate) link_functions: Vec<IrLinkFunction>,
    /// Re-export aliases targeting a native `LINK` function:
    /// `(alias_name, target_alias.func)` (plan-link-update.md §5a). Lets the
    /// backend route a call to the exported alias to the target's thunk.
    pub(crate) link_aliases: Vec<(String, String)>,
    /// Documentation collected from `DOC` blocks for the package's exported
    /// declarations (plan-09-doc.md §5). Carried so the package writer can emit
    /// the optional `doc` section; ignored when building an executable.
    pub(crate) docs: ProjectDocs,
}

/// The documentation surface of a project: an optional package-level entry plus
/// one entry per documented exported declaration (plan-09-doc.md §5).
#[derive(Clone, Default)]
pub(crate) struct ProjectDocs {
    pub(crate) package: Option<IrPackageDoc>,
    pub(crate) decls: Vec<IrDocDecl>,
}

#[derive(Clone)]
pub(crate) struct IrPackageDoc {
    pub(crate) name: String,
    /// Prose blocks as `(kind code, text)` — see `crate::ast::DocProseKind::code`.
    pub(crate) desc: Vec<(u8, String)>,
    /// `Some(message)` when deprecated (message may be empty); `None` otherwise.
    pub(crate) deprecated: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum IrDocKind {
    Func,
    Sub,
    Type,
    Union,
    Enum,
}

#[derive(Clone)]
pub(crate) struct IrDocDecl {
    pub(crate) kind: IrDocKind,
    pub(crate) name: String,
    pub(crate) signature: String,
    /// `GROUP` name for FUNC/SUB, or empty.
    pub(crate) group: String,
    /// Prose blocks as `(kind code, text)` — see `crate::ast::DocProseKind::code`.
    pub(crate) desc: Vec<(u8, String)>,
    pub(crate) args: Vec<(String, String)>,
    pub(crate) props: Vec<(String, String)>,
    pub(crate) ret: String,
    pub(crate) errors: Vec<(String, String)>,
    pub(crate) example: String,
    pub(crate) internal: bool,
    /// `Some(message)` when deprecated (message may be empty); `None` otherwise.
    pub(crate) deprecated: Option<String>,
}

#[derive(Clone)]
pub(crate) struct EntryPoint {
    pub(crate) name: String,
    pub(crate) returns: String,
    pub(crate) accepts_args: bool,
}

#[derive(Clone)]

pub(crate) struct IrFunction {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<IrParam>,
    pub(crate) returns: String,
    pub(crate) body: Vec<IrOp>,
    // Source file (project-relative path) this function was lowered from. Used to
    // build `ErrorLoc.filename` for errors that originate inside this function.
    pub(crate) file: String,
    // Source location of the function declaration.
    pub(crate) loc: IrSourceLoc,
    // Resource ownership decisions (escape analysis, §15.6), keyed by `RES`
    // binding name. Drives where each resource's close obligation is discharged:
    // its own scope, an outer collection's scope (runtime owned-list), or out via
    // a returned collection. Absent names are `Local`.
    pub(crate) resource_owners: HashMap<String, crate::escape::ResOwner>,
}

mod binary;
mod json;
mod link;
mod lower;
mod op;
mod package;
#[cfg(test)]
mod tests;
mod types;
mod value;
mod verify;

pub use binary::{decode_binary_repr, encode_binary_repr, verify_package};
pub(crate) use json::visibility_name;
pub(crate) use link::{IrAbiSlot, IrFree, IrLinkExpr, IrLinkFunction, IrNativeResource};
pub(crate) use lower::collect_project_docs;
pub use lower::{lower_project_with_external_functions, write_ir};
pub(crate) use op::IrOp;
pub use package::{
    apply_package_identity, merge_package, package_qualified_reference_names,
    prefix_package_symbols,
};
pub use types::ExternalFunctionParam;
pub(crate) use types::{
    IrBinding, IrEnumMember, IrField, IrParam, IrRecordUpdate, IrSourceLoc, IrType, IrVariant,
};
pub(crate) use value::{IrMatchCase, IrMatchPattern, IrValue};
pub use verify::check as verify_semantics;
pub use verify::collect_source_diagnostics as verify_source_diagnostics;
pub use verify::RELOCATED_TO_IR_VERIFY;
