use crate::ast::LoopKind;
use crate::binary_repr;
use crate::ir::{
    EntryPoint, IrBinding, IrEnumMember, IrField, IrFunction, IrMatchCase, IrMatchPattern, IrOp,
    IrParam, IrProject, IrRecordUpdate, IrType, IrValue, IrVariant,
};
use crate::json_string;
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::PathBuf;

use super::runtime::RuntimeHelper;

pub(crate) struct NirModule {
    pub(crate) target: String,
    /// Native build mode this module was lowered for (`console` or `macos-app`).
    /// Recorded so downstream plan/code stages and goldens reflect app mode.
    pub(crate) build_mode: crate::target::NativeBuildMode,
    pub(crate) project: String,
    pub(crate) entry: Option<NirEntryPoint>,
    pub(crate) globals: Vec<NirGlobal>,
    pub(crate) types: Vec<NirType>,
    pub(crate) imports: Vec<NirImport>,
    pub(crate) runtime_helpers: Vec<RuntimeHelper>,
    pub(crate) functions: Vec<NirFunction>,
    /// Native `LINK` functions whose marshaling thunks the backend emits
    /// (plan-linker.md §12). Carried verbatim from the IR.
    pub(crate) link_functions: Vec<crate::ir::IrLinkFunction>,
}

/// The internal text symbol of the per-program native `LINK` load-time
/// initializer (plan-linker.md §12.1): runs `dlopen`/`dlsym` before `main`.
pub(crate) const LINK_INIT_SYMBOL: &str = "_mfb_linker_init";

/// The internal text symbol for a native `LINK` function's marshaling thunk
/// (plan-linker.md §12.2): `_mfb_linker_<alias>_<name>`.
pub(crate) fn link_thunk_symbol(alias: &str, name: &str) -> String {
    let sanitize = |part: &str| {
        part.chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    format!("_mfb_linker_{}_{}", sanitize(alias), sanitize(name))
}

pub(crate) struct NirEntryPoint {
    pub(crate) name: String,
    pub(crate) returns: String,
    pub(crate) accepts_args: bool,
}

pub(crate) struct NirType {
    pub(crate) kind: String,
    pub(crate) visibility: String,
    pub(crate) name: String,
    pub(crate) fields: Vec<NirField>,
    pub(crate) includes: Vec<String>,
    pub(crate) variants: Vec<NirVariant>,
    pub(crate) members: Vec<NirEnumMember>,
}

pub(crate) struct NirField {
    pub(crate) visibility: Option<String>,
    pub(crate) name: String,
    pub(crate) type_: String,
}

pub(crate) struct NirVariant {
    pub(crate) name: String,
    pub(crate) fields: Vec<NirField>,
}

pub(crate) struct NirEnumMember {
    pub(crate) name: String,
}

pub(crate) struct NirImport {
    pub(crate) package: String,
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<NirImportParam>,
    pub(crate) returns: String,
}

pub(crate) struct NirImportParam {
    pub(crate) type_: String,
    pub(crate) has_default: bool,
}

pub(crate) struct NirGlobal {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) visibility: String,
    pub(crate) mutable: bool,
    pub(crate) type_: String,
    pub(crate) value: Option<NirValue>,
}

pub(crate) struct NirFunction {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<NirParam>,
    pub(crate) returns: String,
    pub(crate) body: Vec<NirOp>,
    /// Project-relative source file this function was lowered from. Used to build
    /// `ErrorLoc.filename` for errors that originate inside this function.
    pub(crate) file: String,
    /// Resource ownership decisions (escape analysis, §15.6), keyed by `RES`
    /// binding name. Absent names are [`crate::escape::ResOwner::Local`].
    pub(crate) resource_owners: HashMap<String, crate::escape::ResOwner>,
}

pub(crate) struct NirParam {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) default: Option<NirValue>,
}

