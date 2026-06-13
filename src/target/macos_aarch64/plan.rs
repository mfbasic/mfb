use std::collections::HashMap;

use crate::json_string;

use super::nir::{self, NirFunction, NirModule, NirOp, NirValue};
use super::runtime;

pub(crate) struct NativePlan {
    pub(crate) target: String,
    pub(crate) project: String,
    pub(crate) entry_symbol: Option<String>,
    pub(crate) runtime_symbols: Vec<String>,
    pub(crate) external_symbols: Vec<String>,
    pub(crate) platform_imports: Vec<PlatformImport>,
    pub(crate) functions: Vec<PlannedFunction>,
}

pub(crate) struct PlatformImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
    pub(crate) required_by: String,
}

pub(crate) struct PlannedFunction {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) returns: StorageType,
    pub(crate) params: Vec<PlannedParam>,
    pub(crate) local_slots: Vec<StackSlot>,
    pub(crate) labels: Vec<PlanLabel>,
    pub(crate) operations: Vec<String>,
    pub(crate) calls: Vec<PlanCall>,
}

pub(crate) struct PlannedParam {
    pub(crate) name: String,
    pub(crate) storage: StorageType,
}

pub(crate) struct StackSlot {
    pub(crate) name: String,
    pub(crate) storage: StorageType,
    pub(crate) offset: i32,
    pub(crate) mutable: bool,
}

pub(crate) struct PlanLabel {
    pub(crate) name: String,
    pub(crate) kind: LabelKind,
}

pub(crate) enum LabelKind {
    IfElse,
    IfEnd,
    MatchCase,
    MatchEnd,
}

pub(crate) struct PlanCall {
    pub(crate) target: String,
    pub(crate) symbol: String,
    pub(crate) kind: CallKind,
    pub(crate) string_literals: Vec<String>,
}

pub(crate) enum CallKind {
    Local,
    Import,
    Runtime,
    Indirect,
}

#[derive(Clone)]
pub(crate) struct StorageType {
    pub(crate) name: String,
    pub(crate) class: StorageClass,
    pub(crate) size: usize,
    pub(crate) align: usize,
}

#[derive(Clone)]
pub(crate) enum StorageClass {
    Void,
    Integer,
    Float,
    Fixed,
    Boolean,
    Reference,
}

pub(crate) fn lower_module(module: &NirModule) -> Result<NativePlan, String> {
    let function_symbols = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), nir::function_symbol(&function.name)))
        .collect::<HashMap<_, _>>();
    let import_symbols = module
        .imports
        .iter()
        .map(|import| (import.name.clone(), import.symbol.clone()))
        .collect::<HashMap<_, _>>();
    let entry_symbol = module
        .entry
        .as_ref()
        .map(|entry| nir::function_symbol(&entry.name));
    let external_symbols = module
        .imports
        .iter()
        .map(|import| import.symbol.clone())
        .collect::<Vec<_>>();
    let runtime_symbols = runtime_symbols(module);
    let platform_imports = platform_imports(module);
    let type_storage = type_storage(module)?;
    let mut functions = Vec::new();

    for function in &module.functions {
        functions.push(lower_function(
            function,
            &function_symbols,
            &import_symbols,
            &type_storage,
        )?);
    }

    Ok(NativePlan {
        target: module.target.clone(),
        project: module.project.clone(),
        entry_symbol,
        runtime_symbols,
        external_symbols,
        platform_imports,
        functions,
    })
}

