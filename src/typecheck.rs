use crate::ast::{AstFile, AstProject, Expression, Function, FunctionKind, Item, Statement};
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Type {
    Boolean,
    Byte,
    Fixed,
    Float,
    Integer,
    Nothing,
    Result,
    String,
    User(String),
    Unknown,
}

#[derive(Clone)]
struct FunctionSig {
    kind: FunctionKind,
    params: Vec<ParamSig>,
    return_type: Type,
}

#[derive(Clone)]
struct ParamSig {
    type_: Type,
    has_default: bool,
}

pub fn check_project(project_dir: &Path, ast: &AstProject) -> Result<(), ()> {
    let mut checker = TypeChecker::new(project_dir, ast);
    checker.check();
    if checker.had_error {
        Err(())
    } else {
        Ok(())
    }
}

struct TypeChecker<'a> {
    project_dir: &'a Path,
    ast: &'a AstProject,
    functions: HashMap<String, FunctionSig>,
    user_types: HashSet<String>,
    had_error: bool,
}

impl<'a> TypeChecker<'a> {
    fn new(project_dir: &'a Path, ast: &'a AstProject) -> Self {
        let mut checker = Self {
            project_dir,
            ast,
            functions: HashMap::new(),
            user_types: HashSet::new(),
            had_error: false,
        };
        checker.collect_types();
        checker.collect_functions();
        checker
    }

