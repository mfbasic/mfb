use crate::ast::{
    AstProject, EnumMember, Expression, Function, FunctionKind, Item, MatchCase, MatchPattern,
    Param, Statement, TypeDecl, TypeDeclKind, TypeField, UnionVariant, Visibility,
};
use crate::builtins;
use crate::json_string;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct IrProject {
    pub(crate) name: String,
    pub(crate) entry: Option<EntryPoint>,
    pub(crate) types: Vec<IrType>,
    pub(crate) functions: Vec<IrFunction>,
}

#[derive(Clone)]
pub(crate) struct EntryPoint {
    pub(crate) name: String,
    pub(crate) returns: String,
    pub(crate) accepts_args: bool,
}

pub(crate) struct IrType {
    pub(crate) kind: String,
    pub(crate) visibility: String,
    pub(crate) name: String,
    pub(crate) fields: Vec<IrField>,
    pub(crate) includes: Vec<String>,
    pub(crate) variants: Vec<IrVariant>,
    pub(crate) members: Vec<IrEnumMember>,
}

pub(crate) struct IrField {
    pub(crate) visibility: Option<String>,
    pub(crate) name: String,
    pub(crate) type_: String,
}

pub(crate) struct IrVariant {
    pub(crate) name: String,
    pub(crate) fields: Vec<IrField>,
}

pub(crate) struct IrEnumMember {
    pub(crate) name: String,
}

pub(crate) struct IrFunction {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) kind: String,
    pub(crate) params: Vec<IrParam>,
    pub(crate) returns: String,
    pub(crate) body: Vec<IrOp>,
}

pub(crate) struct IrParam {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) default: Option<IrValue>,
}

pub(crate) enum IrOp {
    Bind {
        mutable: bool,
        name: String,
        type_: String,
        value: Option<IrValue>,
    },
    Assign {
        name: String,
        value: IrValue,
    },
    Return {
        value: Option<IrValue>,
    },
    Eval {
        value: IrValue,
    },
    If {
        condition: IrValue,
        then_body: Vec<IrOp>,
        else_body: Vec<IrOp>,
    },
    Match {
        value: IrValue,
        cases: Vec<IrMatchCase>,
    },
    Using {
        name: String,
        type_: String,
        value: IrValue,
        body: Vec<IrOp>,
    },
}

pub(crate) struct IrMatchCase {
    pub(crate) pattern: IrMatchPattern,
    pub(crate) body: Vec<IrOp>,
}

pub(crate) enum IrMatchPattern {
    Else,
    Value(IrValue),
}

pub(crate) enum IrValue {
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
        args: Vec<IrValue>,
    },
    Constructor {
        type_: String,
        args: Vec<IrValue>,
    },
    ListLiteral {
        type_: String,
        values: Vec<IrValue>,
    },
    MapLiteral {
        type_: String,
        entries: Vec<(IrValue, IrValue)>,
    },
    MemberAccess {
        target: Box<IrValue>,
        member: String,
    },
    Binary {
        op: String,
        left: Box<IrValue>,
        right: Box<IrValue>,
    },
    Unary {
        op: String,
        operand: Box<IrValue>,
    },
}

pub fn lower_project(ast: &AstProject, entry: Option<EntryPoint>) -> IrProject {
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let function_returns = function_returns(ast);
    let function_types = function_types(ast);
    let type_index = TypeIndex::new(ast);
    let mut context = LowerContext {
        function_returns: &function_returns,
        function_types: &function_types,
        type_index: &type_index,
        lambdas: Vec::new(),
        next_lambda_id: 0,
    };

    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => functions.push(lower_function(function, &mut context)),
                Item::Type(type_decl) => types.push(lower_type(type_decl)),
            }
        }
    }
    functions.extend(context.lambdas);

    IrProject {
        name: ast.name.clone(),
        entry,
        types,
        functions,
    }
}

