use crate::ast::{
    AstProject, CallArg, ConstructorArg, EnumMember, ExitTarget, Expression, Function,
    FunctionKind, Item, LoopKind, MatchCase, MatchPattern, Param, Statement, TypeDecl,
    TypeDeclKind, TypeField, UnionVariant, Visibility,
};
use crate::builtins;
use crate::json_string;
use crate::numeric;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
#[derive(Clone)]

pub struct IrProject {
    pub(crate) name: String,
    pub(crate) entry: Option<EntryPoint>,
    pub(crate) bindings: Vec<IrBinding>,
    pub(crate) types: Vec<IrType>,
    pub(crate) functions: Vec<IrFunction>,
}

#[derive(Clone)]
pub(crate) struct EntryPoint {
    pub(crate) name: String,
    pub(crate) returns: String,
    pub(crate) accepts_args: bool,
}
#[derive(Clone)]

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
pub(crate) struct IrBinding {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) mutable: bool,
    pub(crate) type_: String,
    pub(crate) value: Option<IrValue>,
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
#[derive(Clone)]

pub(crate) struct IrEnumMember {
    pub(crate) name: String,
}
#[derive(Clone)]

pub(crate) struct IrFunction {
    pub(crate) name: String,
    pub(crate) visibility: String,
    pub(crate) kind: String,
    pub(crate) isolated: bool,
    pub(crate) params: Vec<IrParam>,
    pub(crate) returns: String,
    pub(crate) body: Vec<IrOp>,
    // Source file (project-relative path) this function was lowered from. Used to
    // build `ErrorLoc.filename` for errors that originate inside this function.
    pub(crate) file: String,
}
#[derive(Clone)]

pub(crate) struct IrParam {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) default: Option<IrValue>,
}
#[derive(Clone)]

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
    AssignGlobal {
        name: String,
        value: IrValue,
    },
    /// Replace the `STATE` payload of a `RES` binding (`resource.state = value`).
    StateAssign {
        resource: String,
        value: IrValue,
    },
    Return {
        value: Option<IrValue>,
    },
    ExitLoop {
        kind: LoopKind,
    },
    ContinueLoop {
        kind: LoopKind,
    },
    ExitProgram {
        code: IrValue,
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
        kind: LoopKind,
        condition: IrValue,
        body: Vec<IrOp>,
    },
    For {
        name: String,
        type_: String,
        start: IrValue,
        end: IrValue,
        step: IrValue,
        body: Vec<IrOp>,
        // Source location of the loop header; origin for increment overflow.
        loc: IrSourceLoc,
    },
    DoUntil {
        body: Vec<IrOp>,
        condition: IrValue,
    },
    ForEach {
        name: String,
        type_: String,
        iterable: IrValue,
        body: Vec<IrOp>,
    },
    Trap {
        name: String,
        body: Vec<IrOp>,
    },
}
#[derive(Clone)]

pub(crate) struct IrMatchCase {
    pub(crate) pattern: IrMatchPattern,
    pub(crate) guard: Option<IrValue>,
    pub(crate) body: Vec<IrOp>,
}
#[derive(Clone)]

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
    Global(String),
    FunctionRef {
        name: String,
        type_: String,
    },
    Closure {
        name: String,
        type_: String,
        captures: Vec<IrValue>,
    },
    Capture {
        index: usize,
        type_: String,
    },
    Call {
        target: String,
        args: Vec<IrValue>,
        // Source location of the call expression (origin for helper-generated errors).
        loc: IrSourceLoc,
    },
    CallResult {
        target: String,
        args: Vec<IrValue>,
        // Source location of the call expression (origin for helper-generated errors).
        loc: IrSourceLoc,
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
        // Source location of the operator (origin for arithmetic-generated errors).
        loc: IrSourceLoc,
    },
    Unary {
        op: String,
        operand: Box<IrValue>,
        // Source location of the operator (origin for arithmetic-generated errors).
        loc: IrSourceLoc,
    },
}

/// Compile-time source location attached to error-originating IR nodes. The
/// `file` is carried per-function (see `IrFunction::file`); nodes only need the
/// line and column within that file.
#[derive(Clone, Copy, Default)]
pub(crate) struct IrSourceLoc {
    pub(crate) line: u32,
    pub(crate) column: u32,
}

#[derive(Clone)]
pub(crate) struct IrRecordUpdate {
    pub(crate) field: String,
    pub(crate) value: IrValue,
}

#[derive(Clone)]
pub struct ExternalFunctionParam {
    pub name: String,
    pub type_: String,
}

pub fn lower_project_with_external_functions(
    ast: &AstProject,
    entry: Option<EntryPoint>,
    external_function_types: &HashMap<String, String>,
    external_function_params: &HashMap<String, Vec<ExternalFunctionParam>>,
) -> IrProject {
    let augmented =
        builtins::json::augmented_project(ast).expect("built-in json package source must parse");
    let ast = &augmented;
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let mut function_returns = function_returns(ast);
    let mut function_types = function_types(ast);
    let mut function_params = function_params(ast);
    let binding_types = declared_binding_types(ast);
    function_types.extend(external_function_types.clone());
    for (name, params) in external_function_params {
        function_params.insert(
            name.clone(),
            params
                .iter()
                .map(|param| CallParam {
                    name: param.name.clone(),
                    type_: param.type_.clone(),
                    default: None,
                })
                .collect(),
        );
    }
    for (name, type_) in external_function_types {
        if let Some(return_type) = function_return_from_type(type_) {
            function_returns.insert(name.clone(), return_type);
        }
    }
    let type_index = TypeIndex::new(ast);
    let mut context = LowerContext {
        function_returns: &function_returns,
        function_types: &function_types,
        function_params: &function_params,
        binding_types,
        type_index: &type_index,
        current_imports: HashMap::new(),
        current_file: String::new(),
        bindings: Vec::new(),
        lambdas: Vec::new(),
        next_lambda_id: 0,
        next_temp_id: 0,
        current_return_type: None,
        recover_targets: Vec::new(),
    };
    infer_binding_types(ast, &mut context);
    let bindings = lower_bindings(ast, &mut context);
    context.bindings = bindings.clone();

    for file in &ast.files {
        context.current_imports = file.import_bindings();
        context.current_file = file.path.clone();
        for item in &file.items {
            match item {
                Item::Binding(_) => {}
                Item::Function(function) => functions.push(lower_function(function, &mut context)),
                Item::Type(type_decl) => types.push(lower_type(type_decl, &type_index)),
            }
        }
    }
    functions.extend(context.lambdas);

    IrProject {
        name: ast.name.clone(),
        entry,
        bindings,
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

fn lower_binding(
    binding: &crate::ast::TopLevelBinding,
    context: &mut LowerContext<'_>,
) -> IrBinding {
    let locals = context.binding_types.clone();
    let type_ = binding.type_name.clone().unwrap_or_else(|| {
        binding
            .value
            .as_ref()
            .and_then(|value| expression_type(value, &locals, context))
            .expect("typecheck requires inferred binding type before IR lowering")
    });
    IrBinding {
        name: binding.name.clone(),
        visibility: visibility_name(binding.visibility).to_string(),
        mutable: binding.mutable,
        type_: type_.clone(),
        value: binding
            .value
            .as_ref()
            .map(|value| lower_expression_with_expected(value, Some(&type_), &locals, context)),
    }
}

fn lower_bindings(ast: &AstProject, context: &mut LowerContext<'_>) -> Vec<IrBinding> {
    let mut lowered = Vec::new();
    for file in &ast.files {
        context.current_imports = file.import_bindings();
        context.current_file = file.path.clone();
        for item in &file.items {
            if let Item::Binding(binding) = item {
                lowered.push(lower_binding(binding, context));
            }
        }
    }
    lowered
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
        let type_ = param
            .type_name
            .clone()
            .expect("typecheck requires parameter type before IR lowering");
        // Carry a `RES` parameter's `STATE T` in the local type string so
        // `s.state` resolves inside the callee, matching `lower_param`.
        let type_ = match &param.state_type {
            Some(state) => format!("{type_} STATE {state}"),
            None => type_,
        };
        locals.insert(param.name.clone(), type_);
    }
    let previous_return_type = context.current_return_type.take();
    context.current_return_type = Some(returns.clone());
    let body = lower_function_body(function, &locals, context);
    context.current_return_type = previous_return_type;

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
        body,
        file: context.current_file.clone(),
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
            body: lower_statement_block(
                &trap.body,
                &trap_locals,
                context,
                Some(trap.name.as_str()),
            ),
        });
    }
    body
}

fn lower_param(
    param: &Param,
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> IrParam {
    let type_ = param
        .type_name
        .clone()
        .expect("typecheck requires parameter type before IR lowering");
    // A `RES` parameter's `STATE T` rides in the type string so the callee can
    // address the borrowed resource's shared state payload.
    let type_ = match &param.state_type {
        Some(state) => format!("{type_} STATE {state}"),
        None => type_,
    };
    IrParam {
        name: param.name.clone(),
        type_,
        default: param
            .default
            .as_ref()
            .map(|value| lower_expression(value, locals, context)),
    }
}

struct LowerContext<'a> {
    function_returns: &'a HashMap<String, String>,
    function_types: &'a HashMap<String, String>,
    function_params: &'a HashMap<String, Vec<CallParam>>,
    binding_types: HashMap<String, String>,
    bindings: Vec<IrBinding>,
    type_index: &'a TypeIndex,
    current_imports: HashMap<String, String>,
    /// Project-relative path of the source file currently being lowered, used to
    /// populate `IrFunction::file` and `ErrorLoc.filename` for generated errors.
    current_file: String,
    lambdas: Vec<IrFunction>,
    next_lambda_id: usize,
    next_temp_id: usize,
    /// Declared return type of the function currently being lowered, used to
    /// implicitly wrap a `RETURN`ed member constructor into its union (so the
    /// wrap is explicit in the IR rather than re-derived during codegen).
    current_return_type: Option<String>,
    /// Stack of inline-`TRAP` recover destinations (innermost last). Each entry
    /// is the local slot a `RECOVER` value should be stored into and its type,
    /// or `None` when the trapped value is discarded (bare-statement form).
    recover_targets: Vec<RecoverTarget>,
}

#[derive(Clone)]
struct RecoverTarget {
    slot: Option<String>,
    type_: String,
}

#[derive(Clone)]
struct CallParam {
    name: String,
    type_: String,
    default: Option<Expression>,
}