pub(crate) enum NirOp {
    Bind {
        mutable: bool,
        name: String,
        type_: String,
        value: Option<NirValue>,
    },
    StoreGlobal {
        name: String,
        type_: String,
        value: Option<NirValue>,
    },
    Assign {
        name: String,
        value: NirValue,
    },
    /// Replace the `STATE` payload of a `RES` binding (`resource.state = value`).
    StateAssign {
        resource: String,
        value: NirValue,
    },
    Return {
        value: Option<NirValue>,
    },
    ExitLoop {
        kind: LoopKind,
    },
    ContinueLoop {
        kind: LoopKind,
    },
    ExitProgram {
        code: NirValue,
    },
    Fail {
        error: NirValue,
    },
    Eval {
        value: NirValue,
    },
    If {
        condition: NirValue,
        then_body: Vec<NirOp>,
        else_body: Vec<NirOp>,
    },
    Match {
        value: NirValue,
        cases: Vec<NirMatchCase>,
    },
    While {
        kind: LoopKind,
        condition: NirValue,
        body: Vec<NirOp>,
    },
    For {
        name: String,
        type_: String,
        start: NirValue,
        end: NirValue,
        step: NirValue,
        body: Vec<NirOp>,
        // Source location of the loop header; origin for increment overflow.
        loc: NirSourceLoc,
    },
    DoUntil {
        body: Vec<NirOp>,
        condition: NirValue,
    },
    ForEach {
        name: String,
        type_: String,
        iterable: NirValue,
        body: Vec<NirOp>,
    },
    Trap {
        name: String,
        body: Vec<NirOp>,
    },
}

pub(crate) struct NirMatchCase {
    pub(crate) pattern: NirMatchPattern,
    pub(crate) guard: Option<NirValue>,
    pub(crate) body: Vec<NirOp>,
}

pub(crate) enum NirMatchPattern {
    Else,
    Value(NirValue),
    OneOf(Vec<NirValue>),
}

#[derive(Clone)]
pub(crate) enum NirValue {
    Const {
        type_: String,
        value: String,
    },
    Local(String),
    Global {
        name: String,
        type_: String,
    },
    FunctionRef {
        name: String,
        type_: String,
    },
    Closure {
        name: String,
        type_: String,
        captures: Vec<NirValue>,
    },
    Capture {
        index: usize,
        type_: String,
    },
    Call {
        target: String,
        args: Vec<NirValue>,
        loc: NirSourceLoc,
    },
    CallResult {
        target: String,
        args: Vec<NirValue>,
        loc: NirSourceLoc,
    },
    RuntimeCall {
        helper: RuntimeHelper,
        target: String,
        args: Vec<NirValue>,
        loc: NirSourceLoc,
    },
    Constructor {
        type_: String,
        args: Vec<NirValue>,
    },
    UnionWrap {
        union_type: String,
        member_type: String,
        value: Box<NirValue>,
    },
    UnionExtract {
        type_: String,
        value: Box<NirValue>,
    },
    ResultIsOk {
        value: Box<NirValue>,
    },
    ResultValue {
        value: Box<NirValue>,
    },
    ResultError {
        value: Box<NirValue>,
    },
    WithUpdate {
        type_: String,
        target: Box<NirValue>,
        updates: Vec<NirRecordUpdate>,
    },
    ListLiteral {
        type_: String,
        values: Vec<NirValue>,
    },
    MapLiteral {
        type_: String,
        entries: Vec<(NirValue, NirValue)>,
    },
    MemberAccess {
        target: Box<NirValue>,
        member: String,
    },
    Binary {
        op: String,
        left: Box<NirValue>,
        right: Box<NirValue>,
        loc: NirSourceLoc,
    },
    Unary {
        op: String,
        operand: Box<NirValue>,
        loc: NirSourceLoc,
    },
}

/// Source location (line/column within the owning function's file) attached to
/// NIR nodes that can originate a runtime error. The file is carried on
/// [`NirFunction::file`].
#[derive(Clone, Copy, Default)]
pub(crate) struct NirSourceLoc {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[derive(Clone)]
pub(crate) struct NirRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: NirValue,
}

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
    Ok(merged)
}

pub(crate) fn function_symbol(name: &str) -> String {
    format!("_mfb_fn_{}", symbol_fragment(name))
}

pub(crate) fn global_symbol(project: &str, name: &str) -> String {
    format!(
        "_mfb_global_{}_{}",
        symbol_fragment(project),
        symbol_fragment(name)
    )
}

pub(crate) fn global_initializer_name(project: &str) -> String {
    format!("__mfb_init_globals_{}", symbol_fragment(project))
}

