use crate::ast::{
    AstProject, ConstructorArg, EnumMember, Expression, Function, FunctionKind, Item, MatchCase,
    MatchPattern, Param, Statement, TypeDecl, TypeDeclKind, TypeField, UnionVariant, Visibility,
};
use crate::builtins;
use crate::json_string;
use crate::numeric;
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

#[derive(Clone)]
pub(crate) struct IrField {
    pub(crate) visibility: Option<String>,
    pub(crate) name: String,
    pub(crate) type_: String,
}

#[derive(Clone)]
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
    pub(crate) isolated: bool,
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
    Fail {
        error: IrValue,
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
    While {
        condition: IrValue,
        body: Vec<IrOp>,
    },
    ForEach {
        name: String,
        type_: String,
        iterable: IrValue,
        body: Vec<IrOp>,
    },
    Using {
        name: String,
        type_: String,
        close: String,
        value: IrValue,
        body: Vec<IrOp>,
    },
    Trap {
        name: String,
        body: Vec<IrOp>,
    },
}

pub(crate) struct IrMatchCase {
    pub(crate) pattern: IrMatchPattern,
    pub(crate) guard: Option<IrValue>,
    pub(crate) body: Vec<IrOp>,
}

pub(crate) enum IrMatchPattern {
    Else,
    Value(IrValue),
    OneOf(Vec<IrValue>),
}

#[derive(Clone)]
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
    CallResult {
        target: String,
        args: Vec<IrValue>,
    },
    Constructor {
        type_: String,
        args: Vec<IrValue>,
    },
    UnionWrap {
        union_type: String,
        member_type: String,
        value: Box<IrValue>,
    },
    UnionExtract {
        type_: String,
        value: Box<IrValue>,
    },
    ResultIsOk {
        value: Box<IrValue>,
    },
    ResultValue {
        value: Box<IrValue>,
    },
    ResultError {
        value: Box<IrValue>,
    },
    WithUpdate {
        type_: String,
        target: Box<IrValue>,
        updates: Vec<IrRecordUpdate>,
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

#[derive(Clone)]
pub(crate) struct IrRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: IrValue,
}

pub fn lower_project(ast: &AstProject, entry: Option<EntryPoint>) -> IrProject {
    lower_project_with_external_functions(ast, entry, &HashMap::new())
}

