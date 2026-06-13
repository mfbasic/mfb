use crate::bytecode::{self, BytecodeExportKind};
use crate::ir::{
    EntryPoint, IrEnumMember, IrField, IrFunction, IrMatchCase, IrMatchPattern, IrOp, IrParam,
    IrProject, IrRecordUpdate, IrType, IrValue, IrVariant,
};
use crate::json_string;
use std::path::PathBuf;

use super::runtime::RuntimeHelper;

pub(crate) struct NirModule {
    pub(crate) target: String,
    pub(crate) project: String,
    pub(crate) entry: Option<NirEntryPoint>,
    pub(crate) types: Vec<NirType>,
    pub(crate) imports: Vec<NirImport>,
    pub(crate) runtime_helpers: Vec<RuntimeHelper>,
    pub(crate) functions: Vec<NirFunction>,
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

pub(crate) struct NirFunction {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<NirParam>,
    pub(crate) returns: String,
    pub(crate) body: Vec<NirOp>,
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
    Assign {
        name: String,
        value: NirValue,
    },
    Return {
        value: Option<NirValue>,
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
    ForEach {
        name: String,
        type_: String,
        iterable: NirValue,
        body: Vec<NirOp>,
    },
    Using {
        name: String,
        type_: String,
        close: String,
        value: NirValue,
        body: Vec<NirOp>,
    },
}

pub(crate) struct NirMatchCase {
    pub(crate) pattern: NirMatchPattern,
    pub(crate) body: Vec<NirOp>,
}

pub(crate) enum NirMatchPattern {
    Else,
    Value(NirValue),
}

#[derive(Clone)]
pub(crate) enum NirValue {
    Const {
        type_: String,
        value: String,
    },
    Local(String),
    FunctionRef {
        name: String,
        type_: String,
    },
    Call {
        target: String,
        args: Vec<NirValue>,
    },
    RuntimeCall {
        helper: RuntimeHelper,
        target: String,
        args: Vec<NirValue>,
    },
    Constructor {
        type_: String,
        args: Vec<NirValue>,
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
    },
    Unary {
        op: String,
        operand: Box<NirValue>,
    },
}

#[derive(Clone)]
pub(crate) struct NirRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: NirValue,
}

pub(crate) fn lower_module(
    ir: &IrProject,
    target: String,
    runtime_helpers: Vec<RuntimeHelper>,
    packages: &[PathBuf],
) -> Result<NirModule, String> {
    let imports = lower_imports(packages)?;
    Ok(NirModule {
        target,
        project: ir.name.clone(),
        entry: ir.entry.as_ref().map(lower_entry),
        types: ir.types.iter().map(lower_type).collect(),
        imports,
        runtime_helpers,
        functions: ir.functions.iter().map(lower_function).collect(),
    })
}

pub(crate) fn lower_imports(packages: &[PathBuf]) -> Result<Vec<NirImport>, String> {
    let mut imports = Vec::new();
    for package in packages {
        let info = bytecode::read_package_info(package)?;
        for export in bytecode::read_package_exports(package)? {
            let name = format!("{}.{}", info.manifest_name, export.name);
            imports.push(NirImport {
                symbol: import_symbol(&info.manifest_name, &export.name),
                package: info.manifest_name.clone(),
                name,
                kind: match export.kind {
                    BytecodeExportKind::Func => "func".to_string(),
                    BytecodeExportKind::Sub => "sub".to_string(),
                },
                isolated: export.isolated,
                params: export
                    .params
                    .into_iter()
                    .map(|param| NirImportParam {
                        type_: param.type_,
                        has_default: param.has_default,
                    })
                    .collect(),
                returns: export.return_type,
            });
        }
    }
    Ok(imports)
}

pub(crate) fn function_symbol(name: &str) -> String {
    format!("_mfb_fn_{}", symbol_fragment(name))
}

pub(crate) fn import_symbol(package: &str, name: &str) -> String {
    format!(
        "_mfb_pkg_{}_{}",
        symbol_fragment(package),
        symbol_fragment(name)
    )
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
        IrOp::Return { value } => NirOp::Return {
            value: value.as_ref().map(lower_value),
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
        IrOp::Using {
            name,
            type_,
            close,
            value,
            body,
        } => NirOp::Using {
            name: name.clone(),
            type_: type_.clone(),
            close: close.clone(),
            value: lower_value(value),
            body: lower_ops(body),
        },
    }
}

fn lower_match_case(case: &IrMatchCase) -> NirMatchCase {
    NirMatchCase {
        pattern: lower_match_pattern(&case.pattern),
        body: lower_ops(&case.body),
    }
}

fn lower_match_pattern(pattern: &IrMatchPattern) -> NirMatchPattern {
    match pattern {
        IrMatchPattern::Else => NirMatchPattern::Else,
        IrMatchPattern::Value(value) => NirMatchPattern::Value(lower_value(value)),
    }
}

fn lower_value(value: &IrValue) -> NirValue {
    match value {
        IrValue::Const { type_, value } => NirValue::Const {
            type_: type_.clone(),
            value: value.clone(),
        },
        IrValue::Local(name) => NirValue::Local(name.clone()),
        IrValue::FunctionRef { name, type_ } => NirValue::FunctionRef {
            name: name.clone(),
            type_: type_.clone(),
        },
        IrValue::Call { target, args } => {
            let args = args.iter().map(lower_value).collect();
            if matches!(
                target.as_str(),
                "contains"
                    | "append"
                    | "get"
                    | "getOr"
                    | "hasKey"
                    | "insert"
                    | "find"
                    | "forEach"
                    | "filter"
                    | "keys"
                    | "len"
                    | "mid"
                    | "prepend"
                    | "reduce"
                    | "removeAt"
                    | "removeKey"
                    | "replace"
                    | "set"
                    | "sum"
                    | "transform"
                    | "values"
                    | "toByte"
                    | "toFixed"
                    | "toFloat"
                    | "toInt"
                    | "toString"
                    | "isNumeric"
            ) {
                NirValue::Call {
                    target: target.clone(),
                    args,
                }
            } else if let Some(helper) = super::runtime::helper_for_call(target) {
                NirValue::RuntimeCall {
                    helper,
                    target: target.clone(),
                    args,
                }
            } else {
                NirValue::Call {
                    target: target.clone(),
                    args,
                }
            }
        }
        IrValue::Constructor { type_, args } => NirValue::Constructor {
            type_: type_.clone(),
            args: args.iter().map(lower_value).collect(),
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
        IrValue::Binary { op, left, right } => NirValue::Binary {
            op: op.clone(),
            left: Box::new(lower_value(left)),
            right: Box::new(lower_value(right)),
        },
        IrValue::Unary { op, operand } => NirValue::Unary {
            op: op.clone(),
            operand: Box::new(lower_value(operand)),
        },
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
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-nir\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"project\": {},\n",
                "  \"entry\": {},\n",
                "  \"types\": [{}\n  ],\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"runtimeHelpers\": [{}],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(&self.project),
            self.entry
                .as_ref()
                .map(|entry| entry.to_json(2))
                .unwrap_or_else(|| "null".to_string()),
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
            NirOp::Return { value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!("\n{}{{ \"op\": \"return\", \"value\": {} }}", pad, value)
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
            NirOp::Using {
                name,
                type_,
                close,
                value,
                body,
            } => format!(
                concat!(
                    "\n{}{{\n",
                    "{}  \"op\": \"using\",\n",
                    "{}  \"name\": {},\n",
                    "{}  \"type\": {},\n",
                    "{}  \"close\": {},\n",
                    "{}  \"value\": {},\n",
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
                json_string(close),
                pad,
                value.to_json(indent),
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
                "{}  \"body\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            self.pattern.to_json(indent),
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
            NirValue::FunctionRef { name, type_ } => format!(
                "{{ \"kind\": \"functionRef\", \"name\": {}, \"type\": {} }}",
                json_string(name),
                json_string(type_)
            ),
            NirValue::Call { target, args } => format!(
                "{{ \"kind\": \"call\", \"target\": {}, \"args\": [{}] }}",
                json_string(target),
                join_values(args)
            ),
            NirValue::RuntimeCall {
                helper,
                target,
                args,
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
            NirValue::Binary { op, left, right } => format!(
                "{{ \"kind\": \"binary\", \"op\": {}, \"left\": {}, \"right\": {} }}",
                json_string(op),
                left.to_json(0),
                right.to_json(0)
            ),
            NirValue::Unary { op, operand } => format!(
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

fn join_values(values: &[NirValue]) -> String {
    values
        .iter()
        .map(|value| value.to_json(0))
        .collect::<Vec<_>>()
        .join(", ")
}
