//! Leaf-function inlining — the Level-1 "Leaf function inlining" row
//! (plan-161-C, bug-740 mechanism (c)): a call to a one-expression helper whose
//! arguments are plain locals or literals becomes the helper's expression, with
//! each parameter replaced by its argument.
//!
//! brogue's `gi(x, y)` is `RETURN x * DROWS + y`; every call paid a frame, a
//! `bl`, the result protocol and the caller's error check to evaluate one
//! multiply and one add. The rewrite is exact only under all of these, each
//! checked per callee or per call site, never assumed:
//!
//! **The callee** ([`leaf_of`]):
//!
//! - a `FUNC` (not `ISOLATED`) whose whole body is `RETURN <expression>` — so
//!   there is no statement, local, loop, `TRAP` or cleanup to carry;
//! - every parameter and the result is a one-word scalar (`Integer`, `Byte`,
//!   `Boolean`, `Float`, `Fixed`, `Money`): a value, never storage, so no
//!   copy, ownership or `$own` variant question arises (plan-147 variants exist
//!   only for owned collection parameters);
//! - the expression is built only from literals, parameter reads, global reads,
//!   operators, and calls to names that are **not** module functions (builtins)
//!   whose arguments are themselves literals, parameters, globals or operators —
//!   so it runs no user code, takes no function value, and cannot reach a local
//!   of the caller;
//! - it is small ([`MAX_NODES`]).
//!
//! **The call site** ([`inline_call`]):
//!
//! - a plain `Call` (a `CallResult` belongs to an inline `TRAP`'s desugar and is
//!   left alone), with one argument per parameter;
//! - every argument is a literal of the parameter's type or a read of a caller
//!   local whose one declared type is the parameter's type. Such an argument
//!   has no effect, cannot fail, and its value cannot change while the
//!   expression runs (the expression calls no user code), so reading it where
//!   the parameter was read — once, several times, or not at all — is
//!   indistinguishable from evaluating it once before the call;
//! - a runtime error's `ErrorLoc` takes its line and column from the operator
//!   node, which keeps the callee's, and its file from the function being
//!   compiled — so when the caller is in another file, every located node of
//!   the inlined expression is tagged with the callee's file
//!   (`NirSourceLoc::file`). An error raised inside the inlined expression then
//!   names the helper's own file, line and column, exactly as the call did.
//!
//! What stays the same, and why: the operators are the same nodes (same checks,
//! same trap codes and locations); they run at the point the call ran, in the
//! same order; a failure inside them reaches the caller's `TRAP` handler or
//! propagates to its caller exactly as the call's failure did (both are routes
//! the handler takes — `mfb spec language error-model` §8.6 rule 3, and an
//! operator's own failure reaches it too). The callee is still emitted for every
//! other caller.

use std::collections::{HashMap, HashSet};

use crate::target::shared::nir::visit::walk_ops_values_mut;
use crate::target::shared::nir::{NirFunction, NirModule, NirOp, NirValue};
use crate::types::ParameterType;

/// The largest expression inlined, in NIR nodes. brogue's helpers are 5
/// (`gi`), 4 (`nbDx`) and 15 (`coordinatesAreInMap`) nodes.
const MAX_NODES: usize = 24;

/// A callee this row may substitute.
struct Leaf {
    params: Vec<(String, ParameterType)>,
    body: NirValue,
    file: String,
}

/// Inline every eligible call in the module. Level 1.
pub(crate) fn inline_leaf_calls(module: &mut NirModule) {
    if !crate::optimizer::level_enabled(1) {
        return;
    }
    let functions: HashSet<String> = module
        .functions
        .iter()
        .map(|function| function.name.clone())
        .collect();
    let leaves: HashMap<String, Leaf> = module
        .functions
        .iter()
        .filter_map(|function| Some((function.name.clone(), leaf_of(function, &functions)?)))
        .collect();
    if leaves.is_empty() {
        return;
    }
    let mut inlined = 0u64;
    for function in &mut module.functions {
        if has_call_inside_a_capture(&function.body) {
            continue;
        }
        let types = local_types(function);
        let file = function.file.clone();
        walk_ops_values_mut(&mut function.body, &mut |value| {
            if let Some(replacement) = inline_call(value, &leaves, &types, &file) {
                *value = replacement;
                inlined += 1;
            }
        });
    }
    crate::optimizer::stats::count_leaf_calls_inlined(inlined);
}

