use super::*;

/// Lower an `IrProject` to a `NirModule`. By the time this runs, any imported
/// packages have already been decoded and merged into `ir` (see
/// `lower::lower_project`), so every function flows through this one codegen and
/// there are no native-level imports to resolve.
pub(crate) fn lower_module(
    ir: &IrProject,
    target: String,
    build_mode: crate::target::NativeBuildMode,
    runtime_helpers: Vec<RuntimeHelper>,
) -> Result<NirModule, String> {
    Ok(NirModule {
        target,
        build_mode,
        project: ir.name.clone(),
        entry: ir.entry.as_ref().map(lower_entry),
        globals: ir
            .bindings
            .iter()
            .map(|binding| lower_global(&ir.name, binding))
            .collect(),
        types: ir.types.iter().map(lower_type).collect(),
        imports: link_routing_imports(ir),
        runtime_helpers,
        functions: lower_functions(ir),
        link_functions: ir.link_functions.clone(),
    })
}

/// Build the call-routing entries (name → thunk symbol) for every native `LINK`
/// function and re-export alias, so a call to `alias.func` (or an exported alias)
/// resolves to its generated marshaling thunk through the existing import path.
fn link_routing_imports(ir: &IrProject) -> Vec<NirImport> {
    let mut imports = Vec::new();
    let mut thunk_for: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for function in &ir.link_functions {
        let target = format!("{}.{}", function.alias, function.name);
        let symbol = link_thunk_symbol(&function.alias, &function.name);
        thunk_for.insert(target.clone(), symbol.clone());
        imports.push(NirImport {
            package: "link".to_string(),
            name: target,
            symbol,
            kind: "func".to_string(),
            isolated: false,
            params: Vec::new(),
            returns: String::new(),
        });
    }
    // A re-export alias routes to the same thunk as its LINK target
    // (plan-link-update.md §5a). The alias is referenced by importers as
    // `binding.alias`, but inside this project as the bare alias name; register
    // both so either resolves.
    for (alias_name, target) in &ir.link_aliases {
        if let Some(symbol) = thunk_for.get(target) {
            imports.push(NirImport {
                package: "link".to_string(),
                name: alias_name.clone(),
                symbol: symbol.clone(),
                kind: "func".to_string(),
                isolated: false,
                params: Vec::new(),
                returns: String::new(),
            });
        }
    }
    imports
}

/// Decode each imported package's structured Binary Representation and merge it into `ir`,
/// namespacing package symbols so the consumer's existing `pkg.symbol` references
/// resolve. The returned project carries the executable's and every package's
/// functions/types/globals together.
pub(crate) fn merge_packages(ir: &IrProject, packages: &[PathBuf]) -> Result<IrProject, String> {
    let mut merged = ir.clone();
    // Per package: (identity id, the `package.symbol` names the consumer/other
    // packages reference). Collected so external references can be rewritten to
    // the identity-prefixed definitions after every package is merged.
    let mut identities: Vec<(String, HashSet<String>, HashSet<String>)> = Vec::new();
    for package in packages {
        let (id, mut package_ir) = binary_repr::read_package_ir_with_identity(package)?;
        // Verify the decoded package IR against the package-format invariants
        // before it is merged (decode already rejected bad version/bytes).
        crate::ir::verify_package(&package_ir)?;
        let (ref_fns, ref_globals) = crate::ir::package_qualified_reference_names(&package_ir);
        crate::ir::prefix_package_symbols(&mut package_ir, &id);
        identities.push((id, ref_fns, ref_globals));
        crate::ir::merge_package(&mut merged, package_ir);
    }
    // Rewrite the consumer's (and inter-package) references from `package.symbol`
    // to the merged definitions' identity-prefixed `<id>.package.symbol` form.
    for (id, ref_fns, ref_globals) in &identities {
        crate::ir::apply_package_identity(&mut merged, ref_fns, ref_globals, id);
    }
    // Semantically verify the fully merged IR before it is lowered to native
    // code (plan-19-ir-semantic-verification.md). `verify_package` re-states the
    // package-format structural invariants; this pass adds the semantic ones —
    // typing, closure-capture bounds, call/constructor arity, union-variant
    // membership — that codegen otherwise trusts. Decoded package IR is attacker
    // controlled and never passed the source type checker, so this is the gate
    // that keeps type-confused IR (audit-1 PKG-02) out of the victim's binary.
    crate::ir::verify_semantics(&merged)?;
    Ok(merged)
}

