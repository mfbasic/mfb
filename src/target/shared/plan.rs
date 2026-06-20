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
    WhileLoop,
    WhileEnd,
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
    Byte,
    Integer,
    Float,
    Fixed,
    Boolean,
    Reference,
}

pub(crate) trait NativePlanPlatform {
    fn target(&self) -> &'static str;
    fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport>;
    fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport>;
    fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport>;
    fn runtime_imports(&self, spec: &runtime::RuntimeHelperSpec) -> Vec<PlatformImport>;
    fn native_call_imports(&self, target: &str, required_by: &str) -> Vec<PlatformImport>;
}

pub(crate) fn lower_module_for_platform(
    module: &NirModule,
    platform: &dyn NativePlanPlatform,
) -> Result<NativePlan, String> {
    if module.target != platform.target() {
        return Err(format!(
            "native plan platform '{}' cannot lower module target '{}'",
            platform.target(),
            module.target
        ));
    }
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
    let platform_imports = platform_imports(module, platform);
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
        if self.target.is_empty() {
            return Err("native plan target must not be empty".to_string());
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
                | LabelKind::MatchEnd
                | LabelKind::WhileLoop
                | LabelKind::WhileEnd => {}
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
            | StorageClass::Byte
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
        constants: HashMap::new(),
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
    if module_has_thread_owner(module) {
        push_unique(
            &mut symbols,
            runtime::symbol_for_call(runtime::RuntimeHelper::Thread, "thread.drop"),
        );
    }
    symbols
}

fn platform_imports(module: &NirModule, platform: &dyn NativePlanPlatform) -> Vec<PlatformImport> {
    let mut imports = Vec::new();
    for import in platform.entry_imports(module) {
        push_platform_import(&mut imports, import);
    }
    for import in platform.entry_error_imports(module) {
        push_platform_import(&mut imports, import);
    }
    for function in &module.functions {
        collect_platform_imports_from_ops(
            platform,
            &nir::function_symbol(&function.name),
            &function.body,
            &mut imports,
        );
    }
    if module_has_thread_owner(module) {
        for import in platform_imports_for_runtime_call(platform, "thread.drop") {
            push_platform_import(&mut imports, import);
        }
    }
    imports
}

fn is_thread_type(type_: &str) -> bool {
    type_.starts_with("Thread OF ")
}

fn module_has_thread_owner(module: &NirModule) -> bool {
    module.functions.iter().any(|function| {
        function
            .params
            .iter()
            .any(|param| is_thread_type(&param.type_))
            || ops_have_thread_owner(&function.body)
    })
}

fn ops_have_thread_owner(ops: &[NirOp]) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { type_, .. } | NirOp::StoreGlobal { type_, .. } => is_thread_type(type_),
        NirOp::ForEach { type_, body, .. } => is_thread_type(type_) || ops_have_thread_owner(body),
        NirOp::If {
            then_body,
            else_body,
            ..
        } => ops_have_thread_owner(then_body) || ops_have_thread_owner(else_body),
        NirOp::Match { cases, .. } => cases.iter().any(|case| ops_have_thread_owner(&case.body)),
        NirOp::While { body, .. } | NirOp::Trap { body, .. } => ops_have_thread_owner(body),
        NirOp::For { body, .. } | NirOp::DoUntil { body, .. } => ops_have_thread_owner(body),
        NirOp::Assign { .. }
        | NirOp::Return { .. }
        | NirOp::ExitLoop { .. }
        | NirOp::ContinueLoop { .. }
        | NirOp::ExitProgram { .. }
        | NirOp::Fail { .. }
        | NirOp::Eval { .. } => false,
    })
}