    fn collect_types(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Type(type_decl) = item {
                    self.user_types.insert(type_decl.name.clone());
                }
            }
        }
    }

    fn collect_functions(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Function(function) = item {
                    let return_type = match function.kind {
                        FunctionKind::Func => function
                            .return_type
                            .as_deref()
                            .map(|name| self.parse_type(name))
                            .unwrap_or(Type::Unknown),
                        FunctionKind::Sub => Type::Nothing,
                    };
                    let params = function
                        .params
                        .iter()
                        .map(|param| ParamSig {
                            type_: param
                                .type_name
                                .as_deref()
                                .map(|name| self.parse_type(name))
                                .unwrap_or(Type::Unknown),
                            has_default: param.default.is_some(),
                        })
                        .collect();
                    self.functions.insert(
                        function.name.clone(),
                        FunctionSig {
                            kind: function.kind.clone(),
                            params,
                            return_type,
                        },
                    );
                }
            }
        }
    }

    fn check(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Function(function) = item {
                    self.check_function(file, function);
                }
            }
        }
    }

    fn check_function(&mut self, file: &AstFile, function: &Function) {
        let expected_return = match function.kind {
            FunctionKind::Func => {
                if function.return_type.is_none() {
                    self.report(
                        "TYPE_FUNC_REQUIRES_RETURN_TYPE",
                        &format!("FUNC `{}` must declare an `AS` return type.", function.name),
                        file,
                        function.line,
                    );
                    Type::Unknown
                } else {
                    let return_type = self.parse_type(function.return_type.as_deref().unwrap());
                    if matches!(return_type, Type::Result) {
                        self.report(
                            "TYPE_RESULT_IS_IMPLICIT",
                            "Functions declare their success type; Result wrapping is implicit.",
                            file,
                            function.line,
                        );
                        Type::Unknown
                    } else {
                        return_type
                    }
                }
            }
            FunctionKind::Sub => {
                if function.return_type.is_some() {
                    self.report(
                        "TYPE_SUB_CANNOT_RETURN_VALUE",
                        &format!("SUB `{}` cannot declare a return type.", function.name),
                        file,
                        function.line,
                    );
                }
                Type::Nothing
            }
        };

        let mut locals = HashMap::new();
        let mut seen_default = false;
        for param in &function.params {
            let param_type = param
                .type_name
                .as_deref()
                .map(|name| self.parse_type(name))
                .unwrap_or(Type::Unknown);

            if param.type_name.is_none() {
                self.report(
                    "TYPE_PARAM_REQUIRES_TYPE",
                    &format!("Parameter `{}` must declare an `AS` type.", param.name),
                    file,
                    param.line,
                );
            }

            if param.default.is_some() {
                seen_default = true;
            } else if seen_default {
                self.report(
                    "TYPE_DEFAULT_ARG_ORDER",
                    &format!(
                        "Parameter `{}` must have a default because an earlier parameter has one.",
                        param.name
                    ),
                    file,
                    param.line,
                );
            }

            if let Some(default) = &param.default {
                let default_type = self.infer_expression(file, default, &locals, param.line);
                if matches!(default_type, Type::Unknown) {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!(
                            "Default value for `{}` does not have a known type.",
                            param.name
                        ),
                        file,
                        param.line,
                    );
                }
                if !self.compatible(&param_type, &default_type) {
                    self.report(
                        "TYPE_DEFAULT_VALUE_MISMATCH",
                        &format!(
                            "Default value for `{}` has type {}, expected {}.",
                            param.name,
                            self.type_name(&default_type),
                            self.type_name(&param_type)
                        ),
                        file,
                        param.line,
                    );
                }
            }

            locals.insert(param.name.clone(), param_type);
        }

        let mut saw_return = false;
        for statement in &function.body {
            self.check_statement(
                file,
                statement,
                &expected_return,
                &mut locals,
                &mut saw_return,
            );
        }

        if matches!(function.kind, FunctionKind::Func) && !saw_return {
            self.report(
                "TYPE_FUNC_MISSING_RETURN",
                &format!(
                    "FUNC `{}` must return a {} value.",
                    function.name,
                    self.type_name(&expected_return)
                ),
                file,
                function.line,
            );
        }
    }

    fn check_statement(
        &mut self,
        file: &AstFile,
        statement: &Statement,
        expected_return: &Type,
        locals: &mut HashMap<String, Type>,
        saw_return: &mut bool,
    ) {
        match statement {
            Statement::Let {
                name,
                type_name,
                value,
                line,
                mutable,
                ..
            } => {
                let declared = type_name.as_deref().map(|name| self.parse_type(name));
                let inferred = value
                    .as_ref()
                    .map(|value| self.infer_expression(file, value, locals, *line));

                if matches!(inferred, Some(Type::Unknown)) {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!("Initializer for binding `{name}` does not have a known type."),
                        file,
                        *line,
                    );
                }

                match (&declared, &inferred) {
                    (Some(declared), Some(inferred)) if !self.compatible(declared, inferred) => {
                        self.report(
                            "TYPE_BINDING_MISMATCH",
                            &format!(
                                "Binding `{name}` has initializer type {}, expected {}.",
                                self.type_name(inferred),
                                self.type_name(declared)
                            ),
                            file,
                            *line,
                        );
                    }
                    (None, None) => {
                        self.report(
                            "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
                            &format!("Binding `{name}` needs a type annotation or initializer."),
                            file,
                            *line,
                        );
                    }
                    (Some(_), None) if !mutable => {
                        self.report(
                            "TYPE_LET_REQUIRES_VALUE",
                            &format!("Immutable binding `{name}` must have an initializer."),
                            file,
                            *line,
                        );
                    }
                    _ => {}
                }

                let binding_type = declared.or(inferred).unwrap_or(Type::Unknown);
                locals.insert(name.clone(), binding_type);
            }
            Statement::Return { value, line } => {
                *saw_return = true;
                let actual = value
                    .as_ref()
                    .map(|value| self.infer_expression(file, value, locals, *line))
                    .unwrap_or(Type::Nothing);
                if matches!(actual, Type::Unknown) {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        "RETURN value does not have a known type.",
                        file,
                        *line,
                    );
                }
                if matches!(expected_return, Type::Nothing) && !matches!(actual, Type::Nothing) {
                    self.report(
                        "TYPE_SUB_CANNOT_RETURN_VALUE",
                        &format!(
                            "SUB return paths must produce Nothing, got {}.",
                            self.type_name(&actual)
                        ),
                        file,
                        *line,
                    );
                    return;
                }
                if !self.compatible(expected_return, &actual) {
                    self.report(
                        "TYPE_RETURN_MISMATCH",
                        &format!(
                            "RETURN has type {}, expected {}.",
                            self.type_name(&actual),
                            self.type_name(expected_return)
                        ),
                        file,
                        *line,
                    );
                }
            }
            Statement::Expression { expression, line } => {
                self.infer_expression(file, expression, locals, *line);
            }
        }
    }

    fn infer_expression(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &HashMap<String, Type>,
        line: usize,
    ) -> Type {
        match expression {
            Expression::String(_) => Type::String,
            Expression::Boolean(_) => Type::Boolean,
            Expression::Number(value) => {
                if value.contains('.') {
                    Type::Float
                } else {
                    Type::Integer
                }
            }
            Expression::Identifier(name) => locals.get(name).cloned().unwrap_or(Type::Unknown),
            Expression::Binary {
                left,
                operator,
                right,
            } => {
                let left_type = self.infer_expression(file, left, locals, line);
                let right_type = self.infer_expression(file, right, locals, line);
                self.infer_binary(file, operator, &left_type, &right_type, line)
            }
            Expression::Call { callee, arguments } => {
                if callee == "io.print" {
                    for argument in arguments {
                        self.infer_expression(file, argument, locals, line);
                    }
                    return Type::Nothing;
                }

                if callee.contains('.') {
                    for argument in arguments {
                        self.infer_expression(file, argument, locals, line);
                    }
                    return Type::Unknown;
                }

                let Some(sig) = self.functions.get(callee).cloned() else {
                    return Type::Unknown;
                };

                self.check_call(file, callee, &sig, arguments, locals, line);
                sig.return_type
            }
        }
    }

    fn infer_binary(
        &mut self,
        file: &AstFile,
        operator: &str,
        left: &Type,
        right: &Type,
        line: usize,
    ) -> Type {
        if operator == "&" {
            if self.compatible(&Type::String, left) && self.compatible(&Type::String, right) {
                return Type::String;
            }
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `&` requires String operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }

        if self.is_numeric(left) && self.is_numeric(right) {
            if matches!(left, Type::Float | Type::Fixed)
                || matches!(right, Type::Float | Type::Fixed)
            {
                Type::Float
            } else {
                Type::Integer
            }
        } else {
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `{operator}` requires numeric operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            Type::Unknown
        }
    }

    fn check_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        sig: &FunctionSig,
        arguments: &[Expression],
        locals: &HashMap<String, Type>,
        line: usize,
    ) {
        let required = sig.params.iter().filter(|param| !param.has_default).count();
        if arguments.len() < required || arguments.len() > sig.params.len() {
            self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{callee}` has {} argument(s), expected {} to {}.",
                    arguments.len(),
                    required,
                    sig.params.len()
                ),
                file,
                line,
            );
        }

        for (index, argument) in arguments.iter().enumerate() {
            let actual = self.infer_expression(file, argument, locals, line);
            let Some(param) = sig.params.get(index) else {
                continue;
            };
            if !self.compatible(&param.type_, &actual) {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    &format!(
                        "Argument {} for `{callee}` has type {}, expected {}.",
                        index + 1,
                        self.type_name(&actual),
                        self.type_name(&param.type_)
                    ),
                    file,
                    line,
                );
            }
        }

        if matches!(sig.kind, FunctionKind::Sub) {
            // SUB calls auto-unwrap to successful Nothing under the implicit Result model.
        }
    }

    fn parse_type(&self, name: &str) -> Type {
        match name {
            "Boolean" => Type::Boolean,
            "Byte" => Type::Byte,
            "Fixed" => Type::Fixed,
            "Float" => Type::Float,
            "Integer" => Type::Integer,
            "Nothing" => Type::Nothing,
            "String" => Type::String,
            "Result" => Type::Result,
            other if self.user_types.contains(other) => Type::User(other.to_string()),
            other => Type::User(other.to_string()),
        }
    }

    fn compatible(&self, expected: &Type, actual: &Type) -> bool {
        expected == actual || matches!(expected, Type::Unknown) || matches!(actual, Type::Unknown)
    }

    fn is_numeric(&self, type_: &Type) -> bool {
        matches!(
            type_,
            Type::Byte | Type::Fixed | Type::Float | Type::Integer | Type::Unknown
        )
    }

    fn type_name(&self, type_: &Type) -> String {
        match type_ {
            Type::Boolean => "Boolean".to_string(),
            Type::Byte => "Byte".to_string(),
            Type::Fixed => "Fixed".to_string(),
            Type::Float => "Float".to_string(),
            Type::Integer => "Integer".to_string(),
            Type::Nothing => "Nothing".to_string(),
            Type::Result => "Result".to_string(),
            Type::String => "String".to_string(),
            Type::User(name) => name.clone(),
            Type::Unknown => "Unknown".to_string(),
        }
    }

    fn report(&mut self, rule: &str, detail: &str, file: &AstFile, line: usize) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, &self.project_dir.join(&file.path), line, 1, 1);
    }
}
