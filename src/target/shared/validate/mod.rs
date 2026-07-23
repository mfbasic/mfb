use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::builtins;
use crate::ir::IrProject;
use crate::target::{BackendCapabilities, BuildTarget};

use super::nir::{NirFunction, NirMatchPattern, NirModule, NirOp, NirParam, NirValue};
use super::runtime::{self, RuntimeHelper};

struct TypeValueNames {
    namespaces: HashSet<String>,
    constructors: HashSet<String>,
}

pub fn validate_target(target: &BuildTarget) -> Result<(), String> {
    if target.os.is_empty() || target.arch.is_empty() {
        return Err("native target must include an OS and architecture".to_string());
    }
    Ok(())
}

pub fn validate_project(_ir: &IrProject, _packages: &[PathBuf]) -> Result<(), String> {
    Ok(())
}

pub fn validate_nir(module: &NirModule) -> Result<(), String> {
    if module.target.is_empty() {
        return Err("NIR target must not be empty".to_string());
    }
    if module.project.is_empty() {
        return Err("NIR project name must not be empty".to_string());
    }

    let function_names = unique_function_names(&module.functions)?;
    let global_names = unique_global_names(module)?;
    let import_names = unique_import_names(module)?;
    let type_value_names = type_value_names(module)?;
    validate_entry(module, &function_names)?;
    validate_resource_rules(module)?;

    for helper in &module.runtime_helpers {
        if module
            .runtime_helpers
            .iter()
            .filter(|candidate| *candidate == helper)
            .count()
            > 1
        {
            return Err(format!(
                "NIR runtime helper '{}' is declared more than once",
                helper.name()
            ));
        }
    }

    let mut used_helpers = Vec::new();
    for function in &module.functions {
        validate_function(
            function,
            &function_names,
            &global_names,
            &import_names,
            &type_value_names,
            &mut used_helpers,
        )?;
    }

    // A resource-union bind drops by dispatching to each variant's close op
    // (codegen-emitted, not an NIR call), so count those closes as used helpers
    // to match `required_helpers`.
    let mut bind_types = HashSet::new();
    for function in &module.functions {
        collect_bind_types(&function.body, &mut bind_types);
    }
    for type_ in &module.types {
        if type_.kind != "union" || !bind_types.contains(&type_.name) {
            continue;
        }
        let closes: Option<Vec<&'static str>> = type_
            .variants
            .iter()
            .map(|variant| crate::builtins::resource_close_function(&variant.name))
            .collect();
        if let Some(closes) = closes {
            for close in closes {
                if let Some(helper) = runtime::helper_for_call(close) {
                    if !used_helpers.contains(&helper) {
                        used_helpers.push(helper);
                    }
                }
            }
        }
    }

    for helper in &used_helpers {
        if !module.runtime_helpers.contains(helper) {
            return Err(format!(
                "NIR runtime call requires undeclared helper '{}'",
                helper.name()
            ));
        }
    }
    for helper in &module.runtime_helpers {
        if !used_helpers.contains(helper) {
            return Err(format!(
                "NIR declares unused runtime helper '{}'",
                helper.name()
            ));
        }
    }

    Ok(())
}

/// Whether a NIR type string transitively owns a resource (directly, or as a
/// collection element/value). `STATE`-suffixed resource strings are recognized
/// via `is_resource_type`.
fn type_owns_resource(type_: &str) -> bool {
    if crate::builtins::is_resource_type(type_) {
        return true;
    }
    if let Some(element) = type_.strip_prefix("List OF ") {
        return type_owns_resource(element);
    }
    if let Some(rest) = type_.strip_prefix("Map OF ") {
        if let Some((key, value)) = rest.split_once(" TO ") {
            return type_owns_resource(key) || type_owns_resource(value);
        }
    }
    if let Some(success) = type_.strip_prefix("Result OF ") {
        return type_owns_resource(success);
    }
    false
}

/// Backstop verification of the resource model's structural rules (the type
/// checker is the primary enforcer; this guards against a malformed NIR):
/// a record may not own a resource, and a union may not mix data and resource
/// variants.
fn validate_resource_rules(module: &NirModule) -> Result<(), String> {
    for type_ in &module.types {
        match type_.kind.as_str() {
            "type" => {
                for field in &type_.fields {
                    if type_owns_resource(&field.type_) {
                        return Err(format!(
                            "NIR record '{}' field '{}' owns a resource; records cannot own resources",
                            type_.name, field.name
                        ));
                    }
                }
            }
            "union" => {
                // A union must be uniformly data or uniformly resource. A
                // variant is a resource either by being a bare resource type
                // or by owning one in its payload.
                let mut has_resource = false;
                let mut has_data = false;
                for variant in &type_.variants {
                    let is_resource = crate::builtins::is_resource_type(&variant.name)
                        || variant
                            .fields
                            .iter()
                            .any(|field| type_owns_resource(&field.type_));
                    if is_resource {
                        has_resource = true;
                    } else {
                        has_data = true;
                    }
                }
                if has_resource && has_data {
                    return Err(format!(
                        "NIR union '{}' mixes data and resource variants",
                        type_.name
                    ));
                }
            }
            _ => {}
        }
    }
    Ok(())
}

mod body;
mod capabilities;
mod names;

use body::*;
use capabilities::*;
use names::*;