impl NativePlan {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if !matches!(self.target.as_str(), "macos-aarch64" | "linux-aarch64") {
            return Err(format!(
                "native plan target '{}' does not match a supported aarch64 target",
                self.target
            ));
        }
        if self.project.is_empty() {
            return Err("native plan project name must not be empty".to_string());
        }
        if self.functions.is_empty() {
            return Err("native plan requires at least one function".to_string());
        }
        if let Some(entry_symbol) = &self.entry_symbol {
            if !self
                .functions
                .iter()
                .any(|function| &function.symbol == entry_symbol)
            {
                return Err(format!(
                    "native plan entry symbol '{entry_symbol}' does not resolve"
                ));
            }
        }
        for symbol in self
            .runtime_symbols
            .iter()
            .chain(self.external_symbols.iter())
        {
            if symbol.is_empty() {
                return Err("native plan contains an empty required symbol".to_string());
            }
        }
        for import in &self.platform_imports {
            if import.library.is_empty()
                || import.symbol.is_empty()
                || import.required_by.is_empty()
            {
                return Err("native plan contains an incomplete platform import".to_string());
            }
        }
        for function in &self.functions {
            function.validate()?;
        }
        Ok(())
    }

    pub(crate) fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-native-plan\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"runtimeSymbols\": [{}],\n",
                "  \"externalSymbols\": [{}],\n",
                "  \"platformImports\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(&self.project),
            self.entry_symbol
                .as_ref()
                .map(|symbol| json_string(symbol))
                .unwrap_or_else(|| "null".to_string()),
            json_string_list(&self.runtime_symbols),
            json_string_list(&self.external_symbols),
            join_json(&self.platform_imports, 2),
            join_json(&self.functions, 2)
        )
    }
}

impl PlannedFunction {
    fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() || self.symbol.is_empty() {
            return Err("native plan function name and symbol must not be empty".to_string());
        }
        self.returns.validate()?;
        for param in &self.params {
            if param.name.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty parameter name",
                    self.name
                ));
            }
            param.storage.validate()?;
        }
        for slot in &self.local_slots {
            if slot.name.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty stack slot name",
                    self.name
                ));
            }
            slot.storage.validate()?;
            if slot.offset >= 0 {
                return Err(format!(
                    "native plan stack slot '{}' in '{}' has non-stack offset {}",
                    slot.name, self.name, slot.offset
                ));
            }
            let _is_mutable = slot.mutable;
        }
        for label in &self.labels {
            if label.name.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty label name",
                    self.name
                ));
            }
            match label.kind {
                LabelKind::IfElse
                | LabelKind::IfEnd
                | LabelKind::MatchCase
                | LabelKind::MatchEnd => {}
            }
        }
        if self.operations.is_empty() {
            return Err(format!(
                "native plan function '{}' has no planned operations",
                self.name
            ));
        }
        for call in &self.calls {
            if call.target.is_empty() || call.symbol.is_empty() {
                return Err(format!(
                    "native plan function '{}' has an empty call target or symbol",
                    self.name
                ));
            }
            match call.kind {
                CallKind::Local | CallKind::Import | CallKind::Runtime | CallKind::Indirect => {}
            }
        }
        Ok(())
    }
}

impl StorageType {
    fn validate(&self) -> Result<(), String> {
        if self.name.is_empty() {
            return Err("native plan storage type name must not be empty".to_string());
        }
        match self.class {
            StorageClass::Void => {
                if self.size != 0 || self.align != 1 {
                    return Err(format!(
                        "native plan void storage '{}' must be size 0 align 1",
                        self.name
                    ));
                }
            }
            StorageClass::Boolean
            | StorageClass::Integer
            | StorageClass::Float
            | StorageClass::Fixed
            | StorageClass::Reference => {
                if self.size == 0 || self.align == 0 {
                    return Err(format!(
                        "native plan storage '{}' must have nonzero size and alignment",
                        self.name
                    ));
                }
            }
        }
        Ok(())
    }
}

fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    import_symbols: &HashMap<String, String>,
    type_storage: &HashMap<String, StorageType>,
) -> Result<PlannedFunction, String> {
    let params = function
        .params
        .iter()
        .map(|param| {
            let storage = storage_for_type(&param.type_, type_storage)?;
            Ok(PlannedParam {
                name: param.name.clone(),
                storage,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut builder = FunctionPlanBuilder {
        function_symbols,
        import_symbols,
        type_storage,
        local_slots: Vec::new(),
        labels: Vec::new(),
        operations: Vec::new(),
        calls: Vec::new(),
        next_label: 0,
    };
    builder.lower_ops(&function.body)?;
    if builder.operations.is_empty() {
        builder.operations.push("noOp".to_string());
    }

    Ok(PlannedFunction {
        name: function.name.clone(),
        symbol: nir::function_symbol(&function.name),
        returns: storage_for_type(&function.returns, type_storage)?,
        params,
        local_slots: builder.local_slots,
        labels: builder.labels,
        operations: builder.operations,
        calls: builder.calls,
    })
}

fn runtime_symbols(module: &NirModule) -> Vec<String> {
    let mut symbols = Vec::new();
    for function in &module.functions {
        collect_runtime_symbols_from_ops(&function.body, &mut symbols);
    }
    symbols
}

fn platform_imports(module: &NirModule) -> Vec<PlatformImport> {
    let mut imports = Vec::new();
    if module.target == "macos-aarch64" && module.entry.is_some() {
        push_platform_import(
            &mut imports,
            PlatformImport {
                library: "libSystem".to_string(),
                symbol: "_exit".to_string(),
                required_by: "_main".to_string(),
            },
        );
    }
    if module.target == "linux-aarch64" {
        return imports;
    }
    for function in &module.functions {
        collect_platform_imports_from_ops(&function.body, &mut imports);
    }
    imports
}

fn collect_platform_imports_from_ops(ops: &[NirOp], imports: &mut Vec<PlatformImport>) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } | NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_platform_imports_from_value(value, imports);
                }
            }
            NirOp::Assign { value, .. } | NirOp::Eval { value } => {
                collect_platform_imports_from_value(value, imports);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_platform_imports_from_value(condition, imports);
                collect_platform_imports_from_ops(then_body, imports);
                collect_platform_imports_from_ops(else_body, imports);
            }
            NirOp::Match { value, cases } => {
                collect_platform_imports_from_value(value, imports);
                for case in cases {
                    collect_platform_imports_from_ops(&case.body, imports);
                }
            }
            NirOp::Using { value, body, .. } => {
                collect_platform_imports_from_value(value, imports);
                collect_platform_imports_from_ops(body, imports);
            }
        }
    }
}

fn collect_platform_imports_from_value(value: &NirValue, imports: &mut Vec<PlatformImport>) {
    match value {
        NirValue::RuntimeCall { target, args, .. } => {
            for import in platform_imports_for_runtime_call(target) {
                push_platform_import(imports, import);
            }
            for arg in args {
                collect_platform_imports_from_value(arg, imports);
            }
        }
        NirValue::Call { args, .. } | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_platform_imports_from_value(arg, imports);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_platform_imports_from_value(value, imports);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_platform_imports_from_value(key, imports);
                collect_platform_imports_from_value(value, imports);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_platform_imports_from_value(target, imports)
        }
        NirValue::Binary { left, right, .. } => {
            collect_platform_imports_from_value(left, imports);
            collect_platform_imports_from_value(right, imports);
        }
        NirValue::Unary { operand, .. } => collect_platform_imports_from_value(operand, imports),
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => {}
    }
}

fn platform_imports_for_runtime_call(target: &str) -> Vec<PlatformImport> {
    let Some(spec) = runtime::spec_for_call(target) else {
        return Vec::new();
    };
    if spec.platform_imports.is_empty() {
        return Vec::new();
    }
    spec.platform_imports
        .iter()
        .map(|import| PlatformImport {
            library: import.library.to_string(),
            symbol: import.symbol.to_string(),
            required_by: spec.symbol.to_string(),
        })
        .collect()
}

fn collect_runtime_symbols_from_ops(ops: &[NirOp], symbols: &mut Vec<String>) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } | NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_runtime_symbols_from_value(value, symbols);
                }
            }
            NirOp::Assign { value, .. } | NirOp::Eval { value } => {
                collect_runtime_symbols_from_value(value, symbols);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_runtime_symbols_from_value(condition, symbols);
                collect_runtime_symbols_from_ops(then_body, symbols);
                collect_runtime_symbols_from_ops(else_body, symbols);
            }
            NirOp::Match { value, cases } => {
                collect_runtime_symbols_from_value(value, symbols);
                for case in cases {
                    collect_runtime_symbols_from_ops(&case.body, symbols);
                }
            }
            NirOp::Using { value, body, .. } => {
                collect_runtime_symbols_from_value(value, symbols);
                collect_runtime_symbols_from_ops(body, symbols);
            }
        }
    }
}

