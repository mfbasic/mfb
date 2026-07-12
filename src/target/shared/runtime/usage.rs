use super::*;

use crate::ir::{IrOp, IrProject, IrValue};
use std::collections::HashMap;

pub(crate) fn is_native_direct_call(name: &str) -> bool {
    // The migrated `collections::`/`strings::` members (get, transform, the List
    // and String overloads of find/mid/replace, ...) arrive qualified and are
    // lowered inline; their bare names are freed for user code
    // (plan-01-functions.md §5).
    if crate::builtins::native_builtin_target(name).is_some() {
        return true;
    }
    matches!(
        name,
        "len"
            | "fs.pathBaseName"
            | "fs.pathDirName"
            | "fs.pathExtension"
            | "fs.pathJoin"
            | "fs.pathNormalize"
            | "toByte"
            | "toFixed"
            | "toFloat"
            | "toInt"
            | "toMoney"
            | "toString"
            | "isEmpty"
            | "isEven"
            | "isNegative"
            | "isNotEmpty"
            | "isOdd"
            | "isPositive"
            | "isNumeric"
            | "isZero"
            | "math.abs"
            | "math.min"
            | "math.max"
            | "math.clamp"
            | "math.floor"
            | "math.ceil"
            | "math.round"
            | "math.sqrt"
            | "math.pow"
            | "math.exp"
            | "math.log"
            | "math.log10"
            | "math.sin"
            | "math.cos"
            | "math.tan"
            | "math.asin"
            | "math.acos"
            | "math.atan"
            | "math.atan2"
            | "math.rand"
            | "math.seed"
            | "money.setRounding"
            | "money.getRounding"
            | "money.round"
            | "bits.band"
            | "bits.bor"
            | "bits.bxor"
            | "bits.bnot"
            | "bits.sl"
            | "bits.sr"
            | "bits.sra"
            | "bits.rl32"
            | "bits.rr32"
            | "bits.rl64"
            | "bits.rr64"
            | "bits.clz"
            | "bits.ctz"
            | "bits.popCount"
            | "bits.bswap16"
            | "bits.bswap32"
            | "bits.bswap64"
            | "strings.byteLen"
            | "strings.toBytes"
            | "strings.caseFold"
            | "strings.contains"
            | "strings.endsWith"
            | "strings.graphemes"
            | "strings.lower"
            | "strings.normalizeNfc"
            | "strings.startsWith"
            | "strings.split"
            | "strings.trim"
            | "strings.trimEnd"
            | "strings.trimStart"
            | "strings.upper"
            | "strings.join"
            | "strings.startsWithAny"
            | "strings.endsWithAny"
            | "strings.stripPrefix"
            | "strings.stripSuffix"
            | "strings.count"
            | "strings.left"
            | "strings.right"
            | "strings.repeat"
            | "strings.padLeft"
            | "strings.padRight"
            | "strings.graphemeAt"
            | "strings.graphemesCount"
            | "strings.trimChars"
    )
}

pub fn required_helpers(ir: &IrProject) -> Vec<RuntimeHelper> {
    let mut helpers = Vec::new();
    // Resource unions drop by dispatching to each variant's close op, so a bind
    // of a resource-union type pulls in every variant's close helper.
    let resource_union_closes: HashMap<String, Vec<&'static str>> = ir
        .types
        .iter()
        .filter(|type_| type_.kind == "union")
        .filter_map(|type_| {
            let closes: Vec<&'static str> = type_
                .variants
                .iter()
                .map(|variant| crate::builtins::resource_close_function(&variant.name))
                .collect::<Option<Vec<_>>>()?;
            if closes.is_empty() {
                return None;
            }
            Some((type_.name.clone(), closes))
        })
        .collect();
    for function in &ir.functions {
        push_op_helpers(&function.body, &resource_union_closes, &mut helpers);
    }
    helpers
}

