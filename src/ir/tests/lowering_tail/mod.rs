//! `ir::lower` at the forms the program corpora and the golden replay never
//! write — one file per lowering module. Each test lowers a program (or, where
//! no source can reach a total-lowering arm, the concrete HIR the program
//! elaborates to, adjusted) and asserts the IR it must produce.

use std::collections::HashMap;

use crate::hir::HirMatchPattern;
use crate::ir::{IrOp, IrProject, IrValue};
use crate::testutil::{check_src, lower_src_concrete};
use crate::types::ParameterType;

mod concat_fold;
mod statement;

/// The lowered function named `name`.
fn function<'a>(ir: &'a IrProject, name: &str) -> &'a crate::ir::IrFunction {
    ir.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

/// Visit every op of `ops`, at every depth.
fn walk_ops(ops: &[IrOp], visit: &mut impl FnMut(&IrOp)) {
    for op in ops {
        visit(op);
        match op {
            IrOp::If {
                then_body,
                else_body,
                ..
            } => {
                walk_ops(then_body, visit);
                walk_ops(else_body, visit);
            }
            IrOp::Match { cases, .. } => {
                for case in cases {
                    walk_ops(&case.body, visit);
                }
            }
            IrOp::While { body, .. }
            | IrOp::For { body, .. }
            | IrOp::DoUntil { body, .. }
            | IrOp::ForEach { body, .. }
            | IrOp::Trap { body, .. } => walk_ops(body, visit),
            _ => {}
        }
    }
}