fn collect_runtime_symbols_from_value(value: &NirValue, symbols: &mut Vec<String>) {
    match value {
        NirValue::RuntimeCall {
            helper,
            target,
            args,
        } => {
            push_unique(symbols, runtime::symbol_for_call(*helper, target));
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols);
            }
        }
        NirValue::Call { args, .. } | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_runtime_symbols_from_value(value, symbols);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_runtime_symbols_from_value(key, symbols);
                collect_runtime_symbols_from_value(value, symbols);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_runtime_symbols_from_value(target, symbols)
        }
        NirValue::Binary { left, right, .. } => {
            collect_runtime_symbols_from_value(left, symbols);
            collect_runtime_symbols_from_value(right, symbols);
        }
        NirValue::Unary { operand, .. } => collect_runtime_symbols_from_value(operand, symbols),
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => {}
    }
}

struct FunctionPlanBuilder<'a> {
    function_symbols: &'a HashMap<String, String>,
    import_symbols: &'a HashMap<String, String>,
    type_storage: &'a HashMap<String, StorageType>,
    local_slots: Vec<StackSlot>,
    labels: Vec<PlanLabel>,
    operations: Vec<String>,
    calls: Vec<PlanCall>,
    next_label: usize,
}

impl FunctionPlanBuilder<'_> {
    fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
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
                NirOp::Assign { name, value } => {
                    self.lower_value(value)?;
                    self.operations
                        .push(format!("assign {name} = {}", describe_value(value)));
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
                NirOp::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    self.lower_value(condition)?;
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
                    if !else_body.is_empty() {
                        self.lower_ops(else_body)?;
                    }
                    self.operations.push(format!("label {end_label}"));
                }
                NirOp::Match { value, cases } => {
                    self.lower_value(value)?;
                    self.operations
                        .push(format!("match {}", describe_value(value)));
                    let end_label = self.add_label(LabelKind::MatchEnd);
                    for case in cases {
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
                }
                NirOp::Using {
                    name,
                    type_,
                    close,
                    value,
                    body,
                } => {
                    self.lower_value(value)?;
                    self.operations.push(format!(
                        "using {name} AS {type_} = {}",
                        describe_value(value)
                    ));
                    self.add_stack_slot(name, type_, false)?;
                    self.lower_ops(body)?;
                    self.operations.push(format!("close {close}"));
                    self.add_call(close);
                }
            }
        }
        Ok(())
    }

    fn lower_value(&mut self, value: &NirValue) -> Result<(), String> {
        match value {
            NirValue::Call { target, args } => {
                for arg in args {
                    self.lower_value(arg)?;
                }
                self.add_call(target);
            }
            NirValue::RuntimeCall {
                helper,
                target,
                args,
            } => {
                for arg in args {
                    self.lower_value(arg)?;
                }
                self.add_runtime_call(*helper, target, args);
            }
            NirValue::Constructor { args, .. } => {
                for arg in args {
                    self.lower_value(arg)?;
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
            NirValue::Local(_) => {}
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
        } else {
            (CallKind::Indirect, target.to_string())
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

fn type_storage(module: &NirModule) -> Result<HashMap<String, StorageType>, String> {
    let mut storage = HashMap::new();
    for type_ in &module.types {
        let type_storage = match type_.kind.as_str() {
            "enum" => StorageType {
                name: type_.name.clone(),
                class: StorageClass::Integer,
                size: 8,
                align: 8,
            },
            "record" | "resource" | "union" => StorageType {
                name: type_.name.clone(),
                class: StorageClass::Reference,
                size: 8,
                align: 8,
            },
            other => {
                return Err(format!(
                    "macos-aarch64 native plan has no storage class for type kind '{other}'"
                ));
            }
        };
        storage.insert(type_.name.clone(), type_storage);
    }
    Ok(storage)
}

fn storage_for_type(
    type_: &str,
    type_storage: &HashMap<String, StorageType>,
) -> Result<StorageType, String> {
    if let Some(storage) = type_storage.get(type_) {
        return Ok(storage.clone());
    }
    let (class, size, align) = if type_ == "Nothing" {
        (StorageClass::Void, 0, 1)
    } else if type_ == "Boolean" {
        (StorageClass::Boolean, 1, 1)
    } else if type_ == "Integer" {
        (StorageClass::Integer, 8, 8)
    } else if type_ == "Float" {
        (StorageClass::Float, 8, 8)
    } else if type_ == "Fixed" {
        (StorageClass::Fixed, 8, 8)
    } else if is_reference_type(type_) {
        (StorageClass::Reference, 8, 8)
    } else {
        return Err(format!(
            "macos-aarch64 native plan has no storage class for type '{type_}'"
        ));
    };
    Ok(StorageType {
        name: type_.to_string(),
        class,
        size,
        align,
    })
}

fn is_reference_type(type_: &str) -> bool {
    type_ == "String"
        || type_.starts_with("List OF ")
        || type_.starts_with("Map OF ")
        || type_.starts_with("Result OF ")
        || type_.starts_with("Thread OF ")
        || type_.starts_with("FUNC(")
        || type_.starts_with("ISOLATED FUNC(")
        || matches!(type_, "FileHandle" | "DirHandle")
}

fn push_call(calls: &mut Vec<PlanCall>, target: &str, symbol: String, kind: CallKind) {
    push_call_with_literals(calls, target, symbol, kind, Vec::new());
}

fn push_call_with_literals(
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

fn string_literals(values: &[NirValue]) -> Vec<String> {
    let mut literals = Vec::new();
    for value in values {
        collect_string_literals(value, &mut literals);
    }
    literals
}

fn describe_value(value: &NirValue) -> String {
    match value {
        NirValue::Const { type_, value } => format!("{type_}({value})"),
        NirValue::Local(name) => format!("local {name}"),
        NirValue::FunctionRef { name, .. } => format!("functionRef {name}"),
        NirValue::Call { target, args } => {
            let args = args
                .iter()
                .map(describe_value)
                .collect::<Vec<_>>()
                .join(", ");
            format!("call {target}({args})")
        }
        NirValue::RuntimeCall {
            helper,
            target,
            args,
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
        NirValue::Binary { op, left, right } => {
            format!(
                "({} {} {})",
                describe_value(left),
                op,
                describe_value(right)
            )
        }
        NirValue::Unary { op, operand } => format!("({op} {})", describe_value(operand)),
    }
}

fn describe_match_pattern(pattern: &super::nir::NirMatchPattern) -> String {
    match pattern {
        super::nir::NirMatchPattern::Else => "else".to_string(),
        super::nir::NirMatchPattern::Value(NirValue::Local(name)) => name.clone(),
        super::nir::NirMatchPattern::Value(value) => describe_value(value),
    }
}

fn collect_string_literals(value: &NirValue, literals: &mut Vec<String>) {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => {
            if !literals.contains(value) {
                literals.push(value.clone());
            }
        }
        NirValue::Call { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_literals(arg, literals);
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
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => {}
    }
}

fn push_platform_import(imports: &mut Vec<PlatformImport>, import: PlatformImport) {
    if imports.iter().any(|existing| {
        existing.library == import.library
            && existing.symbol == import.symbol
            && existing.required_by == import.required_by
    }) {
        return;
    }
    imports.push(import);
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

trait ToPlanJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToPlanJson for PlatformImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"library\": {}, \"symbol\": {}, \"requiredBy\": {} }}",
            pad,
            json_string(&self.library),
            json_string(&self.symbol),
            json_string(&self.required_by)
        )
    }
}

impl ToPlanJson for PlannedFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"localSlots\": [{}\n{}  ],\n",
                "{}  \"labels\": [{}\n{}  ],\n",
                "{}  \"operations\": [{}],\n",
                "{}  \"calls\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            self.returns.to_json(0),
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            join_json(&self.local_slots, indent + 2),
            pad,
            pad,
            join_json(&self.labels, indent + 2),
            pad,
            pad,
            json_string_list(&self.operations),
            pad,
            join_json(&self.calls, indent + 2),
            pad,
            pad
        )
    }
}

impl ToPlanJson for PlannedParam {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"storage\": {} }}",
            pad,
            json_string(&self.name),
            self.storage.to_json(0)
        )
    }
}