pub(crate) fn lower_ops(ops: &[IrOp]) -> Vec<NirOp> {
    ops.iter().map(lower_op).collect()
}

fn lower_entry(entry: &EntryPoint) -> NirEntryPoint {
    NirEntryPoint {
        name: entry.name.clone(),
        returns: entry.returns.clone(),
        accepts_args: entry.accepts_args,
    }
}

fn lower_global(project: &str, binding: &IrBinding) -> NirGlobal {
    NirGlobal {
        name: binding.name.clone(),
        symbol: global_symbol(project, &binding.name),
        visibility: binding.visibility.clone(),
        mutable: binding.mutable,
        type_: binding.type_.clone(),
        value: binding.value.as_ref().map(lower_value),
    }
}

fn lower_functions(ir: &IrProject) -> Vec<NirFunction> {
    let mut functions = Vec::new();
    if !ir.bindings.is_empty() {
        functions.push(lower_global_initializer(ir));
    }
    functions.extend(ir.functions.iter().map(lower_function));
    functions
}

fn lower_global_initializer(ir: &IrProject) -> NirFunction {
    NirFunction {
        name: global_initializer_name(&ir.name),
        visibility: "private".to_string(),
        kind: "sub".to_string(),
        isolated: false,
        params: Vec::new(),
        returns: "Nothing".to_string(),
        body: ir
            .bindings
            .iter()
            .map(|binding| NirOp::StoreGlobal {
                name: binding.name.clone(),
                type_: binding.type_.clone(),
                value: binding.value.as_ref().map(lower_value),
            })
            .collect(),
        // A binding initializer that allocates (a collection/string/record
        // literal, a constructor, a fallible call) can emit an error-report path,
        // and that path loads the source file as `ErrorLoc.filename`. Give the
        // synthesized initializer a real file so the filename is a defined string
        // constant (registered by `string_symbols`) rather than the
        // `_mfb_str_empty` sentinel — which is only emitted when a *function*
        // demands it, leaving a dangling data relocation for global-only
        // initializers (bug-05). Using the first binding's file also makes
        // global-init error origins point at the source rather than nowhere.
        file: ir
            .bindings
            .iter()
            .map(|binding| binding.file.clone())
            .find(|file| !file.is_empty())
            .unwrap_or_default(),
        resource_owners: HashMap::new(),
    }
}

fn lower_type(type_: &IrType) -> NirType {
    NirType {
        kind: type_.kind.clone(),
        visibility: type_.visibility.clone(),
        name: type_.name.clone(),
        fields: type_.fields.iter().map(lower_field).collect(),
        includes: type_.includes.clone(),
        variants: type_.variants.iter().map(lower_variant).collect(),
        members: type_.members.iter().map(lower_enum_member).collect(),
    }
}

fn lower_field(field: &IrField) -> NirField {
    NirField {
        visibility: field.visibility.clone(),
        name: field.name.clone(),
        type_: field.type_.clone(),
    }
}

fn lower_variant(variant: &IrVariant) -> NirVariant {
    NirVariant {
        name: variant.name.clone(),
        fields: variant.fields.iter().map(lower_field).collect(),
    }
}

fn lower_enum_member(member: &IrEnumMember) -> NirEnumMember {
    NirEnumMember {
        name: member.name.clone(),
    }
}

fn lower_function(function: &IrFunction) -> NirFunction {
    NirFunction {
        name: function.name.clone(),
        visibility: function.visibility.clone(),
        kind: function.kind.clone(),
        isolated: function.isolated,
        params: function.params.iter().map(lower_param).collect(),
        returns: function.returns.clone(),
        body: lower_ops(&function.body),
        file: function.file.clone(),
        resource_owners: function.resource_owners.clone(),
    }
}

fn lower_param(param: &IrParam) -> NirParam {
    NirParam {
        name: param.name.clone(),
        type_: param.type_.clone(),
        default: param.default.as_ref().map(lower_value),
    }
}

