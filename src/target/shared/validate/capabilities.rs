use super::*;

use super::super::nir::constfold::{
    native_constant_value, native_static_graphemes_value, native_static_string_value,
};

pub(crate) fn validate_capabilities(
    module: &NirModule,
    capabilities: &BackendCapabilities,
) -> Result<(), String> {
    let mut runtime_calls = Vec::new();
    for function in &module.functions {
        collect_runtime_calls_from_ops(&function.body, &mut runtime_calls);
    }
    for call in &runtime_calls {
        if runtime::is_native_direct_call(call) {
            continue;
        }
        if !capabilities.runtime_calls.contains(&call.as_str()) {
            return Err(format!(
                "native backend does not support runtime call '{call}'"
            ));
        }
    }
    for helper in &module.runtime_helpers {
        let helper_used_by_emitted_call = runtime_calls
            .iter()
            .any(|call| runtime::helper_for_call(call) == Some(*helper));
        if !helper_used_by_emitted_call {
            continue;
        }
        // A family is implemented when at least one catalogued spec exists for
        // it with a non-empty `returns`. The former `params`/`clobbers`
        // conditions went away with the fields themselves (bug-329): they were
        // unread transcriptions, and because this is an `any()` over the whole
        // family, a single sibling spec satisfied them anyway — they could
        // never detect an under-described helper. `catalog_is_consistent`
        // asserts every catalogued spec has a non-empty `returns`.
        let helper_supported = runtime::supported_helper_specs()
            .iter()
            .any(|spec| spec.helper == *helper && !spec.abi.returns.is_empty());
        if !helper_supported {
            return Err(format!(
                "native backend does not implement runtime helper '{}'",
                helper.name()
            ));
        }
    }
    Ok(())
}

/// Collect the type strings of every `Bind` op (recursively) so resource-union
/// binds can be matched against union type definitions. Descends through the
/// shared NIR seam (bug-328).
pub(super) fn collect_bind_types(ops: &[NirOp], types: &mut HashSet<String>) {
    use super::super::nir::visit::{walk_op, NirVisitor};
    struct Collector<'a> {
        types: &'a mut HashSet<String>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Bind { type_, .. } = op {
                // Strip any `STATE T` suffix (plan-74): a stateful resource-union
                // bind (`Stream STATE Cursor`) names the same union as its bare
                // form, and the union used-helper check below matches against the
                // bare union type name — so its variants' close helpers must be
                // recognized as used, or a valid stateful union bind trips the
                // "declares unused runtime helper" guard.
                self.types.insert(type_.without_state().name().into_owned());
            }
            walk_op(self, op);
        }
    }
    Collector { types }.visit_ops(ops);
}

/// Collect the declared type of every `Bind` that **owns** the resource it
/// binds — i.e. every bind whose scope exit emits the registered close op.
///
/// bug-535: a plain built-in resource `Bind` drops through a codegen-emitted
/// close, never through an NIR call, so `validate_nir`'s `used_helpers` set
/// could not see it and a module whose only reference to a package was such a
/// bind (`RES s AS tcp::Socket = thread::accept(t, 1000)` in a worker) was
/// rejected with "NIR declares unused runtime helper". This is the collector
/// for the arm that fixes it, and it is deliberately the twin of
/// `runtime::usage::push_op_helpers`, the code that DECLARES those helpers:
/// the two sets are compared against each other, so any divergence turns one
/// arm of that comparison into a false error.
///
/// Hence the aliasing gate, mirroring `runtime::usage::value_aliases_live_resource`
/// (bug-375, §15.6): a bind whose initializer only names an already-live
/// resource emits no close, the declarer adds no helper for it, and counting one
/// as used here would trip the opposite arm, "NIR runtime call requires
/// undeclared helper".
pub(super) fn collect_owning_resource_bind_types(ops: &[NirOp], types: &mut Vec<ParameterType>) {
    use super::super::nir::visit::{walk_op, NirVisitor};
    struct Collector<'a> {
        types: &'a mut Vec<ParameterType>,
    }
    impl NirVisitor for Collector<'_> {
        fn visit_op(&mut self, op: &NirOp) {
            if let NirOp::Bind { type_, value, .. } = op {
                if !value.as_ref().is_some_and(value_aliases_live_resource)
                    && !self.types.contains(type_)
                {
                    self.types.push(type_.clone());
                }
            }
            walk_op(self, op);
        }
    }
    Collector { types }.visit_ops(ops);
}

