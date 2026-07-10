use super::*;

use std::collections::HashMap;

pub(super) struct FunctionPlanBuilder<'a> {
    pub(super) function_symbols: &'a HashMap<String, String>,
    pub(super) import_symbols: &'a HashMap<String, String>,
    pub(super) type_storage: &'a HashMap<String, StorageType>,
    pub(super) local_slots: Vec<StackSlot>,
    pub(super) labels: Vec<PlanLabel>,
    pub(super) operations: Vec<String>,
    pub(super) calls: Vec<PlanCall>,
    pub(super) constants: HashMap<String, NirValue>,
    pub(super) next_label: usize,
}

impl FunctionPlanBuilder<'_> {
    pub(super) fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
        for op in ops {
            match op {
                NirOp::Bind {
                    name,
                    type_,
                    mutable,
                    value,
                } => {
                    if let Some(value) = value {
                        self.lower_value(value)?;
                    }
                    if let Some(value) = value {
                        if let Some(constant) = native_constant_value(value, &self.constants) {
                            self.constants.insert(name.clone(), constant);
                        } else {
                            self.constants.remove(name);
                        }
                    } else {
                        self.constants.remove(name);
                    }
                    let initializer = value
                        .as_ref()
                        .map(describe_value)
                        .unwrap_or_else(|| "default".to_string());
                    self.operations.push(format!(
                        "bind {} {} AS {} = {}",
                        if *mutable { "mutable" } else { "immutable" },
                        name,
                        type_,
                        initializer
                    ));
                    self.add_stack_slot(name, type_, *mutable)?;
                }
                NirOp::StateAssign { value, .. } => {
                    self.lower_value(value)?;
                }
                NirOp::Assign { name, value } => {
                    self.lower_value(value)?;
                    if let Some(constant) = native_constant_value(value, &self.constants) {
                        self.constants.insert(name.clone(), constant);
                    } else {
                        self.constants.remove(name);
                    }
                    self.operations
                        .push(format!("assign {name} = {}", describe_value(value)));
                }
                NirOp::StoreGlobal { name, type_, value } => {
                    if let Some(value) = value {
                        self.lower_value(value)?;
                    }
                    let initializer = value
                        .as_ref()
                        .map(describe_value)
                        .unwrap_or_else(|| "default".to_string());
                    let type_suffix = if type_.is_empty() {
                        String::new()
                    } else {
                        format!(" AS {type_}")
                    };
                    self.operations
                        .push(format!("storeGlobal {name}{type_suffix} = {initializer}"));
                }
                NirOp::Eval { value } => {
                    self.lower_value(value)?;
                    self.operations
                        .push(format!("eval {}", describe_value(value)));
                }
                NirOp::Return { value } => {
                    if let Some(value) = value {
                        self.lower_value(value)?;
                        self.operations
                            .push(format!("return {}", describe_value(value)));
                    } else {
                        self.operations.push("return".to_string());
                    }
                }
                NirOp::ExitLoop { kind } => {
                    self.operations
                        .push(format!("exit {}", plan_loop_kind_name(*kind)));
                }
                NirOp::ContinueLoop { kind } => {
                    self.operations
                        .push(format!("continue {}", plan_loop_kind_name(*kind)));
                }
                NirOp::ExitProgram { code } => {
                    self.lower_value(code)?;
                    self.operations
                        .push(format!("exitProgram {}", describe_value(code)));
                }
                NirOp::Fail { error } => {
                    self.lower_value(error)?;
                    self.operations
                        .push(format!("fail {}", describe_value(error)));
                }
                NirOp::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    self.lower_value(condition)?;
                    let constants_before_if = self.constants.clone();
                    let else_label = self.add_label(LabelKind::IfElse);
                    let end_label = self.add_label(LabelKind::IfEnd);
                    self.operations.push(format!(
                        "branchIfFalse {} -> {}",
                        describe_value(condition),
                        else_label
                    ));
                    self.lower_ops(then_body)?;
                    self.operations.push(format!("branch -> {end_label}"));
                    self.operations.push(format!("label {else_label}"));
                    self.constants = constants_before_if;
                    if !else_body.is_empty() {
                        self.lower_ops(else_body)?;
                    }
                    self.operations.push(format!("label {end_label}"));
                    self.constants.clear();
                }
                NirOp::Match { value, cases } => {
                    self.lower_value(value)?;
                    self.operations
                        .push(format!("match {}", describe_value(value)));
                    let end_label = self.add_label(LabelKind::MatchEnd);
                    let constants_before_match = self.constants.clone();
                    for case in cases {
                        self.constants = constants_before_match.clone();
                        let case_label = self.add_label(LabelKind::MatchCase);
                        self.operations.push(format!(
                            "case {} -> {}",
                            describe_match_pattern(&case.pattern),
                            case_label
                        ));
                        self.operations.push(format!("label {case_label}"));
                        self.lower_ops(&case.body)?;
                        self.operations.push(format!("branch -> {end_label}"));
                    }
                    self.operations.push(format!("label {end_label}"));
                    self.constants.clear();
                }
                NirOp::While {
                    condition, body, ..
                } => {
                    let loop_label = self.add_label(LabelKind::WhileLoop);
                    let end_label = self.add_label(LabelKind::WhileEnd);
                    self.operations.push(format!("label {loop_label}"));
                    self.lower_value(condition)?;
                    self.operations.push(format!(
                        "branchIfFalse {} -> {end_label}",
                        describe_value(condition)
                    ));
                    self.lower_ops(body)?;
                    self.operations.push(format!("branch -> {loop_label}"));
                    self.operations.push(format!("label {end_label}"));
                    self.constants.clear();
                }
                NirOp::For {
                    name,
                    type_,
                    start,
                    end,
                    step,
                    body,
                    ..
                } => {
                    self.lower_value(start)?;
                    self.lower_value(end)?;
                    self.lower_value(step)?;
                    let loop_label = self.add_label(LabelKind::WhileLoop);
                    let end_label = self.add_label(LabelKind::WhileEnd);
                    self.operations.push(format!(
                        "for {name} AS {type_} = {} TO {} STEP {}",
                        describe_value(start),
                        describe_value(end),
                        describe_value(step)
                    ));
                    self.operations.push(format!("label {loop_label}"));
                    self.add_stack_slot(name, type_, true)?;
                    self.lower_ops(body)?;
                    self.operations.push(format!("branch -> {loop_label}"));
                    self.operations.push(format!("label {end_label}"));
                    self.constants.clear();
                }
                NirOp::DoUntil { body, condition } => {
                    let loop_label = self.add_label(LabelKind::WhileLoop);
                    let end_label = self.add_label(LabelKind::WhileEnd);
                    self.operations.push(format!("label {loop_label}"));
                    self.lower_ops(body)?;
                    self.lower_value(condition)?;
                    self.operations.push(format!(
                        "branchIfFalse {} -> {loop_label}",
                        describe_value(condition)
                    ));
                    self.operations.push(format!("label {end_label}"));
                    self.constants.clear();
                }
                NirOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                } => {
                    self.lower_value(iterable)?;
                    self.operations.push(format!(
                        "forEach {name} AS {type_} IN {}",
                        describe_value(iterable)
                    ));
                    let constants_before_loop = self.constants.clone();
                    self.add_stack_slot(name, type_, false)?;
                    self.lower_ops(body)?;
                    self.constants = constants_before_loop;
                    self.operations.push("next".to_string());
                }
                NirOp::Trap { name, body } => {
                    self.operations.push(format!("trap {name}"));
                    self.lower_ops(body)?;
                    self.operations.push("end trap".to_string());
                }
            }
        }
        Ok(())
    }

    fn lower_value(&mut self, value: &NirValue) -> Result<(), String> {
        if native_static_string_value(value, &self.constants).is_some() {
            return Ok(());
        }
        if let NirValue::RuntimeCall { target, args, .. }
        | NirValue::Call { target, args, .. }
        | NirValue::CallResult { target, args, .. } = value
        {
            if native_static_graphemes_value(target, args, &self.constants).is_some() {
                return Ok(());
            }
        }
        match value {
            NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
                for arg in args {
                    self.lower_value(arg)?;
                }
                if runtime::is_native_direct_call(target) {
                    // direct call: nothing to register.
                } else if let Some(helper) = runtime::helper_for_call(target) {
                    // An inline `TRAP` on a helper-backed built-in (a
                    // helper-targeted `CallResult`) needs the runtime helper
                    // symbol registered, exactly like a `RuntimeCall`.
                    self.add_runtime_call(helper, target, args);
                } else {
                    self.add_call(target);
                }
            }
            NirValue::RuntimeCall {
                helper,
                target,
                args,
                ..
            } => {
                if target == "typeName" {
                    for arg in args {
                        self.lower_value(arg)?;
                    }
                    return Ok(());
                }
                for arg in args {
                    self.lower_value(arg)?;
                }
                if !runtime::is_native_direct_call(target) {
                    self.add_runtime_call(*helper, target, args);
                }
            }
            NirValue::Constructor { args, .. } => {
                for arg in args {
                    self.lower_value(arg)?;
                }
            }
            NirValue::UnionWrap { value, .. }
            | NirValue::UnionExtract { value, .. }
            | NirValue::ResultIsOk { value }
            | NirValue::ResultValue { value }
            | NirValue::ResultError { value } => {
                self.lower_value(value)?;
            }
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                self.lower_value(target)?;
                for update in updates {
                    self.lower_value(&update.value)?;
                }
            }
            NirValue::ListLiteral { values, .. } => {
                for value in values {
                    self.lower_value(value)?;
                }
            }
            NirValue::MapLiteral { entries, .. } => {
                for (key, value) in entries {
                    self.lower_value(key)?;
                    self.lower_value(value)?;
                }
            }
            NirValue::MemberAccess { target, .. } => self.lower_value(target)?,
            NirValue::Binary { left, right, .. } => {
                self.lower_value(left)?;
                self.lower_value(right)?;
            }
            NirValue::Unary { operand, .. } => self.lower_value(operand)?,
            NirValue::Const { type_, .. } => {
                storage_for_type(type_, self.type_storage)?;
            }
            NirValue::FunctionRef { type_, .. } => {
                storage_for_type(type_, self.type_storage)?;
            }
            NirValue::Closure {
                type_, captures, ..
            } => {
                storage_for_type(type_, self.type_storage)?;
                for value in captures {
                    self.lower_value(value)?;
                }
            }
            NirValue::Capture { type_, .. } => {
                storage_for_type(type_, self.type_storage)?;
            }
            NirValue::Local(_) | NirValue::LocalRef { .. } | NirValue::Global { .. } => {}
        }
        Ok(())
    }

    fn add_stack_slot(&mut self, name: &str, type_: &str, mutable: bool) -> Result<(), String> {
        let storage = storage_for_type(type_, self.type_storage)?;
        let offset = -((self.local_slots.len() as i32 + 1) * storage.size.max(8) as i32);
        self.local_slots.push(StackSlot {
            name: name.to_string(),
            storage,
            offset,
            mutable,
        });
        Ok(())
    }

    fn add_label(&mut self, kind: LabelKind) -> String {
        let id = self.next_label;
        self.next_label += 1;
        let prefix = match kind {
            LabelKind::IfElse => "if_else",
            LabelKind::IfEnd => "if_end",
            LabelKind::MatchCase => "match_case",
            LabelKind::MatchEnd => "match_end",
            LabelKind::WhileLoop => "while_loop",
            LabelKind::WhileEnd => "while_end",
        };
        let name = format!("{prefix}_{id}");
        self.labels.push(PlanLabel {
            name: name.clone(),
            kind,
        });
        name
    }

    fn add_call(&mut self, target: &str) {
        let (kind, symbol) = if let Some(symbol) = self.function_symbols.get(target) {
            (CallKind::Local, symbol.clone())
        } else if let Some(symbol) = self.import_symbols.get(target) {
            (CallKind::Import, symbol.clone())
        } else if let Some(helper) = runtime::helper_for_call(target) {
            (CallKind::Runtime, runtime::symbol_for_call(helper, target))
        } else {
            // An indirect call dispatches through a `FUNC`-typed value (a local,
            // parameter, or lambda binding); there is no linker symbol for the
            // callee. Record no symbol so the object plan cannot mistake the
            // source binding's name for a relocation target (bug-72).
            (CallKind::Indirect, String::new())
        };
        push_call(&mut self.calls, target, symbol, kind);
    }

    fn add_runtime_call(
        &mut self,
        helper: super::runtime::RuntimeHelper,
        target: &str,
        args: &[NirValue],
    ) {
        push_call_with_literals(
            &mut self.calls,
            target,
            runtime::symbol_for_call(helper, target),
            CallKind::Runtime,
            string_literals(args),
        );
    }
}