pub fn write_ir(project_dir: &Path, ir: &IrProject) -> Result<PathBuf, String> {
    let ir_path = project_dir.join(format!("{}.ir", ir.name));
    fs::write(&ir_path, ir.to_json())
        .map_err(|err| format!("failed to write '{}': {err}", ir_path.display()))?;
    Ok(ir_path)
}

fn lower_type(type_decl: &TypeDecl) -> IrType {
    let kind = match type_decl.kind {
        TypeDeclKind::Type => "type",
        TypeDeclKind::Union => "union",
        TypeDeclKind::Enum => "enum",
    };
    IrType {
        kind: kind.to_string(),
        visibility: visibility_name(type_decl.visibility).to_string(),
        name: type_decl.name.clone(),
        fields: type_decl.fields.iter().map(lower_field).collect(),
        includes: type_decl.includes.clone(),
        variants: type_decl.variants.iter().map(lower_variant).collect(),
        members: type_decl.members.iter().map(lower_enum_member).collect(),
    }
}

fn lower_field(field: &TypeField) -> IrField {
    IrField {
        visibility: field.visibility.map(visibility_name).map(str::to_string),
        name: field.name.clone(),
        type_: field.type_name.clone(),
    }
}

fn lower_variant(variant: &UnionVariant) -> IrVariant {
    IrVariant {
        name: variant.name.clone(),
        fields: variant.fields.iter().map(lower_field).collect(),
    }
}

fn lower_enum_member(member: &EnumMember) -> IrEnumMember {
    IrEnumMember {
        name: member.name.clone(),
    }
}

fn lower_function(function: &Function, context: &mut LowerContext<'_>) -> IrFunction {
    let kind = match function.kind {
        FunctionKind::Func => "func",
        FunctionKind::Sub => "sub",
    };
    let returns = match function.kind {
        FunctionKind::Func => function
            .return_type
            .clone()
            .expect("typecheck requires FUNC return type before IR lowering"),
        FunctionKind::Sub => "Nothing".to_string(),
    };
    let mut locals = HashMap::new();
    for param in &function.params {
        locals.insert(
            param.name.clone(),
            param
                .type_name
                .clone()
                .expect("typecheck requires parameter type before IR lowering"),
        );
    }
    IrFunction {
        name: function.name.clone(),
        visibility: visibility_name(function.visibility).to_string(),
        kind: kind.to_string(),
        params: function
            .params
            .iter()
            .map(|param| lower_param(param, &locals, context))
            .collect(),
        returns,
        body: function
            .body
            .iter()
            .map(|statement| lower_statement(statement, &mut locals, context))
            .collect(),
    }
}

fn lower_param(
    param: &Param,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrParam {
    IrParam {
        name: param.name.clone(),
        type_: param
            .type_name
            .clone()
            .expect("typecheck requires parameter type before IR lowering"),
        default: param
            .default
            .as_ref()
            .map(|value| lower_expression(value, locals, context)),
    }
}

struct LowerContext<'a> {
    function_returns: &'a HashMap<String, String>,
    function_types: &'a HashMap<String, String>,
    type_index: &'a TypeIndex,
    lambdas: Vec<IrFunction>,
    next_lambda_id: usize,
}

