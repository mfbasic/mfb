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
mod tests {
    use super::super::local_rewrites::testutil::*;
    use super::*;
    use crate::operators::BinaryOp;
    use crate::optimizer::{with_opt_level, OptLevel};
    use crate::target::shared::nir::{NirParam, NirSourceLoc};

    fn loc(line: u32) -> NirSourceLoc {
        NirSourceLoc {
            line,
            column: 3,
            file: 0,
        }
    }

    fn local(name: &str) -> NirValue {
        NirValue::Local(name.to_string())
    }

    fn binary(op: BinaryOp, left: NirValue, right: NirValue, line: u32) -> NirValue {
        NirValue::Binary {
            op,
            left: Box::new(left),
            right: Box::new(right),
            loc: loc(line),
        }
    }

    fn call(target: &str, args: Vec<NirValue>) -> NirValue {
        NirValue::Call {
            target: target.to_string(),
            args,
            loc: loc(99),
        }
    }

    fn function(
        name: &str,
        file: &str,
        params: &[(&str, ParameterType)],
        returns: ParameterType,
        body: Vec<NirOp>,
    ) -> NirFunction {
        NirFunction {
            name: name.to_string(),
            visibility: "public".to_string(),
            kind: "func".to_string(),
            isolated: false,
            params: params
                .iter()
                .map(|(name, type_)| NirParam {
                    name: name.to_string(),
                    type_: type_.clone(),
                    default: None,
                })
                .collect(),
            returns,
            body,
            file: file.to_string(),
            resource_owners: HashMap::new(),
        }
    }

    /// `gi(x, y) = x * 29 + y`, written in `grid.mfb` at line 38.
    fn gi(file: &str) -> NirFunction {
        function(
            "gi",
            file,
            &[("x", ParameterType::Integer), ("y", ParameterType::Integer)],
            ParameterType::Integer,
            vec![NirOp::Return {
                value: Some(binary(
                    BinaryOp::Add,
                    binary(BinaryOp::Multiply, local("x"), int_const("29"), 38),
                    local("y"),
                    38,
                )),
            }],
        )
    }

    /// A caller in `file` binding `i` and `j` as Integers and returning
    /// `value`.
    fn caller(file: &str, value: NirValue) -> NirFunction {
        function(
            "caller",
            file,
            &[("i", ParameterType::Integer)],
            ParameterType::Integer,
            vec![
                NirOp::Bind {
                    mutable: false,
                    name: "j".to_string(),
                    type_: ParameterType::Integer,
                    value: Some(int_const("4")),
                },
                NirOp::Return { value: Some(value) },
            ],
        )
    }

    fn run(module: &mut NirModule, level: u8) {
        with_opt_level(OptLevel(level), || inline_leaf_calls(module));
    }

    fn returned(module: &NirModule) -> NirValue {
        match &module.functions[1].body[1] {
            NirOp::Return { value: Some(value) } => value.clone(),
            _ => panic!("expected the caller's return"),
        }
    }

    /// The row's core: the call becomes the helper's expression with each
    /// parameter replaced by its argument — here in swapped order, so a naive
    /// sequential rename would be caught.
    #[test]
    fn a_leaf_call_becomes_its_expression() {
        let mut module = test_module(vec![
            gi("grid.mfb"),
            caller("grid.mfb", call("gi", vec![local("j"), local("i")])),
        ]);
        run(&mut module, 1);
        let NirValue::Binary {
            op: BinaryOp::Add,
            left,
            right,
            loc,
        } = returned(&module)
        else {
            panic!("expected the inlined add");
        };
        assert_eq!((loc.line, loc.file), (38, 0), "same file: no file tag");
        assert!(matches!(*right, NirValue::Local(ref name) if name == "i"));
        let NirValue::Binary { left: x, .. } = *left else {
            panic!("expected the inlined multiply");
        };
        assert!(matches!(*x, NirValue::Local(ref name) if name == "j"));
    }

    /// Across files, every located node of the inlined expression names the
    /// helper's file, so its errors report where they were written.
    #[test]
    fn a_cross_file_inline_tags_the_helpers_file() {
        let mut module = test_module(vec![
            gi("src/grid.mfb"),
            caller(
                "src/rooms.mfb",
                call("gi", vec![local("i"), int_const("7")]),
            ),
        ]);
        run(&mut module, 1);
        let NirValue::Binary { loc, left, .. } = returned(&module) else {
            panic!("expected the inlined add");
        };
        let grid = crate::target::shared::nir::source_file_id("src/grid.mfb");
        assert_eq!(loc.file, grid);
        let NirValue::Binary { loc: inner, .. } = *left else {
            panic!("expected the inlined multiply");
        };
        assert_eq!(inner.file, grid);
        assert_eq!(
            crate::target::shared::nir::source_file_name(grid).as_deref(),
            Some("src/grid.mfb")
        );
    }