fn collect_platform_imports_from_ops(
    platform: &dyn NativePlanPlatform,
    required_by: &str,
    ops: &[NirOp],
    imports: &mut Vec<PlatformImport>,
) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. }
            | NirOp::StoreGlobal { value, .. }
            | NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_platform_imports_from_value(platform, required_by, value, imports);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_platform_imports_from_value(platform, required_by, code, imports);
                for import in platform.program_exit_imports(required_by) {
                    push_platform_import(imports, import);
                }
            }
            NirOp::Fail { error } => {
                collect_platform_imports_from_value(platform, required_by, error, imports);
            }
            NirOp::Assign { value, .. } | NirOp::Eval { value } => {
                collect_platform_imports_from_value(platform, required_by, value, imports);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_platform_imports_from_value(platform, required_by, condition, imports);
                collect_platform_imports_from_ops(platform, required_by, then_body, imports);
                collect_platform_imports_from_ops(platform, required_by, else_body, imports);
            }
            NirOp::Match { value, cases } => {
                collect_platform_imports_from_value(platform, required_by, value, imports);
                for case in cases {
                    collect_platform_imports_from_ops(platform, required_by, &case.body, imports);
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_platform_imports_from_value(platform, required_by, condition, imports);
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_platform_imports_from_value(platform, required_by, start, imports);
                collect_platform_imports_from_value(platform, required_by, end, imports);
                collect_platform_imports_from_value(platform, required_by, step, imports);
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
            NirOp::DoUntil { body, condition } => {
                collect_platform_imports_from_ops(platform, required_by, body, imports);
                collect_platform_imports_from_value(platform, required_by, condition, imports);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_platform_imports_from_value(platform, required_by, iterable, imports);
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
            NirOp::Trap { body, .. } => {
                collect_platform_imports_from_ops(platform, required_by, body, imports);
            }
        }
    }
}

