use super::super::*;
use super::helpers::{function, lower_src, try_lower_src};

mod builtin_resolution_and_captures;
mod declarations_statements_and_expressions;
mod traps_unions_links_and_docs;
mod type_inference_and_named_arguments;

// ---- helpers on the produced IR --------------------------------------

fn main_body(ir: &IrProject) -> &[IrOp] {
    &function(ir, "main").body
}

fn binding<'a>(ir: &'a IrProject, name: &str) -> &'a IrBinding {
    ir.bindings
        .iter()
        .find(|b| b.name == name)
        .unwrap_or_else(|| panic!("no binding {name}"))
}

fn bind_value<'a>(body: &'a [IrOp], name: &str) -> Option<&'a IrValue> {
    body.iter().find_map(|op| match op {
        IrOp::Bind {
            name: n,
            value: Some(v),
            ..
        } if n == name => Some(v),
        _ => None,
    })
}

/// bug-990: a one-line rendering of a lowered argument's wrap structure,
/// for assertion messages (`IrValue` has no `Debug`).
fn describe(value: &IrValue) -> String {
    match value {
        IrValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => format!(
            "unionWrap({} <- {}, {})",
            union_type.name(),
            member_type.name(),
            describe(value)
        ),
        IrValue::Call { target, .. } => format!("call {target}"),
        IrValue::MemberAccess { member, .. } => format!("member .{member}"),
        IrValue::WithUpdate { .. } => "with".to_string(),
        IrValue::Local(name) => format!("local {name}"),
        IrValue::Constructor { type_, .. } => format!("constructor {}", type_.name()),
        _ => "other".to_string(),
    }
}