pub(crate) fn symbol_fragment(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect()
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
        file: String::new(),
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
        } => NirOp::Bind {
            mutable: *mutable,
            name: name.clone(),
            type_: type_.clone(),
            value: value.as_ref().map(lower_value),
        },
        IrOp::Assign { name, value } => NirOp::Assign {
            name: name.clone(),
            value: lower_value(value),
        },
        IrOp::StateAssign { resource, value } => NirOp::StateAssign {
            resource: resource.clone(),
            value: lower_value(value),
        },
        IrOp::AssignGlobal { name, value } => NirOp::StoreGlobal {
            name: name.clone(),
            type_: String::new(),
            value: Some(lower_value(value)),
        },
        IrOp::Return { value } => NirOp::Return {
            value: value.as_ref().map(lower_value),
        },
        IrOp::ExitLoop { kind } => NirOp::ExitLoop { kind: *kind },
        IrOp::ContinueLoop { kind } => NirOp::ContinueLoop { kind: *kind },
        IrOp::ExitProgram { code } => NirOp::ExitProgram {
            code: lower_value(code),
        },
        IrOp::Fail { error } => NirOp::Fail {
            error: lower_value(error),
        },
        IrOp::Eval { value } => NirOp::Eval {
            value: lower_value(value),
        },
        IrOp::If {
            condition,
            then_body,
            else_body,
        } => NirOp::If {
            condition: lower_value(condition),
            then_body: lower_ops(then_body),
            else_body: lower_ops(else_body),
        },
        IrOp::Match { value, cases } => NirOp::Match {
            value: lower_value(value),
            cases: cases.iter().map(lower_match_case).collect(),
        },
        IrOp::While {
            kind,
            condition,
            body,
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
        IrOp::DoUntil { body, condition } => NirOp::DoUntil {
            body: lower_ops(body),
            condition: lower_value(condition),
        },
        IrOp::ForEach {
            name,
            type_,
            iterable,
            body,
        } => NirOp::ForEach {
            name: name.clone(),
            type_: type_.clone(),
            iterable: lower_value(iterable),
            body: lower_ops(body),
        },
        IrOp::Trap { name, body } => NirOp::Trap {
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
        IrValue::Capture { index, type_ } => NirValue::Capture {
            index: *index,
            type_: type_.clone(),
        },
        IrValue::Call { target, args, loc } => {
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
        IrValue::CallResult { target, args, loc } => NirValue::CallResult {
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
        IrValue::ResultValue { value } => NirValue::ResultValue {
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
        IrValue::MemberAccess { target, member } => NirValue::MemberAccess {
            target: Box::new(lower_value(target)),
            member: member.clone(),
        },
        IrValue::Binary {
            op,
            left,
            right,
            loc,
        } => NirValue::Binary {
            op: op.clone(),
            left: Box::new(lower_value(left)),
            right: Box::new(lower_value(right)),
            loc: lower_loc(*loc),
        },
        IrValue::Unary { op, operand, loc } => NirValue::Unary {
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

impl NirModule {
    pub(crate) fn to_json(&self) -> String {
        let globals = if self.globals.is_empty() {
            String::new()
        } else {
            format!("  \"globals\": [{}\n  ],\n", join_json(&self.globals, 2))
        };
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-nir\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"buildMode\": {},\n",
                "  \"project\": {},\n",
                "  \"entry\": {},\n",
                "{}",
                "  \"types\": [{}\n  ],\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"runtimeHelpers\": [{}],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(self.build_mode.as_str()),
            json_string(&self.project),
            self.entry
                .as_ref()
                .map(|entry| entry.to_json(2))
                .unwrap_or_else(|| "null".to_string()),
            globals,
            join_json(&self.types, 2),
            join_json(&self.imports, 2),
            self.runtime_helpers
                .iter()
                .map(|helper| json_string(helper.name()))
                .collect::<Vec<_>>()
                .join(", "),
            join_json(&self.functions, 2)
        )
    }
}

impl ToNirJson for NirGlobal {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let value = self
            .value
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            concat!(
                "\n{}{{ \"name\": {}, \"symbol\": {}, \"visibility\": {}, ",
                "\"mutable\": {}, \"type\": {}, \"value\": {} }}"
            ),
            pad,
            json_string(&self.name),
            json_string(&self.symbol),
            json_string(&self.visibility),
            self.mutable,
            json_string(&self.type_),
            value
        )
    }
}

trait ToNirJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToNirJson for NirEntryPoint {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "{{\n",
                "{}  \"name\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"accepts_args\": {}\n",
                "{}}}"
            ),
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.returns),
            pad,
            self.accepts_args,
            pad
        )
    }
}