fn collect_platform_imports_from_value(
    platform: &dyn NativePlanPlatform,
    required_by: &str,
    value: &NirValue,
    imports: &mut Vec<PlatformImport>,
) {
    match value {
        NirValue::RuntimeCall { target, args, .. } => {
            if target != "typeName" && !runtime::is_native_direct_call(target) {
                for import in platform_imports_for_runtime_call(platform, target) {
                    push_platform_import(imports, import);
                }
            }
            for arg in args {
                collect_platform_imports_from_value(platform, required_by, arg, imports);
            }
        }
        NirValue::Call { target, args, .. } | NirValue::CallResult { target, args, .. } => {
            // A helper-backed `CallResult` (inline `TRAP` on a built-in) pulls in
            // the same platform imports as the equivalent `RuntimeCall`.
            if target != "typeName"
                && !runtime::is_native_direct_call(target)
                && runtime::helper_for_call(target).is_some()
            {
                for import in platform_imports_for_runtime_call(platform, target) {
                    push_platform_import(imports, import);
                }
            } else {
                for import in platform.native_call_imports(target, required_by) {
                    push_platform_import(imports, import);
                }
            }
            for arg in args {
                collect_platform_imports_from_value(platform, required_by, arg, imports);
            }
        }
        NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_platform_imports_from_value(platform, required_by, arg, imports);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_platform_imports_from_value(platform, required_by, value, imports);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_platform_imports_from_value(platform, required_by, target, imports);
            for update in updates {
                collect_platform_imports_from_value(platform, required_by, &update.value, imports);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_platform_imports_from_value(platform, required_by, value, imports);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_platform_imports_from_value(platform, required_by, key, imports);
                collect_platform_imports_from_value(platform, required_by, value, imports);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_platform_imports_from_value(platform, required_by, target, imports)
        }
        NirValue::Binary { op, left, right, .. } => {
            if op == "MOD" {
                for import in platform.native_call_imports("math.fmod", required_by) {
                    push_platform_import(imports, import);
                }
            }
            collect_platform_imports_from_value(platform, required_by, left, imports);
            collect_platform_imports_from_value(platform, required_by, right, imports);
        }
        NirValue::Unary { operand, .. } => {
            collect_platform_imports_from_value(platform, required_by, operand, imports)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_platform_imports_from_value(platform, required_by, value, imports);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}

fn platform_imports_for_runtime_call(
    platform: &dyn NativePlanPlatform,
    target: &str,
) -> Vec<PlatformImport> {
    let Some(spec) = runtime::spec_for_call(target) else {
        return Vec::new();
    };
    platform.runtime_imports(spec)
}

fn collect_runtime_symbols_from_ops(ops: &[NirOp], symbols: &mut Vec<String>) {
    let mut constants = HashMap::new();
    collect_runtime_symbols_from_ops_with_constants(ops, symbols, &mut constants);
}

fn collect_runtime_symbols_from_ops_with_constants(
    ops: &[NirOp],
    symbols: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
) {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                if let Some(close) = crate::builtins::resource_close_function(type_) {
                    if let Some(helper) = runtime::helper_for_call(close) {
                        push_unique(symbols, runtime::symbol_for_call(helper, close));
                    }
                }
                if let Some(value) = value {
                    collect_runtime_symbols_from_value(value, symbols, constants);
                    if let Some(constant) = native_constant_value(value, constants) {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::StoreGlobal { value, .. } => {
                if let Some(value) = value {
                    collect_runtime_symbols_from_value(value, symbols, constants);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_runtime_symbols_from_value(value, symbols, constants);
                }
            }
            NirOp::ExitLoop { .. } | NirOp::ContinueLoop { .. } => {}
            NirOp::ExitProgram { code } => {
                collect_runtime_symbols_from_value(code, symbols, constants);
            }
            NirOp::Fail { error } => {
                collect_runtime_symbols_from_value(error, symbols, constants);
            }
            NirOp::Assign { name, value } => {
                collect_runtime_symbols_from_value(value, symbols, constants);
                if let Some(constant) = native_constant_value(value, constants) {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } => {
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_runtime_symbols_from_value(condition, symbols, constants);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(
                    then_body,
                    symbols,
                    &mut then_constants,
                );
                collect_runtime_symbols_from_ops_with_constants(
                    else_body,
                    symbols,
                    &mut else_constants,
                );
            }
            NirOp::Match { value, cases } => {
                collect_runtime_symbols_from_value(value, symbols, constants);
                for case in cases {
                    let mut case_constants = constants.clone();
                    collect_runtime_symbols_from_ops_with_constants(
                        &case.body,
                        symbols,
                        &mut case_constants,
                    );
                }
            }
            NirOp::While {
                condition, body, ..
            } => {
                collect_runtime_symbols_from_value(condition, symbols, constants);
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
            }
            NirOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                collect_runtime_symbols_from_value(start, symbols, constants);
                collect_runtime_symbols_from_value(end, symbols, constants);
                collect_runtime_symbols_from_value(step, symbols, constants);
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
            }
            NirOp::DoUntil { body, condition } => {
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
                collect_runtime_symbols_from_value(condition, symbols, constants);
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_runtime_symbols_from_value(iterable, symbols, constants);
                let mut body_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut body_constants);
            }
            NirOp::Trap { body, .. } => {
                let mut trap_constants = constants.clone();
                collect_runtime_symbols_from_ops_with_constants(body, symbols, &mut trap_constants);
            }
        }
    }
}

fn collect_runtime_symbols_from_value(
    value: &NirValue,
    symbols: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
) {
    match value {
        NirValue::RuntimeCall {
            helper,
            target,
            args,
                ..
        } => {
            if target != "typeName"
                && !runtime::is_native_direct_call(target)
                && native_static_string_value(value, constants).is_none()
                && native_static_graphemes_value(target, args, constants).is_none()
            {
                push_unique(symbols, runtime::symbol_for_call(*helper, target));
            }
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols, constants);
            }
        }
        NirValue::CallResult { target, args, .. } => {
            // A helper-backed `CallResult` (inline `TRAP` on a built-in) invokes
            // the runtime helper directly, so its symbol must be defined just
            // like the equivalent `RuntimeCall`.
            if !runtime::is_native_direct_call(target) {
                if let Some(helper) = runtime::helper_for_call(target) {
                    push_unique(symbols, runtime::symbol_for_call(helper, target));
                }
            }
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols, constants);
            }
        }
        NirValue::Call { args, .. } | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_runtime_symbols_from_value(arg, symbols, constants);
            }
        }
        NirValue::UnionWrap { value, .. }
        | NirValue::UnionExtract { value, .. }
        | NirValue::ResultIsOk { value }
        | NirValue::ResultValue { value }
        | NirValue::ResultError { value } => {
            collect_runtime_symbols_from_value(value, symbols, constants);
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_runtime_symbols_from_value(target, symbols, constants);
            for update in updates {
                collect_runtime_symbols_from_value(&update.value, symbols, constants);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_runtime_symbols_from_value(key, symbols, constants);
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
        }
        NirValue::MemberAccess { target, member } => {
            if member == "result" {
                push_unique(
                    symbols,
                    runtime::symbol_for_call(runtime::RuntimeHelper::Thread, "thread.waitFor"),
                );
            }
            collect_runtime_symbols_from_value(target, symbols, constants)
        }
        NirValue::Binary { left, right, .. } => {
            collect_runtime_symbols_from_value(left, symbols, constants);
            collect_runtime_symbols_from_value(right, symbols, constants);
        }
        NirValue::Unary { operand, .. } => {
            collect_runtime_symbols_from_value(operand, symbols, constants)
        }
        NirValue::Closure { captures, .. } => {
            for value in captures {
                collect_runtime_symbols_from_value(value, symbols, constants);
            }
        }
        NirValue::Capture { .. }
        | NirValue::Const { .. }
        | NirValue::Local(_)
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
    }
}

