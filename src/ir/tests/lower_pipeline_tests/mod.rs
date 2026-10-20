use super::super::IrOp;
use crate::ast;
use crate::manifest::validate_project_manifest;
use crate::monomorph;
use crate::resolver;
// From a part, `super` is `lower_pipeline_tests`; part bodies spell `super::IrProject`,
// `super::write_ir` and the like, which `ir::tests` brings in.
use super::{write_ir, EntryPoint, ExternalFunctionParam, ExternalSignature, IrBinding, IrProject};

mod expressions_statements_and_lambdas;
mod links_docs_state_and_captures;
mod type_inference_and_reachable_arms;

/// Lower a single-file program through the whole front end and return the IR.
/// Panics with the failing stage if any pre-lowering stage rejects the source.
/// `name` labels the throwaway project's temp dir; the project is `irlower`.
fn lower_src(name: &str, source: &str) -> super::IrProject {
    super::helpers::lower_src_named("irlower", name, source)
}

/// Like [`lower_src`] but returns `None` when a pre-lowering stage rejects the
/// program. Diagnostic noise from the front end is suppressed.
fn try_lower_src(name: &str, source: &str) -> Option<super::IrProject> {
    super::helpers::try_lower_src_named("irlower", name, source)
}

/// Find a lowered function body by name (user functions only, not injected
/// package or lambda bodies).
fn function<'a>(ir: &'a super::IrProject, name: &str) -> &'a super::IrFunction {
    super::helpers::function(ir, name)
}

/// Serialize the whole project to JSON — a convenient way to assert a
/// lowering path was reached without matching every nested op by hand.
fn json_of(ir: &super::IrProject) -> String {
    ir.to_json()
}