#[derive(Clone)]
struct CapturedLocal {
    name: String,
    type_: String,
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
            state_type,
            ..
        } => {
            if let Some(Expression::Trapped {
                expression,
                binding,
                handler,
                ..
            }) = value
            {
                let success_type = type_name
                    .clone()
                    .or_else(|| expression_type(expression, locals, context))
                    .expect("typecheck requires inferred binding type before IR lowering");
                return lower_inline_trap(
                    expression,
                    binding,
                    handler,
                    InlineTrapTarget::Bind {
                        mutable: *mutable,
                        name: name.clone(),
                        type_: success_type,
                    },
                    locals,
                    context,
                );
            }
            let lowered_type = type_name.clone().unwrap_or_else(|| {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, context))
                    .expect("typecheck requires inferred binding type before IR lowering")
            });
            let lowered_value = value.as_ref().map(|value| {
                lower_expression_with_expected(value, Some(&lowered_type), locals, context)
            });
            // A `RES` binding's `STATE T` rides in the lowered type string
            // (`File STATE T`) so codegen can default-initialize and address the
            // state payload; the bare resource name is recovered for recognition.
            let lowered_type = match state_type {
                Some(state) => format!("{lowered_type} STATE {state}"),
                None => lowered_type,
            };
            locals.insert(name.clone(), lowered_type.clone());
            vec![IrOp::Bind {
                mutable: *mutable,
                name: name.clone(),
                type_: lowered_type,
                value: lowered_value,
            }]
        }
        Statement::Return { value, .. } => vec![IrOp::Return {
            value: value.as_ref().map(|value| {
                let base = lower_expression(value, locals, context);
                // Implicitly wrap a returned member constructor into the
                // function's declared union return type, so the wrap is explicit
                // in the IR (and faithfully serialized into Binary Representation) rather
                // than re-derived during native codegen.
                let expected = context.current_return_type.clone();
                wrap_union_value(base, value, expected.as_deref(), locals, context)
            }),
        }],
        Statement::Exit { target, code, .. } => match target {
            ExitTarget::For => vec![IrOp::ExitLoop {
                kind: LoopKind::For,
            }],
            ExitTarget::Do => vec![IrOp::ExitLoop { kind: LoopKind::Do }],
            ExitTarget::While => vec![IrOp::ExitLoop {
                kind: LoopKind::While,
            }],
            ExitTarget::Sub => vec![IrOp::Return { value: None }],
            ExitTarget::Func => Vec::new(),
            ExitTarget::Program => vec![IrOp::ExitProgram {
                code: lower_expression(
                    code.as_ref()
                        .expect("parser requires EXIT PROGRAM to include a code expression"),
                    locals,
                    context,
                ),
            }],
        },
        Statement::Continue { kind, .. } => vec![IrOp::ContinueLoop { kind: *kind }],
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
        Statement::Recover { value, .. } => {
            let target = context
                .recover_targets
                .last()
                .cloned()
                .expect("typecheck requires RECOVER to appear only in inline TRAP handlers");
            match (target.slot, value) {
                (Some(slot), Some(value)) => {
                    let lowered =
                        lower_expression_with_expected(value, Some(&target.type_), locals, context);
                    vec![IrOp::Assign {
                        name: slot,
                        value: lowered,
                    }]
                }
                (None, Some(value)) => vec![IrOp::Eval {
                    value: lower_expression_with_expected(
                        value,
                        Some(&target.type_),
                        locals,
                        context,
                    ),
                }],
                (_, None) => Vec::new(),
            }
        }
        Statement::Assign { name, value, .. } => {
            if let Expression::Trapped {
                expression,
                binding,
                handler,
                ..
            } = value
            {
                return lower_inline_trap(
                    expression,
                    binding,
                    handler,
                    InlineTrapTarget::Assign { name: name.clone() },
                    locals,
                    context,
                );
            }
            let expected = locals
                .get(name)
                .or_else(|| context.binding_types.get(name))
                .cloned();
            let lowered =
                lower_expression_with_expected(value, expected.as_deref(), locals, context);
            if locals.contains_key(name) {
                vec![IrOp::Assign {
                    name: name.clone(),
                    value: lowered,
                }]
            } else {
                vec![IrOp::AssignGlobal {
                    name: name.clone(),
                    value: lowered,
                }]
            }
        }
        Statement::StateAssign {
            resource, value, ..
        } => {
            let resource_type = locals
                .get(resource)
                .or_else(|| context.binding_types.get(resource))
                .cloned();
            let state_type = resource_type
                .as_deref()
                .and_then(crate::builtins::resource::state_type_name)
                .map(str::to_string);
            let lowered =
                lower_expression_with_expected(value, state_type.as_deref(), locals, context);
            vec![IrOp::StateAssign {
                resource: resource.clone(),
                value: lowered,
            }]
        }
        Statement::Expression { expression, .. } => {
            if let Expression::Trapped {
                expression: inner,
                binding,
                handler,
                ..
            } = expression
            {
                return lower_inline_trap(
                    inner,
                    binding,
                    handler,
                    InlineTrapTarget::Discard,
                    locals,
                    context,
                );
            }
            vec![IrOp::Eval {
                value: lower_expression(expression, locals, context),
            }]
        }
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
            line,
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

            let start_value =
                lower_expression_with_expected(start, Some(&loop_type), locals, context);
            let end_value = lower_expression_with_expected(end, Some(&loop_type), locals, context);
            let step_value = step
                .as_ref()
                .map(|value| {
                    lower_expression_with_expected(value, Some(&loop_type), locals, context)
                })
                .unwrap_or_else(|| numeric_constant_for_type(&loop_type, "1"));

            locals.insert(iter_name.clone(), loop_type.clone());
            locals.insert(end_name.clone(), loop_type.clone());
            locals.insert(step_name.clone(), loop_type.clone());

            let step_local = IrValue::Local(step_name.clone());
            let iter_local = IrValue::Local(iter_name.clone());
            let end_local = IrValue::Local(end_name.clone());

            let mut nested = locals.clone();
            nested.insert(name.clone(), loop_type.clone());
            let mut loop_body = vec![IrOp::Bind {
                mutable: false,
                name: name.clone(),
                type_: loop_type.clone(),
                value: Some(iter_local.clone()),
            }];
            loop_body.extend(lower_statement_block(body, &nested, context, trap_name));

            vec![
                IrOp::Bind {
                    mutable: false,
                    name: end_name,
                    type_: loop_type.clone(),
                    value: Some(end_value),
                },
                IrOp::Bind {
                    mutable: false,
                    name: step_name,
                    type_: loop_type.clone(),
                    value: Some(step_value),
                },
                IrOp::For {
                    name: iter_name,
                    type_: loop_type.clone(),
                    start: start_value,
                    end: end_local,
                    step: step_local,
                    body: loop_body,
                    loc: IrSourceLoc {
                        line: *line as u32,
                        column: 0,
                    },
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
            kind,
            condition,
            body,
            ..
        } => vec![IrOp::While {
            kind: *kind,
            condition: lower_expression(condition, locals, context),
            body: lower_statement_block(body, locals, context, trap_name),
        }],
        Statement::DoUntil {
            body, condition, ..
        } => vec![IrOp::DoUntil {
            body: lower_statement_block(body, locals, context, trap_name),
            condition: lower_expression(condition, locals, context),
        }],
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

/// Where the recovered/`Ok` value of an inline `TRAP` is delivered.
enum InlineTrapTarget {
    /// `LET`/`MUT name = <call> TRAP(e) …`
    Bind {
        mutable: bool,
        name: String,
        type_: String,
    },
    /// `name = <call> TRAP(e) …`
    Assign { name: String },
    /// `<call> TRAP(e) …` as a bare statement (value discarded).
    Discard,
}

/// Lowers an inline `TRAP` to existing IR primitives (no backend support is
/// required). The trapped call is evaluated as a raw `Result`; on `Ok` its value
/// flows to the target; on `Err` the handler runs with `e` bound. `RECOVER`
/// stores its value into a shared slot and then falls through to the delivery of
/// the target, while diverging handler paths (`RETURN`/`FAIL`/`PROPAGATE`) leave
/// as usual. The handler is normalized so that statements following a `RECOVER`
/// in a branch do not execute after recovery (see [`treeify_handler`]).
fn lower_inline_trap(
    inner: &Expression,
    binding: &str,
    handler: &[Statement],
    target: InlineTrapTarget,
    locals: &mut HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrOp> {
    let success_type = expression_type(inner, locals, context)
        .expect("typecheck requires inline TRAP expression type before IR lowering");
    let result_type = format!("Result OF {success_type}");
    let raw = lower_expression(inner, locals, context);
    let call_result = match raw {
        IrValue::Call { target, args, loc } => IrValue::CallResult { target, args, loc },
        other => other,
    };

    let res_name = make_temp_local_name(context, "trap_res");
    let mut ops = vec![IrOp::Bind {
        mutable: false,
        name: res_name.clone(),
        type_: result_type.clone(),
        value: Some(call_result),
    }];
    locals.insert(res_name.clone(), result_type);

    // A shared slot carries the value on both the Ok and RECOVER paths so the
    // target binding/assignment is produced exactly once after the branch.
    let slot = match &target {
        InlineTrapTarget::Bind { .. } | InlineTrapTarget::Assign { .. } => {
            let val_name = make_temp_local_name(context, "trap_val");
            ops.push(IrOp::Bind {
                mutable: true,
                name: val_name.clone(),
                type_: success_type.clone(),
                value: None,
            });
            locals.insert(val_name.clone(), success_type.clone());
            Some(val_name)
        }
        InlineTrapTarget::Discard => None,
    };

    let then_body = match &slot {
        Some(val_name) => vec![IrOp::Assign {
            name: val_name.clone(),
            value: IrValue::ResultValue {
                value: Box::new(IrValue::Local(res_name.clone())),
            },
        }],
        None => Vec::new(),
    };

    let mut handler_locals = locals.clone();
    handler_locals.insert(binding.to_string(), "Error".to_string());
    let mut else_body = vec![IrOp::Bind {
        mutable: false,
        name: binding.to_string(),
        type_: "Error".to_string(),
        value: Some(IrValue::ResultError {
            value: Box::new(IrValue::Local(res_name.clone())),
        }),
    }];
    context.recover_targets.push(RecoverTarget {
        slot: slot.clone(),
        type_: success_type.clone(),
    });
    let normalized = treeify_handler(handler);
    else_body.extend(lower_statement_block(
        &normalized,
        &handler_locals,
        context,
        Some(binding),
    ));
    context.recover_targets.pop();

    ops.push(IrOp::If {
        condition: IrValue::ResultIsOk {
            value: Box::new(IrValue::Local(res_name.clone())),
        },
        then_body,
        else_body,
    });

    match target {
        InlineTrapTarget::Bind {
            mutable,
            name,
            type_,
        } => {
            ops.push(IrOp::Bind {
                mutable,
                name: name.clone(),
                type_: type_.clone(),
                value: Some(IrValue::Local(slot.expect("bind target has a value slot"))),
            });
            locals.insert(name, type_);
        }
        InlineTrapTarget::Assign { name } => {
            let value = IrValue::Local(slot.expect("assign target has a value slot"));
            if locals.contains_key(&name) {
                ops.push(IrOp::Assign { name, value });
            } else {
                ops.push(IrOp::AssignGlobal { name, value });
            }
        }
        InlineTrapTarget::Discard => {}
    }

    ops
}

/// Normalizes an inline-`TRAP` handler so that a `RECOVER` (which is lowered as
/// an assignment that falls through to the post-trap continuation) never lets
/// statements that follow it in a sibling position execute. Statements after a
/// branching statement (`IF`/`MATCH`) whose branch falls through are pushed into
/// that fall-through branch, so each leaf path ends in its own terminator and
/// the structured lowering needs no jumps. Statements after a terminator are
/// unreachable and dropped.
fn treeify_handler(stmts: &[Statement]) -> Vec<Statement> {
    let Some((head, tail)) = stmts.split_first() else {
        return Vec::new();
    };

    if tail.is_empty() {
        return vec![treeify_statement(head)];
    }
    if statement_terminates(head) {
        // Anything after a terminator cannot run.
        return vec![treeify_statement(head)];
    }

    match head {
        Statement::If {
            condition,
            then_body,
            else_body,
            line,
        } => {
            let then_body = distribute_continuation(then_body, tail);
            let else_body = distribute_continuation(else_body, tail);
            vec![Statement::If {
                condition: condition.clone(),
                then_body,
                else_body,
                line: *line,
            }]
        }
        Statement::Match {
            expression,
            cases,
            line,
        } => {
            let mut new_cases: Vec<MatchCase> = cases
                .iter()
                .map(|case| MatchCase {
                    pattern: case.pattern.clone(),
                    guard: case.guard.clone(),
                    body: distribute_continuation(&case.body, tail),
                    line: case.line,
                })
                .collect();
            // An unmatched scrutinee falls through to the continuation, so make
            // that path explicit unless an ELSE arm already covers it.
            let has_else = cases
                .iter()
                .any(|case| matches!(case.pattern, MatchPattern::Else) && case.guard.is_none());
            if !has_else {
                new_cases.push(MatchCase {
                    pattern: MatchPattern::Else,
                    guard: None,
                    body: treeify_handler(tail),
                    line: *line,
                });
            }
            vec![Statement::Match {
                expression: expression.clone(),
                cases: new_cases,
                line: *line,
            }]
        }
        _ => {
            // A non-branching, non-terminating statement falls through to the
            // continuation; keep it and continue normalizing the tail.
            let mut result = vec![treeify_statement(head)];
            result.extend(treeify_handler(tail));
            result
        }
    }
}

/// Appends `continuation` to a block's fall-through paths, then normalizes it.
fn distribute_continuation(body: &[Statement], continuation: &[Statement]) -> Vec<Statement> {
    if block_terminates(body) {
        treeify_handler(body)
    } else {
        let mut combined = body.to_vec();
        combined.extend_from_slice(continuation);
        treeify_handler(&combined)
    }
}

/// Recurses into a statement's nested blocks without distributing any
/// continuation (used when there is nothing following the statement).
fn treeify_statement(statement: &Statement) -> Statement {
    match statement {
        Statement::If {
            condition,
            then_body,
            else_body,
            line,
        } => Statement::If {
            condition: condition.clone(),
            then_body: treeify_handler(then_body),
            else_body: treeify_handler(else_body),
            line: *line,
        },
        Statement::Match {
            expression,
            cases,
            line,
        } => Statement::Match {
            expression: expression.clone(),
            cases: cases
                .iter()
                .map(|case| MatchCase {
                    pattern: case.pattern.clone(),
                    guard: case.guard.clone(),
                    body: treeify_handler(&case.body),
                    line: case.line,
                })
                .collect(),
            line: *line,
        },
        Statement::While {
            kind,
            condition,
            body,
            line,
        } => Statement::While {
            kind: *kind,
            condition: condition.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        Statement::DoUntil {
            body,
            condition,
            line,
        } => Statement::DoUntil {
            body: treeify_handler(body),
            condition: condition.clone(),
            line: *line,
        },
        Statement::For {
            name,
            start,
            end,
            step,
            body,
            line,
        } => Statement::For {
            name: name.clone(),
            start: start.clone(),
            end: end.clone(),
            step: step.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        Statement::ForEach {
            name,
            iterable,
            body,
            line,
        } => Statement::ForEach {
            name: name.clone(),
            iterable: iterable.clone(),
            body: treeify_handler(body),
            line: *line,
        },
        other => other.clone(),
    }
}

/// Whether executing `stmts` always ends in a terminator (never reaches the end
/// of the block).
fn block_terminates(stmts: &[Statement]) -> bool {
    stmts.iter().any(statement_terminates)
}

/// Whether a statement always diverges or recovers (ends its enclosing handler
/// path). Mirrors the typecheck flow analysis for the constructs an inline-trap
/// handler may contain.
fn statement_terminates(statement: &Statement) -> bool {
    match statement {
        Statement::Return { .. }
        | Statement::Exit { .. }
        | Statement::Continue { .. }
        | Statement::Fail { .. }
        | Statement::Propagate { .. }
        | Statement::Recover { .. } => true,
        Statement::If {
            then_body,
            else_body,
            ..
        } => !else_body.is_empty() && block_terminates(then_body) && block_terminates(else_body),
        Statement::Match { cases, .. } => {
            let has_else = cases
                .iter()
                .any(|case| matches!(case.pattern, MatchPattern::Else) && case.guard.is_none());
            has_else && !cases.is_empty() && cases.iter().all(|case| block_terminates(&case.body))
        }
        _ => false,
    }
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
    body.extend(lower_statement_block(
        &case.body,
        &case_locals,
        context,
        trap_name,
    ));
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
    // A `MATCH` scrutinee that is a call auto-unwraps like any other call site
    // (local error handling now uses an inline `TRAP`), so the scrutinee lowers
    // to its ordinary value. A `Result`-typed *value* (a local or field) keeps
    // its `Result OF …` type and is matched with `CASE Ok`/`CASE Error`.
    lower_expression_with_expected(expression, Some(matched_type), locals, context)
}

fn match_expression_type(
    expression: &Expression,
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    // Call scrutinees auto-unwrap; only a value already of `Result` type keeps
    // its `Result OF …` shape for `CASE Ok`/`CASE Error` matching.
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

fn function_params(ast: &AstProject) -> HashMap<String, Vec<CallParam>> {
    let mut params = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Function(function) = item {
                params.insert(
                    function.name.clone(),
                    function
                        .params
                        .iter()
                        .map(|param| CallParam {
                            name: param.name.clone(),
                            type_: param
                                .type_name
                                .clone()
                                .expect("typecheck requires parameter type before IR lowering"),
                            default: param.default.clone(),
                        })
                        .collect(),
                );
            }
        }
    }
    params
}

fn declared_binding_types(ast: &AstProject) -> HashMap<String, String> {
    let mut bindings = HashMap::new();
    for file in &ast.files {
        for item in &file.items {
            if let Item::Binding(binding) = item {
                let type_ = binding
                    .type_name
                    .clone()
                    .unwrap_or_else(|| "Unknown".to_string());
                bindings.insert(binding.name.clone(), type_);
            }
        }
    }
    bindings
}

fn infer_binding_types(ast: &AstProject, context: &mut LowerContext<'_>) {
    for file in &ast.files {
        context.current_imports = file.import_bindings();
        for item in &file.items {
            if let Item::Binding(binding) = item {
                if binding.type_name.is_some() {
                    continue;
                }
                if let Some(value) = &binding.value {
                    let locals = HashMap::new();
                    if let Some(type_) = expression_type(value, &locals, context) {
                        context.binding_types.insert(binding.name.clone(), type_);
                    }
                }
            }
        }
    }
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
        Expression::Identifier(value) => {
            let canonical_value = canonical_import_name(value, context);
            if builtins::math::is_math_constant(&canonical_value) {
                builtins::math::constant_type_name(&canonical_value).map(str::to_string)
            } else {
                locals
                    .get(value)
                    .cloned()
                    .or_else(|| context.binding_types.get(value).cloned())
                    .or_else(|| context.function_types.get(value).cloned())
                    .or_else(|| context.function_types.get(&canonical_value).cloned())
            }
        }
        Expression::Constructor { type_name, .. } => {
            let canonical_type_name = canonical_import_name(type_name, context);
            context
                .type_index
                .constructor_result(&canonical_type_name)
                .or_else(|| context.type_index.constructor_result(type_name))
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
            // `s.state` on a `RES` value yields its `STATE` record type, carried
            // in the resource type string (`File STATE FileState`).
            if member == "state" {
                if let Some(state) = crate::builtins::resource::state_type_name(&target_type) {
                    return Some(state.to_string());
                }
            }
            // `t.result` is removed; worker outcomes are retrieved only via
            // `thread::waitFor`. (Typecheck rejects `.result` before IR.)
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
        Expression::Call {
            callee, arguments, ..
        } => {
            let canonical_callee = canonical_import_name(callee, context);
            if builtins::general::is_general_call(&canonical_callee) {
                let normalized =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
                if callee == "filter" && normalized.len() == 2 {
                    if let Expression::Identifier(predicate) = normalized[1] {
                        if let Some(collection_type) =
                            expression_type(normalized[0], locals, context)
                        {
                            if let Some(predicate_type) = collection_type
                                .strip_prefix("List OF ")
                                .and_then(|element| {
                                    builtins::general::filter_predicate_type(predicate, element)
                                })
                            {
                                let arg_types = vec![collection_type, predicate_type];
                                return builtins::general::resolve_call(
                                    &canonical_callee,
                                    &arg_types,
                                )
                                .map(|resolved| resolved.return_type.to_string());
                            }
                        }
                    }
                }
                let arg_types = normalized
                    .iter()
                    .map(|argument| expression_type(argument, locals, context))
                    .collect::<Option<Vec<_>>>()?;
                return builtins::general::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::strings::is_strings_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::strings::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::math::is_math_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::math::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::fs::is_fs_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::fs::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::io::is_io_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::io::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::net::is_net_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::net::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::json::is_json_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::json::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            if builtins::thread::is_thread_call(&canonical_callee) {
                let arg_types =
                    normalize_builtin_call_arguments(canonical_callee.as_str(), arguments)
                        .iter()
                        .map(|argument| expression_type(argument, locals, context))
                        .collect::<Option<Vec<_>>>()?;
                return builtins::thread::resolve_call(&canonical_callee, &arg_types)
                    .map(|resolved| resolved.return_type.to_string());
            }
            builtins::call_return_type_name(&canonical_callee)
                .map(str::to_string)
                .or_else(|| context.function_returns.get(callee).cloned())
                .or_else(|| context.function_returns.get(&canonical_callee).cloned())
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
            ..
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
        Expression::Unary {
            operator, operand, ..
        } => {
            if operator == "NOT" {
                Some("Boolean".to_string())
            } else {
                expression_type(operand, locals, context)
            }
        }
        Expression::Trapped { expression, .. } => expression_type(expression, locals, context),
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

fn canonical_import_name(name: &str, context: &LowerContext<'_>) -> String {
    let Some((binding, rest)) = name.split_once('.') else {
        return name.to_string();
    };
    let Some(package) = context.current_imports.get(binding) else {
        return name.to_string();
    };
    format!("{package}.{rest}")
}

fn call_argument_expected_type(
    callee: &str,
    index: usize,
    arguments: &[CallArg],
    locals: &HashMap<String, String>,
    context: &LowerContext<'_>,
) -> Option<String> {
    let canonical_callee = canonical_import_name(callee, context);
    if callee == "toString" && index == 1 && arguments.len() == 2 {
        return Some("Byte".to_string());
    }
    if let Some(params) = builtin_argument_types(&canonical_callee) {
        return params.get(index).cloned();
    }
    context
        .function_params
        .get(callee)
        .or_else(|| context.function_params.get(&canonical_callee))
        .and_then(|params| params.get(index).map(|param| param.type_.clone()))
        .or_else(|| {
            locals
                .get(callee)
                .and_then(|type_| function_param_types_from_type(type_))
                .and_then(|params| params.get(index).cloned())
        })
}

fn builtin_argument_types(callee: &str) -> Option<Vec<String>> {
    let expected = builtins::general::expected_arguments(callee)
        .or_else(|| builtins::strings::expected_arguments(callee))
        .or_else(|| builtins::math::expected_arguments(callee))
        .or_else(|| builtins::fs::expected_arguments(callee))
        .or_else(|| builtins::io::expected_arguments(callee))
        .or_else(|| builtins::json::expected_arguments(callee))
        .or_else(|| builtins::net::argument_types(callee))
        .or_else(|| builtins::thread::expected_arguments(callee))?;
    let params = expected.split(", ").map(str::to_string).collect::<Vec<_>>();
    if params.iter().any(|param| uses_generic_placeholder(param)) {
        return None;
    }
    Some(params)
}

fn normalize_builtin_call_arguments<'a>(
    callee: &str,
    arguments: &'a [CallArg],
) -> Vec<&'a Expression> {
    if !arguments
        .iter()
        .any(|argument| matches!(argument, CallArg::Named { .. }))
    {
        return arguments.iter().map(call_arg_value).collect();
    }
    let Some(param_names) = builtins::call_param_names(callee) else {
        return arguments.iter().map(call_arg_value).collect();
    };
    let mut ordered = vec![None; param_names.len()];
    let mut next_positional = 0usize;
    let mut extras = Vec::new();
    for argument in arguments {
        match argument {
            CallArg::Positional(value) => {
                while next_positional < ordered.len() && ordered[next_positional].is_some() {
                    next_positional += 1;
                }
                if next_positional < ordered.len() {
                    ordered[next_positional] = Some(value);
                    next_positional += 1;
                } else {
                    extras.push(value);
                }
            }
            CallArg::Named { name, value, .. } => {
                if let Some(index) = param_names
                    .iter()
                    .position(|aliases| aliases.iter().any(|alias| alias == name))
                {
                    ordered[index] = Some(value);
                }
            }
        }
    }
    let mut normalized = ordered.into_iter().flatten().collect::<Vec<_>>();
    normalized.extend(extras);
    normalized
}

fn normalize_local_call_arguments<'a>(
    callee: &str,
    arguments: &'a [CallArg],
    context: &LowerContext<'_>,
) -> Vec<Option<&'a Expression>> {
    let Some(params) = context.function_params.get(callee) else {
        return arguments
            .iter()
            .map(|argument| Some(call_arg_value(argument)))
            .collect();
    };
    let mut ordered = vec![None; params.len()];
    let mut next_positional = 0usize;
    for argument in arguments {
        match argument {
            CallArg::Positional(value) => {
                while next_positional < ordered.len() && ordered[next_positional].is_some() {
                    next_positional += 1;
                }
                if next_positional < ordered.len() {
                    ordered[next_positional] = Some(value);
                    next_positional += 1;
                }
            }
            CallArg::Named { name, value, .. } => {
                if let Some(index) = params.iter().position(|param| param.name == *name) {
                    ordered[index] = Some(value);
                }
            }
        }
    }
    ordered
}

fn lower_local_call_arguments(
    callee: &str,
    arguments: &[CallArg],
    locals: &HashMap<String, String>,
    context: &mut LowerContext<'_>,
) -> Vec<IrValue> {
    let canonical_callee = canonical_import_name(callee, context);
    let params = context
        .function_params
        .get(callee)
        .or_else(|| context.function_params.get(&canonical_callee))
        .expect("local call lowering requires known function parameters");
    normalize_local_call_arguments(callee, arguments, context)
        .into_iter()
        .enumerate()
        .filter_map(|(index, argument)| {
            let expected = call_argument_expected_type(callee, index, arguments, locals, context);
            match argument {
                Some(argument) => Some(lower_expression_with_expected(
                    argument,
                    expected.as_deref(),
                    locals,
                    context,
                )),
                None => params.get(index).and_then(|param| {
                    param.default.as_ref().map(|default| {
                        lower_expression_with_expected(default, Some(&param.type_), locals, context)
                    })
                }),
            }
        })
        .collect()
}

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
    }
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
        Expression::Identifier(value) => {
            let canonical_value = canonical_import_name(value, context);
            if builtins::math::is_math_constant(&canonical_value) {
                let type_ = builtins::math::constant_type_name(&canonical_value)
                    .unwrap_or("Unknown")
                    .to_string();
                let value = builtins::math::constant_value(&canonical_value)
                    .expect("recognized math constant has a value")
                    .to_string();
                return IrValue::Const { type_, value };
            }

            let base = if locals.contains_key(value) {
                IrValue::Local(value.clone())
            } else if let Some(type_) = context
                .function_types
                .get(value)
                .or_else(|| context.function_types.get(&canonical_value))
            {
                IrValue::FunctionRef {
                    name: canonical_value,
                    type_: type_.clone(),
                }
            } else if context.binding_types.contains_key(value) {
                IrValue::Global(value.clone())
            } else {
                IrValue::Local(value.clone())
            };
            wrap_union_value(base, expression, expected, locals, context)
        }
        Expression::Call {
            callee,
            arguments,
            line,
            column,
        } => {
            let canonical_callee = canonical_import_name(callee, context);
            let loc = IrSourceLoc {
                line: *line as u32,
                column: *column as u32,
            };
            // `error(code, message)` is a language built-in that produces a
            // read-only `Error` record stamped with the source location of this
            // call expression. Lower it to ordinary record constructors so the
            // rest of the pipeline treats `Error`/`ErrorLoc` as plain records.
            if canonical_callee == "error"
                && !context.function_params.contains_key(callee)
                && !context.function_params.contains_key(&canonical_callee)
            {
                let mut lowered = arguments
                    .iter()
                    .map(|argument| lower_expression(call_arg_value(argument), locals, context));
                let code = lowered
                    .next()
                    .expect("typecheck requires error() code argument before IR lowering");
                let message = lowered
                    .next()
                    .expect("typecheck requires error() message argument before IR lowering");
                return build_error_value(code, message, &context.current_file, loc);
            }
            let normalized_builtin =
                normalize_builtin_call_arguments(canonical_callee.as_str(), arguments);
            let args = if callee == "filter" && normalized_builtin.len() == 2 {
                if let Expression::Identifier(predicate) = normalized_builtin[1] {
                    let predicate_type = expression_type(normalized_builtin[0], locals, context)
                        .and_then(|collection_type| {
                            collection_type
                                .strip_prefix("List OF ")
                                .and_then(|element| {
                                    builtins::general::filter_predicate_type(predicate, element)
                                })
                        });
                    if let Some(predicate_type) = predicate_type {
                        vec![
                            lower_expression(normalized_builtin[0], locals, context),
                            IrValue::FunctionRef {
                                name: predicate.clone(),
                                type_: predicate_type,
                            },
                        ]
                    } else {
                        normalized_builtin
                            .iter()
                            .map(|argument| lower_expression(argument, locals, context))
                            .collect()
                    }
                } else {
                    normalized_builtin
                        .iter()
                        .map(|argument| lower_expression(argument, locals, context))
                        .collect()
                }
            } else if context.function_params.contains_key(callee)
                || context.function_params.contains_key(&canonical_callee)
            {
                lower_local_call_arguments(callee, arguments, locals, context)
            } else {
                normalized_builtin
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
                target: builtins::json::implementation_name(&canonical_callee)
                    .unwrap_or(&canonical_callee)
                    .to_string(),
                args,
                loc,
            }
        }
        Expression::Lambda { params, body } => {
            let name = format!("$lambda{}", context.next_lambda_id);
            context.next_lambda_id += 1;
            let param_names = params
                .iter()
                .map(|param| param.name.clone())
                .collect::<HashSet<_>>();
            let captures = captured_locals(body, locals, &param_names);
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
            let mut body_ops = captures
                .iter()
                .enumerate()
                .map(|(index, capture)| IrOp::Bind {
                    mutable: false,
                    name: capture.name.clone(),
                    type_: capture.type_.clone(),
                    value: Some(IrValue::Capture {
                        index,
                        type_: capture.type_.clone(),
                    }),
                })
                .collect::<Vec<_>>();
            for capture in &captures {
                lambda_locals.insert(capture.name.clone(), capture.type_.clone());
            }
            let returns = expression_type(body, &lambda_locals, context)
                .expect("typecheck requires lambda return type before IR lowering");
            let value = lower_expression(body, &lambda_locals, context);
            body_ops.push(IrOp::Return { value: Some(value) });
            context.lambdas.push(IrFunction {
                name: name.clone(),
                visibility: "private".to_string(),
                kind: "func".to_string(),
                isolated: false,
                params: ir_params,
                returns: returns.clone(),
                body: body_ops,
                file: context.current_file.clone(),
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
            let type_ = format!("FUNC({params}) AS {returns}");
            if captures.is_empty() {
                IrValue::FunctionRef { name, type_ }
            } else {
                IrValue::Closure {
                    name,
                    type_,
                    captures: captures
                        .iter()
                        .map(|capture| {
                            lower_expression(
                                &Expression::Identifier(capture.name.clone()),
                                locals,
                                context,
                            )
                        })
                        .collect(),
                }
            }
        }
        Expression::Constructor {
            type_name,
            arguments,
        } => {
            let canonical_type_name = canonical_import_name(type_name, context);
            let fields = context
                .type_index
                .records
                .get(&canonical_type_name)
                .or_else(|| context.type_index.records.get(type_name))
                .or_else(|| context.type_index.variant_fields.get(&canonical_type_name))
                .or_else(|| context.type_index.variant_fields.get(type_name));
            let base = IrValue::Constructor {
                type_: canonical_type_name,
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
                            lower_expression_with_expected(value, expected_value, locals, context),
                        )
                    })
                    .collect(),
            }
        }
        Expression::MemberAccess { target, member } => IrValue::MemberAccess {
            target: Box::new(lower_expression(target, locals, context)),
            member: member.clone(),
        },
        Expression::Trapped { .. } => {
            // Inline traps are only constructed as the value of a binding,
            // assignment, or bare-expression statement, where `lower_statement`
            // desugars them directly; they never reach value lowering.
            unreachable!("inline TRAP must be lowered as a statement value")
        }
        Expression::Binary {
            left,
            operator,
            right,
            line,
            column,
        } => IrValue::Binary {
            op: operator.clone(),
            left: Box::new(lower_expression(left, locals, context)),
            right: Box::new(lower_expression(right, locals, context)),
            loc: IrSourceLoc {
                line: *line as u32,
                column: *column as u32,
            },
        },
        Expression::Unary {
            operator,
            operand,
            line,
            column,
        } => IrValue::Unary {
            op: operator.clone(),
            operand: Box::new(lower_expression(operand, locals, context)),
            loc: IrSourceLoc {
                line: *line as u32,
                column: *column as u32,
            },
        },
    }
}