pub(crate) use capabilities::validate_capabilities;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::shared::nir::{
        NirEntryPoint, NirFunction, NirModule, NirOp, NirSourceLoc, NirType, NirValue, NirVariant,
    };

    fn module(runtime_helpers: Vec<RuntimeHelper>) -> NirModule {
        NirModule {
            target: "test-target".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::target::shared::code::STDIN_LOG_CAP_DEFAULT,
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Nothing".to_string(),
                accepts_args: false,
            }),
            types: Vec::new(),
            globals: Vec::new(),
            imports: Vec::new(),
            runtime_helpers,
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
                        loc: NirSourceLoc::default(),
                    },
                }],
                file: "src/main.mfb".to_string(),
                resource_owners: std::collections::HashMap::new(),
            }],
            link_functions: Vec::new(),
            link_cstructs: Vec::new(),
            native_resources: Vec::new(),
            native_libraries: Default::default(),
            max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        }
    }

    #[test]
    fn validates_declared_runtime_helper() {
        validate_nir(&module(vec![RuntimeHelper::Io])).expect("valid NIR");
    }

    #[test]
    fn rejects_undeclared_runtime_helper() {
        let err = validate_nir(&module(Vec::new())).expect_err("missing helper");
        assert_eq!(err, "NIR runtime call requires undeclared helper 'io'");
    }

    fn test_capabilities(
        runtime_calls: &'static [&'static str],
    ) -> crate::target::BackendCapabilities {
        crate::target::BackendCapabilities {
            executable: true,
            native_ir: true,
            native_plan: true,
            native_object_plan: true,
            native_code_plan: true,
            runtime_calls,
        }
    }

    // The is-implemented gate: a declared-and-used helper family with no
    // catalogued spec must be rejected (bug-329 — the gate now keys on a
    // family having a spec with non-empty `returns`, the one machine-read abi
    // field). `general` is such a family: fully native-direct, so a General
    // runtime call is legal NIR, but no `_mfb_rt_general_*` helper can be
    // emitted for it.
    #[test]
    fn rejects_helper_family_with_no_catalogued_spec() {
        let mut nir = module(vec![RuntimeHelper::General]);
        nir.functions[0].body = vec![NirOp::Eval {
            value: NirValue::RuntimeCall {
                helper: RuntimeHelper::General,
                target: "len".to_string(),
                args: vec![NirValue::Const {
                    type_: "String".to_string(),
                    value: "x".to_string(),
                }],
                loc: NirSourceLoc::default(),
            },
        }];
        let err = validate_capabilities(&nir, &test_capabilities(&[])).expect_err("no spec");
        assert_eq!(
            err,
            "native backend does not implement runtime helper 'general'"
        );
    }

    #[test]
    fn accepts_helper_family_with_catalogued_spec() {
        let nir = module(vec![RuntimeHelper::Io]);
        validate_capabilities(&nir, &test_capabilities(&["io.print"]))
            .expect("io has catalogued specs");
    }

    /// A resource-union bind nested inside a `FOR EACH` body drops by dispatching
    /// to each variant's close op, so those close helpers must be counted as used.
    /// bug-45: `collect_bind_types` skipped `NirOp::ForEach` bodies, so the union
    /// bind went unseen and `validate_nir` wrongly rejected the declared `net`
    /// helper as unused. Build the module directly so the collector is exercised
    /// in isolation from the front end.
    fn module_with_union_bind(body: Vec<NirOp>) -> NirModule {
        NirModule {
            target: "test-target".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::target::shared::code::STDIN_LOG_CAP_DEFAULT,
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: "Integer".to_string(),
                accepts_args: false,
            }),
            types: vec![NirType {
                kind: "union".to_string(),
                visibility: "public".to_string(),
                name: "Stream".to_string(),
                fields: Vec::new(),
                includes: Vec::new(),
                variants: vec![
                    NirVariant {
                        name: "File".to_string(),
                        fields: Vec::new(),
                    },
                    NirVariant {
                        name: "Socket".to_string(),
                        fields: Vec::new(),
                    },
                ],
                members: Vec::new(),
            }],
            globals: Vec::new(),
            imports: Vec::new(),
            // `File` closes via `fs`, `Socket` via `net`; both are declared so the
            // cross-check must find both in `used_helpers`.
            runtime_helpers: vec![RuntimeHelper::Fs, RuntimeHelper::Net],
            functions: vec![NirFunction {
                name: "main".to_string(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: "Integer".to_string(),
                body,
                file: "src/main.mfb".to_string(),
                resource_owners: std::collections::HashMap::new(),
            }],
            link_functions: Vec::new(),
            link_cstructs: Vec::new(),
            native_resources: Vec::new(),
            native_libraries: Default::default(),
            max_buffer_bytes: crate::manifest::DEFAULT_MAX_BUFFER_MIB * 1024 * 1024,
        }
    }

    fn union_bind() -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: "s".to_string(),
            type_: "Stream".to_string(),
            value: None,
        }
    }

    fn integer_list() -> NirValue {
        NirValue::ListLiteral {
            type_: "List OF Integer".to_string(),
            values: vec![NirValue::Const {
                type_: "Integer".to_string(),
                value: "1".to_string(),
            }],
        }
    }

    #[test]
    fn collects_resource_union_bind_inside_for_each() {
        let module = module_with_union_bind(vec![NirOp::ForEach {
            name: "n".to_string(),
            type_: "Integer".to_string(),
            iterable: integer_list(),
            body: vec![union_bind()],
        }]);
        validate_nir(&module).expect("resource-union bind inside FOR EACH must validate");
    }

    #[test]
    fn collects_resource_union_bind_at_top_level() {
        // The contrast case the bug doc names: the same bind at function scope has
        // always been collected. Guards that the ForEach fix did not change it.
        let module = module_with_union_bind(vec![union_bind()]);
        validate_nir(&module).expect("resource-union bind at top level must validate");
    }
}