pub(super) fn push_call(calls: &mut Vec<PlanCall>, target: &str, symbol: String, kind: CallKind) {
    push_call_with_literals(calls, target, symbol, kind, Vec::new());
}

pub(super) fn push_call_with_literals(
    calls: &mut Vec<PlanCall>,
    target: &str,
    symbol: String,
    kind: CallKind,
    string_literals: Vec<String>,
) {
    if calls
        .iter()
        .any(|call| call.target == target && call.symbol == symbol)
    {
        return;
    }
    calls.push(PlanCall {
        target: target.to_string(),
        symbol,
        kind,
        string_literals,
    });
}

pub(super) fn string_literals(values: &[NirValue]) -> Vec<String> {
    let mut literals = Vec::new();
    for value in values {
        collect_string_literals(value, &mut literals);
    }
    literals
}

pub(super) fn describe_value(value: &NirValue) -> String {
    match value {
        NirValue::Const { type_, value } => format!("{type_}({value})"),
        NirValue::Local(name) => format!("local {name}"),
        NirValue::LocalRef { name, .. } => format!("localRef {name}"),
        NirValue::Global { name, .. } => format!("global {name}"),
        NirValue::FunctionRef { name, .. } => format!("functionRef {name}"),
        NirValue::Closure { name, captures, .. } => format!(
            "closure {name}[{}]",
            captures
                .iter()
                .map(describe_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        NirValue::Capture { index, .. } => format!("capture[{index}]"),
        NirValue::Call { target, args, .. } => {
            let args = args
                .iter()
                .map(describe_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("call {target}({args})")
        }
        NirValue::CallResult { target, args, .. } => {
            let args = args
                .iter()
                .map(describe_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("callResult {target}({args})")
        }
        NirValue::RuntimeCall {
            helper,
            target,
            args,
            ..
        } => {
            let args = args
                .iter()
                .map(describe_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("runtimeCall {} {target}({args})", helper.name())
        }
        NirValue::Constructor { type_, args } => {
            let args = args
                .iter()
                .map(describe_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("construct {type_}({args})")
        }
        NirValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => format!(
            "wrap {member_type} as {union_type} ({})",
            describe_value(value)
        ),
        NirValue::UnionExtract { type_, value } => {
            format!("extract {type_} ({})", describe_value(value))
        }
        NirValue::ResultIsOk { value } => {
            format!("resultIsOk ({})", describe_value(value))
        }
        NirValue::ResultValue { value } => {
            format!("resultValue ({})", describe_value(value))
        }
        NirValue::ResultError { value } => {
            format!("resultError ({})", describe_value(value))
        }
        NirValue::WithUpdate {
            type_,
            target,
            updates,
        } => {
            let updates = updates
                .iter()
                .map(|update| format!("{} := {}", update.field, describe_value(&update.value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("with {type_} {} {{ {updates} }}", describe_value(target))
        }
        NirValue::ListLiteral { values, .. } => {
            let values = values
                .iter()
                .map(describe_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("list [{values}]")
        }
        NirValue::MapLiteral { entries, .. } => {
            let entries = entries
                .iter()
                .map(|(key, value)| format!("{}: {}", describe_value(key), describe_value(value)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("map {{{entries}}}")
        }
        NirValue::MemberAccess { target, member } => match target.as_ref() {
            NirValue::Local(name) => format!("{name}.{member}"),
            _ => format!("{}.{}", describe_value(target), member),
        },
        NirValue::Binary {
            op, left, right, ..
        } => {
            format!(
                "({} {} {})",
                describe_value(left),
                op,
                describe_value(right)
            )
        }
        NirValue::Unary { op, operand, .. } => format!("({op} {})", describe_value(operand)),
    }
}

pub(super) fn plan_loop_kind_name(kind: crate::ast::LoopKind) -> &'static str {
    match kind {
        crate::ast::LoopKind::For => "FOR",
        crate::ast::LoopKind::Do => "DO",
        crate::ast::LoopKind::While => "WHILE",
    }
}

pub(super) fn describe_match_pattern(pattern: &super::nir::NirMatchPattern) -> String {
    match pattern {
        super::nir::NirMatchPattern::Else => "else".to_string(),
        super::nir::NirMatchPattern::Value(NirValue::Local(name)) => name.clone(),
        super::nir::NirMatchPattern::Value(value) => describe_value(value),
        super::nir::NirMatchPattern::OneOf(values) => values
            .iter()
            .map(describe_value)
            .collect::<Vec<_>>()
            .join(", "),
    }
}

pub(super) fn collect_string_literals(value: &NirValue, literals: &mut Vec<String>) {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => {
            if !literals.contains(value) {
                literals.push(value.clone());
            }
        }
        NirValue::Call { args, .. }
        | NirValue::CallResult { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_literals(arg, literals);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => collect_string_literals(value, literals),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_string_literals(target, literals);
            for update in updates {
                collect_string_literals(&update.value, literals);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_string_literals(value, literals);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_string_literals(key, literals);
                collect_string_literals(value, literals);
            }
        }
        NirValue::MemberAccess { target, .. } => collect_string_literals(target, literals),
        NirValue::Binary { left, right, .. } => {
            collect_string_literals(left, literals);
            collect_string_literals(right, literals);
        }
        NirValue::Unary { operand, .. } => collect_string_literals(operand, literals),
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_string_literals(value, literals);
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
