//! The IR data model and its module root.
//!
//! Type declarations are split by node role, not scattered across `mod.rs`:
//!
//! - `types.rs` — structural/nominal types: the project and function containers
//!   (`IrProject`, `IrFunction`, `EntryPoint`) and the declared entities
//!   (`IrType`, `IrBinding`, `IrField`, `IrVariant`, `IrEnumMember`, `IrParam`,
//!   `IrSourceLoc`, `IrRecordUpdate`, `ExternalFunctionParam`).
//! - `op.rs` — `IrOp`, the statement/operation nodes.
//! - `value.rs` — `IrValue`, `IrMatchCase`, `IrMatchPattern`: value and pattern
//!   nodes.
//! - `docs.rs` — the documentation surface (`ProjectDocs`, `IrPackageDoc`,
//!   `IrDocKind`, `IrDocDecl`) alongside its collector.
//! - `link.rs` — the native-`LINK` model (`IrLinkFunction`, `IrCStruct`, …).
//!
//! `mod.rs` itself declares no IR types; it is the module root and the re-export
//! hub through which the rest of the crate reaches the model.

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

mod binary;
#[cfg(test)]
mod coverage_tests;
mod docs;
mod json;
mod link;
mod lower;
mod lower_link;
mod op;
mod package;
// bug-343 A3: resource-escape analysis (was the misleadingly-named crate-root
// `escape.rs`); pub(crate) so its `src/target/` consumers can reach it.
pub(crate) mod resource_escape;
#[cfg(test)]
mod tests;
mod types;
mod value;
pub(crate) mod verify;

pub use binary::{decode_binary_repr, encode_binary_repr, verify_package};
pub(crate) use docs::{collect_project_docs, IrDocDecl, IrDocKind, IrPackageDoc, ProjectDocs};
pub(crate) use json::visibility_name;
pub(crate) use link::{
    abi_ctype_valid_as_argument, abi_ctype_valid_as_return, check_buffer_slots, check_cstruct,
    check_struct_slot, compute_c_layout, link_expr_var_names, AbiDirection, BufferSlotsView,
    CLayout, IrAbiSlot, IrBindIn, IrBindInField, IrBuffer, IrCStruct, IrCStructField, IrFree,
    IrLinkExpr, IrLinkFunction, IrNativeResource, StructSlotView, BYTE_LIST_TYPE,
};
pub use lower::{lower_project_with_external_functions, write_ir};
pub(crate) use op::IrOp;
pub use package::{
    apply_package_identity, merge_package, package_qualified_reference_names,
    prefix_package_symbols,
};
pub use types::{ExternalFunctionParam, IrProject};
pub(crate) use types::{
    EntryPoint, IrBinding, IrEnumMember, IrField, IrFunction, IrParam, IrRecordUpdate, IrSourceLoc,
    IrType, IrVariant,
};
pub(crate) use value::{IrMatchCase, IrMatchPattern, IrValue};
pub use verify::check as verify_semantics;
pub use verify::collect_source_diagnostics as verify_source_diagnostics;
pub use verify::RELOCATED_TO_IR_VERIFY;