fn push_op_helpers(
    ops: &[IrOp],
    resource_union_closes: &HashMap<String, Vec<&'static str>>,
    helpers: &mut Vec<RuntimeHelper>,
) {
    for op in ops {
        match op {
            IrOp::Bind { type_, value, .. } => {
                if let Some(close) = crate::builtins::resource_close_function(type_) {
                    if let Some(helper) = helper_for_call(close) {
                        push_unique(helpers, helper);
                    }
                }
                if let Some(closes) = resource_union_closes.get(type_) {
                    for close in closes {
                        if let Some(helper) = helper_for_call(close) {
                            push_unique(helpers, helper);
                        }
                    }
                }
                if let Some(value) = value {
                    push_value_helpers(value, helpers);
                }
            }
            IrOp::Fail { error, .. } => {
                push_value_helpers(error, helpers);
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value, .. } => {
                push_value_helpers(value, helpers);
            }
            IrOp::Return { value, .. } => {
                if let Some(value) = value {
                    push_value_helpers(value, helpers);
                }
            }
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::ExitProgram { code, .. } => push_value_helpers(code, helpers),
            IrOp::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                push_value_helpers(condition, helpers);
                push_op_helpers(then_body, resource_union_closes, helpers);
                push_op_helpers(else_body, resource_union_closes, helpers);
            }
            IrOp::Match { value, cases, .. } => {
                push_value_helpers(value, helpers);
                for case in cases {
                    // A helper called only inside a `WHEN` guard must be
                    // collected too; `validate_nir` walks guards into its
                    // used-helper set, so skipping them here trips the strict
                    // declared-vs-used parity check (bug-118).
                    if let Some(guard) = &case.guard {
                        push_value_helpers(guard, helpers);
                    }
                    push_op_helpers(&case.body, resource_union_closes, helpers);
                }
            }
            IrOp::While {
                condition, body, ..
            } => {
                push_value_helpers(condition, helpers);
                push_op_helpers(body, resource_union_closes, helpers);
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                push_value_helpers(start, helpers);
                push_value_helpers(end, helpers);
                push_value_helpers(step, helpers);
                push_op_helpers(body, resource_union_closes, helpers);
            }
            IrOp::DoUntil {
                body, condition, ..
            } => {
                push_op_helpers(body, resource_union_closes, helpers);
                push_value_helpers(condition, helpers);
            }
            IrOp::ForEach { iterable, body, .. } => {
                push_value_helpers(iterable, helpers);
                push_op_helpers(body, resource_union_closes, helpers);
            }
            IrOp::Trap { body, .. } => {
                push_op_helpers(body, resource_union_closes, helpers);
            }
        }
    }
}

fn push_value_helpers(value: &IrValue, helpers: &mut Vec<RuntimeHelper>) {
    match value {
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            if !is_native_direct_call(target) {
                if let Some(helper) = helper_for_call(target) {
                    push_unique(helpers, helper);
                }
            }
            for arg in args {
                push_value_helpers(arg, helpers);
            }
        }
        IrValue::MemberAccess { target, .. } => {
            // No `.result` heuristic: `Thread.result` was removed from the
            // language (TYPE_THREAD_RESULT_REMOVED), so every surviving
            // `.result` is a user record/enum field access — declaring the
            // Thread helper for it was a pure false positive that rejected valid
            // programs (bug-119). `validate_nir` never counted MemberAccess, so
            // the declared-but-unused check fired.
            push_value_helpers(target, helpers);
        }
        IrValue::Binary { left, right, .. } => {
            push_value_helpers(left, helpers);
            push_value_helpers(right, helpers);
        }
        IrValue::Unary { operand, .. } => push_value_helpers(operand, helpers),
        IrValue::Constructor { args, .. } => {
            for arg in args {
                push_value_helpers(arg, helpers);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value, .. }
        | IrValue::ResultError { value } => {
            push_value_helpers(value, helpers);
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            push_value_helpers(target, helpers);
            for update in updates {
                push_value_helpers(&update.value, helpers);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for value in values {
                push_value_helpers(value, helpers);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                push_value_helpers(key, helpers);
                push_value_helpers(value, helpers);
            }
        }
        IrValue::Closure { captures, .. } => {
            for value in captures {
                push_value_helpers(value, helpers);
            }
        }
        IrValue::Capture { .. }
        | IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::LocalRef { .. }
        | IrValue::Global(_)
        | IrValue::FunctionRef { .. } => {}
    }
}

fn push_unique(helpers: &mut Vec<RuntimeHelper>, helper: RuntimeHelper) {
    if !helpers.contains(&helper) {
        helpers.push(helper);
    }
}