/// Whether a plain `Call` sits inside a `CallResult` or `Checked` — the inline
/// `TRAP` desugar's error-capturing wrappers. The desugar lifts every call out
/// of a trapped expression (`.ai/codegen-invariants.md`, "The inline-TRAP region
/// is built in `ir::lower`"), so this never holds today; if it ever did, an
/// inlined operator there would raise past the capture instead of into it, so
/// such a function is left untouched.
fn has_call_inside_a_capture(ops: &[NirOp]) -> bool {
    use crate::target::shared::nir::visit::{walk_value, NirVisitor};
    struct Scan {
        depth: usize,
        found: bool,
    }
    impl NirVisitor for Scan {
        fn visit_value(&mut self, value: &NirValue) {
            match value {
                NirValue::CallResult { .. } | NirValue::Checked { .. } => {
                    self.depth += 1;
                    walk_value(self, value);
                    self.depth -= 1;
                }
                NirValue::Call { .. } if self.depth > 0 => self.found = true,
                _ => walk_value(self, value),
            }
        }
    }
    let mut scan = Scan {
        depth: 0,
        found: false,
    };
    scan.visit_ops(ops);
    scan.found
}

/// A type whose value is one machine word with no storage of its own.
fn is_word_scalar(type_: &ParameterType) -> bool {
    matches!(
        type_,
        ParameterType::Integer
            | ParameterType::Byte
            | ParameterType::Boolean
            | ParameterType::Float
            | ParameterType::Fixed
            | ParameterType::Money
    )
}

/// `function` as a [`Leaf`], or `None` when it is not one (module docs).
fn leaf_of(function: &NirFunction, functions: &HashSet<String>) -> Option<Leaf> {
    if function.kind != "func" || function.isolated || !is_word_scalar(&function.returns) {
        return None;
    }
    let [NirOp::Return { value: Some(body) }] = function.body.as_slice() else {
        return None;
    };
    let params: Vec<(String, ParameterType)> = function
        .params
        .iter()
        .map(|param| (param.name.clone(), param.type_.clone()))
        .collect();
    if params.iter().any(|(_, type_)| !is_word_scalar(type_)) {
        return None;
    }
    let names: HashSet<&str> = params.iter().map(|(name, _)| name.as_str()).collect();
    let mut nodes = 0;
    if !pure_expression(body, &names, functions, true, &mut nodes) || nodes > MAX_NODES {
        return None;
    }
    Some(Leaf {
        params,
        body: body.clone(),
        file: function.file.clone(),
    })
}

/// Whether `value` is built only from literals, parameter reads, global reads,
/// operators, and (when `calls` allows it) builtin calls over those. Counts
/// nodes into `nodes`.
fn pure_expression(
    value: &NirValue,
    params: &HashSet<&str>,
    functions: &HashSet<String>,
    calls: bool,
    nodes: &mut usize,
) -> bool {
    *nodes += 1;
    match value {
        NirValue::Const { .. } | NirValue::Global { .. } => true,
        NirValue::Local(name) => params.contains(name.as_str()),
        NirValue::Binary { left, right, .. } => {
            pure_expression(left, params, functions, calls, nodes)
                && pure_expression(right, params, functions, calls, nodes)
        }
        NirValue::Unary { operand, .. } => {
            pure_expression(operand, params, functions, calls, nodes)
        }
        NirValue::Call { target, args, .. } => {
            calls
                && !functions.contains(target)
                && args
                    .iter()
                    .all(|arg| pure_expression(arg, params, functions, false, nodes))
        }
        // Everything else can hold storage, run user code, observe a local
        // by reference, or capture an error route: never part of a leaf.
        _ => false,
    }
}