fn lower_statement(
    statement: &Statement,
    locals: &mut HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrOp {
    match statement {
        Statement::Let {
            mutable,
            name,
            type_name,
            value,
            ..
        } => {
            let lowered_value = value
                .as_ref()
                .map(|value| lower_expression(value, locals, context));
            let lowered_type = type_name.clone().unwrap_or_else(|| {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, context))
                    .expect("typecheck requires inferred binding type before IR lowering")
            });
            locals.insert(name.clone(), lowered_type.clone());
            IrOp::Bind {
                mutable: *mutable,
                name: name.clone(),
                type_: lowered_type,
                value: lowered_value,
            }
        }
        Statement::Return { value, .. } => IrOp::Return {
            value: value
                .as_ref()
                .map(|value| lower_expression(value, locals, context)),
        },
        Statement::Assign { name, value, .. } => IrOp::Assign {
            name: name.clone(),
            value: lower_expression(value, locals, context),
        },
        Statement::Expression { expression, .. } => IrOp::Eval {
            value: lower_expression(expression, locals, context),
        },
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => IrOp::If {
            condition: lower_expression(condition, locals, context),
            then_body: lower_statement_block(then_body, locals, context),
            else_body: lower_statement_block(else_body, locals, context),
        },
        Statement::Match {
            expression, cases, ..
        } => IrOp::Match {
            value: lower_expression(expression, locals, context),
            cases: cases
                .iter()
                .map(|case| lower_match_case(case, locals, context))
                .collect(),
        },
        Statement::Using {
            name, value, body, ..
        } => {
            let type_ = expression_type(value, locals, context)
                .expect("typecheck requires inferred USING resource type before IR lowering");
            let value = lower_expression(value, locals, context);
            let mut nested = locals.clone();
            nested.insert(name.clone(), type_.clone());
            IrOp::Using {
                name: name.clone(),
                type_,
                value,
                body: lower_statement_block(body, &nested, context),
            }
        }
    }
}

fn lower_statement_block(
    body: &[Statement],
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    let mut nested = locals.clone();
    body.iter()
        .map(|statement| lower_statement(statement, &mut nested, context))
        .collect()
}

fn lower_match_case(
    case: &MatchCase,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrMatchCase {
    let pattern = match &case.pattern {
        MatchPattern::Else => IrMatchPattern::Else,
        MatchPattern::Expression(expression) => {
            IrMatchPattern::Value(lower_expression(expression, locals, context))
        }
    };
    IrMatchCase {
        pattern,
        body: lower_statement_block(&case.body, locals, context),
    }
}

fn function_returns(ast: &AstProject) -> HashMap<String, String> {
    let mut returns = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Function(function) = item {
                let return_type = match function.kind {
                    FunctionKind::Func => function
                        .return_type
                        .clone()
                        .expect("typecheck requires FUNC return type before IR lowering"),
                    FunctionKind::Sub => "Nothing".to_string(),
                };
                returns.insert(function.name.clone(), return_type);
            }
        }
    }
    returns
}

fn function_types(ast: &AstProject) -> HashMap<String, String> {
    let mut types = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Function(function) = item {
                let params = function
                    .params
                    .iter()
                    .map(|param| {
                        param
                            .type_name
                            .clone()
                            .expect("typecheck requires parameter type before IR lowering")
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                let returns = match function.kind {
                    FunctionKind::Func => function
                        .return_type
                        .clone()
                        .expect("typecheck requires FUNC return type before IR lowering"),
                    FunctionKind::Sub => "Nothing".to_string(),
                };
                types.insert(
                    function.name.clone(),
                    format!("FUNC({params}) AS {returns}"),
                );
            }
        }
    }
    types
}