fn native_constant_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Global { .. } => None,
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Binary { op, .. } if op == "&" => native_static_string_value(value, constants)
            .map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            }),
        _ => None,
    }
}

fn native_static_string_value(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| native_static_string_value(constant, constants)),
        NirValue::Global { .. } => None,
        NirValue::Call { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            native_primitive_text(&args[0], constants)
        }
        NirValue::Call { target, args, .. } | NirValue::RuntimeCall { target, args, .. } => {
            native_strings_package_static_string_value(target, args, constants)
        }
        NirValue::Binary { op, left, right, .. } if op == "&" => {
            let left = native_static_string_value(left, constants)?;
            let right = native_static_string_value(right, constants)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

fn native_strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| native_static_string_value(arg, constants))?;
    match target {
        "strings.upper" if args.len() == 1 => Some(crate::unicode_backend::upper(&value)),
        "strings.lower" if args.len() == 1 => Some(crate::unicode_backend::lower(&value)),
        "strings.caseFold" if args.len() == 1 => Some(crate::unicode_backend::case_fold(&value)),
        "strings.normalizeNfc" if args.len() == 1 => {
            Some(crate::unicode_backend::normalize_nfc(&value))
        }
        _ => None,
    }
}

fn native_static_graphemes_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
) -> Option<Vec<String>> {
    if target != "strings.graphemes" || args.len() != 1 {
        return None;
    }
    let value = native_static_string_value(&args[0], constants)?;
    Some(crate::unicode_backend::graphemes(&value))
}