fn lower_op(op: &IrOp) -> NirOp {
    match op {
        IrOp::Bind {
            mutable,
            name,
            type_,
            value,
            ..
        } => NirOp::Bind {
            mutable: *mutable,
            name: name.clone(),
            type_: type_.clone(),
            value: value.as_ref().map(lower_value),
        },
        IrOp::Assign { name, value, .. } => NirOp::Assign {
            name: name.clone(),
            value: lower_value(value),
        },
        IrOp::StateAssign {
            resource, value, ..
        } => NirOp::StateAssign {
            resource: resource.clone(),
            value: lower_value(value),
        },
        IrOp::AssignGlobal { name, value, .. } => NirOp::StoreGlobal {
            name: name.clone(),
            type_: String::new(),
            value: Some(lower_value(value)),
        },
        IrOp::Return { value, .. } => NirOp::Return {
            value: value.as_ref().map(lower_value),
        },
        IrOp::ExitLoop { kind, .. } => NirOp::ExitLoop { kind: *kind },
        IrOp::ContinueLoop { kind, .. } => NirOp::ContinueLoop { kind: *kind },
        IrOp::ExitProgram { code, .. } => NirOp::ExitProgram {
            code: lower_value(code),
        },
        IrOp::Fail { error, .. } => NirOp::Fail {
            error: lower_value(error),
        },
        IrOp::Eval { value, .. } => NirOp::Eval {
            value: lower_value(value),
        },
        IrOp::If {
            condition,
            then_body,
            else_body,
            ..
        } => NirOp::If {
            condition: lower_value(condition),
            then_body: lower_ops(then_body),
            else_body: lower_ops(else_body),
        },
        IrOp::Match { value, cases, .. } => NirOp::Match {
            value: lower_value(value),
            cases: cases.iter().map(lower_match_case).collect(),
        },
        IrOp::While {
            kind,
            condition,
            body,
            ..
        } => NirOp::While {
            kind: *kind,
            condition: lower_value(condition),
            body: lower_ops(body),
        },
        IrOp::For {
            name,
            type_,
            start,
            end,
            step,
            body,
            loc,
        } => NirOp::For {
            name: name.clone(),
            type_: type_.clone(),
            start: lower_value(start),
            end: lower_value(end),
            step: lower_value(step),
            body: lower_ops(body),
            loc: lower_loc(*loc),
        },
        IrOp::DoUntil {
            body, condition, ..
        } => NirOp::DoUntil {
            body: lower_ops(body),
            condition: lower_value(condition),
        },
        IrOp::ForEach {
            name,
            type_,
            iterable,
            body,
            ..
        } => NirOp::ForEach {
            name: name.clone(),
            type_: type_.clone(),
            iterable: lower_value(iterable),
            body: lower_ops(body),
        },
        IrOp::Trap { name, body, .. } => NirOp::Trap {
            name: name.clone(),
            body: lower_ops(body),
        },
    }
}

fn lower_match_case(case: &IrMatchCase) -> NirMatchCase {
    NirMatchCase {
        pattern: lower_match_pattern(&case.pattern),
        guard: case.guard.as_ref().map(lower_value),
        body: lower_ops(&case.body),
    }
}

fn lower_match_pattern(pattern: &IrMatchPattern) -> NirMatchPattern {
    match pattern {
        IrMatchPattern::Else => NirMatchPattern::Else,
        IrMatchPattern::Value(value) => NirMatchPattern::Value(lower_value(value)),
        IrMatchPattern::OneOf(values) => {
            NirMatchPattern::OneOf(values.iter().map(lower_value).collect())
        }
    }
}