    /// Everything the module docs exclude keeps its call.
    #[test]
    fn ineligible_calls_are_left_alone() {
        let non_atomic = call(
            "gi",
            vec![
                binary(BinaryOp::Add, local("i"), int_const("1"), 5),
                local("j"),
            ],
        );
        let mistyped = call(
            "gi",
            vec![
                NirValue::Const {
                    type_: ParameterType::Byte,
                    value: "1".to_string(),
                },
                local("j"),
            ],
        );
        let unknown_local = call("gi", vec![local("nowhere"), local("j")]);
        let short = call("gi", vec![local("i")]);
        for value in [non_atomic, mistyped, unknown_local, short] {
            let mut module = test_module(vec![gi("grid.mfb"), caller("grid.mfb", value)]);
            run(&mut module, 1);
            assert!(matches!(returned(&module), NirValue::Call { .. }));
        }
    }

    /// A call whose target names a local (a `FUNC`-typed parameter sharing the
    /// module function's name) is an indirect call and keeps it — the
    /// `rt-behavior/general/typename_user_call` shape.
    #[test]
    fn a_call_through_a_same_named_local_is_left_alone() {
        let mut through_param = caller("grid.mfb", call("gi", vec![local("i"), local("j")]));
        through_param.params.push(NirParam {
            name: "gi".to_string(),
            type_: ParameterType::Unknown,
            default: None,
        });
        let mut module = test_module(vec![gi("grid.mfb"), through_param]);
        run(&mut module, 1);
        assert!(matches!(returned(&module), NirValue::Call { .. }));
    }

    /// A callee that is not a scalar one-expression leaf is never inlined.
    #[test]
    fn non_leaf_callees_are_left_alone() {
        let helper_calling_user_code = function(
            "gi",
            "grid.mfb",
            &[("x", ParameterType::Integer)],
            ParameterType::Integer,
            vec![NirOp::Return {
                value: Some(call("caller", vec![local("x")])),
            }],
        );
        let two_statements = function(
            "gi",
            "grid.mfb",
            &[("x", ParameterType::Integer)],
            ParameterType::Integer,
            vec![
                NirOp::Eval { value: local("x") },
                NirOp::Return {
                    value: Some(local("x")),
                },
            ],
        );
        let string_result = function(
            "gi",
            "grid.mfb",
            &[("x", ParameterType::Integer)],
            ParameterType::String,
            vec![NirOp::Return {
                value: Some(NirValue::Const {
                    type_: ParameterType::String,
                    value: "s".to_string(),
                }),
            }],
        );
        let mut isolated = gi("grid.mfb");
        isolated.isolated = true;
        // A body the optimizer synthesized (kind `function`) is not a user `func`.
        let mut synthesized = gi("grid.mfb");
        synthesized.kind = "function".to_string();
        for callee in [
            helper_calling_user_code,
            two_statements,
            string_result,
            isolated,
            synthesized,
        ] {
            let args = callee.params.iter().map(|_| local("i")).collect();
            let mut module = test_module(vec![callee, caller("grid.mfb", call("gi", args))]);
            run(&mut module, 1);
            assert!(matches!(returned(&module), NirValue::Call { .. }));
        }
    }

    /// A call the inline-`TRAP` desugar wraps (`CallResult`), or one sitting
    /// under an error capture, is left alone.
    #[test]
    fn calls_under_an_error_capture_are_left_alone() {
        let captured = NirValue::Checked {
            type_: ParameterType::Integer,
            value: Box::new(call("gi", vec![local("i"), local("j")])),
        };
        let mut module = test_module(vec![gi("grid.mfb"), caller("grid.mfb", captured)]);
        run(&mut module, 1);
        let NirValue::Checked { value, .. } = returned(&module) else {
            panic!("expected the capture");
        };
        assert!(matches!(*value, NirValue::Call { .. }));

        let result = NirValue::CallResult {
            target: "gi".to_string(),
            args: vec![local("i"), local("j")],
            loc: loc(1),
        };
        let mut module = test_module(vec![gi("grid.mfb"), caller("grid.mfb", result)]);
        run(&mut module, 1);
        assert!(matches!(returned(&module), NirValue::CallResult { .. }));
    }

    /// Off at `-O0`.
    #[test]
    fn the_row_is_off_at_level_zero() {
        let mut module = test_module(vec![
            gi("grid.mfb"),
            caller("grid.mfb", call("gi", vec![local("i"), local("j")])),
        ]);
        run(&mut module, 0);
        assert!(matches!(returned(&module), NirValue::Call { .. }));
    }
}
