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
/// layout and drop lowering. That is no longer true: a resource field is an
/// ordinary 8-byte handle slot (`record_field_is_pointer` classifies it as a
/// plain scalar slot, `record_field_is_inlined` as not inlined, so
/// `emit_record_block_size_to_slot` contributes its 8 bytes and skips it), and
/// `type_is_memcpy_copyable` now says a `memcpy` of that slot is a correct
/// aliasing copy while `type_is_arena_transferable` says the block may not cross
/// an arena. The union half is unchanged — a `{tag, record-ptr}` block really
/// does make drop dispatch tag-dependent.
fn validate_resource_rules(module: &NirModule) -> Result<(), String> {
    for type_ in &module.types {
        match type_.kind.as_str() {
            "union" => {
                // A union must be uniformly data or uniformly resource. A
                // variant is a resource either by being a bare resource type
                // or by owning one in its payload.
                let mut has_resource = false;
                let mut has_data = false;
                for variant in &type_.variants {
                    let is_resource = crate::codegen::builtins::is_resource_type(
                        &crate::types::ParameterType::declared(&variant.name),
                    ) || variant
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
        NirEntryPoint, NirFunction, NirMatchCase, NirMatchPattern, NirModule, NirOp, NirSourceLoc,
        NirType, NirValue, NirVariant,
    };
    use crate::types::ParameterType;

    fn module(runtime_helpers: Vec<RuntimeHelper>) -> NirModule {
        NirModule {
            target: "test-target".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::codegen::error::constants::STDIN_LOG_CAP_DEFAULT,
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: ParameterType::Nothing,
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
                returns: ParameterType::Nothing,
                body: vec![NirOp::Eval {
                    value: NirValue::RuntimeCall {
                        helper: RuntimeHelper::Io,
                        target: "io.print".to_string(),
                        args: vec![NirValue::Const {
                            type_: ParameterType::String,
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

    /// plan-114-B: the NIR backstop's record half is gone — a record field may
    /// own a resource at the NIR level. The union half must be untouched.
    fn record_type(name: &str, fields: &[(&str, &str)]) -> NirType {
        NirType {
            kind: "type".to_string(),
            visibility: "export".to_string(),
            name: name.to_string(),
            fields: fields
                .iter()
                .map(|(n, t)| crate::target::shared::nir::NirField {
                    visibility: None,
                    name: (*n).to_string(),
                    type_: ParameterType::parse(t),
                })
                .collect(),
            includes: Vec::new(),
            variants: Vec::new(),
            members: Vec::new(),
        }
    }

    #[test]
    fn a_record_field_may_own_a_resource_at_nir_level() {
        // The shape letters C-E build on: an 8-byte handle slot beside ordinary
        // data. Before plan-114-B this was refused with "records cannot own
        // resources", on the grounds that it would mislead the layout and drop
        // lowering — which is no longer true.
        for spelling in ["RES fs.File", "fs.File", "List OF RES fs.File"] {
            let mut m = module(vec![RuntimeHelper::Io]);
            m.types = vec![record_type(
                "Holder",
                &[("name", "String"), ("handle", spelling)],
            )];
            validate_nir(&m)
                .unwrap_or_else(|e| panic!("`{spelling}` field must validate, got: {e}"));
        }
    }

    #[test]
    fn a_union_still_may_not_mix_data_and_resource_variants() {
        // The half of validate_resource_rules that plan-114-B deliberately did
        // NOT relax: a `{tag, record-ptr}` block really does make drop dispatch
        // tag-dependent, so this must keep failing.
        let mut m = module(vec![RuntimeHelper::Io]);
        m.types = vec![NirType {
            kind: "union".to_string(),
            visibility: "export".to_string(),
            name: "Mixed".to_string(),
            fields: Vec::new(),
            includes: Vec::new(),
            variants: vec![
                NirVariant {
                    name: "Plain".to_string(),
                    fields: vec![crate::target::shared::nir::NirField {
                        visibility: None,
                        name: "count".to_string(),
                        type_: ParameterType::Integer,
                    }],
                },
                NirVariant {
                    name: "Held".to_string(),
                    fields: vec![crate::target::shared::nir::NirField {
                        visibility: None,
                        name: "handle".to_string(),
                        // BARE, not `RES fs.File`: `type_owns_resource` has no
                        // `Res(_)` arm (see its NB comment), so a `RES`-marked
                        // field is invisible to this backstop. That gap is
                        // pre-existing and deliberate — the front-end
                        // `TYPE_MIXED_RESOURCE_UNION` is the primary enforcer —
                        // but it means a test written with the `RES` spelling
                        // would pass for the wrong reason and prove nothing.
                        type_: ParameterType::declared("fs.File"),
                    }],
                },
            ],
            members: Vec::new(),
        }];
        let err = validate_nir(&m).expect_err("a mixed union must still be refused");
        assert!(
            err.contains("mixes data and resource variants"),
            "wrong rejection: {err}"
        );
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
                    type_: ParameterType::String,
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

    /// bug-328: a runtime call that appears only inside a `WHEN … WHERE` guard
    /// still executes, so capability validation must see it. Before the fix the
    /// capability collector walked the scrutinee and each case body but skipped
    /// `case.guard`, so a backend-unsupported call hidden in a guard slipped
    /// through unchecked (the same omission bug-118 fixed on the sibling passes).
    /// Build the module directly so the collector is exercised in isolation.
    #[test]
    fn capability_check_sees_runtime_call_in_match_guard() {
        let mut nir = module(vec![RuntimeHelper::Io]);
        nir.functions[0].body = vec![NirOp::Match {
            value: NirValue::Const {
                type_: ParameterType::Integer,
                value: "1".to_string(),
            },
            cases: vec![NirMatchCase {
                pattern: NirMatchPattern::Else,
                guard: Some(NirValue::RuntimeCall {
                    helper: RuntimeHelper::Io,
                    target: "io.print".to_string(),
                    args: vec![NirValue::Const {
                        type_: ParameterType::String,
                        value: "guarded".to_string(),
                    }],
                    loc: NirSourceLoc::default(),
                }),
                body: Vec::new(),
            }],
        }];
        let err = validate_capabilities(&nir, &test_capabilities(&[]))
            .expect_err("a runtime call in a match guard must be checked against capabilities");
        assert_eq!(
            err,
            "native backend does not support runtime call 'io.print'"
        );
    }

    /// A resource-union bind nested inside a `FOR EACH` body drops by dispatching
    /// to each variant's close op, so those close helpers must be counted as used.
    /// bug-45: `collect_bind_types` skipped `NirOp::ForEach` bodies, so the union
    /// bind went unseen and `validate_nir` wrongly rejected the declared transport
    /// helper as unused. Build the module directly so the collector is exercised
    /// in isolation from the front end.
    fn module_with_union_bind(body: Vec<NirOp>) -> NirModule {
        NirModule {
            target: "test-target".to_string(),
            build_mode: crate::target::NativeBuildMode::Console,
            stdin_log_cap: crate::codegen::error::constants::STDIN_LOG_CAP_DEFAULT,
            project: "hello".to_string(),
            entry: Some(NirEntryPoint {
                name: "main".to_string(),
                returns: ParameterType::Integer,
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
                        name: "fs.File".to_string(),
                        fields: Vec::new(),
                    },
                    NirVariant {
                        name: "tcp.Socket".to_string(),
                        fields: Vec::new(),
                    },
                ],
                members: Vec::new(),
            }],
            globals: Vec::new(),
            imports: Vec::new(),
            // `File` closes via `fs`, `Socket` via `tcp`; both are declared so the
            // cross-check must find both in `used_helpers`.
            runtime_helpers: vec![RuntimeHelper::Fs, RuntimeHelper::Tcp],
            functions: vec![NirFunction {
                name: "main".to_string(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: ParameterType::Integer,
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
            type_: ParameterType::named("Stream"),
            value: None,
        }
    }

    fn integer_list() -> NirValue {
        NirValue::ListLiteral {
            type_: ParameterType::list_of(ParameterType::Integer),
            values: vec![NirValue::Const {
                type_: ParameterType::Integer,
                value: "1".to_string(),
            }],
        }
    }

    /// bug-535: a plain built-in resource `Bind`. `value: None` matches the
    /// `thread::accept` shape after lowering — an owning bind whose close op is
    /// emitted by codegen at scope exit, never as an NIR call.
    fn plain_resource_bind(type_: &str, value: Option<NirValue>) -> NirOp {
        NirOp::Bind {
            mutable: false,
            name: "s".to_string(),
            type_: ParameterType::parse(type_),
            value,
        }
    }

    /// A module whose `main` runs `body`, declaring `helpers` on top of the `Io`
    /// the shared `module()` body's `io.print` uses.
    fn module_with_body(helpers: Vec<RuntimeHelper>, body: Vec<NirOp>) -> NirModule {
        let mut m = module(helpers);
        m.functions[0].body.extend(body);
        m
    }

    #[test]
    fn a_plain_resource_bind_counts_its_close_helper_as_used() {
        // The bug-535 reproduction, at the layer that rejected it: a worker whose
        // only reference to `tcp` is `RES s AS tcp::Socket = thread::accept(...)`
        // declares the `tcp` helper (codegen needs it to emit the scope-exit
        // close) and made no NIR call into the package, so the "declares unused"
        // arm fired on valid source.
        let m = module_with_body(
            vec![RuntimeHelper::Io, RuntimeHelper::Tcp],
            vec![plain_resource_bind("tcp.Socket", None)],
        );
        validate_nir(&m)
            .unwrap_or_else(|e| panic!("a `tcp.Socket` bind must count `tcp` as used, got: {e}"));
    }

    #[test]
    fn every_sendable_builtin_resource_bind_counts_its_helper() {
        // `thread::accept` can deliver any sendable resource, so the hole was
        // never `tcp`-specific. Each of these is a distinct package helper.
        for (type_, helper) in [
            ("tcp.Socket", RuntimeHelper::Tcp),
            ("tcp.Listener", RuntimeHelper::Tcp),
            ("tls.Socket", RuntimeHelper::Tls),
            ("tls.Listener", RuntimeHelper::Tls),
            ("udp.Socket", RuntimeHelper::Udp),
            ("fs.File", RuntimeHelper::Fs),
        ] {
            let m = module_with_body(
                vec![RuntimeHelper::Io, helper],
                vec![plain_resource_bind(type_, None)],
            );
            validate_nir(&m)
                .unwrap_or_else(|e| panic!("`{type_}` bind must count {helper:?}, got: {e}"));
        }
    }

    #[test]
    fn a_stateful_plain_resource_bind_counts_the_same_helper() {
        // plan-74's requirement for the union arm, on the plain arm: a
        // `RES f AS fs::File STATE Progress` names the same resource as its bare
        // form. `resource_close_function` peels the suffix itself, so the arm
        // needs no second textual strip -- this pins that it really does.
        let stateful = ParameterType::parse("fs.File STATE Progress");
        assert_ne!(
            stateful.name(),
            stateful.without_state().name(),
            "the fixture must actually carry a STATE clause"
        );
        let m = module_with_body(
            vec![RuntimeHelper::Io, RuntimeHelper::Fs],
            vec![plain_resource_bind("fs.File STATE Progress", None)],
        );
        validate_nir(&m).expect("a stateful plain resource bind must count `fs` as used");
    }

    #[test]
    fn an_aliasing_resource_bind_counts_no_helper() {
        // The over-count guard, and the reason the new arm carries the declarer's
        // aliasing gate. `runtime::required_helpers` declares NOTHING for a bind
        // that merely names an already-live resource (bug-375), so counting one as
        // used here would turn bug-535 into its mirror image: "NIR runtime call
        // requires undeclared helper 'tcp'" on a program that builds today.
        // Both aliasing shapes the declarer recognizes. Each is preceded by the
        // bind that introduces the local it names, because `validate_function`
        // resolves every local reference by name first.
        let from_local = vec![
            NirOp::Bind {
                mutable: false,
                name: "other".to_string(),
                type_: ParameterType::Integer,
                value: None,
            },
            plain_resource_bind("tcp.Socket", Some(NirValue::Local("other".to_string()))),
        ];
        let from_collection_get = vec![
            NirOp::Bind {
                mutable: false,
                name: "socks".to_string(),
                type_: ParameterType::parse("List OF RES tcp.Socket"),
                value: None,
            },
            plain_resource_bind(
                "tcp.Socket",
                Some(NirValue::Call {
                    target: "collections.get".to_string(),
                    args: vec![NirValue::Local("socks".to_string())],
                    loc: NirSourceLoc::default(),
                }),
            ),
        ];
        for body in [from_local, from_collection_get] {
            let m = module_with_body(vec![RuntimeHelper::Io], body);
            validate_nir(&m).unwrap_or_else(|e| {
                panic!("an aliasing bind must declare no helper of its own, got: {e}")
            });
        }
    }

    #[test]
    fn a_genuinely_unused_helper_is_still_rejected() {
        // The arm bug-535 must NOT disable: a helper declared with nothing that
        // needs it still fails. This is what catches a helper left declared after
        // its only user was optimized away.
        let m = module(vec![RuntimeHelper::Io, RuntimeHelper::Tcp]);
        let err = validate_nir(&m).expect_err("an unreferenced declared helper must be rejected");
        assert!(
            err.contains("declares unused runtime helper 'tcp'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn a_resource_bind_whose_helper_is_undeclared_is_still_rejected() {
        // The opposite arm, on the new path specifically: the plain-resource arm
        // adds `tcp` to the used set, and a module that failed to declare it is
        // still caught -- a link failure the check exists to prevent.
        let m = module_with_body(
            vec![RuntimeHelper::Io],
            vec![plain_resource_bind("tcp.Socket", None)],
        );
        let err = validate_nir(&m).expect_err("an undeclared close helper must still be rejected");
        assert!(
            err.contains("requires undeclared helper 'tcp'"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn collects_resource_union_bind_inside_for_each() {
        let module = module_with_union_bind(vec![NirOp::ForEach {
            name: "n".to_string(),
            type_: ParameterType::Integer,
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