/// Every local name the function binds, parameters included, with its one
/// declared type — or `None` for a name bound at two different types
/// (shadowing in different scopes), so no argument read of it is ever treated
/// as typed. The key set is also what [`inline_call`] checks a call target
/// against: a call whose target names a local is an indirect call through a
/// `FUNC` value (codegen resolves a local before a module function), never a
/// call to the module function that happens to share the name.
fn local_types(function: &NirFunction) -> HashMap<String, Option<ParameterType>> {
    fn note(types: &mut HashMap<String, Option<ParameterType>>, name: &str, type_: &ParameterType) {
        let entry = types
            .entry(name.to_string())
            .or_insert_with(|| Some(type_.clone()));
        if entry.as_ref() != Some(type_) {
            *entry = None;
        }
    }
    fn walk(ops: &[NirOp], types: &mut HashMap<String, Option<ParameterType>>) {
        for op in ops {
            match op {
                NirOp::Bind { name, type_, .. } => note(types, name, type_),
                NirOp::For {
                    name, type_, body, ..
                }
                | NirOp::ForEach {
                    name, type_, body, ..
                } => {
                    note(types, name, type_);
                    walk(body, types);
                }
                NirOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    walk(then_body, types);
                    walk(else_body, types);
                }
                NirOp::Match { cases, .. } => {
                    for case in cases {
                        walk(&case.body, types);
                    }
                }
                NirOp::While { body, .. } | NirOp::DoUntil { body, .. } => walk(body, types),
                NirOp::Trap { name, body } => {
                    // The handler's error binding is never a scalar.
                    note(types, name, &ParameterType::named("Error"));
                    walk(body, types);
                }
                NirOp::StoreGlobal { .. }
                | NirOp::Assign { .. }
                | NirOp::StateAssign { .. }
                | NirOp::Return { .. }
                | NirOp::ExitLoop { .. }
                | NirOp::ContinueLoop { .. }
                | NirOp::ExitProgram { .. }
                | NirOp::Fail { .. }
                | NirOp::Eval { .. } => {}
            }
        }
    }
    let mut types: HashMap<String, Option<ParameterType>> = HashMap::new();
    for param in &function.params {
        note(&mut types, &param.name, &param.type_);
    }
    walk(&function.body, &mut types);
    types
}

/// The inlined expression for `value` when it is an eligible call (module
/// docs), else `None`.
fn inline_call(
    value: &NirValue,
    leaves: &HashMap<String, Leaf>,
    types: &HashMap<String, Option<ParameterType>>,
    file: &str,
) -> Option<NirValue> {
    let NirValue::Call { target, args, .. } = value else {
        return None;
    };
    if types.contains_key(target) {
        // A local of this name shadows the module function: an indirect call.
        return None;
    }
    let leaf = leaves.get(target)?;
    if args.len() != leaf.params.len() {
        return None;
    }
    let mut bound: HashMap<&str, &NirValue> = HashMap::new();
    for ((name, type_), arg) in leaf.params.iter().zip(args) {
        let typed = match arg {
            NirValue::Const {
                type_: arg_type, ..
            } => arg_type == type_,
            NirValue::Local(local) => types.get(local).and_then(Option::as_ref) == Some(type_),
            _ => false,
        };
        if !typed {
            return None;
        }
        bound.insert(name.as_str(), arg);
    }
    let mut body = leaf.body.clone();
    if leaf.file != file {
        let callee_file = crate::target::shared::nir::source_file_id(&leaf.file);
        crate::target::shared::nir::visit::walk_value_mut(&mut body, &mut |node| {
            if let Some(loc) = located(node) {
                loc.file = callee_file;
            }
        });
    }
    // Arguments carry no location (a literal or a local read raises nothing),
    // so substituting after tagging leaves them exactly as the caller wrote
    // them.
    substitute(&mut body, &bound);
    Some(body)
}

/// The source location a node carries, if it can raise an error.
fn located(node: &mut NirValue) -> Option<&mut crate::target::shared::nir::NirSourceLoc> {
    match node {
        NirValue::Call { loc, .. }
        | NirValue::CallResult { loc, .. }
        | NirValue::RuntimeCall { loc, .. }
        | NirValue::Binary { loc, .. }
        | NirValue::Unary { loc, .. } => Some(loc),
        _ => None,
    }
}

/// Replace every parameter read in `value` with its argument.
fn substitute(value: &mut NirValue, bound: &HashMap<&str, &NirValue>) {
    crate::target::shared::nir::visit::walk_value_mut(value, &mut |node| {
        if let NirValue::Local(name) = node {
            if let Some(&arg) = bound.get(name.as_str()) {
                *node = arg.clone();
            }
        }
    });
}

#[cfg(test)]
mod tests;