fn native_primitive_text(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_.as_str() {
            "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
            "Boolean" => match value.as_str() {
                "true" => Some("TRUE".to_string()),
                "false" => Some("FALSE".to_string()),
                _ => None,
            },
            _ => None,
        },
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| native_primitive_text(constant, constants)),
        NirValue::Global { .. } => None,
        _ => None,
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
    constants: HashMap<String, NirValue>,
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
            NirValue::Local(_) | NirValue::Global { .. } => {}
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
            "type" | "record" | "resource" | "union" => StorageType {
                name: type_.name.clone(),
                class: StorageClass::Reference,
                size: 8,
                align: 8,
            },
            other => {
                return Err(format!(
                    "native plan has no storage class for type kind '{other}'"
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
    } else if type_ == "Byte" {
        (StorageClass::Byte, 1, 1)
    } else if type_ == "Integer" {
        (StorageClass::Integer, 8, 8)
    } else if type_ == "Float" {
        (StorageClass::Float, 8, 8)
    } else if type_ == "Fixed" {
        (StorageClass::Fixed, 8, 8)
    } else if is_reference_type(type_) {
        (StorageClass::Reference, 8, 8)
    } else if is_user_type_name(type_) {
        (StorageClass::Reference, 8, 8)
    } else {
        return Err(format!(
            "native plan has no storage class for type '{type_}'"
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
        || type_ == "TerminalSize"
        || type_ == "Error"
        || type_.starts_with("List OF ")
        || type_.starts_with("Map OF ")
        || type_.starts_with("MapEntry OF ")
        || type_.starts_with("Result OF ")
        || type_.starts_with("Thread OF ")
        || type_.starts_with("ThreadWorker OF ")
        || type_.starts_with("FUNC(")
        || type_.starts_with("ISOLATED FUNC(")
        || matches!(type_, "File" | "FileHandle" | "DirHandle")
}

fn is_user_type_name(type_: &str) -> bool {
    !type_.is_empty()
        && type_ != "Unknown"
        && type_
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
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
        NirValue::Binary { op, left, right, .. } => {
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

fn plan_loop_kind_name(kind: crate::ast::LoopKind) -> &'static str {
    match kind {
        crate::ast::LoopKind::For => "FOR",
        crate::ast::LoopKind::Do => "DO",
        crate::ast::LoopKind::While => "WHILE",
    }
}

fn describe_match_pattern(pattern: &super::nir::NirMatchPattern) -> String {
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

fn collect_string_literals(value: &NirValue, literals: &mut Vec<String>) {
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
        | NirValue::Global { .. }
        | NirValue::FunctionRef { .. } => {}
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
            LabelKind::WhileLoop => "whileLoop",
            LabelKind::WhileEnd => "whileEnd",
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
            StorageClass::Byte => "byte",
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
    use crate::target::shared::nir::{NirEntryPoint, NirFunction, NirModule, NirOp, NirValue};
    use crate::target::shared::runtime::{RuntimeHelper, RuntimeHelperSpec};

    struct TestPlatform;

    impl NativePlanPlatform for TestPlatform {
        fn target(&self) -> &'static str {
            "test-target"
        }

        fn entry_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
            if module.entry.is_none() {
                return Vec::new();
            }
            vec![PlatformImport {
                library: "testRuntime".to_string(),
                symbol: "test_program_done".to_string(),
                required_by: "_main".to_string(),
            }]
        }

        fn entry_error_imports(&self, module: &NirModule) -> Vec<PlatformImport> {
            if module.entry.is_none() {
                return Vec::new();
            }
            vec![PlatformImport {
                library: "testRuntime".to_string(),
                symbol: "test_error_output".to_string(),
                required_by: "_main".to_string(),
            }]
        }

        fn program_exit_imports(&self, required_by: &str) -> Vec<PlatformImport> {
            vec![PlatformImport {
                library: "testRuntime".to_string(),
                symbol: "test_program_exit".to_string(),
                required_by: required_by.to_string(),
            }]
        }

        fn runtime_imports(&self, spec: &RuntimeHelperSpec) -> Vec<PlatformImport> {
            match spec.call {
                "io.print" | "io.write" | "io.printError" | "io.writeError" => {
                    vec![PlatformImport {
                        library: "testRuntime".to_string(),
                        symbol: "test_output".to_string(),
                        required_by: spec.symbol.to_string(),
                    }]
                }
                "io.input" | "io.readLine" | "io.readChar" | "io.readByte" => {
                    vec![PlatformImport {
                        library: "testRuntime".to_string(),
                        symbol: "test_input".to_string(),
                        required_by: spec.symbol.to_string(),
                    }]
                }
                "io.pollInput" => vec![PlatformImport {
                    library: "testRuntime".to_string(),
                    symbol: "test_poll".to_string(),
                    required_by: spec.symbol.to_string(),
                }],
                _ => Vec::new(),
            }
        }

        fn native_call_imports(&self, _target: &str, _required_by: &str) -> Vec<PlatformImport> {
            Vec::new()
        }
    }

    #[test]
    fn plans_runtime_symbol_and_entry_function() {
        let module = NirModule {
            target: "test-target".to_string(),
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Nothing".to_string(),
                accepts_args: false,
            }),
            types: Vec::new(),
            globals: Vec::new(),
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
                        loc: nir::NirSourceLoc::default(),
                    },
                }],
                file: "src/main.mfb".to_string(),
            }],
        };

        let plan = lower_module_for_platform(&module, &TestPlatform).expect("native plan");
        plan.validate().expect("valid native plan");
        assert_eq!(plan.entry_symbol.as_deref(), Some("_mfb_fn_main"));
        assert_eq!(plan.runtime_symbols, vec!["_mfb_rt_io_io_print"]);
        assert_eq!(plan.platform_imports[0].library, "testRuntime");
        assert_eq!(plan.platform_imports[0].symbol, "test_program_done");
        assert_eq!(plan.platform_imports[0].required_by, "_main");
        assert_eq!(plan.platform_imports[1].library, "testRuntime");
        assert_eq!(plan.platform_imports[1].symbol, "test_error_output");
        assert_eq!(plan.platform_imports[1].required_by, "_main");
        assert_eq!(plan.platform_imports[2].library, "testRuntime");
        assert_eq!(plan.platform_imports[2].symbol, "test_output");
        assert_eq!(plan.platform_imports[2].required_by, "_mfb_rt_io_io_print");
        assert_eq!(plan.functions[0].calls[0].symbol, "_mfb_rt_io_io_print");
    }
}