/// The NIR twin of `runtime::usage::value_aliases_live_resource`: whether a
/// `RES` bind's initializer merely names an already-live resource instead of
/// producing one (bug-375).
///
/// Kept deliberately identical to the IR-level rule the helper DECLARER uses,
/// rather than to `CodeBuilder::value_aliases_live_resource`, which recognizes
/// three further shapes. Validation compares the used set against the declared
/// one; matching the declarer means this arm can only ever add a helper the
/// declarer also added, so it can never manufacture an "undeclared helper"
/// error. Recognizing MORE shapes than the declarer would.
fn value_aliases_live_resource(value: &NirValue) -> bool {
    match value {
        NirValue::Local(_) => true,
        NirValue::Call { target, .. } | NirValue::CallResult { target, .. } => matches!(
            crate::codegen::registry::native_bare_target(target),
            Some("get" | "getOr")
        ),
        _ => false,
    }
}

pub(super) fn collect_runtime_calls_from_ops(ops: &[NirOp], calls: &mut Vec<String>) {
    let mut constants = HashMap::new();
    collect_runtime_calls_from_ops_with_constants(ops, calls, &mut constants);
}

/// The constant environment a loop body is analyzed under (bug-300 E12).
///
/// Empty, mirroring codegen: `builder_control` calls `clear_local_constants()`
/// before every loop body, because a local can be reassigned inside the body and a
/// loop-entry value therefore says nothing about later iterations. This pass used
/// to clone the enclosing constants instead and never invalidate anything, so a
/// call like `strings.upper(s)` folded away here while codegen emitted it for
/// real -- validate could clear a capability gate for a call the binary actually
/// makes. Clearing outright is exactly what codegen does, so the two now agree by
/// construction rather than by a second, parallel invalidation rule that could
/// drift.
pub(super) fn loop_body_constants() -> HashMap<String, NirValue> {
    HashMap::new()
}