fn lower_value(value: &IrValue) -> NirValue {
    match value {
        IrValue::Const { type_, value } => NirValue::Const {
            type_: type_.clone(),
            value: value.clone(),
        },
        IrValue::Local(name) => NirValue::Local(name.clone()),
        IrValue::LocalRef { name, type_ } => NirValue::LocalRef {
            name: name.clone(),
            type_: type_.clone(),
        },
        IrValue::Global(name) => NirValue::Global {
            name: name.clone(),
            type_: String::new(),
        },
        IrValue::FunctionRef { name, type_ } => NirValue::FunctionRef {
            name: name.clone(),
            type_: type_.clone(),
        },
        IrValue::Closure {
            name,
            type_,
            captures,
        } => NirValue::Closure {
            name: name.clone(),
            type_: type_.clone(),
            captures: captures.iter().map(lower_value).collect(),
        },
        IrValue::Capture {
            index,
            type_,
            by_ref,
        } => NirValue::Capture {
            index: *index,
            type_: type_.clone(),
            by_ref: *by_ref,
        },
        IrValue::Call {
            target, args, loc, ..
        } => {
            let loc = lower_loc(*loc);
            let mut args = args.iter().map(lower_value).collect::<Vec<_>>();
            match (target.as_str(), args.len()) {
                ("fs.openFile" | "fs.openFileNoFollow", 1) => {
                    args.push(NirValue::Const {
                        type_: "String".to_string(),
                        value: "read".to_string(),
                    });
                }
                ("fs.createTempFile", 0) => {
                    args.push(NirValue::RuntimeCall {
                        helper: super::runtime::RuntimeHelper::Fs,
                        target: "fs.tempDirectory".to_string(),
                        args: Vec::new(),
                        loc,
                    });
                }
                _ => {}
            }
            if super::runtime::is_native_direct_call(target) {
                NirValue::Call {
                    target: target.clone(),
                    args,
                    loc,
                }
            } else if let Some(helper) = super::runtime::helper_for_call(target) {
                NirValue::RuntimeCall {
                    helper,
                    target: target.clone(),
                    args,
                    loc,
                }
            } else {
                NirValue::Call {
                    target: target.clone(),
                    args,
                    loc,
                }
            }
        }
        IrValue::CallResult {
            target, args, loc, ..
        } => NirValue::CallResult {
            target: target.clone(),
            args: args.iter().map(lower_value).collect(),
            loc: lower_loc(*loc),
        },
        IrValue::Constructor { type_, args } => NirValue::Constructor {
            type_: type_.clone(),
            args: args.iter().map(lower_value).collect(),
        },
        IrValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => NirValue::UnionWrap {
            union_type: union_type.clone(),
            member_type: member_type.clone(),
            value: Box::new(lower_value(value)),
        },
        IrValue::UnionExtract { type_, value } => NirValue::UnionExtract {
            type_: type_.clone(),
            value: Box::new(lower_value(value)),
        },
        IrValue::ResultIsOk { value } => NirValue::ResultIsOk {
            value: Box::new(lower_value(value)),
        },
        IrValue::ResultValue { value, .. } => NirValue::ResultValue {
            value: Box::new(lower_value(value)),
        },
        IrValue::ResultError { value } => NirValue::ResultError {
            value: Box::new(lower_value(value)),
        },
        IrValue::WithUpdate {
            type_,
            target,
            updates,
        } => NirValue::WithUpdate {
            type_: type_.clone(),
            target: Box::new(lower_value(target)),
            updates: updates.iter().map(lower_record_update).collect(),
        },
        IrValue::ListLiteral { type_, values } => NirValue::ListLiteral {
            type_: type_.clone(),
            values: values.iter().map(lower_value).collect(),
        },
        IrValue::MapLiteral { type_, entries } => NirValue::MapLiteral {
            type_: type_.clone(),
            entries: entries
                .iter()
                .map(|(key, value)| (lower_value(key), lower_value(value)))
                .collect(),
        },
        IrValue::MemberAccess { target, member, .. } => NirValue::MemberAccess {
            target: Box::new(lower_value(target)),
            member: member.clone(),
        },
        IrValue::Binary {
            op,
            left,
            right,
            loc,
            ..
        } => NirValue::Binary {
            op: op.clone(),
            left: Box::new(lower_value(left)),
            right: Box::new(lower_value(right)),
            loc: lower_loc(*loc),
        },
        IrValue::Unary {
            op, operand, loc, ..
        } => NirValue::Unary {
            op: op.clone(),
            operand: Box::new(lower_value(operand)),
            loc: lower_loc(*loc),
        },
    }
}

fn lower_loc(loc: crate::ir::IrSourceLoc) -> NirSourceLoc {
    NirSourceLoc {
        line: loc.line,
        column: loc.column,
    }
}

fn lower_record_update(update: &IrRecordUpdate) -> NirRecordUpdate {
    NirRecordUpdate {
        field: update.field.clone(),
        value: lower_value(&update.value),
    }
}
