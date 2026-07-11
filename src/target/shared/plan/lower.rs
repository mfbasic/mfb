use super::*;

use std::collections::HashMap;

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
    let mut function_symbols = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), nir::function_symbol(&function.name)))
        .collect::<HashMap<_, _>>();
    // A native `LINK` call routes to its internal marshaling thunk
    // (plan-linker.md §12), so treat the routing entries as local functions.
    for import in &module.imports {
        function_symbols.insert(import.name.clone(), import.symbol.clone());
    }
    let entry_symbol = module
        .entry
        .as_ref()
        .map(|entry| nir::function_symbol(&entry.name));
    let external_symbols: Vec<String> = Vec::new();
    // The internal symbols the backend defines for native `LINK` bindings: the
    // load-time initializer plus one marshaling thunk per function. The object
    // plan must treat these as defined symbols (plan-linker.md §12).
    let link_symbols = if module.link_functions.is_empty() {
        Vec::new()
    } else {
        let mut symbols = vec![nir::LINK_INIT_SYMBOL.to_string()];
        for function in &module.link_functions {
            symbols.push(nir::link_thunk_symbol(&function.alias, &function.name));
        }
        symbols
    };
    let runtime_symbols = runtime_symbols(module);
    let platform_imports = platform_imports(module, platform);
    let type_storage = type_storage(module)?;
    let mut functions = Vec::new();

    for function in &module.functions {
        functions.push(lower_function(
            function,
            &function_symbols,
            &type_storage,
        )?);
    }

    Ok(NativePlan {
        target: module.target.clone(),
        build_mode: module.build_mode,
        project: module.project.clone(),
        entry_symbol,
        runtime_symbols,
        external_symbols,
        platform_imports,
        functions,
        link_symbols,
    })
}

pub(super) fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
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

pub(super) fn type_storage(module: &NirModule) -> Result<HashMap<String, StorageType>, String> {
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

pub(super) fn storage_for_type(
    type_: &str,
    type_storage: &HashMap<String, StorageType>,
) -> Result<StorageType, String> {
    // A `RES`-marked collection element (`RES File`) stores exactly like the
    // bare resource it borrows: a pointer to the record (§15.6).
    let type_ = type_.strip_prefix("RES ").unwrap_or(type_);
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
    } else if crate::builtins::is_resource_type(type_) {
        // A resource (optionally `File STATE T`) is a pointer to its record.
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

pub(super) fn is_reference_type(type_: &str) -> bool {
    type_ == "String"
        || type_ == "TermColor"
        || type_ == "TermSize"
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

pub(super) fn is_user_type_name(type_: &str) -> bool {
    !type_.is_empty()
        && type_ != "Unknown"
        && type_
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '.')
}