pub(super) fn collect_runtime_calls_from_ops_with_constants(
    ops: &[NirOp],
    calls: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
) {
    for op in ops {
        match op {
            NirOp::Bind { name, value, .. } => {
                if let Some(value) = value {
                    collect_runtime_calls_from_value(value, calls, constants);
                    if let Some(constant) = native_constant_value(value, constants) {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_runtime_calls_from_value(value, calls, constants);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_runtime_calls_from_value(code, calls, constants);
            }
            NirOp::Fail { error } => {
                collect_runtime_calls_from_value(error, calls, constants);
            }
            NirOp::StateAssign { value, .. } => {
                collect_runtime_calls_from_value(value, calls, constants);
            }
            NirOp::Assign { name, value } => {
                collect_runtime_calls_from_value(value, calls, constants);
                if let Some(constant) = native_constant_value(value, constants) {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_runtime_calls_from_value(value, calls, constants);
                }
            }
            NirOp::Eval { value } => {
                collect_runtime_calls_from_value(value, calls, constants);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_runtime_calls_from_value(condition, calls, constants);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(
                    then_body,
                    calls,
                    &mut then_constants,
                );
                collect_runtime_calls_from_ops_with_constants(
                    else_body,
                    calls,
                    &mut else_constants,
                );
            }
            NirOp::Match { value, cases } => {
                collect_runtime_calls_from_value(value, calls, constants);
                for case in cases {
                    // A runtime call used only in a `WHEN … WHERE` guard still
                    // executes when the guard is evaluated, so capability
                    // validation must see it too — otherwise a backend-gated
                    // call hidden in a guard slips through unchecked. This is the
                    // exact traversal bug-118 added to the sibling passes in
                    // `plan/symbols.rs`; bug-328 makes it uniform. The guard is
                    // evaluated in the case's scope but before its body binds, so
                    // it reads the pre-case `constants`.
                    if let Some(guard) = &case.guard {
                        collect_runtime_calls_from_value(guard, calls, constants);
                    }
                    let mut case_constants = constants.clone();
                    collect_runtime_calls_from_ops_with_constants(
                        &case.body,
                        calls,
                        &mut case_constants,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_runtime_calls_from_value(condition, calls, constants);
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_runtime_calls_from_value(start, calls, constants);
                collect_runtime_calls_from_value(end, calls, constants);
                collect_runtime_calls_from_value(step, calls, constants);
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
                collect_runtime_calls_from_value(condition, calls, constants);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_runtime_calls_from_value(iterable, calls, constants);
                let mut body_constants = loop_body_constants();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut body_constants);
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                collect_runtime_calls_from_ops_with_constants(body, calls, &mut trap_constants);
            }
        }
    }
}

pub(super) fn collect_runtime_calls_from_value(
    value: &NirValue,
    calls: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
) {
    match value {
        NirValue::RuntimeCall { target, args, .. } => {
            if target != "typeName"
                && native_static_string_value(value, constants).is_none()
                && native_static_graphemes_value(target, args, constants).is_none()
                && !calls.contains(target)
            {
                calls.push(target.clone());
            }
            for arg in args {
                collect_runtime_calls_from_value(arg, calls, constants);
            }
        }
        // A **trapped** fallible runtime call is a `CallResult`, not a
        // `RuntimeCall`: `canvas::present(x) TRAP(e)` desugars to
        // `bind $trap_res0 = callResult{target: "canvas.present"}`. Walking only the
        // args therefore let every trapped call skip capability validation
        // entirely — the same call built fine wrapped in a TRAP and was correctly
        // rejected without one. Since a program almost always traps a fallible
        // call, that is the common case, not the rare one.
        //
        // `helper_for_call` is the predicate the sibling pass
        // (`runtime::usage::push_value_helpers`) already uses to tell a runtime
        // target from a user function, so the two agree by construction; a user
        // `FUNC` returning a `Result` does not name a helper family and is not
        // collected. `Constructor` is split out because its target is a type name,
        // which could never be a runtime call.
        NirValue::CallResult { target, args, .. } => {
            // Only a **package-qualified** target can be capability-gated: every
            // entry in a backend's `runtime_calls` is `pkg.member`. The bare-named
            // `general` family (`toString`, `toInteger`, …) also answers to
            // `helper_for_call`, but it is unconditionally available and appears in
            // no `runtime_calls` list — collecting it made every program that traps
            // a conversion fail validation. `NirValue::Call` needs no arm at all:
            // an untrapped runtime call arrives as `RuntimeCall`, so a `Call` is
            // either a user function or a bare general builtin.
            if target.contains('.')
                && crate::target::shared::runtime::helper_for_call(target).is_some()
                && !calls.contains(target)
            {
                calls.push(target.clone());
            }
            for arg in args {
                collect_runtime_calls_from_value(arg, calls, constants);
            }
        }
        NirValue::Call { args, .. } | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_runtime_calls_from_value(arg, calls, constants);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value }
        | NirValue::Checked { value, .. } => {
            collect_runtime_calls_from_value(value, calls, constants);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_runtime_calls_from_value(target, calls, constants);
            for update in updates {
                collect_runtime_calls_from_value(&update.value, calls, constants);
            }
        }
        NirValue::ListLiteral { values, .. } | NirValue::SetLiteral { values, .. } => {
            for value in values {
                collect_runtime_calls_from_value(value, calls, constants);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_runtime_calls_from_value(key, calls, constants);
                collect_runtime_calls_from_value(value, calls, constants);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_runtime_calls_from_value(target, calls, constants)
        }
        NirValue::Binary { left, right, .. } => {
            collect_runtime_calls_from_value(left, calls, constants);
            collect_runtime_calls_from_value(right, calls, constants);
        }
        NirValue::Unary { operand, .. } => {
            collect_runtime_calls_from_value(operand, calls, constants)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_runtime_calls_from_value(value, calls, constants);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::LocalRef { .. }
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}