/// Build an `ErrorLoc` record value for a compile-time source location.
fn error_loc_value(file: &str, loc: IrSourceLoc) -> IrValue {
    IrValue::Constructor {
        type_: "ErrorLoc".to_string(),
        args: vec![
            IrValue::Const {
                type_: "String".to_string(),
                value: file.to_string(),
            },
            IrValue::Const {
                type_: "Integer".to_string(),
                value: loc.line.to_string(),
            },
            IrValue::Const {
                type_: "Integer".to_string(),
                value: loc.column.to_string(),
            },
        ],
    }
}

/// Build an `Error` record value (code, message, source) for `error(...)`.
fn build_error_value(code: IrValue, message: IrValue, file: &str, loc: IrSourceLoc) -> IrValue {
    IrValue::Constructor {
        type_: "Error".to_string(),
        args: vec![code, message, error_loc_value(file, loc)],
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
    if context
        .type_index
        .variant_belongs_to_union(&actual_type, union_type)
    {
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

fn captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, String>,
    local_names: &HashSet<String>,
) -> Vec<CapturedLocal> {
    let mut captures = Vec::new();
    let mut seen = HashSet::new();
    collect_captured_locals(
        expression,
        outer_locals,
        local_names,
        &mut seen,
        &mut captures,
    );
    captures
}

fn collect_captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, String>,
    local_names: &HashSet<String>,
    seen: &mut HashSet<String>,
    captures: &mut Vec<CapturedLocal>,
) {
    match expression {
        Expression::Identifier(name) => {
            if let Some(type_) = outer_locals.get(name) {
                if !local_names.contains(name) && seen.insert(name.clone()) {
                    captures.push(CapturedLocal {
                        name: name.clone(),
                        type_: type_.clone(),
                    });
                }
            }
        }
        Expression::Call {
            callee, arguments, ..
        } => {
            if let Some(type_) = outer_locals.get(callee) {
                if !local_names.contains(callee) && seen.insert(callee.clone()) {
                    captures.push(CapturedLocal {
                        name: callee.clone(),
                        type_: type_.clone(),
                    });
                }
            }
            for argument in arguments {
                collect_captured_locals(
                    call_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        Expression::Lambda { .. } => {}
        Expression::Binary { left, right, .. } => {
            collect_captured_locals(left, outer_locals, local_names, seen, captures);
            collect_captured_locals(right, outer_locals, local_names, seen, captures);
        }
        Expression::Unary { operand, .. } => {
            collect_captured_locals(operand, outer_locals, local_names, seen, captures);
        }
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                collect_captured_locals(
                    constructor_arg_value(argument),
                    outer_locals,
                    local_names,
                    seen,
                    captures,
                );
            }
        }
        Expression::ListLiteral(values) => {
            for value in values {
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_captured_locals(key, outer_locals, local_names, seen, captures);
                collect_captured_locals(value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::MemberAccess { target, .. } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
        }
        Expression::WithUpdate { target, updates } => {
            collect_captured_locals(target, outer_locals, local_names, seen, captures);
            for update in updates {
                collect_captured_locals(&update.value, outer_locals, local_names, seen, captures);
            }
        }
        Expression::Trapped { expression, .. } => {
            collect_captured_locals(expression, outer_locals, local_names, seen, captures);
        }
        Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
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
    variant_unions: HashMap<String, HashSet<String>>,
    variant_fields: HashMap<String, Vec<IrField>>,
}

impl TypeIndex {
    fn new(ast: &AstProject) -> Self {
        let mut records = HashMap::new();
        let mut enums = HashMap::new();
        let mut variants = HashMap::new();
        let mut variant_unions = HashMap::<String, HashSet<String>>::new();
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
                            variants
                                .entry(variant.name.clone())
                                .or_insert_with(|| type_decl.name.clone());
                            variant_unions
                                .entry(variant.name.clone())
                                .or_default()
                                .insert(type_decl.name.clone());
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
            variant_unions,
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
            .or_else(|| builtins::net::builtin_type_fields(type_name))
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

    fn variant_belongs_to_union(&self, variant_name: &str, union_name: &str) -> bool {
        self.variant_unions
            .get(variant_name)
            .is_some_and(|unions| unions.contains(union_name))
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
        let bindings = if self.bindings.is_empty() {
            String::new()
        } else {
            format!("  \"bindings\": [{}\n  ],\n", join_json(&self.bindings, 2))
        };
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-ir\",\n",
                "  \"version\": 1,\n",
                "  \"project\": {},\n",
                "  \"entry\": {},\n",
                "{}",
                "  \"types\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.name),
            self.entry
                .as_ref()
                .map(|entry| entry.to_json(2))
                .unwrap_or_else(|| "null".to_string()),
            bindings,
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

impl ToIrJson for IrBinding {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let value = self
            .value
            .as_ref()
            .map(|value| value.to_json(indent))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"name\": {}, \"visibility\": {}, \"mutable\": {}, \"type\": {}, \"value\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.visibility),
            self.mutable,
            json_string(&self.type_),
            value
        )
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
            IrOp::AssignGlobal { name, value } => {
                format!(
                    "\n{}{{ \"op\": \"assignGlobal\", \"name\": {}, \"value\": {} }}",
                    pad,
                    json_string(name),
                    value.to_json(indent)
                )
            }
            IrOp::Return { value } => {
                let value = value
                    .as_ref()
                    .map(|value| value.to_json(indent))
                    .unwrap_or_else(|| "null".to_string());
                format!("\n{}{{ \"op\": \"return\", \"value\": {} }}", pad, value)
            }
            IrOp::ExitLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"exitLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            IrOp::ContinueLoop { kind } => {
                format!(
                    "\n{}{{ \"op\": \"continueLoop\", \"loop\": {} }}",
                    pad,
                    json_string(loop_kind_name(*kind))
                )
            }
            IrOp::ExitProgram { code } => {
                format!(
                    "\n{}{{ \"op\": \"exitProgram\", \"code\": {} }}",
                    pad,
                    code.to_json(indent)
                )
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
            IrOp::StateAssign { resource, value } => {
                format!(
                    "\n{}{{ \"op\": \"stateAssign\", \"resource\": {}, \"value\": {} }}",
                    pad,
                    json_string(resource),
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
            IrOp::While {
                kind,
                condition,
                body,
            } => {
                format!(
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
                )
            }
            IrOp::For {
                name,
                type_,
                start,
                end,
                step,
                body,
                ..
            } => {
                format!(
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
                )
            }
            IrOp::DoUntil { body, condition } => {
                format!(
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
            IrValue::Global(name) => {
                format!(
                    "{{ \"kind\": \"global\", \"name\": {} }}",
                    json_string(name)
                )
            }
            IrValue::FunctionRef { name, type_ } => {
                format!(
                    "{{ \"kind\": \"functionRef\", \"name\": {}, \"type\": {} }}",
                    json_string(name),
                    json_string(type_)
                )
            }
            IrValue::Closure {
                name,
                type_,
                captures,
            } => {
                let captures = captures
                    .iter()
                    .map(|arg| arg.to_json(0))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{{ \"kind\": \"closure\", \"name\": {}, \"type\": {}, \"captures\": [{}] }}",
                    json_string(name),
                    json_string(type_),
                    captures
                )
            }
            IrValue::Capture { index, type_ } => {
                format!(
                    "{{ \"kind\": \"capture\", \"index\": {}, \"type\": {} }}",
                    index,
                    json_string(type_)
                )
            }
            IrValue::Call { target, args, .. } => {
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
            IrValue::CallResult { target, args, .. } => {
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
            IrValue::Binary {
                op, left, right, ..
            } => {
                format!(
                    "{{ \"kind\": \"binary\", \"op\": {}, \"left\": {}, \"right\": {} }}",
                    json_string(op),
                    left.to_json(0),
                    right.to_json(0)
                )
            }
            IrValue::Unary { op, operand, .. } => {
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

fn loop_kind_name(kind: LoopKind) -> &'static str {
    match kind {
        LoopKind::For => "for",
        LoopKind::Do => "do",
        LoopKind::While => "while",
    }
}

fn visibility_name(visibility: Visibility) -> &'static str {
    match visibility {
        Visibility::Private => "private",
        Visibility::Package => "package",
        Visibility::Export => "export",
    }
}

// ===========================================================================
// Binary Representation (structured) encode/decode
// ===========================================================================
//
// The package payload is a faithful, versioned binary serialization of the
// compiler's IR (`IrProject` / `IrFunction` / `IrOp` / `IrValue` / `IrType`).
// It is *not* a flat opcode stream: control flow stays nested (regions with
// explicit ends) and expressions stay as trees, exactly as in memory. The
// in-memory IR is free to change behind this format; this encoding is the
// stable contract.
//
// The format is self-contained: strings are inline length-prefixed, integers
// are little-endian. There is no separate interned pool here — the `.mfp`
// container's tables (manifest/ABI/import/export) are kept and derived
// alongside this payload by `binary_repr.rs`, but a function body is faithfully
// reconstructable from this payload alone.

/// Magic bytes prefixing a Binary Representation payload.
pub const BINARY_REPR_MAGIC: &[u8; 4] = b"MFBR";
/// Binary Representation format version. Bump on any incompatible change to the encoding.
/// Version 2 adds per-node source locations (`loc` on Call/CallResult/Binary/Unary/For)
/// and a per-function source `file`, backing read-only `Error.source` / `ErrorLoc`.
pub const BINARY_REPR_VERSION: u16 = 2;

// --- low-level writers -----------------------------------------------------

fn put_u8(out: &mut Vec<u8>, v: u8) {
    out.push(v);
}

fn put_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_le_bytes());
}

fn put_bool(out: &mut Vec<u8>, v: bool) {
    out.push(if v { 1 } else { 0 });
}

fn put_loop_kind(out: &mut Vec<u8>, kind: LoopKind) {
    put_u8(
        out,
        match kind {
            LoopKind::For => 0,
            LoopKind::Do => 1,
            LoopKind::While => 2,
        },
    );
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_u32(out, s.len() as u32);
    out.extend_from_slice(s.as_bytes());
}

fn put_opt_str(out: &mut Vec<u8>, s: &Option<String>) {
    match s {
        Some(v) => {
            put_u8(out, 1);
            put_str(out, v);
        }
        None => put_u8(out, 0),
    }
}

fn put_vec<T, F: Fn(&mut Vec<u8>, &T)>(out: &mut Vec<u8>, items: &[T], f: F) {
    put_u32(out, items.len() as u32);
    for item in items {
        f(out, item);
    }
}

fn put_opt_value(out: &mut Vec<u8>, value: &Option<IrValue>) {
    match value {
        Some(v) => {
            put_u8(out, 1);
            encode_value(out, v);
        }
        None => put_u8(out, 0),
    }
}

// --- low-level reader ------------------------------------------------------

struct IrReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> IrReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        IrReader { bytes, pos: 0 }
    }

    fn need(&self, n: usize) -> Result<(), String> {
        if self.pos + n > self.bytes.len() {
            Err(format!(
                "Binary Representation truncated: needed {n} bytes at offset {}, have {}",
                self.pos,
                self.bytes.len()
            ))
        } else {
            Ok(())
        }
    }

    fn u8(&mut self) -> Result<u8, String> {
        self.need(1)?;
        let v = self.bytes[self.pos];
        self.pos += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, String> {
        self.need(2)?;
        let v = u16::from_le_bytes([self.bytes[self.pos], self.bytes[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn u32(&mut self) -> Result<u32, String> {
        self.need(4)?;
        let v = u32::from_le_bytes([
            self.bytes[self.pos],
            self.bytes[self.pos + 1],
            self.bytes[self.pos + 2],
            self.bytes[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    fn bool(&mut self) -> Result<bool, String> {
        Ok(self.u8()? != 0)
    }

    fn string(&mut self) -> Result<String, String> {
        let len = self.u32()? as usize;
        self.need(len)?;
        let s = std::str::from_utf8(&self.bytes[self.pos..self.pos + len])
            .map_err(|err| format!("Binary Representation: invalid UTF-8 string: {err}"))?
            .to_string();
        self.pos += len;
        Ok(s)
    }

    fn opt_string(&mut self) -> Result<Option<String>, String> {
        if self.u8()? != 0 {
            Ok(Some(self.string()?))
        } else {
            Ok(None)
        }
    }

    fn count(&mut self) -> Result<usize, String> {
        Ok(self.u32()? as usize)
    }

    fn opt_value(&mut self) -> Result<Option<IrValue>, String> {
        if self.u8()? != 0 {
            Ok(Some(decode_value(self)?))
        } else {
            Ok(None)
        }
    }
}

// --- public entry points ---------------------------------------------------

/// Serialize an `IrProject` to the versioned Binary Representation byte format.
pub fn encode_binary_repr(project: &IrProject) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(BINARY_REPR_MAGIC);
    put_u16(&mut out, BINARY_REPR_VERSION);
    encode_project(&mut out, project);
    out
}

/// Decode a Binary Representation byte payload back into an `IrProject`.
pub fn decode_binary_repr(bytes: &[u8]) -> Result<IrProject, String> {
    let mut r = IrReader::new(bytes);
    r.need(4)?;
    if &bytes[0..4] != BINARY_REPR_MAGIC {
        return Err("Binary Representation: bad magic (expected MFBR)".to_string());
    }
    r.pos = 4;
    let version = r.u16()?;
    if version != BINARY_REPR_VERSION {
        return Err(format!(
            "Binary Representation version {version} unsupported (expected {BINARY_REPR_VERSION})"
        ));
    }
    decode_project(&mut r)
}

// --- IrProject -------------------------------------------------------------

fn encode_project(out: &mut Vec<u8>, project: &IrProject) {
    put_str(out, &project.name);
    match &project.entry {
        Some(entry) => {
            put_u8(out, 1);
            put_str(out, &entry.name);
            put_str(out, &entry.returns);
            put_bool(out, entry.accepts_args);
        }
        None => put_u8(out, 0),
    }
    put_vec(out, &project.bindings, encode_binding);
    put_vec(out, &project.types, encode_type);
    put_vec(out, &project.functions, encode_function);
}

fn decode_project(r: &mut IrReader) -> Result<IrProject, String> {
    let name = r.string()?;
    let entry = if r.u8()? != 0 {
        Some(EntryPoint {
            name: r.string()?,
            returns: r.string()?,
            accepts_args: r.bool()?,
        })
    } else {
        None
    };
    let bindings = decode_vec(r, decode_binding)?;
    let types = decode_vec(r, decode_type)?;
    let functions = decode_vec(r, decode_function)?;
    Ok(IrProject {
        name,
        entry,
        bindings,
        types,
        functions,
    })
}

fn decode_vec<T, F: Fn(&mut IrReader) -> Result<T, String>>(
    r: &mut IrReader,
    f: F,
) -> Result<Vec<T>, String> {
    let n = r.count()?;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(f(r)?);
    }
    Ok(out)
}

// --- IrBinding -------------------------------------------------------------

fn encode_binding(out: &mut Vec<u8>, b: &IrBinding) {
    put_str(out, &b.name);
    put_str(out, &b.visibility);
    put_bool(out, b.mutable);
    put_str(out, &b.type_);
    put_opt_value(out, &b.value);
}

fn decode_binding(r: &mut IrReader) -> Result<IrBinding, String> {
    Ok(IrBinding {
        name: r.string()?,
        visibility: r.string()?,
        mutable: r.bool()?,
        type_: r.string()?,
        value: r.opt_value()?,
    })
}

// --- IrType / IrField / IrVariant / IrEnumMember ---------------------------

fn encode_field(out: &mut Vec<u8>, f: &IrField) {
    put_opt_str(out, &f.visibility);
    put_str(out, &f.name);
    put_str(out, &f.type_);
}

fn decode_field(r: &mut IrReader) -> Result<IrField, String> {
    Ok(IrField {
        visibility: r.opt_string()?,
        name: r.string()?,
        type_: r.string()?,
    })
}

fn encode_variant(out: &mut Vec<u8>, v: &IrVariant) {
    put_str(out, &v.name);
    put_vec(out, &v.fields, encode_field);
}

fn decode_variant(r: &mut IrReader) -> Result<IrVariant, String> {
    Ok(IrVariant {
        name: r.string()?,
        fields: decode_vec(r, decode_field)?,
    })
}

fn encode_type(out: &mut Vec<u8>, t: &IrType) {
    put_str(out, &t.kind);
    put_str(out, &t.visibility);
    put_str(out, &t.name);
    put_vec(out, &t.fields, encode_field);
    put_vec(out, &t.includes, |o, s| put_str(o, s));
    put_vec(out, &t.variants, encode_variant);
    put_vec(out, &t.members, |o, m| put_str(o, &m.name));
}

fn decode_type(r: &mut IrReader) -> Result<IrType, String> {
    Ok(IrType {
        kind: r.string()?,
        visibility: r.string()?,
        name: r.string()?,
        fields: decode_vec(r, decode_field)?,
        includes: decode_vec(r, |r| r.string())?,
        variants: decode_vec(r, decode_variant)?,
        members: decode_vec(r, |r| Ok(IrEnumMember { name: r.string()? }))?,
    })
}

// --- IrFunction / IrParam --------------------------------------------------

fn encode_param(out: &mut Vec<u8>, p: &IrParam) {
    put_str(out, &p.name);
    put_str(out, &p.type_);
    put_opt_value(out, &p.default);
}

fn decode_param(r: &mut IrReader) -> Result<IrParam, String> {
    Ok(IrParam {
        name: r.string()?,
        type_: r.string()?,
        default: r.opt_value()?,
    })
}

fn encode_function(out: &mut Vec<u8>, f: &IrFunction) {
    put_str(out, &f.name);
    put_str(out, &f.visibility);
    put_str(out, &f.kind);
    put_bool(out, f.isolated);
    put_vec(out, &f.params, encode_param);
    put_str(out, &f.returns);
    put_vec(out, &f.body, encode_op);
    put_str(out, &f.file);
}

fn decode_function(r: &mut IrReader) -> Result<IrFunction, String> {
    Ok(IrFunction {
        name: r.string()?,
        visibility: r.string()?,
        kind: r.string()?,
        isolated: r.bool()?,
        params: decode_vec(r, decode_param)?,
        returns: r.string()?,
        body: decode_vec(r, decode_op)?,
        file: r.string()?,
    })
}

// --- IrOp ------------------------------------------------------------------

fn encode_op(out: &mut Vec<u8>, op: &IrOp) {
    match op {
        IrOp::Bind {
            mutable,
            name,
            type_,
            value,
        } => {
            put_u8(out, 0);
            put_bool(out, *mutable);
            put_str(out, name);
            put_str(out, type_);
            put_opt_value(out, value);
        }
        IrOp::Assign { name, value } => {
            put_u8(out, 1);
            put_str(out, name);
            encode_value(out, value);
        }
        IrOp::AssignGlobal { name, value } => {
            put_u8(out, 2);
            put_str(out, name);
            encode_value(out, value);
        }
        IrOp::StateAssign { resource, value } => {
            put_u8(out, 17);
            put_str(out, resource);
            encode_value(out, value);
        }
        IrOp::Return { value } => {
            put_u8(out, 3);
            put_opt_value(out, value);
        }
        IrOp::ExitLoop { kind } => {
            put_u8(out, 11);
            put_loop_kind(out, *kind);
        }
        IrOp::ContinueLoop { kind } => {
            put_u8(out, 12);
            put_loop_kind(out, *kind);
        }
        IrOp::ExitProgram { code } => {
            put_u8(out, 13);
            encode_value(out, code);
        }
        IrOp::Fail { error } => {
            put_u8(out, 4);
            encode_value(out, error);
        }
        IrOp::Eval { value } => {
            put_u8(out, 5);
            encode_value(out, value);
        }
        IrOp::If {
            condition,
            then_body,
            else_body,
        } => {
            put_u8(out, 6);
            encode_value(out, condition);
            put_vec(out, then_body, encode_op);
            put_vec(out, else_body, encode_op);
        }
        IrOp::Match { value, cases } => {
            put_u8(out, 7);
            encode_value(out, value);
            put_vec(out, cases, encode_match_case);
        }
        IrOp::While {
            kind,
            condition,
            body,
        } => {
            if matches!(kind, LoopKind::While) {
                put_u8(out, 8);
            } else {
                put_u8(out, 16);
                put_loop_kind(out, *kind);
            }
            encode_value(out, condition);
            put_vec(out, body, encode_op);
        }
        IrOp::For {
            name,
            type_,
            start,
            end,
            step,
            body,
            loc,
        } => {
            put_u8(out, 14);
            put_str(out, name);
            put_str(out, type_);
            encode_value(out, start);
            encode_value(out, end);
            encode_value(out, step);
            put_vec(out, body, encode_op);
            put_loc(out, *loc);
        }
        IrOp::DoUntil { body, condition } => {
            put_u8(out, 15);
            put_vec(out, body, encode_op);
            encode_value(out, condition);
        }
        IrOp::ForEach {
            name,
            type_,
            iterable,
            body,
        } => {
            put_u8(out, 9);
            put_str(out, name);
            put_str(out, type_);
            encode_value(out, iterable);
            put_vec(out, body, encode_op);
        }
        IrOp::Trap { name, body } => {
            put_u8(out, 10);
            put_str(out, name);
            put_vec(out, body, encode_op);
        }
    }
}

fn decode_op(r: &mut IrReader) -> Result<IrOp, String> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => IrOp::Bind {
            mutable: r.bool()?,
            name: r.string()?,
            type_: r.string()?,
            value: r.opt_value()?,
        },
        1 => IrOp::Assign {
            name: r.string()?,
            value: decode_value(r)?,
        },
        2 => IrOp::AssignGlobal {
            name: r.string()?,
            value: decode_value(r)?,
        },
        17 => IrOp::StateAssign {
            resource: r.string()?,
            value: decode_value(r)?,
        },
        3 => IrOp::Return {
            value: r.opt_value()?,
        },
        11 => IrOp::ExitLoop {
            kind: decode_loop_kind(r)?,
        },
        12 => IrOp::ContinueLoop {
            kind: decode_loop_kind(r)?,
        },
        13 => IrOp::ExitProgram {
            code: decode_value(r)?,
        },
        4 => IrOp::Fail {
            error: decode_value(r)?,
        },
        5 => IrOp::Eval {
            value: decode_value(r)?,
        },
        6 => IrOp::If {
            condition: decode_value(r)?,
            then_body: decode_vec(r, decode_op)?,
            else_body: decode_vec(r, decode_op)?,
        },
        7 => IrOp::Match {
            value: decode_value(r)?,
            cases: decode_vec(r, decode_match_case)?,
        },
        8 => IrOp::While {
            kind: LoopKind::While,
            condition: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
        },
        9 => IrOp::ForEach {
            name: r.string()?,
            type_: r.string()?,
            iterable: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
        },
        10 => IrOp::Trap {
            name: r.string()?,
            body: decode_vec(r, decode_op)?,
        },
        14 => IrOp::For {
            name: r.string()?,
            type_: r.string()?,
            start: decode_value(r)?,
            end: decode_value(r)?,
            step: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
            loc: get_loc(r)?,
        },
        15 => IrOp::DoUntil {
            body: decode_vec(r, decode_op)?,
            condition: decode_value(r)?,
        },
        16 => IrOp::While {
            kind: decode_loop_kind(r)?,
            condition: decode_value(r)?,
            body: decode_vec(r, decode_op)?,
        },
        other => return Err(format!("Binary Representation: unknown IrOp tag {other}")),
    })
}

fn decode_loop_kind(r: &mut IrReader) -> Result<LoopKind, String> {
    match r.u8()? {
        0 => Ok(LoopKind::For),
        1 => Ok(LoopKind::Do),
        2 => Ok(LoopKind::While),
        other => Err(format!(
            "Binary Representation: unknown loop kind tag {other}"
        )),
    }
}

// --- IrMatchCase / IrMatchPattern ------------------------------------------

fn encode_match_case(out: &mut Vec<u8>, c: &IrMatchCase) {
    encode_match_pattern(out, &c.pattern);
    put_opt_value(out, &c.guard);
    put_vec(out, &c.body, encode_op);
}

fn decode_match_case(r: &mut IrReader) -> Result<IrMatchCase, String> {
    Ok(IrMatchCase {
        pattern: decode_match_pattern(r)?,
        guard: r.opt_value()?,
        body: decode_vec(r, decode_op)?,
    })
}

fn encode_match_pattern(out: &mut Vec<u8>, p: &IrMatchPattern) {
    match p {
        IrMatchPattern::Else => put_u8(out, 0),
        IrMatchPattern::Value(v) => {
            put_u8(out, 1);
            encode_value(out, v);
        }
        IrMatchPattern::OneOf(vs) => {
            put_u8(out, 2);
            put_vec(out, vs, encode_value);
        }
    }
}

fn decode_match_pattern(r: &mut IrReader) -> Result<IrMatchPattern, String> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => IrMatchPattern::Else,
        1 => IrMatchPattern::Value(decode_value(r)?),
        2 => IrMatchPattern::OneOf(decode_vec(r, decode_value)?),
        other => {
            return Err(format!(
                "Binary Representation: unknown IrMatchPattern tag {other}"
            ))
        }
    })
}

// --- IrValue / IrRecordUpdate ----------------------------------------------

fn encode_value(out: &mut Vec<u8>, v: &IrValue) {
    match v {
        IrValue::Const { type_, value } => {
            put_u8(out, 0);
            put_str(out, type_);
            put_str(out, value);
        }
        IrValue::Local(name) => {
            put_u8(out, 1);
            put_str(out, name);
        }
        IrValue::Global(name) => {
            put_u8(out, 2);
            put_str(out, name);
        }
        IrValue::FunctionRef { name, type_ } => {
            put_u8(out, 3);
            put_str(out, name);
            put_str(out, type_);
        }
        IrValue::Closure {
            name,
            type_,
            captures,
        } => {
            put_u8(out, 4);
            put_str(out, name);
            put_str(out, type_);
            put_vec(out, captures, encode_value);
        }
        IrValue::Capture { index, type_ } => {
            put_u8(out, 5);
            put_u32(out, *index as u32);
            put_str(out, type_);
        }
        IrValue::Call { target, args, loc } => {
            put_u8(out, 6);
            put_str(out, target);
            put_vec(out, args, encode_value);
            put_loc(out, *loc);
        }
        IrValue::CallResult { target, args, loc } => {
            put_u8(out, 7);
            put_str(out, target);
            put_vec(out, args, encode_value);
            put_loc(out, *loc);
        }
        IrValue::Constructor { type_, args } => {
            put_u8(out, 8);
            put_str(out, type_);
            put_vec(out, args, encode_value);
        }
        IrValue::UnionWrap {
            union_type,
            member_type,
            value,
        } => {
            put_u8(out, 9);
            put_str(out, union_type);
            put_str(out, member_type);
            encode_value(out, value);
        }
        IrValue::UnionExtract { type_, value } => {
            put_u8(out, 10);
            put_str(out, type_);
            encode_value(out, value);
        }
        IrValue::ResultIsOk { value } => {
            put_u8(out, 11);
            encode_value(out, value);
        }
        IrValue::ResultValue { value } => {
            put_u8(out, 12);
            encode_value(out, value);
        }
        IrValue::ResultError { value } => {
            put_u8(out, 13);
            encode_value(out, value);
        }
        IrValue::WithUpdate {
            type_,
            target,
            updates,
        } => {
            put_u8(out, 14);
            put_str(out, type_);
            encode_value(out, target);
            put_vec(out, updates, |o, u| {
                put_str(o, &u.field);
                encode_value(o, &u.value);
            });
        }
        IrValue::ListLiteral { type_, values } => {
            put_u8(out, 15);
            put_str(out, type_);
            put_vec(out, values, encode_value);
        }
        IrValue::MapLiteral { type_, entries } => {
            put_u8(out, 16);
            put_str(out, type_);
            put_vec(out, entries, |o, (k, val)| {
                encode_value(o, k);
                encode_value(o, val);
            });
        }
        IrValue::MemberAccess { target, member } => {
            put_u8(out, 17);
            encode_value(out, target);
            put_str(out, member);
        }
        IrValue::Binary {
            op,
            left,
            right,
            loc,
        } => {
            put_u8(out, 18);
            put_str(out, op);
            encode_value(out, left);
            encode_value(out, right);
            put_loc(out, *loc);
        }
        IrValue::Unary { op, operand, loc } => {
            put_u8(out, 19);
            put_str(out, op);
            encode_value(out, operand);
            put_loc(out, *loc);
        }
    }
}

fn put_loc(out: &mut Vec<u8>, loc: IrSourceLoc) {
    put_u32(out, loc.line);
    put_u32(out, loc.column);
}

fn get_loc(r: &mut IrReader) -> Result<IrSourceLoc, String> {
    let line = r.u32()?;
    let column = r.u32()?;
    Ok(IrSourceLoc { line, column })
}

fn decode_value(r: &mut IrReader) -> Result<IrValue, String> {
    let tag = r.u8()?;
    Ok(match tag {
        0 => IrValue::Const {
            type_: r.string()?,
            value: r.string()?,
        },
        1 => IrValue::Local(r.string()?),
        2 => IrValue::Global(r.string()?),
        3 => IrValue::FunctionRef {
            name: r.string()?,
            type_: r.string()?,
        },
        4 => IrValue::Closure {
            name: r.string()?,
            type_: r.string()?,
            captures: decode_vec(r, decode_value)?,
        },
        5 => IrValue::Capture {
            index: r.u32()? as usize,
            type_: r.string()?,
        },
        6 => IrValue::Call {
            target: r.string()?,
            args: decode_vec(r, decode_value)?,
            loc: get_loc(r)?,
        },
        7 => IrValue::CallResult {
            target: r.string()?,
            args: decode_vec(r, decode_value)?,
            loc: get_loc(r)?,
        },
        8 => IrValue::Constructor {
            type_: r.string()?,
            args: decode_vec(r, decode_value)?,
        },
        9 => IrValue::UnionWrap {
            union_type: r.string()?,
            member_type: r.string()?,
            value: Box::new(decode_value(r)?),
        },
        10 => IrValue::UnionExtract {
            type_: r.string()?,
            value: Box::new(decode_value(r)?),
        },
        11 => IrValue::ResultIsOk {
            value: Box::new(decode_value(r)?),
        },
        12 => IrValue::ResultValue {
            value: Box::new(decode_value(r)?),
        },
        13 => IrValue::ResultError {
            value: Box::new(decode_value(r)?),
        },
        14 => IrValue::WithUpdate {
            type_: r.string()?,
            target: Box::new(decode_value(r)?),
            updates: decode_vec(r, |r| {
                Ok(IrRecordUpdate {
                    field: r.string()?,
                    value: decode_value(r)?,
                })
            })?,
        },
        15 => IrValue::ListLiteral {
            type_: r.string()?,
            values: decode_vec(r, decode_value)?,
        },
        16 => IrValue::MapLiteral {
            type_: r.string()?,
            entries: decode_vec(r, |r| {
                let k = decode_value(r)?;
                let v = decode_value(r)?;
                Ok((k, v))
            })?,
        },
        17 => IrValue::MemberAccess {
            target: Box::new(decode_value(r)?),
            member: r.string()?,
        },
        18 => IrValue::Binary {
            op: r.string()?,
            left: Box::new(decode_value(r)?),
            right: Box::new(decode_value(r)?),
            loc: get_loc(r)?,
        },
        19 => IrValue::Unary {
            op: r.string()?,
            operand: Box::new(decode_value(r)?),
            loc: get_loc(r)?,
        },
        other => {
            return Err(format!(
                "Binary Representation: unknown IrValue tag {other}"
            ))
        }
    })
}

// ===========================================================================
// Package IR merge
// ===========================================================================
//
// A consumer decodes each imported package's Binary Representation back to an `IrProject`
// and merges it into the project that flows through `IR -> NIR -> native`. To
// keep symbols unambiguous, a package's functions and globals are namespaced by
// the package name (`pkg.symbol`) — exactly how the consumer already names them
// (a `functionRef "thread_workers.echoText"` resolves to the merged function
// `thread_workers.echoText`). Imported *types* are referenced by their bare name
// by consumers, so type names stay unqualified and are de-duplicated by name.
// Every internal reference inside the package (sibling calls, function refs,
// global loads/stores) is rewritten to the namespaced form consistently.

/// Verify a freshly decoded package `IrProject` before it is merged into the
/// consuming project. The decoder already rejects a wrong magic/version
/// (`PACKAGE_BINARY_REPRESENTATION_VERSION_UNSUPPORTED`) and malformed bytes
/// (`PACKAGE_BINARY_REPRESENTATION_DECODE_FAILED`); this pass re-states the package-format
/// invariants at the IR level (the structured form makes them direct checks
/// rather than CFG reconstruction). Checks here are conservative — they must
/// never reject IR this compiler legitimately produced — and surface as
/// `PACKAGE_BINARY_REPRESENTATION_VERIFY_*` diagnostics.
pub fn verify_package(pir: &IrProject) -> Result<(), String> {
    // Structural well-formedness: names are non-empty and functions are unique
    // (the link-time identity prefix relies on a function appearing once).
    let mut seen_functions: HashSet<&str> = HashSet::new();
    for function in &pir.functions {
        if function.name.is_empty() {
            return Err(
                "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: package contains an unnamed function"
                    .to_string(),
            );
        }
        if !seen_functions.insert(function.name.as_str()) {
            return Err(format!(
                "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: duplicate function `{}` in package `{}`",
                function.name, pir.name
            ));
        }
    }
    let mut seen_types: HashSet<&str> = HashSet::new();
    for ty in &pir.types {
        if !seen_types.insert(ty.name.as_str()) {
            return Err(format!(
                "PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE: duplicate type `{}` in package `{}`",
                ty.name, pir.name
            ));
        }
    }
    // Control-flow / trap structure: every MATCH must carry at least one case
    // (an empty MATCH cannot be exhaustive), and is checked recursively.
    for function in &pir.functions {
        verify_ops(&function.body)?;
    }
    Ok(())
}

fn verify_ops(ops: &[IrOp]) -> Result<(), String> {
    for op in ops {
        match op {
            IrOp::If {
                then_body,
                else_body,
                ..
            } => {
                verify_ops(then_body)?;
                verify_ops(else_body)?;
            }
            IrOp::While { body, .. } | IrOp::ForEach { body, .. } | IrOp::Trap { body, .. } => {
                verify_ops(body)?
            }
            IrOp::Match { cases, .. } => {
                if cases.is_empty() {
                    return Err(
                        "PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH: MATCH has no cases (not exhaustive)".to_string(),
                    );
                }
                for case in cases {
                    verify_ops(&case.body)?;
                }
            }
            _ => {}
        }
    }
    Ok(())
}

/// Namespace a decoded package's own functions and globals by its deterministic
/// identity prefix `<id>.<package>` (see `binary_repr::package_identity_id`),
/// rewriting every internal reference to match. Types are left unqualified.
///
/// The `<id>` segment makes the prefix content-addressed: identical packages
/// reached via two dependency paths collapse to one copy at merge time, while
/// two distinct packages that share a name stay separate instead of colliding.
pub fn prefix_package_symbols(pir: &mut IrProject, id: &str) {
    let prefix = format!("{id}.{}", pir.name);
    let own_fns: HashSet<String> = pir.functions.iter().map(|f| f.name.clone()).collect();
    let own_globals: HashSet<String> = pir.bindings.iter().map(|b| b.name.clone()).collect();

    for function in &mut pir.functions {
        for op in &mut function.body {
            rewrite_op_targets(op, &own_fns, &own_globals, &prefix);
        }
        for param in &mut function.params {
            if let Some(default) = &mut param.default {
                rewrite_value_targets(default, &own_fns, &own_globals, &prefix);
            }
        }
        function.name = format!("{prefix}.{}", function.name);
    }
    for binding in &mut pir.bindings {
        if let Some(value) = &mut binding.value {
            rewrite_value_targets(value, &own_fns, &own_globals, &prefix);
        }
        binding.name = format!("{prefix}.{}", binding.name);
    }
    if let Some(entry) = &mut pir.entry {
        entry.name = format!("{prefix}.{}", entry.name);
    }
}

/// The package-qualified names (`package.symbol`) by which a *consumer* and
/// other packages reference this package's functions and globals. Computed
/// *before* `prefix_package_symbols` rewrites the definitions into their
/// identity-prefixed `<id>.package.symbol` form, so `apply_package_identity`
/// can rewrite those external references to match.
pub fn package_qualified_reference_names(pir: &IrProject) -> (HashSet<String>, HashSet<String>) {
    let pkg = &pir.name;
    let fns = pir
        .functions
        .iter()
        .map(|f| format!("{pkg}.{}", f.name))
        .collect();
    let globals = pir
        .bindings
        .iter()
        .map(|b| format!("{pkg}.{}", b.name))
        .collect();
    (fns, globals)
}

/// Rewrite every *external* reference to a package's symbols — from the
/// consumer and from other packages — from `package.symbol` to the
/// identity-prefixed `<id>.package.symbol` produced by `prefix_package_symbols`.
/// The package's own internal references are already identity-prefixed and so
/// are not in `fns`/`globals`; they are left untouched.
pub fn apply_package_identity(
    project: &mut IrProject,
    fns: &HashSet<String>,
    globals: &HashSet<String>,
    id: &str,
) {
    for function in &mut project.functions {
        for op in &mut function.body {
            rewrite_op_targets(op, fns, globals, id);
        }
        for param in &mut function.params {
            if let Some(default) = &mut param.default {
                rewrite_value_targets(default, fns, globals, id);
            }
        }
    }
    for binding in &mut project.bindings {
        if let Some(value) = &mut binding.value {
            rewrite_value_targets(value, fns, globals, id);
        }
    }
}

/// Merge a namespaced package `IrProject` into `project`. Functions and globals
/// are de-duplicated by their (already namespaced) name; types by bare name.
/// Call `prefix_package_symbols` on `package` first.
pub fn merge_package(project: &mut IrProject, package: IrProject) {
    for ty in package.types {
        if !project
            .types
            .iter()
            .any(|existing| existing.name == ty.name)
        {
            project.types.push(ty);
        }
    }
    for binding in package.bindings {
        if !project
            .bindings
            .iter()
            .any(|existing| existing.name == binding.name)
        {
            project.bindings.push(binding);
        }
    }
    for function in package.functions {
        if !project
            .functions
            .iter()
            .any(|existing| existing.name == function.name)
        {
            project.functions.push(function);
        }
    }
}

fn qualify_target(name: &mut String, pkg: &str) {
    *name = format!("{pkg}.{name}");
}

fn rewrite_op_targets(op: &mut IrOp, fns: &HashSet<String>, globals: &HashSet<String>, pkg: &str) {
    match op {
        IrOp::Bind { value, .. } => {
            if let Some(v) = value {
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrOp::Assign { value, .. }
        | IrOp::StateAssign { value, .. }
        | IrOp::Eval { value }
        | IrOp::Fail { error: value } => rewrite_value_targets(value, fns, globals, pkg),
        IrOp::AssignGlobal { name, value } => {
            if globals.contains(name) {
                qualify_target(name, pkg);
            }
            rewrite_value_targets(value, fns, globals, pkg);
        }
        IrOp::Return { value } => {
            if let Some(v) = value {
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
        IrOp::ExitProgram { code } => rewrite_value_targets(code, fns, globals, pkg),
        IrOp::If {
            condition,
            then_body,
            else_body,
        } => {
            rewrite_value_targets(condition, fns, globals, pkg);
            for op in then_body.iter_mut().chain(else_body.iter_mut()) {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::Match { value, cases } => {
            rewrite_value_targets(value, fns, globals, pkg);
            for case in cases {
                match &mut case.pattern {
                    IrMatchPattern::Else => {}
                    IrMatchPattern::Value(v) => rewrite_value_targets(v, fns, globals, pkg),
                    IrMatchPattern::OneOf(vs) => {
                        for v in vs {
                            rewrite_value_targets(v, fns, globals, pkg);
                        }
                    }
                }
                if let Some(guard) = &mut case.guard {
                    rewrite_value_targets(guard, fns, globals, pkg);
                }
                for op in &mut case.body {
                    rewrite_op_targets(op, fns, globals, pkg);
                }
            }
        }
        IrOp::While {
            condition, body, ..
        } => {
            rewrite_value_targets(condition, fns, globals, pkg);
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            rewrite_value_targets(start, fns, globals, pkg);
            rewrite_value_targets(end, fns, globals, pkg);
            rewrite_value_targets(step, fns, globals, pkg);
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::DoUntil { body, condition } => {
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
            rewrite_value_targets(condition, fns, globals, pkg);
        }
        IrOp::ForEach { iterable, body, .. } => {
            rewrite_value_targets(iterable, fns, globals, pkg);
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
        IrOp::Trap { body, .. } => {
            for op in body {
                rewrite_op_targets(op, fns, globals, pkg);
            }
        }
    }
}

fn rewrite_value_targets(
    value: &mut IrValue,
    fns: &HashSet<String>,
    globals: &HashSet<String>,
    pkg: &str,
) {
    match value {
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            if fns.contains(target) {
                qualify_target(target, pkg);
            }
            for arg in args {
                rewrite_value_targets(arg, fns, globals, pkg);
            }
        }
        IrValue::FunctionRef { name, .. } => {
            if fns.contains(name) {
                qualify_target(name, pkg);
            }
        }
        IrValue::Closure { name, captures, .. } => {
            if fns.contains(name) {
                qualify_target(name, pkg);
            }
            for capture in captures {
                rewrite_value_targets(capture, fns, globals, pkg);
            }
        }
        IrValue::Global(name) => {
            if globals.contains(name) {
                qualify_target(name, pkg);
            }
        }
        IrValue::Constructor { args, .. } => {
            for arg in args {
                rewrite_value_targets(arg, fns, globals, pkg);
            }
        }
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value }
        | IrValue::ResultError { value }
        | IrValue::Unary { operand: value, .. }
        | IrValue::MemberAccess { target: value, .. } => {
            rewrite_value_targets(value, fns, globals, pkg)
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            rewrite_value_targets(target, fns, globals, pkg);
            for update in updates {
                rewrite_value_targets(&mut update.value, fns, globals, pkg);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for v in values {
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (k, v) in entries {
                rewrite_value_targets(k, fns, globals, pkg);
                rewrite_value_targets(v, fns, globals, pkg);
            }
        }
        IrValue::Binary { left, right, .. } => {
            rewrite_value_targets(left, fns, globals, pkg);
            rewrite_value_targets(right, fns, globals, pkg);
        }
        IrValue::Const { .. } | IrValue::Local(_) | IrValue::Capture { .. } => {}
    }
}

#[cfg(test)]
mod binary_repr_tests {
    use super::*;

    fn sample_value() -> IrValue {
        IrValue::Binary {
            op: "+".to_string(),
            left: Box::new(IrValue::Const {
                type_: "Integer".to_string(),
                value: "1".to_string(),
            }),
            right: Box::new(IrValue::Unary {
                op: "-".to_string(),
                operand: Box::new(IrValue::Local("x".to_string())),
                loc: IrSourceLoc::default(),
            }),
            loc: IrSourceLoc::default(),
        }
    }

    // Build a project exercising every IrType, IrOp, IrValue, and IrMatchPattern kind.
    fn corpus_project() -> IrProject {
        let every_value = vec![
            IrValue::Const {
                type_: "String".to_string(),
                value: "hi".to_string(),
            },
            IrValue::Local("a".to_string()),
            IrValue::Global("g".to_string()),
            IrValue::FunctionRef {
                name: "f".to_string(),
                type_: "() -> Integer".to_string(),
            },
            IrValue::Closure {
                name: "lam".to_string(),
                type_: "() -> Integer".to_string(),
                captures: vec![IrValue::Local("a".to_string())],
            },
            IrValue::Capture {
                index: 3,
                type_: "Integer".to_string(),
            },
            IrValue::Call {
                target: "g".to_string(),
                args: vec![sample_value()],
                loc: IrSourceLoc::default(),
            },
            IrValue::CallResult {
                target: "toInt".to_string(),
                args: vec![IrValue::Local("s".to_string())],
                loc: IrSourceLoc::default(),
            },
            IrValue::Constructor {
                type_: "Point".to_string(),
                args: vec![sample_value(), sample_value()],
            },
            IrValue::UnionWrap {
                union_type: "Shape".to_string(),
                member_type: "Point".to_string(),
                value: Box::new(IrValue::Local("p".to_string())),
            },
            IrValue::UnionExtract {
                type_: "Point".to_string(),
                value: Box::new(IrValue::Local("s".to_string())),
            },
            IrValue::ResultIsOk {
                value: Box::new(IrValue::Local("r".to_string())),
            },
            IrValue::ResultValue {
                value: Box::new(IrValue::Local("r".to_string())),
            },
            IrValue::ResultError {
                value: Box::new(IrValue::Local("r".to_string())),
            },
            IrValue::WithUpdate {
                type_: "Point".to_string(),
                target: Box::new(IrValue::Local("p".to_string())),
                updates: vec![IrRecordUpdate {
                    field: "x".to_string(),
                    value: sample_value(),
                }],
            },
            IrValue::ListLiteral {
                type_: "List OF Integer".to_string(),
                values: vec![sample_value()],
            },
            IrValue::MapLiteral {
                type_: "Map OF String TO Integer".to_string(),
                entries: vec![(
                    IrValue::Const {
                        type_: "String".to_string(),
                        value: "k".to_string(),
                    },
                    sample_value(),
                )],
            },
            IrValue::MemberAccess {
                target: Box::new(IrValue::Local("p".to_string())),
                member: "x".to_string(),
            },
            sample_value(),
            IrValue::Unary {
                op: "NOT".to_string(),
                operand: Box::new(IrValue::Local("b".to_string())),
                loc: IrSourceLoc::default(),
            },
        ];

        let body = vec![
            IrOp::Bind {
                mutable: true,
                name: "a".to_string(),
                type_: "Integer".to_string(),
                value: Some(sample_value()),
            },
            IrOp::Assign {
                name: "a".to_string(),
                value: sample_value(),
            },
            IrOp::AssignGlobal {
                name: "g".to_string(),
                value: sample_value(),
            },
            IrOp::Eval {
                value: IrValue::Call {
                    target: "g".to_string(),
                    args: every_value.clone(),
                    loc: IrSourceLoc::default(),
                },
            },
            IrOp::If {
                condition: IrValue::Local("b".to_string()),
                then_body: vec![IrOp::Return {
                    value: Some(IrValue::Local("a".to_string())),
                }],
                else_body: vec![IrOp::Return { value: None }],
            },
            IrOp::While {
                kind: LoopKind::While,
                condition: IrValue::Local("b".to_string()),
                body: vec![IrOp::Eval {
                    value: IrValue::Local("a".to_string()),
                }],
            },
            IrOp::ForEach {
                name: "item".to_string(),
                type_: "Integer".to_string(),
                iterable: IrValue::Local("list".to_string()),
                body: vec![IrOp::Eval {
                    value: IrValue::Local("item".to_string()),
                }],
            },
            IrOp::Match {
                value: IrValue::Local("s".to_string()),
                cases: vec![
                    IrMatchCase {
                        pattern: IrMatchPattern::Value(IrValue::Local("p".to_string())),
                        guard: Some(IrValue::Local("b".to_string())),
                        body: vec![IrOp::Eval {
                            value: IrValue::Local("p".to_string()),
                        }],
                    },
                    IrMatchCase {
                        pattern: IrMatchPattern::OneOf(vec![
                            IrValue::Local("p".to_string()),
                            IrValue::Local("q".to_string()),
                        ]),
                        guard: None,
                        body: vec![],
                    },
                    IrMatchCase {
                        pattern: IrMatchPattern::Else,
                        guard: None,
                        body: vec![IrOp::Fail {
                            error: IrValue::Local("e".to_string()),
                        }],
                    },
                ],
            },
            IrOp::Trap {
                name: "err".to_string(),
                body: vec![IrOp::Eval {
                    value: IrValue::CallResult {
                        target: "toInt".to_string(),
                        args: vec![IrValue::Local("s".to_string())],
                        loc: IrSourceLoc::default(),
                    },
                }],
            },
        ];

        IrProject {
            name: "corpus".to_string(),
            entry: Some(EntryPoint {
                name: "main".to_string(),
                returns: "Integer".to_string(),
                accepts_args: true,
            }),
            bindings: vec![IrBinding {
                name: "g".to_string(),
                visibility: "package".to_string(),
                mutable: false,
                type_: "Integer".to_string(),
                value: Some(sample_value()),
            }],
            types: vec![
                IrType {
                    kind: "type".to_string(),
                    visibility: "export".to_string(),
                    name: "Point".to_string(),
                    fields: vec![
                        IrField {
                            visibility: Some("export".to_string()),
                            name: "x".to_string(),
                            type_: "Integer".to_string(),
                        },
                        IrField {
                            visibility: None,
                            name: "y".to_string(),
                            type_: "Integer".to_string(),
                        },
                    ],
                    includes: vec![],
                    variants: vec![],
                    members: vec![],
                },
                IrType {
                    kind: "union".to_string(),
                    visibility: "export".to_string(),
                    name: "Shape".to_string(),
                    fields: vec![],
                    includes: vec!["Base".to_string()],
                    variants: vec![IrVariant {
                        name: "Point".to_string(),
                        fields: vec![IrField {
                            visibility: None,
                            name: "x".to_string(),
                            type_: "Integer".to_string(),
                        }],
                    }],
                    members: vec![],
                },
                IrType {
                    kind: "enum".to_string(),
                    visibility: "private".to_string(),
                    name: "Color".to_string(),
                    fields: vec![],
                    includes: vec![],
                    variants: vec![],
                    members: vec![
                        IrEnumMember {
                            name: "Red".to_string(),
                        },
                        IrEnumMember {
                            name: "Green".to_string(),
                        },
                    ],
                },
            ],
            functions: vec![IrFunction {
                name: "main".to_string(),
                visibility: "export".to_string(),
                kind: "function".to_string(),
                isolated: false,
                params: vec![
                    IrParam {
                        name: "x".to_string(),
                        type_: "Integer".to_string(),
                        default: None,
                    },
                    IrParam {
                        name: "y".to_string(),
                        type_: "Integer".to_string(),
                        default: Some(IrValue::Const {
                            type_: "Integer".to_string(),
                            value: "0".to_string(),
                        }),
                    },
                ],
                returns: "Integer".to_string(),
                body,
                file: "src/main.mfb".to_string(),
            }],
        }
    }

    #[test]
    fn binary_repr_round_trip_is_identity() {
        let project = corpus_project();
        let bytes = encode_binary_repr(&project);
        let decoded = decode_binary_repr(&bytes).expect("decode");
        // The JSON projection is a faithful view of every field; comparing it
        // proves the decode reconstructed the project exactly.
        assert_eq!(project.to_json(), decoded.to_json());
        // Re-encoding the decoded project must be byte-identical.
        let bytes2 = encode_binary_repr(&decoded);
        assert_eq!(bytes, bytes2);
    }

    #[test]
    fn binary_repr_rejects_bad_magic() {
        let mut bytes = encode_binary_repr(&corpus_project());
        bytes[0] = b'X';
        assert!(decode_binary_repr(&bytes).is_err());
    }

    #[test]
    fn binary_repr_rejects_bad_version() {
        let mut bytes = encode_binary_repr(&corpus_project());
        bytes[4] = 0xFF;
        bytes[5] = 0xFF;
        assert!(decode_binary_repr(&bytes).is_err());
    }

    fn fn_named(name: &str, body: Vec<IrOp>) -> IrFunction {
        IrFunction {
            name: name.to_string(),
            visibility: "export".to_string(),
            kind: "function".to_string(),
            isolated: false,
            params: vec![],
            returns: "Integer".to_string(),
            body,
            file: "src/main.mfb".to_string(),
        }
    }

    fn project_named(name: &str, functions: Vec<IrFunction>) -> IrProject {
        IrProject {
            name: name.to_string(),
            entry: None,
            bindings: vec![],
            types: vec![],
            functions,
        }
    }

    // The identity prefix `<id>.package.symbol` must be applied consistently to
    // a package's definitions and to every external reference, so the consumer's
    // `package.symbol` call resolves to the merged, identity-prefixed definition.
    #[test]
    fn package_identity_prefix_is_applied_consistently() {
        // Package `pkg`: `f` calls its own `g`.
        let mut pkg = project_named(
            "pkg",
            vec![
                fn_named(
                    "f",
                    vec![IrOp::Eval {
                        value: IrValue::Call {
                            target: "g".to_string(),
                            args: vec![],
                            loc: IrSourceLoc::default(),
                        },
                    }],
                ),
                fn_named("g", vec![]),
            ],
        );

        // Names the consumer references, captured before the rename.
        let (ref_fns, ref_globals) = package_qualified_reference_names(&pkg);
        assert!(ref_fns.contains("pkg.f"));
        assert!(ref_fns.contains("pkg.g"));

        let id = "abcd1234";
        prefix_package_symbols(&mut pkg, id);

        // Definitions carry the full `<id>.package.symbol` prefix...
        assert_eq!(pkg.functions[0].name, "abcd1234.pkg.f");
        assert_eq!(pkg.functions[1].name, "abcd1234.pkg.g");
        // ...and the package's own internal reference was rewritten to match.
        match &pkg.functions[0].body[0] {
            IrOp::Eval {
                value: IrValue::Call { target, .. },
            } => assert_eq!(target, "abcd1234.pkg.g"),
            _ => panic!("expected an Eval(Call) op"),
        }

        // A consumer that calls `pkg.f` has that reference rewritten to the
        // identity-prefixed definition name.
        let mut consumer = project_named(
            "app",
            vec![fn_named(
                "main",
                vec![IrOp::Eval {
                    value: IrValue::Call {
                        target: "pkg.f".to_string(),
                        args: vec![],
                        loc: IrSourceLoc::default(),
                    },
                }],
            )],
        );
        apply_package_identity(&mut consumer, &ref_fns, &ref_globals, id);
        match &consumer.functions[0].body[0] {
            IrOp::Eval {
                value: IrValue::Call { target, .. },
            } => assert_eq!(target, "abcd1234.pkg.f"),
            _ => panic!("expected an Eval(Call) op"),
        }
    }

    // Decoded package IR is verified against the package-format invariants
    // before it is merged; these exercise the PACKAGE_BINARY_REPRESENTATION_VERIFY_* diagnostics.
    #[test]
    fn verify_package_rejects_duplicate_function() {
        let pir = project_named("pkg", vec![fn_named("f", vec![]), fn_named("f", vec![])]);
        let err = verify_package(&pir).expect_err("duplicate function must be rejected");
        assert!(
            err.contains("PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE"),
            "{err}"
        );
    }

    #[test]
    fn verify_package_rejects_unnamed_function() {
        let pir = project_named("pkg", vec![fn_named("", vec![])]);
        let err = verify_package(&pir).expect_err("unnamed function must be rejected");
        assert!(
            err.contains("PACKAGE_BINARY_REPRESENTATION_VERIFY_TYPE"),
            "{err}"
        );
    }

    #[test]
    fn verify_package_rejects_empty_match() {
        let body = vec![IrOp::Match {
            value: IrValue::Local("x".to_string()),
            cases: vec![],
        }];
        let pir = project_named("pkg", vec![fn_named("f", body)]);
        let err = verify_package(&pir).expect_err("non-exhaustive MATCH must be rejected");
        assert!(
            err.contains("PACKAGE_BINARY_REPRESENTATION_VERIFY_MATCH"),
            "{err}"
        );
    }
}
