use crate::ast::{
    AstProject, Expression, Function, FunctionKind, Item, Param, Statement, TypeDecl, TypeDeclKind,
};
use crate::json_string;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

pub struct IrProject {
    pub(crate) name: String,
    pub(crate) types: Vec<IrType>,
    pub(crate) functions: Vec<IrFunction>,
}

pub(crate) struct IrType {
    pub(crate) kind: String,
    pub(crate) name: String,
}

pub(crate) struct IrFunction {
    pub(crate) name: String,
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
    Return {
        value: Option<IrValue>,
    },
    Eval {
        value: IrValue,
    },
}

pub(crate) enum IrValue {
    Const {
        type_: String,
        value: String,
    },
    Local(String),
    Call {
        target: String,
        args: Vec<IrValue>,
    },
    Binary {
        op: String,
        left: Box<IrValue>,
        right: Box<IrValue>,
    },
}

pub fn lower_project(ast: &AstProject) -> IrProject {
    let mut types = Vec::new();
    let mut functions = Vec::new();
    let function_returns = function_returns(ast);

    for file in &ast.files {
        for item in &file.items {
            match item {
                Item::Function(function) => {
                    functions.push(lower_function(function, &function_returns))
                }
                Item::Type(type_decl) => types.push(lower_type(type_decl)),
            }
        }
    }

    IrProject {
        name: ast.name.clone(),
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
        name: type_decl.name.clone(),
    }
}

fn lower_function(function: &Function, function_returns: &HashMap<String, String>) -> IrFunction {
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
        kind: kind.to_string(),
        params: function.params.iter().map(lower_param).collect(),
        returns,
        body: function
            .body
            .iter()
            .map(|statement| lower_statement(statement, &mut locals, function_returns))
            .collect(),
    }
}

fn lower_param(param: &Param) -> IrParam {
    IrParam {
        name: param.name.clone(),
        type_: param
            .type_name
            .clone()
            .expect("typecheck requires parameter type before IR lowering"),
        default: param.default.as_ref().map(lower_expression),
    }
}

fn lower_statement(
    statement: &Statement,
    locals: &mut HashMap<String, String>,
    function_returns: &HashMap<String, String>,
) -> IrOp {
    match statement {
        Statement::Let {
            mutable,
            name,
            type_name,
            value,
            ..
        } => {
            let lowered_value = value.as_ref().map(lower_expression);
            let lowered_type = type_name.clone().unwrap_or_else(|| {
                value
                    .as_ref()
                    .and_then(|value| expression_type(value, locals, function_returns))
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
            value: value.as_ref().map(lower_expression),
        },
        Statement::Expression { expression, .. } => IrOp::Eval {
            value: lower_expression(expression),
        },
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

fn expression_type(
    expression: &Expression,
    locals: &HashMap<String, String>,
    function_returns: &HashMap<String, String>,
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
        Expression::Identifier(value) => locals.get(value).cloned(),
        Expression::Call { callee, .. } => {
            if callee == "io.print" {
                Some("Nothing".to_string())
            } else {
                function_returns.get(callee).cloned()
            }
        }
        Expression::Binary {
            left,
            operator,
            right,
        } => {
            if operator == "&" {
                return Some("String".to_string());
            }
            let left = expression_type(left, locals, function_returns)?;
            let right = expression_type(right, locals, function_returns)?;
            if left == "Float" || left == "Fixed" || right == "Float" || right == "Fixed" {
                Some("Float".to_string())
            } else {
                Some("Integer".to_string())
            }
        }
    }
}

fn lower_expression(expression: &Expression) -> IrValue {
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
        Expression::Identifier(value) => IrValue::Local(value.clone()),
        Expression::Call { callee, arguments } => IrValue::Call {
            target: callee.clone(),
            args: arguments.iter().map(lower_expression).collect(),
        },
        Expression::Binary {
            left,
            operator,
            right,
        } => IrValue::Binary {
            op: operator.clone(),
            left: Box::new(lower_expression(left)),
            right: Box::new(lower_expression(right)),
        },
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
                "  \"types\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.name),
            join_json(&self.types, 2),
            join_json(&self.functions, 2)
        )
    }
}

trait ToIrJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToIrJson for IrType {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"kind\": {}, \"name\": {} }}",
            pad,
            json_string(&self.kind),
            json_string(&self.name)
        )
    }
}

impl ToIrJson for IrFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
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
            IrOp::Eval { value } => {
                format!(
                    "\n{}{{ \"op\": \"eval\", \"value\": {} }}",
                    pad,
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
            IrValue::Binary { op, left, right } => {
                format!(
                    "{{ \"kind\": \"binary\", \"op\": {}, \"left\": {}, \"right\": {} }}",
                    json_string(op),
                    left.to_json(0),
                    right.to_json(0)
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