impl ToNirJson for NirType {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self.kind.as_str() {
            "type" => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}  \"fields\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(&self.kind),
                pad,
                json_string(&self.visibility),
                pad,
                json_string(&self.name),
                pad,
                join_json(&self.fields, indent + 2),
                pad,
                pad
            ),
            "union" => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}  \"includes\": [{}],\n",
                    "{}  \"variants\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(&self.kind),
                pad,
                json_string(&self.visibility),
                pad,
                json_string(&self.name),
                pad,
                self.includes
                    .iter()
                    .map(|value| json_string(value))
                    .collect::<Vec<_>>()
                    .join(", "),
                pad,
                join_json(&self.variants, indent + 2),
                pad,
                pad
            ),
            "enum" => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"kind\": {},\n",
                    "{}  \"visibility\": {},\n",
                    "{}  \"name\": {},\n",
                    "{}  \"members\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                json_string(&self.kind),
                pad,
                json_string(&self.visibility),
                pad,
                json_string(&self.name),
                pad,
                join_json(&self.members, indent + 2),
                pad,
                pad
            ),
            _ => unreachable!("known NIR type kind"),
        }
    }
}

impl ToNirJson for NirField {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let visibility = self
            .visibility
            .as_ref()
            .map(|value| json_string(value))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"visibility\": {}, \"name\": {}, \"type\": {} }}",
            pad,
            visibility,
            json_string(&self.name),
            json_string(&self.type_)
        )
    }
}

impl ToNirJson for NirVariant {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"fields\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            join_json(&self.fields, indent + 2),
            pad,
            pad
        )
    }
}

impl ToNirJson for NirEnumMember {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!("\n{}{{ \"name\": {} }}", pad, json_string(&self.name))
    }
}

impl ToNirJson for NirImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"package\": {},\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"kind\": {},\n",
                "{}  \"isolated\": {},\n",
                "{}  \"params\": [{}],\n",
                "{}  \"returns\": {}\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.package),
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            json_string(&self.kind),
            pad,
            self.isolated,
            pad,
            self.params
                .iter()
                .map(|param| param.to_json(0))
                .collect::<Vec<_>>()
                .join(", "),
            pad,
            json_string(&self.returns),
            pad
        )
    }
}

impl ToNirJson for NirImportParam {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"type\": {}, \"hasDefault\": {} }}",
            json_string(&self.type_),
            self.has_default
        )
    }
}

impl ToNirJson for NirFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"visibility\": {},\n",
                "{}  \"kind\": {},\n",
                "{}  \"isolated\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"returns\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.visibility),
            pad,
            json_string(&self.kind),
            pad,
            self.isolated,
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            json_string(&self.returns),
            pad,
            join_json(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToNirJson for NirParam {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let default = self
            .default
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"default\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            default
        )
    }
}

