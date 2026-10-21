use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use crate::ir::IrProject;
use crate::target::{BackendCapabilities, BuildTarget};

use super::nir::{NirFunction, NirMatchPattern, NirModule, NirOp, NirParam, NirValue};
use super::runtime::{self, RuntimeHelper};
use crate::types::ParameterType;

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
            .map(|variant| {
                crate::codegen::builtins::resource_close_function(
                    &crate::types::ParameterType::declared(&variant.name),
                )
            })
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

    // A plain built-in resource bind drops the same way: its registered close op
    // is emitted by codegen at scope exit, not as an NIR call, so nothing above
    // counts it. Until a `Bind` could arrive from `thread::accept`, every program
    // holding a resource also called into its package and the helper was counted
    // through that call; a worker that only receives handles has no such call, and
    // the "declares unused" arm below rejected it (bug-535).
    //
    // `resource_close_function` peels a `STATE T` suffix itself
    // (`builtin_resource_close_function` resolves `type_.without_state()`), so
    // `RES f AS fs::File STATE Progress` resolves to the same `fs.close` as its
    // bare form — the plan-74 requirement, met structurally rather than by a
    // second textual strip.
    //
    // The collector applies the declarer's aliasing gate, so this can only add a
    // helper `runtime::required_helpers` also declared; see
    // `collect_owning_resource_bind_types`.
    let mut owning_bind_types = Vec::new();
    for function in &module.functions {
        collect_owning_resource_bind_types(&function.body, &mut owning_bind_types);
    }
    for type_ in &owning_bind_types {
        let Some(close) = crate::codegen::builtins::resource_close_function(type_) else {
            continue;
        };
        if let Some(helper) = runtime::helper_for_call(close) {
            if !used_helpers.contains(&helper) {
                used_helpers.push(helper);
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

/// Whether a NIR type transitively owns a resource (directly, or as a
/// collection element/value). `STATE`-suffixed resource spellings are
/// recognized via `is_resource_type` on the rendered nominal name.
fn type_owns_resource(type_: &ParameterType) -> bool {
    match type_ {
        ParameterType::ListOf(element) => type_owns_resource(element),
        ParameterType::MapOf(key, value) => type_owns_resource(key) || type_owns_resource(value),
        ParameterType::ResultOf(success) => type_owns_resource(success),
        // NB: no `Res(inner)` arm — the string form never stripped the `RES `
        // marker (`is_resource_type("RES File")` is false), so a `Res`-wrapped
        // element falls to the name check below exactly as the string walk did.
        other => crate::codegen::builtins::is_resource_type(&other),
    }
}

/// Backstop verification of the resource model's structural rules (the type
/// checker is the primary enforcer; this guards against a malformed NIR):
/// a union may not mix data and resource variants.
///
/// plan-114-B: the **record** half of this is gone. It used to reject any record
/// field owning a resource, on the grounds that such a field would mislead the
/// layout and drop lowering. That is no longer true: a concrete resource field is
/// an ordinary 8-byte handle slot (`record_field_is_pointer` classifies it as a
/// plain scalar slot, `record_field_is_inlined` as not inlined, so
/// `emit_record_block_size_to_slot` contributes its 8 bytes and skips it), a
/// resource-union field holds the union's 16-byte box inline (bug-1012), and
/// `type_is_memcpy_copyable` now says a `memcpy` of that slot is a correct
/// aliasing copy while `type_is_arena_transferable` says the block may not cross
/// an arena. The union half is unchanged — a `{tag, record-ptr}` block really
/// does make drop dispatch tag-dependent.
fn validate_resource_rules(module: &NirModule) -> Result<(), String> {
    // bug-851: a `RESOURCE`-declared variant is a resource variant too.
    let declared_resources =
        crate::codegen::engine::validation::validation::module_resource_names(module);
    for type_ in &module.types {
        match type_.kind.as_str() {
            "union" => {
                // A union must be uniformly data or uniformly resource. A
                // variant is a resource either by being a bare resource type
                // or by owning one in its payload.
                let mut has_resource = false;
                let mut has_data = false;
                for variant in &type_.variants {
                    let variant_type = crate::types::ParameterType::declared(&variant.name);
                    let is_resource = crate::codegen::builtins::is_resource_type(&variant_type)
                        || declared_resources.contains(&variant_type)
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
mod tests;