impl ToPlanJson for StackSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"storage\": {}, \"offset\": {}, \"mutable\": {} }}",
            pad,
            json_string(&self.name),
            self.storage.to_json(0),
            self.offset,
            self.mutable
        )
    }
}

impl ToPlanJson for PlanLabel {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"kind\": {} }}",
            pad,
            json_string(&self.name),
            json_string(self.kind.name())
        )
    }
}

impl ToPlanJson for PlanCall {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"target\": {}, \"symbol\": {}, \"kind\": {}, \"stringLiterals\": [{}] }}",
            pad,
            json_string(&self.target),
            json_string(&self.symbol),
            json_string(self.kind.name()),
            json_string_list(&self.string_literals)
        )
    }
}

impl ToPlanJson for StorageType {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"name\": {}, \"class\": {}, \"size\": {}, \"align\": {} }}",
            json_string(&self.name),
            json_string(self.class.name()),
            self.size,
            self.align
        )
    }
}

impl LabelKind {
    fn name(&self) -> &'static str {
        match self {
            LabelKind::IfElse => "ifElse",
            LabelKind::IfEnd => "ifEnd",
            LabelKind::MatchCase => "matchCase",
            LabelKind::MatchEnd => "matchEnd",
        }
    }
}

impl CallKind {
    fn name(&self) -> &'static str {
        match self {
            CallKind::Local => "local",
            CallKind::Import => "import",
            CallKind::Runtime => "runtime",
            CallKind::Indirect => "indirect",
        }
    }
}

