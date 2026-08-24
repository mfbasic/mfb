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
use crate::codegen::builtins;
use crate::json::json_string;
use crate::numeric;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

// Single source of truth for the package-format invariants that are enforced at
// two points (bug-342 A3): the binary decoder / `verify_package`'s structural
// re-check in `binary.rs`, and `ir::verify`'s semantic walk. Both forward to
// these so the depth cap and the rule id/message are each spelled once and can
// never drift apart.

/// Maximum statement/expression nesting depth accepted anywhere in the IR.
pub(crate) const MAX_IR_NESTING_DEPTH: usize = 256;
/// Rule-id prefix for a structural package-format violation.
pub(crate) const VERIFY_TYPE: &str = "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE";
/// Rule-id prefix for a non-exhaustive (empty) MATCH in a decoded package.
pub(crate) const VERIFY_MATCH: &str = "PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH";
/// Message body paired with [`VERIFY_MATCH`], shared by the pre-merge structural
/// check and the post-merge semantic walk so the two enforcement points read
/// identically.
pub(crate) const VERIFY_MATCH_EMPTY_MSG: &str = "MATCH has no cases (not exhaustive)";

mod binary;
mod docs;
mod json;
mod link;
mod lower;
mod lower_link;
mod op;
mod package;
#[cfg(test)]
mod variant_corpus_tests;
// bug-343 A3: resource-escape analysis (was the misleadingly-named crate-root
// `escape.rs`); pub(crate) so its `src/target/` consumers can reach it.
pub(crate) mod resource_escape;
#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
mod types;
mod value;
pub(crate) mod verify;

pub use binary::{decode_binary_repr, encode_binary_repr, verify_package};
pub(crate) use docs::{collect_project_docs, IrDocKind, ProjectDocs};
// `IrDocDecl`/`IrPackageDoc` are constructed in `docs.rs` and, outside it, only
// by the binary-repr round-trip tests; re-export them for that test path only.
#[cfg(test)]
pub(crate) use docs::{IrDocDecl, IrPackageDoc};
pub(crate) use json::visibility_name;
pub(crate) use link::{
    abi_ctype_valid_as_argument, abi_ctype_valid_as_return, check_buffer_slots, check_cstruct,
    check_struct_slot, compute_c_layout, link_compare_op_valid, link_expr_var_names, AbiDirection,
    BufferSlotsView, CLayout, IrAbiSlot, IrBindIn, IrBindInField, IrBuffer, IrCStruct,
    IrCStructField, IrFree, IrLinkExpr, IrLinkFunction, IrNativeResource, StructSlotView,
    BYTE_LIST_TYPE,
};
#[cfg(test)]
pub use lower::lower_project_with_external_functions;
pub use lower::{
    lower_augmented_project, write_ir, ImportedTypeDef, ImportedTypeField, ImportedTypeKind,
    ImportedTypeVariant,
};
pub(crate) use op::IrOp;
pub use package::{
    apply_package_identity, merge_package, package_qualified_reference_names,
    prefix_package_symbols,
};
pub(crate) use types::{
    EntryPoint, IrBinding, IrEnumMember, IrField, IrFunction, IrParam, IrRecordUpdate, IrSourceLoc,
    IrType, IrVariant,
};
pub use types::{ExternalFunctionParam, IrProject};
pub(crate) use value::{IrMatchCase, IrMatchPattern, IrValue};
pub use verify::check as verify_semantics;
pub use verify::collect_source_diagnostics as verify_source_diagnostics;
pub use verify::RELOCATED_TO_IR_VERIFY;