impl ToNirJson for NirOp {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            NirOp::Bind {
                mutable,
                name,
                type_,
                value,
            } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"op\": \"bind\", \"mutable\": {}, \"name\": {}, \"type\": {}, \"value\": {} }}",
                    pad,
                    mutable,
                    json_string(name),
                    json_string(type_),
                    value
                )
            }
            NirOp::Assign { name, value } => format!(
                "\n{}{{ \"op\": \"assign\", \"name\": {}, \"value\": {} }}",
                pad,
                json_string(name),
                value.to_json(indent)
            ),
            NirOp::StateAssign { resource, value } => format!(
                "\n{}{{ \"op\": \"stateAssign\", \"resource\": {}, \"value\": {} }}",
                pad,
                json_string(resource),
                value.to_json(indent)
            ),
            NirOp::StoreGlobal { name, type_, value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!(
                    "\n{}{{ \"op\": \"storeGlobal\", \"name\": {}, \"type\": {}, \"value\": {} }}",
                    pad,
                    json_string(name),
                    json_string(type_),
                    value
                )
            }
            NirOp::Return { value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!("\n{}{{ \"op\": \"return\", \"value\": {} }}", pad, value)
            }
            NirOp::ExitLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"exitLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            NirOp::ContinueLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"continueLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            NirOp::ExitProgram { code } => {
                format!(
                    "\n{}{{ \"op\": \"exitProgram\", \"code\": {} }}",
                    pad,
                    code.to_json(indent)
                )
            }
            NirOp::Fail { error } => {
                format!(
                    "\n{}{{ \"op\": \"fail\", \"error\": {} }}",
                    pad,
                    error.to_json(indent)
                )
            }
            NirOp::Eval { value } => format!(
                "\n{}{{ \"op\": \"eval\", \"value\": {} }}",
                pad,
                value.to_json(indent)
            ),
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"if\",\n",
                    "{}  \"condition\": {},\n",
                    "{}  \"then\": [{}\n{}  ],\n",
                    "{}  \"else\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                pad,
                condition.to_json(indent),
                pad,
                join_json(then_body, indent + 2),
                pad,
                pad,
                join_json(else_body, indent + 2),
                pad,
                pad
            ),
            NirOp::Match { value, cases } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"match\",\n",
                    "{}  \"value\": {},\n",
                    "{}  \"cases\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                pad,
                value.to_json(indent),
                pad,
                join_json(cases, indent + 2),
                pad,
                pad
            ),
            NirOp::While {
                kind,
                condition,
                body,
            } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"while\",\n",
                    "{}  \"loop\": {},\n",
                    "{}  \"condition\": {},\n",
                    "{}  \"body\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                pad,
                json_string(loop_kind_name(*kind)),
                pad,
                condition.to_json(indent),
                pad,
                join_json(body, indent + 2),
                pad,
                pad
            ),
            NirOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"for\",\n",
                    "{}  \"name\": {},\n",
                    "{}  \"type\": {},\n",
                    "{}  \"start\": {},\n",
                    "{}  \"end\": {},\n",
                    "{}  \"step\": {},\n",
                    "{}  \"body\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                pad,
                json_string(name),
                pad,
                json_string(type_),
                pad,
                start.to_json(indent),
                pad,
                end.to_json(indent),
                pad,
                step.to_json(indent),
                pad,
                join_json(body, indent + 2),
                pad,
                pad
            ),
            NirOp::DoUntil { body, condition } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"doUntil\",\n",
                    "{}  \"condition\": {},\n",
                    "{}  \"body\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                pad,
                condition.to_json(indent),
                pad,
                join_json(body, indent + 2),
                pad,
                pad
            ),
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"forEach\",\n",
                    "{}  \"name\": {},\n",
                    "{}  \"type\": {},\n",
                    "{}  \"iterable\": {},\n",
                    "{}  \"body\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                pad,
                json_string(name),
                pad,
                json_string(type_),
                pad,
                iterable.to_json(indent),
                pad,
                join_json(body, indent + 2),
                pad,
                pad
            ),
            NirOp::Trap { name, body } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"trap\",\n",
                    "{}  \"name\": {},\n",
                    "{}  \"body\": [{}\n{}  ]\n",
                    "{}}}"
                ),
                pad,
                pad,
                pad,
                json_string(name),
                pad,
                join_json(body, indent + 2),
                pad,
                pad
            ),
        }
    }
}

impl ToNirJson for NirMatchCase {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"pattern\": {},\n",
                "{}  \"guard\": {},\n",
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            self.pattern.to_json(indent),
            pad,
            self.guard
                .as_ref()
                .map(|guard| guard.to_json(indent))
                .unwrap_or_else(|| "null".to_string()),
            pad,
            join_json(&self.body, indent + 2),
            pad,
            pad
        )
    }
}