pub fn lower_project_with_external_functions(
    ast: &AstProject,
    entry: Option<EntryPoint>,
    external_function_types: &HashMap<String, String>,
) -> IrProject {
    let augmented = builtins::json::augmented_project(ast)
        .expect("built-in json package source must parse");
    let ast = &augmented;
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let function_returns = function_returns(ast);
    let mut function_types = function_types(ast);
    function_types.extend(external_function_types.clone());
    let type_index = TypeIndex::new(ast);
    let mut context = LowerContext {
        function_returns: &function_returns,
        function_types: &function_types,
        type_index: &type_index,
        lambdas: Vec::new(),
        next_lambda_id: 0,
        next_temp_id: 0,
    };

    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => functions.push(lower_function(function, &mut context)),
                Item::Type(type_decl) => types.push(lower_type(type_decl, &type_index)),
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

fn lower_type(type_decl: &TypeDecl, type_index: &TypeIndex) -> IrType {
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
        variants: type_decl
            .variants
            .iter()
            .map(|variant| lower_variant(variant, type_index))
            .collect(),
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

fn lower_variant(variant: &UnionVariant, type_index: &TypeIndex) -> IrVariant {
    IrVariant {
        name: variant.name.clone(),
        fields: type_index
            .records
            .get(&variant.name)
            .cloned()
            .unwrap_or_default(),
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
        isolated: function.isolated,
        params: function
            .params
            .iter()
            .map(|param| lower_param(param, &locals, context))
            .collect(),
        returns,
        body: lower_function_body(function, &locals, context),
    }
}

fn lower_function_body(
    function: &Function,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    let mut body = lower_statement_block(&function.body, locals, context, None);
    if let Some(trap) = &function.trap {
        let mut trap_locals = locals.clone();
        trap_locals.insert(trap.name.clone(), "Error".to_string());
        body.push(IrOp::Trap {
            name: trap.name.clone(),
            body: lower_statement_block(&trap.body, &trap_locals, context, Some(trap.name.as_str())),
        });
    }
    body
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
    next_temp_id: usize,
}

fn lower_statement(
    statement: &Statement,
    locals: &mut HashMap<String, String>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> Vec<IrOp> {
    match statement {
        Statement::Let {
            mutable,
            name,
            type_name,
            value,
            ..
        } => {
            let lowered_type = type_name.clone().unwrap_or_else(|| {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, context))
                    .expect("typecheck requires inferred binding type before IR lowering")
            });
            let lowered_value = value.as_ref().map(|value| {
                lower_expression_with_expected(value, Some(&lowered_type), locals, context)
            });
            locals.insert(name.clone(), lowered_type.clone());
            vec![IrOp::Bind {
                mutable: *mutable,
                name: name.clone(),
                type_: lowered_type,
                value: lowered_value,
            }]
        }
        Statement::Return { value, .. } => vec![IrOp::Return {
            value: value
                .as_ref()
                .map(|value| lower_expression(value, locals, context)),
        }],
        Statement::Fail { error, .. } => vec![IrOp::Fail {
            error: lower_expression(error, locals, context),
        }],
        Statement::Propagate { .. } => vec![IrOp::Fail {
            error: IrValue::Local(
                trap_name
                    .expect("typecheck requires PROPAGATE to appear only in trap bodies")
                    .to_string(),
            ),
        }],
        Statement::Assign { name, value, .. } => vec![IrOp::Assign {
            name: name.clone(),
            value: lower_expression_with_expected(
                value,
                locals.get(name).map(String::as_str),
                locals,
                context,
            ),
        }],
        Statement::Expression { expression, .. } => vec![IrOp::Eval {
            value: lower_expression(expression, locals, context),
        }],
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => vec![IrOp::If {
            condition: lower_expression(condition, locals, context),
            then_body: lower_statement_block(then_body, locals, context, trap_name),
            else_body: lower_statement_block(else_body, locals, context, trap_name),
        }],
        Statement::Match {
            expression, cases, ..
        } => {
            let matched_type = match_expression_type(expression, locals, context)
                .expect("typecheck requires MATCH scrutinee type before IR lowering");
            let matched_name = make_temp_local_name(context, "match");
            let mut ops = vec![IrOp::Bind {
                mutable: false,
                name: matched_name.clone(),
                type_: matched_type.clone(),
                    value: Some(lower_match_expression(
                        expression,
                        &matched_type,
                        locals,
                        context,
                )),
            }];
            let mut match_locals = locals.clone();
            match_locals.insert(matched_name.clone(), matched_type);
            let match_value = if match_locals[&matched_name].starts_with("Result OF ") {
                let match_flag_name = make_temp_local_name(context, "match_ok");
                ops.push(IrOp::Bind {
                    mutable: false,
                    name: match_flag_name.clone(),
                    type_: "Boolean".to_string(),
                    value: Some(IrValue::ResultIsOk {
                        value: Box::new(IrValue::Local(matched_name.clone())),
                    }),
                });
                match_locals.insert(match_flag_name.clone(), "Boolean".to_string());
                IrValue::Local(match_flag_name)
            } else {
                IrValue::Local(matched_name.clone())
            };
            ops.push(IrOp::Match {
                value: match_value,
                cases: cases
                    .iter()
                    .map(|case| {
                        lower_match_case(case, &matched_name, &match_locals, context, trap_name)
                    })
                    .collect(),
            });
            ops
        }
        Statement::For {
            name,
            start,
            end,
            step,
            body,
            ..
        } => {
            let start_type = expression_type(start, locals, context)
                .expect("typecheck requires FOR start type before IR lowering");
            let end_type = expression_type(end, locals, context)
                .expect("typecheck requires FOR end type before IR lowering");
            let step_type = step
                .as_ref()
                .and_then(|value| expression_type(value, locals, context))
                .unwrap_or_else(|| "Integer".to_string());
            let loop_type = promote_loop_numeric_type_name(&start_type, &end_type, &step_type);
            let iter_name = make_temp_local_name(context, "for_iter");
            let end_name = make_temp_local_name(context, "for_end");
            let step_name = make_temp_local_name(context, "for_step");

            let start_value = lower_expression_with_expected(start, Some(&loop_type), locals, context);
            let end_value = lower_expression_with_expected(end, Some(&loop_type), locals, context);
            let step_value = step
                .as_ref()
                .map(|value| lower_expression_with_expected(value, Some(&loop_type), locals, context))
                .unwrap_or_else(|| numeric_constant_for_type(&loop_type, "1"));

            locals.insert(iter_name.clone(), loop_type.clone());
            locals.insert(end_name.clone(), loop_type.clone());
            locals.insert(step_name.clone(), loop_type.clone());

            let step_local = IrValue::Local(step_name.clone());
            let iter_local = IrValue::Local(iter_name.clone());
            let end_local = IrValue::Local(end_name.clone());
            let zero = numeric_constant_for_type(&loop_type, "0");
            let condition = IrValue::Binary {
                op: "OR".to_string(),
                left: Box::new(IrValue::Binary {
                    op: "AND".to_string(),
                    left: Box::new(IrValue::Binary {
                        op: ">=".to_string(),
                        left: Box::new(step_local.clone()),
                        right: Box::new(zero.clone()),
                    }),
                    right: Box::new(IrValue::Binary {
                        op: "<=".to_string(),
                        left: Box::new(iter_local.clone()),
                        right: Box::new(end_local.clone()),
                    }),
                }),
                right: Box::new(IrValue::Binary {
                    op: "AND".to_string(),
                    left: Box::new(IrValue::Binary {
                        op: "<".to_string(),
                        left: Box::new(step_local.clone()),
                        right: Box::new(zero),
                    }),
                    right: Box::new(IrValue::Binary {
                        op: ">=".to_string(),
                        left: Box::new(iter_local.clone()),
                        right: Box::new(end_local.clone()),
                    }),
                }),
            };

            let mut nested = locals.clone();
            nested.insert(name.clone(), loop_type.clone());
            let mut while_body = vec![IrOp::Bind {
                mutable: false,
                name: name.clone(),
                type_: loop_type.clone(),
                value: Some(iter_local.clone()),
            }];
            while_body.extend(lower_statement_block(body, &nested, context, trap_name));
            while_body.push(IrOp::Assign {
                name: iter_name.clone(),
                value: IrValue::Binary {
                    op: "+".to_string(),
                    left: Box::new(iter_local),
                    right: Box::new(step_local),
                },
            });

            vec![
                IrOp::Bind {
                    mutable: true,
                    name: iter_name,
                    type_: loop_type.clone(),
                    value: Some(start_value),
                },
                IrOp::Bind {
                    mutable: false,
                    name: end_name,
                    type_: loop_type.clone(),
                    value: Some(end_value),
                },
                IrOp::Bind {
                    mutable: false,
                    name: step_name,
                    type_: loop_type,
                    value: Some(step_value),
                },
                IrOp::While {
                    condition,
                    body: while_body,
                },
            ]
        }
        Statement::ForEach {
            name,
            iterable,
            body,
            ..
        } => {
            let iterable_type = expression_type(iterable, locals, context)
                .expect("typecheck requires FOR EACH iterable type before IR lowering");
            let element_type = collection_iteration_type(&iterable_type)
                .expect("typecheck requires FOR EACH collection before IR lowering");
            let mut nested = locals.clone();
            nested.insert(name.clone(), element_type.clone());
            vec![IrOp::ForEach {
                name: name.clone(),
                type_: element_type,
                iterable: lower_expression(iterable, locals, context),
                body: lower_statement_block(body, &nested, context, trap_name),
            }]
        }
        Statement::While {
            condition, body, ..
        } => vec![IrOp::While {
            condition: lower_expression(condition, locals, context),
            body: lower_statement_block(body, locals, context, trap_name),
        }],
        Statement::DoUntil {
            body, condition, ..
        } => {
            let body_ops = lower_statement_block(body, locals, context, trap_name);
            let loop_body = lower_statement_block(body, locals, context, trap_name);
            vec![
                body_ops,
                vec![IrOp::While {
                    condition: IrValue::Unary {
                        op: "NOT".to_string(),
                        operand: Box::new(lower_expression(condition, locals, context)),
                    },
                    body: loop_body,
                }],
            ]
            .into_iter()
            .flatten()
            .collect()
        }
        Statement::Using {
            name, value, body, ..
        } => {
            let type_ = expression_type(value, locals, context)
                .expect("typecheck requires inferred USING resource type before IR lowering");
            let value = lower_expression(value, locals, context);
            let close = builtins::resource_close_function(&type_)
                .expect("typecheck requires USING close function before IR lowering")
                .to_string();
            let mut nested = locals.clone();
            nested.insert(name.clone(), type_.clone());
            vec![IrOp::Using {
                name: name.clone(),
                type_,
                close,
                value,
                body: lower_statement_block(body, &nested, context, trap_name),
            }]
        }
    }
}

fn lower_statement_block(
    body: &[Statement],
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> Vec<IrOp> {
    let mut nested = locals.clone();
    body.iter()
        .flat_map(|statement| lower_statement(statement, &mut nested, context, trap_name))
        .collect()
}

fn collection_iteration_type(type_: &str) -> Option<String> {
    type_
        .strip_prefix("List OF ")
        .map(str::to_string)
        .or_else(|| {
            parse_map_type(type_).map(|(key, value)| format!("MapEntry OF {key} TO {value}"))
        })
}

fn make_temp_local_name(context: &mut LowerContext<'_>, prefix: &str) -> String {
    let name = format!("${prefix}{}", context.next_temp_id);
    context.next_temp_id += 1;
    name
}

fn promote_loop_numeric_type_name(start: &str, end: &str, step: &str) -> String {
    let first = numeric_binary_result_type("+", start, end);
    numeric_binary_result_type("+", first, step).to_string()
}

fn numeric_constant_for_type(type_: &str, value: &str) -> IrValue {
    IrValue::Const {
        type_: type_.to_string(),
        value: value.to_string(),
    }
}

fn parse_map_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn parse_map_entry_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("MapEntry OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn lower_match_case(
    case: &MatchCase,
    matched_local: &str,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
    trap_name: Option<&str>,
) -> IrMatchCase {
    let matched_type = locals
        .get(matched_local)
        .cloned()
        .expect("typecheck requires MATCH local type before IR lowering");
    let pattern = match &case.pattern {
        MatchPattern::Else => IrMatchPattern::Else,
        MatchPattern::Literal(expression) => {
            IrMatchPattern::Value(lower_expression(expression, locals, context))
        }
        MatchPattern::Union { type_name, .. } if matched_type.starts_with("Result OF ") => {
            let matched = match type_name.as_str() {
                "Ok" => "true",
                "Error" => "false",
                _ => "false",
            };
            IrMatchPattern::Value(IrValue::Const {
                type_: "Boolean".to_string(),
                value: matched.to_string(),
            })
        }
        MatchPattern::Union { type_name, .. } => {
            IrMatchPattern::Value(IrValue::Local(type_name.clone()))
        }
        MatchPattern::OneOf(expressions) => IrMatchPattern::OneOf(
            expressions
                .iter()
                .map(|expression| lower_expression(expression, locals, context))
                .collect(),
        ),
    };
    let mut case_locals = locals.clone();
    let mut body = Vec::new();
    if let Some((binding, binding_type, value)) =
        match_case_binding(&case.pattern, matched_local, &matched_type)
    {
        case_locals.insert(binding.clone(), binding_type.clone());
        body.push(IrOp::Bind {
            mutable: false,
            name: binding,
            type_: binding_type,
            value: Some(value),
        });
    }
    body.extend(lower_statement_block(&case.body, &case_locals, context, trap_name));
    IrMatchCase {
        pattern,
        guard: case
            .guard
            .as_ref()
            .map(|guard| lower_expression(guard, &case_locals, context)),
        body,
    }
}

fn match_case_binding(
    pattern: &MatchPattern,
    matched_local: &str,
    matched_type: &str,
) -> Option<(String, String, IrValue)> {
    match pattern {
        MatchPattern::Union { type_name, binding } => {
            if let Some(success) = matched_type.strip_prefix("Result OF ") {
                return match type_name.as_str() {
                    "Ok" => Some((
                        binding.clone(),
                        success.to_string(),
                        IrValue::ResultValue {
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    )),
                    "Error" => Some((
                        binding.clone(),
                        "Error".to_string(),
                        IrValue::ResultError {
                            value: Box::new(IrValue::Local(matched_local.to_string())),
                        },
                    )),
                    _ => None,
                };
            }
            Some((
                binding.clone(),
                type_name.clone(),
                IrValue::UnionExtract {
                    type_: type_name.clone(),
                    value: Box::new(IrValue::Local(matched_local.to_string())),
                },
            ))
        }
        _ => None,
    }
}

fn lower_match_expression(
    expression: &Expression,
    matched_type: &str,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    if matched_type.starts_with("Result OF ") {
        if let Expression::Call { callee, arguments } = expression {
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
                    .enumerate()
                    .map(|(index, argument)| {
                        let expected =
                            call_argument_expected_type(callee, index, arguments, locals, context);
                        lower_expression_with_expected(
                            argument,
                            expected.as_deref(),
                            locals,
                            context,
                        )
                    })
                    .collect()
            };
            return IrValue::CallResult {
                target: builtins::json::implementation_name(callee)
                    .unwrap_or(callee)
                    .to_string(),
                args,
            };
        }
    }
    lower_expression_with_expected(expression, Some(matched_type), locals, context)
}

fn match_expression_type(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    if let Expression::Call { callee, .. } = expression {
        return builtins::call_return_type_name(callee)
            .map(|success| format!("Result OF {success}"))
            .or_else(|| {
                context
                    .function_returns
                    .get(callee)
                    .cloned()
                    .map(|success| format!("Result OF {success}"))
            })
            .or_else(|| {
                locals
                    .get(callee)
                    .and_then(|type_| function_return_from_type(type_))
                    .map(|success| format!("Result OF {success}"))
            });
    }
    expression_type(expression, locals, context)
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
                    format!(
                        "{}FUNC({params}) AS {returns}",
                        if function.isolated { "ISOLATED " } else { "" }
                    ),
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
        Expression::Identifier(value) if builtins::math::is_math_constant(value) => {
            builtins::math::constant_type_name(value).map(str::to_string)
        }
        Expression::Identifier(value) => locals
            .get(value)
            .cloned()
            .or_else(|| context.function_types.get(value).cloned()),
        Expression::Constructor { type_name, .. } => {
            context.type_index.constructor_result(type_name)
        }
        Expression::WithUpdate { target, .. } => expression_type(target, locals, context),
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
            if member == "result" {
                if let Some(output) = builtins::thread::thread_output(&target_type) {
                    return Some(format!("Result OF {output}"));
                }
            }
            if target_type == "Error" {
                return match member.as_str() {
                    "code" => Some("Integer".to_string()),
                    "message" => Some("String".to_string()),
                    _ => None,
                };
            }
            if let Some((key_type, value_type)) = parse_map_entry_type(&target_type) {
                return match member.as_str() {
                    "key" => Some(key_type),
                    "value" => Some(value_type),
                    _ => None,
                };
            }
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
            if builtins::math::is_math_call(callee) {
                let arg_types = arguments
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::math::resolve_call(callee, &arg_types)
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
            if builtins::json::is_json_call(callee) {
                let arg_types = arguments
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::json::resolve_call(callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::thread::is_thread_call(callee) {
                let arg_types = arguments
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::thread::resolve_call(callee, &arg_types)
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
            Some(numeric_binary_result_type(operator, &left, &right).to_string())
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
        .or_else(|| type_.strip_prefix("ISOLATED FUNC("))
        .and_then(|rest| rest.split_once(") AS "))
        .map(|(_, return_type)| return_type.to_string())
}

fn function_param_types_from_type(type_: &str) -> Option<Vec<String>> {
    let rest = type_
        .strip_prefix("FUNC(")
        .or_else(|| type_.strip_prefix("ISOLATED FUNC("))?;
    let (params, _) = rest.split_once(") AS ")?;
    if params.trim().is_empty() {
        return Some(Vec::new());
    }
    Some(params.split(", ").map(str::to_string).collect())
}

fn call_argument_expected_type(
    callee: &str,
    index: usize,
    arguments: &[Expression],
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    if callee == "toString" && index == 1 && arguments.len() == 2 {
        return Some("Byte".to_string());
    }
    if let Some(params) = builtin_argument_types(callee) {
        return params.get(index).cloned();
    }
    context
        .function_types
        .get(callee)
        .or_else(|| locals.get(callee))
        .and_then(|type_| function_param_types_from_type(type_))
        .and_then(|params| params.get(index).cloned())
}

fn builtin_argument_types(callee: &str) -> Option<Vec<String>> {
    let expected = builtins::general::expected_arguments(callee)
        .or_else(|| builtins::strings::expected_arguments(callee))
        .or_else(|| builtins::math::expected_arguments(callee))
        .or_else(|| builtins::fs::expected_arguments(callee))
        .or_else(|| builtins::io::expected_arguments(callee))
        .or_else(|| builtins::json::expected_arguments(callee))
        .or_else(|| builtins::thread::expected_arguments(callee))?;
    let params = expected.split(", ").map(str::to_string).collect::<Vec<_>>();
    if params.iter().any(|param| uses_generic_placeholder(param)) {
        return None;
    }
    Some(params)
}

fn uses_generic_placeholder(type_: &str) -> bool {
    matches!(type_, "T" | "K" | "V")
        || type_.contains(" OF T")
        || type_.contains(" OF K")
        || type_.contains(" OF V")
        || type_.contains(" TO T")
        || type_.contains(" TO K")
        || type_.contains(" TO V")
}

fn lower_expression(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    lower_expression_with_expected(expression, None, locals, context)
}

fn lower_expression_with_expected(
    expression: &Expression,
    expected: Option<&str>,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrValue {
    match expression {
        Expression::String(value) => IrValue::Const {
            type_: "String".to_string(),
            value: value.clone(),
        },
        Expression::Number(value) => IrValue::Const {
            type_: if expected == Some("Fixed") {
                "Fixed".to_string()
            } else if expected == Some("Byte") {
                "Byte".to_string()
            } else if value.contains('.') {
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
        Expression::Identifier(value) if builtins::math::is_math_constant(value) => {
            let type_ = builtins::math::constant_type_name(value)
                .unwrap_or("Unknown")
                .to_string();
            let value = builtins::math::constant_value(value)
                .expect("recognized math constant has a value")
                .to_string();
            IrValue::Const { type_, value }
        }
        Expression::Identifier(value) => {
            let base = if let Some(type_) = context.function_types.get(value) {
                IrValue::FunctionRef {
                    name: value.clone(),
                    type_: type_.clone(),
                }
            } else {
                IrValue::Local(value.clone())
            };
            wrap_union_value(base, expression, expected, locals, context)
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
                    .enumerate()
                    .map(|(index, argument)| {
                        let expected =
                            call_argument_expected_type(callee, index, arguments, locals, context);
                        lower_expression_with_expected(
                            argument,
                            expected.as_deref(),
                            locals,
                            context,
                        )
                    })
                    .collect()
            };
            IrValue::Call {
                target: builtins::json::implementation_name(callee)
                    .unwrap_or(callee)
                    .to_string(),
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
                isolated: false,
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
        } => {
            let fields = context
                .type_index
                .records
                .get(type_name)
                .or_else(|| context.type_index.variant_fields.get(type_name));
            let base = IrValue::Constructor {
                type_: type_name.clone(),
                args: lower_constructor_args(arguments, fields, locals, context),
            };
            wrap_union_value(base, expression, expected, locals, context)
        }
        Expression::WithUpdate { target, updates } => {
            let type_ = expression_type(target, locals, context)
                .expect("typecheck requires WITH target type before IR lowering");
            IrValue::WithUpdate {
                type_: type_,
                target: Box::new(lower_expression(target, locals, context)),
                updates: updates
                    .iter()
                    .map(|update| IrRecordUpdate {
                        field: update.field.clone(),
                        value: lower_expression(&update.value, locals, context),
                    })
                    .collect(),
            }
        }
        Expression::ListLiteral(values) => {
            let expected_element = expected.and_then(|type_| type_.strip_prefix("List OF "));
            let lowered = values
                .iter()
                .map(|value| {
                    lower_expression_with_expected(value, expected_element, locals, context)
                })
                .collect::<Vec<_>>();
            let element_type = expected_element.map(str::to_string).unwrap_or_else(|| {
                values
                    .first()
                    .and_then(|value| literal_expression_type(value))
                    .unwrap_or_else(|| "Unknown".to_string())
            });
            IrValue::ListLiteral {
                type_: format!("List OF {element_type}"),
                values: lowered,
            }
        }
        Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        } => {
            let expected_map = expected.and_then(parse_map_type);
            let expected_key = expected_map.as_ref().map(|(key, _)| key.as_str());
            let expected_value = expected_map.as_ref().map(|(_, value)| value.as_str());
            IrValue::MapLiteral {
                type_: format!("Map OF {key_type} TO {value_type}"),
                entries: entries
                    .iter()
                    .map(|(key, value)| {
                        (
                            lower_expression_with_expected(key, expected_key, locals, context),
                            lower_expression_with_expected(
                                value,
                                expected_value,
                                locals,
                                context,
                            ),
                        )
                    })
                    .collect(),
            }
        }
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

fn wrap_union_value(
    base: IrValue,
    expression: &Expression,
    expected: Option<&str>,
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> IrValue {
    let Some(union_type) = expected else {
        return base;
    };
    let Some(actual_type) = expression_type(expression, locals, context) else {
        return base;
    };
    if context.type_index.variants.get(&actual_type) == Some(&union_type.to_string()) {
        return IrValue::UnionWrap {
            union_type: union_type.to_string(),
            member_type: actual_type,
            value: Box::new(base),
        };
    }
    base
}

fn lower_constructor_args(
    arguments: &[ConstructorArg],
    fields: Option<&Vec<IrField>>,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrValue> {
    let Some(fields) = fields else {
        return arguments
            .iter()
            .map(|argument| lower_expression(constructor_arg_value(argument), locals, context))
            .collect();
    };
    if arguments
        .iter()
        .all(|argument| matches!(argument, ConstructorArg::Named { .. }))
    {
        return fields
            .iter()
            .filter_map(|field| {
                arguments.iter().find_map(|argument| match argument {
                    ConstructorArg::Named { name, value, .. } if name == &field.name => Some(
                        lower_expression_with_expected(value, Some(&field.type_), locals, context),
                    ),
                    _ => None,
                })
            })
            .collect();
    }
    arguments
        .iter()
        .enumerate()
        .map(|(index, argument)| {
            let expected = fields.get(index).map(|field| field.type_.as_str());
            lower_expression_with_expected(
                constructor_arg_value(argument),
                expected,
                locals,
                context,
            )
        })
        .collect()
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}

fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
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
    variant_fields: HashMap<String, Vec<IrField>>,
}

impl TypeIndex {
    fn new(ast: &AstProject) -> Self {
        let mut records = HashMap::new();
        let mut enums = HashMap::new();
        let mut variants = HashMap::new();
        let mut variant_fields = HashMap::new();
        let union_decls = ast
            .files
            .iter()
            .flat_map(|file| &file.items)
            .filter_map(|item| {
                let Item::Type(type_decl) = item else {
                    return None;
                };
                if matches!(type_decl.kind, TypeDeclKind::Union) {
                    Some((type_decl.name.clone(), type_decl))
                } else {
                    None
                }
            })
            .collect::<HashMap<_, _>>();
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
                        for variant in expanded_union_variants(type_decl, &union_decls) {
                            variants.insert(variant.name.clone(), type_decl.name.clone());
                            variant_fields.insert(
                                variant.name.clone(),
                                records.get(&variant.name).cloned().unwrap_or_default(),
                            );
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
            variant_fields,
        }
    }

    fn constructor_result(&self, name: &str) -> Option<String> {
        if name == "Error" {
            Some("Error".to_string())
        } else if name == "Ok" {
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
            .get(type_name)
            .or_else(|| self.variant_fields.get(type_name))?
            .iter()
            .find(|field| field.name == member)
            .map(|field| field.type_.clone())
    }
}

fn expanded_union_variants<'a>(
    type_decl: &'a TypeDecl,
    union_decls: &HashMap<String, &'a TypeDecl>,
) -> Vec<&'a UnionVariant> {
    let mut variants = Vec::new();
    for include in &type_decl.includes {
        if let Some(included) = union_decls.get(include) {
            variants.extend(expanded_union_variants(included, union_decls));
        }
    }
    variants.extend(type_decl.variants.iter());
    variants
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
            IrOp::Fail { error } => {
                format!(
                    "\n{}{{ \"op\": \"fail\", \"error\": {} }}",
                    pad,
                    error.to_json(indent)
                )
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
            IrOp::While { condition, body } => {
                format!(
                    concat!(
                        "\n{}{{\n",
                        "{}  \"op\": \"while\",\n",
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
                )
            }
            IrOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                format!(
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
                )
            }
            IrOp::Using {
                name,
                type_,
                close,
                value,
                body,
            } => {
                format!(
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
                )
            }
            IrOp::Trap { name, body } => {
                format!(
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
            IrMatchPattern::OneOf(values) => format!(
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
            IrValue::CallResult { target, args } => {
                let args = args
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"callResult\", \"target\": {}, \"args\": [{}] }}",
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
            IrValue::UnionWrap {
                union_type,
                member_type,
                value,
            } => format!(
                "{{ \"kind\": \"unionWrap\", \"union\": {}, \"member\": {}, \"value\": {} }}",
                json_string(union_type),
                json_string(member_type),
                value.to_json(0)
            ),
            IrValue::UnionExtract { type_, value } => format!(
                "{{ \"kind\": \"unionExtract\", \"type\": {}, \"value\": {} }}",
                json_string(type_),
                value.to_json(0)
            ),
            IrValue::ResultIsOk { value } => format!(
                "{{ \"kind\": \"resultIsOk\", \"value\": {} }}",
                value.to_json(0)
            ),
            IrValue::ResultValue { value } => format!(
                "{{ \"kind\": \"resultValue\", \"value\": {} }}",
                value.to_json(0)
            ),
            IrValue::ResultError { value } => format!(
                "{{ \"kind\": \"resultError\", \"value\": {} }}",
                value.to_json(0)
            ),
            IrValue::WithUpdate {
                type_,
                target,
                updates,
            } => {
                let updates = updates
                    .iter()
                    .map(|update| update.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"with\", \"type\": {}, \"target\": {}, \"updates\": [{}] }}",
                    json_string(type_),
                    target.to_json(0),
                    updates
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

impl ToIrJson for IrRecordUpdate {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"field\": {}, \"value\": {} }}",
            json_string(&self.field),
            self.value.to_json(0)
        )
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
