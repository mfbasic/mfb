use super::*;

pub(super) fn unique_global_names(module: &NirModule) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    let mut symbols = HashSet::new();
    for global in &module.globals {
        if global.name.is_empty() || global.symbol.is_empty() || global.type_.is_empty() {
            return Err("NIR global name, symbol, and type must not be empty".to_string());
        }
        if !matches!(global.visibility.as_str(), "private" | "public" | "export") {
            return Err(format!(
                "NIR global '{}' has invalid visibility '{}'",
                global.name, global.visibility
            ));
        }
        if !names.insert(global.name.clone()) {
            return Err(format!(
                "NIR global '{}' is declared more than once",
                global.name
            ));
        }
        if !symbols.insert(global.symbol.clone()) {
            return Err(format!(
                "NIR global symbol '{}' is declared more than once",
                global.symbol
            ));
        }
    }
    Ok(names)
}

pub(super) fn type_value_names(module: &NirModule) -> Result<TypeValueNames, String> {
    let mut namespaces = HashSet::new();
    let mut constructors = HashSet::new();
    for type_ in &module.types {
        if type_.name.is_empty() {
            return Err("NIR type has empty name".to_string());
        }
        namespaces.insert(type_.name.clone());
        match type_.kind.as_str() {
            "enum" => {
                for member in &type_.members {
                    if member.name.is_empty() {
                        return Err(format!("NIR enum '{}' has empty member name", type_.name));
                    }
                    // bug-176 B: track enum member names explicitly so a bare
                    // `Local(member)` reference resolves against the type table
                    // rather than the first-letter-case heuristic.
                    constructors.insert(member.name.clone());
                }
            }
            "union" => {
                for variant in &type_.variants {
                    if variant.name.is_empty() {
                        return Err(format!("NIR union '{}' has empty variant name", type_.name));
                    }
                    constructors.insert(variant.name.clone());
                }
            }
            "type" | "record" | "resource" => {}
            other => {
                return Err(format!(
                    "NIR type '{}' has invalid kind '{other}'",
                    type_.name
                ));
            }
        }
    }
    Ok(TypeValueNames {
        namespaces,
        constructors,
    })
}

pub(super) fn unique_function_names(functions: &[NirFunction]) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    for function in functions {
        if function.name.is_empty() {
            return Err("NIR function name must not be empty".to_string());
        }
        if !matches!(function.kind.as_str(), "func" | "sub") {
            return Err(format!(
                "NIR function '{}' has invalid kind '{}'",
                function.name, function.kind
            ));
        }
        if !matches!(
            function.visibility.as_str(),
            "private" | "public" | "export"
        ) {
            return Err(format!(
                "NIR function '{}' has invalid visibility '{}'",
                function.name, function.visibility
            ));
        }
        if !names.insert(function.name.clone()) {
            return Err(format!(
                "NIR function '{}' is declared more than once",
                function.name
            ));
        }
    }
    Ok(names)
}

pub(super) fn unique_import_names(module: &NirModule) -> Result<HashSet<String>, String> {
    let mut names = HashSet::new();
    for import in &module.imports {
        if import.name.is_empty() || import.package.is_empty() || import.symbol.is_empty() {
            return Err("NIR import package, name, and symbol must not be empty".to_string());
        }
        if !matches!(import.kind.as_str(), "func" | "sub") {
            return Err(format!(
                "NIR import '{}' has invalid kind '{}'",
                import.name, import.kind
            ));
        }
        if !names.insert(import.name.clone()) {
            return Err(format!(
                "NIR import '{}' is declared more than once",
                import.name
            ));
        }
    }
    Ok(names)
}