impl ToNirJson for NirMatchPattern {
    fn to_json(&self, indent: usize) -> String {
        match self {
            NirMatchPattern::Else => "{ \"kind\": \"else\" }".to_string(),
            NirMatchPattern::Value(value) => {
                format!(
                    "{{ \"kind\": \"value\", \"value\": {} }}",
                    value.to_json(indent)
                )
            }
            NirMatchPattern::OneOf(values) => format!(
                "{{ \"kind\": \"oneOf\", \"values\": [{}] }}",
                values
                    .iter()
                    .map(|value| value.to_json(indent))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    }
}

impl ToNirJson for NirValue {
    fn to_json(&self, _indent: usize) -> String {
        match self {
            NirValue::Const { type_, value } => format!(
                "{{ \"kind\": \"const\", \"type\": {}, \"value\": {} }}",
                json_string(type_),
                json_string(value)
            ),
            NirValue::Local(name) => {
                format!("{{ \"kind\": \"local\", \"name\": {} }}", json_string(name))
            }
            NirValue::Global { name, type_ } => format!(
                "{{ \"kind\": \"global\", \"name\": {}, \"type\": {} }}",
                json_string(name),
                json_string(type_)
            ),
            NirValue::FunctionRef { name, type_ } => format!(
                "{{ \"kind\": \"functionRef\", \"name\": {}, \"type\": {} }}",
                json_string(name),
                json_string(type_)
            ),
            NirValue::Closure {
                name,
                type_,
                captures,
            } => format!(
                "{{ \"kind\": \"closure\", \"name\": {}, \"type\": {}, \"captures\": [{}] }}",
                json_string(name),
                json_string(type_),
                join_values(captures)
            ),
            NirValue::Capture { index, type_ } => format!(
                "{{ \"kind\": \"capture\", \"index\": {}, \"type\": {} }}",
                index,
                json_string(type_)
            ),
            NirValue::Call { target, args, .. } => format!(
                "{{ \"kind\": \"call\", \"target\": {}, \"args\": [{}] }}",
                json_string(target),
                join_values(args)
            ),
            NirValue::CallResult { target, args, .. } => format!(
                "{{ \"kind\": \"callResult\", \"target\": {}, \"args\": [{}] }}",
                json_string(target),
                join_values(args)
            ),
            NirValue::RuntimeCall {
                helper,
                target,
                args,
                ..
            } => format!(
                "{{ \"kind\": \"runtimeCall\", \"helper\": {}, \"target\": {}, \"args\": [{}] }}",
                json_string(helper.name()),
                json_string(target),
                join_values(args)
            ),
            NirValue::Constructor { type_, args } => format!(
                "{{ \"kind\": \"constructor\", \"type\": {}, \"args\": [{}] }}",
                json_string(type_),
                join_values(args)
            ),
            NirValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => format!(
                "{{ \"kind\": \"unionWrap\", \"union\": {}, \"member\": {}, \"value\": {} }}",
                json_string(union_type),
                json_string(member_type),
                value.to_json(0)
            ),
            NirValue::UnionExtract { type_, value } => format!(
                "{{ \"kind\": \"unionExtract\", \"type\": {}, \"value\": {} }}",
                json_string(type_),
                value.to_json(0)
            ),
            NirValue::ResultIsOk { value } => format!(
                "{{ \"kind\": \"resultIsOk\", \"value\": {} }}",
                value.to_json(0)
            ),
            NirValue::ResultValue { value } => format!(
                "{{ \"kind\": \"resultValue\", \"value\": {} }}",
                value.to_json(0)
            ),
            NirValue::ResultError { value } => format!(
                "{{ \"kind\": \"resultError\", \"value\": {} }}",
                value.to_json(0)
            ),
            NirValue::WithUpdate {
                type_,
                target,
                updates,
            } => format!(
                "{{ \"kind\": \"with\", \"type\": {}, \"target\": {}, \"updates\": [{}] }}",
                json_string(type_),
                target.to_json(0),
                updates
                    .iter()
                    .map(|update| update.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            NirValue::ListLiteral { type_, values } => format!(
                "{{ \"kind\": \"list\", \"type\": {}, \"values\": [{}] }}",
                json_string(type_),
                join_values(values)
            ),
            NirValue::MapLiteral { type_, entries } => {
                let entries = entries
                    .iter()
                    .map(|(key, value)| {
                        format!(
                            "{{ \"key\": {}, \"value\": {} }}",
                            key.to_json(0),
                            value.to_json(0)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"map\", \"type\": {}, \"entries\": [{}] }}",
                    json_string(type_),
                    entries
                )
            }
            NirValue::MemberAccess { target, member } => format!(
                "{{ \"kind\": \"memberAccess\", \"target\": {}, \"member\": {} }}",
                target.to_json(0),
                json_string(member)
            ),
            NirValue::Binary { op, left, right, .. } => format!(
                "{{ \"kind\": \"binary\", \"op\": {}, \"left\": {}, \"right\": {} }}",
                json_string(op),
                left.to_json(0),
                right.to_json(0)
            ),
            NirValue::Unary { op, operand, .. } => format!(
                "{{ \"kind\": \"unary\", \"op\": {}, \"operand\": {} }}",
                json_string(op),
                operand.to_json(0)
            ),
        }
    }
}

impl ToNirJson for NirRecordUpdate {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"field\": {}, \"value\": {} }}",
            json_string(&self.field),
            self.value.to_json(0)
        )
    }
}

fn join_json<T: ToNirJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn loop_kind_name(kind: LoopKind) -> &'static str {
    match kind {
        LoopKind::For => "for",
        LoopKind::Do => "do",
        LoopKind::While => "while",
    }
}

fn join_values(values: &[NirValue]) -> String {
    values
        .iter()
        .map(|value| value.to_json(0))
        .collect::<Vec<_>>()
        .join(", ")
}