impl StorageClass {
    fn name(&self) -> &'static str {
        match self {
            StorageClass::Void => "void",
            StorageClass::Integer => "integer",
            StorageClass::Float => "float",
            StorageClass::Fixed => "fixed",
            StorageClass::Boolean => "boolean",
            StorageClass::Reference => "reference",
        }
    }
}

fn join_json<T: ToPlanJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::macos_aarch64::nir::{
        NirEntryPoint, NirFunction, NirModule, NirOp, NirValue,
    };
    use crate::target::macos_aarch64::runtime::RuntimeHelper;

    #[test]
    fn plans_runtime_symbol_and_entry_function() {
        let module = NirModule {
            target: "macos-aarch64".to_string(),
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Nothing".to_string(),
                accepts_args: false,
            }),
            types: Vec::new(),
            imports: Vec::new(),
            runtime_helpers: vec![RuntimeHelper::Io],
            functions: vec![NirFunction {
                name: "main".to_string(),
                visibility: "private".to_string(),
                kind: "sub".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: "Nothing".to_string(),
                body: vec![NirOp::Eval {
                    value: NirValue::RuntimeCall {
                        helper: RuntimeHelper::Io,
                        target: "io.print".to_string(),
                        args: vec![NirValue::Const {
                            type_: "String".to_string(),
                            value: "Hello World".to_string(),
                        }],
                    },
                }],
            }],
        };

        let plan = lower_module(&module).expect("native plan");
        plan.validate().expect("valid native plan");
        assert_eq!(plan.entry_symbol.as_deref(), Some("_mfb_fn_main"));
        assert_eq!(plan.runtime_symbols, vec!["_mfb_rt_io_io_print"]);
        assert_eq!(plan.platform_imports[0].library, "libSystem");
        assert_eq!(plan.platform_imports[0].symbol, "_exit");
        assert_eq!(plan.platform_imports[0].required_by, "_main");
        assert_eq!(plan.platform_imports[1].library, "libSystem");
        assert_eq!(plan.platform_imports[1].symbol, "_write");
        assert_eq!(plan.platform_imports[1].required_by, "_mfb_rt_io_io_print");
        assert_eq!(plan.functions[0].calls[0].symbol, "_mfb_rt_io_io_print");
    }
}