fn expression_type(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    match expression {
        Expression::String(_) => Some("String".to_string()),
        Expression::Number(value) => {
            if value.contains('.') {
                Some("Float".to_string())
            } else {
                Some("Integer".to_string())
            }
        }
        Expression::Boolean(_) => Some("Boolean".to_string()),
        Expression::Identifier(value) if value == "NOTHING" => Some("Nothing".to_string()),
        Expression::Identifier(value) => locals
            .get(value)
            .cloned()
            .or_else(|| context.function_types.get(value).cloned()),
        Expression::Constructor { type_name, .. } => {
            context.type_index.constructor_result(type_name)
        }
        Expression::ListLiteral(values) => {
            let Some(first) = values.first() else {
                return Some("List OF Unknown".to_string());
            };
            expression_type(first, locals, context).map(|element| format!("List OF {element}"))
        }
        Expression::MapLiteral {
            key_type,
            value_type,
            ..
        } => Some(format!("Map OF {key_type} TO {value_type}")),
        Expression::MemberAccess { target, member } => {
            if let Expression::Identifier(type_name) = target.as_ref() {
                if context
                    .type_index
                    .enums
                    .get(type_name)
                    .is_some_and(|members| members.iter().any(|name| name == member))
                {
                    return Some(type_name.clone());
                }
            }
            let target_type = expression_type(target, locals, context)?;
            context.type_index.record_field_type(&target_type, member)
        }
        Expression::Call { callee, arguments } => {
            if builtins::general::is_general_call(callee) {
                if callee == "filter" && arguments.len() == 2 {
                    if let Expression::Identifier(predicate) = &arguments[1] {
                        if let Some(collection_type) =
                            expression_type(&arguments[0], locals, context)
                        {
                            if let Some(predicate_type) = collection_type
                                .strip_prefix("List OF ")
                                .and_then(|element| {
                                    builtins::general::filter_predicate_type(predicate, element)
                                })
                            {
                                let arg_types = vec![collection_type, predicate_type];
                                return builtins::general::resolve_call(callee, &arg_types)
                                    .map(|resolved| resolved.return_type.to_string());
                            }
                        }
                    }
                }
                let arg_types = arguments
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::general::resolve_call(callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::strings::is_strings_call(callee) {
                let arg_types = arguments
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::strings::resolve_call(callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::fs::is_fs_call(callee) {
                let arg_types = arguments
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::fs::resolve_call(callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::io::is_io_call(callee) {
                let arg_types = arguments
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::io::resolve_call(callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            builtins::call_return_type_name(callee)
                .map(str::to_string)
                .or_else(|| context.function_returns.get(callee).cloned())
                .or_else(|| {
                    locals
                        .get(callee)
                        .and_then(|type_| function_return_from_type(type_))
                })
        }
        Expression::Lambda { params, body } => {
            let mut nested = locals.clone();
            let param_types = params
                .iter()
                .map(|param| {
                    let type_ = param
                        .type_name
                        .clone()
                        .unwrap_or_else(|| "Unknown".to_string());
                    nested.insert(param.name.clone(), type_.clone());
                    type_
                })
                .collect::<Vec<_>>();
            let returns = expression_type(body, &nested, context)?;
            Some(format!("FUNC({}) AS {returns}", param_types.join(", ")))
        }
        Expression::Binary {
            left,
            operator,
            right,
        } => {
            if matches!(
                operator.as_str(),
                "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
            ) {
                return Some("Boolean".to_string());
            }
            if operator == "&" {
                return Some("String".to_string());
            }
            let left = expression_type(left, locals, context)?;
            let right = expression_type(right, locals, context)?;
            if left == "Float" || left == "Fixed" || right == "Float" || right == "Fixed" {
                Some("Float".to_string())
            } else {
                Some("Integer".to_string())
            }
        }
        Expression::Unary { operator, operand } => {
            if operator == "NOT" {
                Some("Boolean".to_string())
            } else {
                expression_type(operand, locals, context)
            }
        }
    }
}

fn function_return_from_type(type_: &str) -> Option<String> {
    type_
        .strip_prefix("FUNC(")
        .and_then(|rest| rest.split_once(") AS "))
        .map(|(_, return_type)| return_type.to_string())
}

fn lower_expression(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    match expression {
        Expression::String(value) => IrValue::Const {
            type_: "String".to_string(),
            value: value.clone(),
        },
        Expression::Number(value) => IrValue::Const {
            type_: if value.contains('.') {
                "Float".to_string()
            } else {
                "Integer".to_string()
            },
            value: value.clone(),
        },
        Expression::Boolean(value) => IrValue::Const {
            type_: "Boolean".to_string(),
            value: value.to_string(),
        },
        Expression::Identifier(value) if value == "NOTHING" => IrValue::Const {
            type_: "Nothing".to_string(),
            value: "NOTHING".to_string(),
        },
        Expression::Identifier(value) => {
            if let Some(type_) = context.function_types.get(value) {
                IrValue::FunctionRef {
                    name: value.clone(),
                    type_: type_.clone(),
                }
            } else {
                IrValue::Local(value.clone())
            }
        }
        Expression::Call { callee, arguments } => {
            let args = if callee == "filter" && arguments.len() == 2 {
                if let Expression::Identifier(predicate) = &arguments[1] {
                    let predicate_type = expression_type(&arguments[0], locals, context).and_then(
                        |collection_type| {
                            collection_type
                                .strip_prefix("List OF ")
                                .and_then(|element| {
                                    builtins::general::filter_predicate_type(predicate, element)
                                })
                        },
                    );
                    if let Some(predicate_type) = predicate_type {
                        vec![
                            lower_expression(&arguments[0], locals, context),
                            IrValue::FunctionRef {
                                name: predicate.clone(),
                                type_: predicate_type,
                            },
                        ]
                    } else {
                        arguments
                            .iter()
                            .map(|argument| lower_expression(argument, locals, context))
                            .collect()
                    }
                } else {
                    arguments
                        .iter()
                        .map(|argument| lower_expression(argument, locals, context))
                        .collect()
                }
            } else {
                arguments
                    .iter()
                    .map(|argument| lower_expression(argument, locals, context))
                    .collect()
            };
            IrValue::Call {
                target: callee.clone(),
                args,
            }
        }
        Expression::Lambda { params, body } => {
            let name = format!("$lambda{}", context.next_lambda_id);
            context.next_lambda_id += 1;
            let mut lambda_locals = HashMap::new();
            let ir_params = params
                .iter()
                .map(|param| {
                    let type_ = param
                        .type_name
                        .clone()
                        .expect("typecheck requires lambda parameter type before IR lowering");
                    lambda_locals.insert(param.name.clone(), type_.clone());
                    IrParam {
                        name: param.name.clone(),
                        type_,
                        default: None,
                    }
                })
                .collect::<Vec<_>>();
            let returns = expression_type(body, &lambda_locals, context)
                .expect("typecheck requires lambda return type before IR lowering");
            let value = lower_expression(body, &lambda_locals, context);
            context.lambdas.push(IrFunction {
                name: name.clone(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                params: ir_params,
                returns: returns.clone(),
                body: vec![IrOp::Return { value: Some(value) }],
            });
            let params = params
                .iter()
                .map(|param| {
                    param
                        .type_name
                        .clone()
                        .expect("typecheck requires lambda parameter type before IR lowering")
                })
                .collect::<Vec<_>>()
                .join(", ");
            IrValue::FunctionRef {
                name,
                type_: format!("FUNC({params}) AS {returns}"),
            }
        }
        Expression::Constructor {
            type_name,
            arguments,
        } => IrValue::Constructor {
            type_: type_name.clone(),
            args: arguments
                .iter()
                .map(|argument| lower_expression(argument, locals, context))
                .collect(),
        },
        Expression::ListLiteral(values) => {
            let lowered = values
                .iter()
                .map(|value| lower_expression(value, locals, context))
                .collect::<Vec<_>>();
            let element_type = values
                .first()
                .and_then(|value| literal_expression_type(value))
                .unwrap_or_else(|| "Unknown".to_string());
            IrValue::ListLiteral {
                type_: format!("List OF {element_type}"),
                values: lowered,
            }
        }
        Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => IrValue::MapLiteral {
            type_: format!("Map OF {key_type} TO {value_type}"),
            entries: entries
                .iter()
                .map(|(key, value)| {
                    (
                        lower_expression(key, locals, context),
                        lower_expression(value, locals, context),
                    )
                })
                .collect(),
        },
        Expression::MemberAccess { target, member } => IrValue::MemberAccess {
            target: Box::new(lower_expression(target, locals, context)),
            member: member.clone(),
        },
        Expression::Binary {
            left,
            operator,
            right,
        } => IrValue::Binary {
            op: operator.clone(),
            left: Box::new(lower_expression(left, locals, context)),
            right: Box::new(lower_expression(right, locals, context)),
        },
        Expression::Unary { operator, operand } => IrValue::Unary {
            op: operator.clone(),
            operand: Box::new(lower_expression(operand, locals, context)),
        },
    }
}

fn literal_expression_type(expression: &Expression) -> Option<String> {
    match expression {
        Expression::String(_) => Some("String".to_string()),
        Expression::Number(value) => {
            if value.contains('.') {
                Some("Float".to_string())
            } else {
                Some("Integer".to_string())
            }
        }
        Expression::Boolean(_) => Some("Boolean".to_string()),
        Expression::Identifier(value) if value == "NOTHING" => Some("Nothing".to_string()),
        _ => None,
    }
}

struct TypeIndex {
    records: HashMap<String, Vec<IrField>>,
    enums: HashMap<String, Vec<String>>,
    variants: HashMap<String, String>,
}

impl TypeIndex {
    fn new(ast: &AstProject) -> Self {
        let mut records = HashMap::new();
        let mut enums = HashMap::new();
        let mut variants = HashMap::new();
        for file in &ast.files {
            for item in &file.items {
                let Item::Type(type_decl) = item else {
                    continue;
                };
                match type_decl.kind {
                    TypeDeclKind::Type => {
                        records.insert(
                            type_decl.name.clone(),
                            type_decl.fields.iter().map(lower_field).collect(),
                        );
                    }
                    TypeDeclKind::Union => {
                        for variant in &type_decl.variants {
                            variants.insert(variant.name.clone(), type_decl.name.clone());
                        }
                    }
                    TypeDeclKind::Enum => {
                        enums.insert(
                            type_decl.name.clone(),
                            type_decl
                                .members
                                .iter()
                                .map(|member| member.name.clone())
                                .collect(),
                        );
                    }
                }
            }
        }
        Self {
            records,
            enums,
            variants,
        }
    }

    fn constructor_result(&self, name: &str) -> Option<String> {
        if name == "Error" {
            Some("Error".to_string())
        } else if name == "Ok" || name == "Err" {
            Some("Result OF Unknown".to_string())
        } else if self.records.contains_key(name) {
            Some(name.to_string())
        } else {
            self.variants.get(name).cloned()
        }
    }

    fn record_field_type(&self, type_name: &str, member: &str) -> Option<String> {
        if let Some(type_) = builtins::io::builtin_type_fields(type_name)
            .and_then(|fields| fields.iter().find(|(name, _)| *name == member))
            .map(|(_, type_)| (*type_).to_string())
        {
            return Some(type_);
        }
        self.records
            .get(type_name)?
            .iter()
            .find(|field| field.name == member)
            .map(|field| field.type_.clone())
    }
}

impl IrProject {
    fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-ir\",\n",
                "  \"version\": 1,\n",
                "  \"project\": {},\n",
                "  \"entry\": {},\n",
                "  \"types\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.name),
            self.entry
                .as_ref()
                .map(|entry| entry.to_json(2))
                .unwrap_or_else(|| "null".to_string()),
            join_json(&self.types, 2),
            join_json(&self.functions, 2)
        )
    }
}

impl EntryPoint {
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

trait ToIrJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToIrJson for IrType {
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
            _ => unreachable!("known IR type kind"),
        }
    }
}

impl ToIrJson for IrField {
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

impl ToIrJson for IrVariant {
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

impl ToIrJson for IrEnumMember {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!("\n{}{{ \"name\": {} }}", pad, json_string(&self.name))
    }
}

impl ToIrJson for IrFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"visibility\": {},\n",
                "{}  \"kind\": {},\n",
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

impl ToIrJson for IrParam {
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

impl ToIrJson for IrOp {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        match self {
            IrOp::Bind {
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
            IrOp::Return { value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!("\n{}{{ \"op\": \"return\", \"value\": {} }}", pad, value)
            }
            IrOp::Assign { name, value } => {
                format!(
                    "\n{}{{ \"op\": \"assign\", \"name\": {}, \"value\": {} }}",
                    pad,
                    json_string(name),
                    value.to_json(indent)
                )
            }
            IrOp::Eval { value } => {
                format!(
                    "\n{}{{ \"op\": \"eval\", \"value\": {} }}",
                    pad,
                    value.to_json(indent)
                )
            }
            IrOp::If {
                condition,
                then_body,
                else_body,
            } => {
                format!(
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
                )
            }
            IrOp::Match { value, cases } => {
                format!(
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
                )
            }
            IrOp::Using {
                name,
                type_,
                value,
                body,
            } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"using\",\n",
                        "{}  \"name\": {},\n",
                        "{}  \"type\": {},\n",
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
                    value.to_json(indent),
                    pad,
                    join_json(body, indent + 2),
                    pad,
                    pad
                )
            }
        }
    }
}

impl ToIrJson for IrMatchCase {
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

impl ToIrJson for IrMatchPattern {
    fn to_json(&self, indent: usize) -> String {
        match self {
            IrMatchPattern::Else => "{ \"kind\": \"else\" }".to_string(),
            IrMatchPattern::Value(value) => {
                format!(
                    "{{ \"kind\": \"value\", \"value\": {} }}",
                    value.to_json(indent)
                )
            }
        }
    }
}

impl ToIrJson for IrValue {
    fn to_json(&self, _indent: usize) -> String {
        match self {
            IrValue::Const { type_, value } => {
                format!(
                    "{{ \"kind\": \"const\", \"type\": {}, \"value\": {} }}",
                    json_string(type_),
                    json_string(value)
                )
            }
            IrValue::Local(name) => {
                format!("{{ \"kind\": \"local\", \"name\": {} }}", json_string(name))
            }
            IrValue::FunctionRef { name, type_ } => {
                format!(
                    "{{ \"kind\": \"functionRef\", \"name\": {}, \"type\": {} }}",
                    json_string(name),
                    json_string(type_)
                )
            }
            IrValue::Call { target, args } => {
                let args = args
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"call\", \"target\": {}, \"args\": [{}] }}",
                    json_string(target),
                    args
                )
            }
            IrValue::Constructor { type_, args } => {
                let args = args
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"constructor\", \"type\": {}, \"args\": [{}] }}",
                    json_string(type_),
                    args
                )
            }
            IrValue::ListLiteral { type_, values } => {
                let values = values
                    .iter()
                    .map(|value| value.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"list\", \"type\": {}, \"values\": [{}] }}",
                    json_string(type_),
                    values
                )
            }
            IrValue::MapLiteral { type_, entries } => {
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
            IrValue::MemberAccess { target, member } => {
                format!(
                    "{{ \"kind\": \"memberAccess\", \"target\": {}, \"member\": {} }}",
                    target.to_json(0),
                    json_string(member)
                )
            }
            IrValue::Binary { op, left, right } => {
                format!(
                    "{{ \"kind\": \"binary\", \"op\": {}, \"left\": {}, \"right\": {} }}",
                    json_string(op),
                    left.to_json(0),
                    right.to_json(0)
                )
            }
            IrValue::Unary { op, operand } => {
                format!(
                    "{{ \"kind\": \"unary\", \"op\": {}, \"operand\": {} }}",
                    json_string(op),
                    operand.to_json(0)
                )
            }
        }
    }
}

fn join_json<T: ToIrJson>(items: &[T], indent: usize) -> String {
    items
        .iter()
        .map(|item| item.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private",
        Visibility::Package => "package",
        Visibility::Export => "export",
    }
}
