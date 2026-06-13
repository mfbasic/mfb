use crate::ast::{
    AstFile, AstProject, ConstructorArg, Expression, Function, FunctionKind, Item, MatchPattern,
    RecordUpdate, Statement, TypeDecl, TypeDeclKind, TypeField, Visibility,
};
use crate::builtins;
use crate::bytecode::{self, BytecodeExportKind};
use crate::numeric;
use crate::rules;
use std::collections::{HashMap, HashSet};
use std::path::Path;

#[derive(Clone, Debug, Eq, PartialEq)]
enum Type {
    Boolean,
    Byte,
    Error,
    Fixed,
    Float,
    Integer,
    List(Box<Type>),
    Map(Box<Type>, Box<Type>),
    Function {
        params: Vec<Type>,
        return_type: Box<Type>,
        isolated: bool,
    },
    Nothing,
    Result(Box<Type>),
    String,
    Thread(Box<Type>, Box<Type>),
    User(String),
    Unknown,
}

#[derive(Clone)]
struct LocalInfo {
    type_: Type,
    mutable: bool,
}

#[derive(Clone)]
struct FunctionSig {
    kind: FunctionKind,
    params: Vec<ParamSig>,
    return_type: Type,
    isolated: bool,
}

#[derive(Clone)]
struct ParamSig {
    type_: Type,
    has_default: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Flow {
    FallsThrough,
    AlwaysReturns,
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
    user_type_kinds: HashMap<String, TypeDeclKind>,
    type_infos: HashMap<String, TypeInfo>,
    variant_constructors: HashMap<String, Vec<VariantConstructor>>,
    had_error: bool,
}

#[derive(Clone)]
struct TypeInfo {
    kind: TypeDeclKind,
    visibility: Visibility,
    file_path: String,
    fields: Vec<FieldInfo>,
    variants: Vec<VariantConstructor>,
    members: HashSet<String>,
}

#[derive(Clone)]
struct FieldInfo {
    name: String,
    type_: Type,
    visibility: Visibility,
}

#[derive(Clone)]
struct VariantConstructor {
    name: String,
    union_name: String,
    visibility: Visibility,
    file_path: String,
    fields: Vec<FieldInfo>,
}

impl<'a> TypeChecker<'a> {
    fn new(project_dir: &'a Path, ast: &'a AstProject) -> Self {
        let mut checker = Self {
            project_dir,
            ast,
            functions: HashMap::new(),
            user_types: HashSet::new(),
            user_type_kinds: HashMap::new(),
            type_infos: HashMap::new(),
            variant_constructors: HashMap::new(),
            had_error: false,
        };
        checker.collect_types();
        checker.collect_functions();
        checker.collect_package_functions();
        checker
    }

    fn collect_types(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Type(type_decl) = item {
                    self.user_types.insert(type_decl.name.clone());
                    self.user_type_kinds
                        .insert(type_decl.name.clone(), type_decl.kind);
                }
            }
        }

        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Type(type_decl) = item {
                    let info = self.type_info(file, type_decl);
                    self.type_infos.insert(type_decl.name.clone(), info);
                }
            }
        }

        let names = self.type_infos.keys().cloned().collect::<Vec<_>>();
        for name in names {
            let Some(TypeInfo {
                kind: TypeDeclKind::Union,
                ..
            }) = self.type_infos.get(&name)
            else {
                continue;
            };
            let expanded = self.expanded_union_variants(&name, &mut HashSet::new());
            if let Some(info) = self.type_infos.get_mut(&name) {
                info.variants = expanded;
            }
        }

        for info in self.type_infos.values() {
            for variant in &info.variants {
                self.variant_constructors
                    .entry(variant_name_key(variant))
                    .or_default()
                    .push(variant.clone());
            }
        }
    }

    fn expanded_union_variants(
        &self,
        union_name: &str,
        visiting: &mut HashSet<String>,
    ) -> Vec<VariantConstructor> {
        if !visiting.insert(union_name.to_string()) {
            return Vec::new();
        }
        let mut variants = Vec::new();
        let includes = self
            .ast
            .files
            .iter()
            .flat_map(|file| &file.items)
            .find_map(|item| {
                let Item::Type(type_decl) = item else {
                    return None;
                };
                if type_decl.name == union_name {
                    Some(type_decl.includes.clone())
                } else {
                    None
                }
            })
            .unwrap_or_default();
        for include in includes {
            for mut variant in self.expanded_union_variants(&include, visiting) {
                variant.union_name = union_name.to_string();
                variants.push(variant);
            }
        }
        if let Some(info) = self.type_infos.get(union_name) {
            variants.extend(info.variants.clone());
        }
        visiting.remove(union_name);
        variants
    }

    fn type_info(&self, file: &AstFile, type_decl: &TypeDecl) -> TypeInfo {
        let fields = type_decl
            .fields
            .iter()
            .map(|field| self.field_info(field, type_decl.visibility))
            .collect();
        let variants = type_decl
            .variants
            .iter()
            .map(|variant| VariantConstructor {
                name: variant.name.clone(),
                union_name: type_decl.name.clone(),
                visibility: type_decl.visibility,
                file_path: file.path.clone(),
                fields: variant
                    .fields
                    .iter()
                    .map(|field| self.field_info(field, Visibility::Export))
                    .collect(),
            })
            .collect();
        let members = type_decl
            .members
            .iter()
            .map(|member| member.name.clone())
            .collect();
        TypeInfo {
            kind: type_decl.kind,
            visibility: type_decl.visibility,
            file_path: file.path.clone(),
            fields,
            variants,
            members,
        }
    }

    fn field_info(&self, field: &TypeField, containing_visibility: Visibility) -> FieldInfo {
        FieldInfo {
            name: field.name.clone(),
            type_: self.parse_type(&field.type_name),
            visibility: effective_field_visibility(field.visibility, containing_visibility),
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
                            isolated: function.isolated,
                        },
                    );
                }
            }
        }
    }

    fn collect_package_functions(&mut self) {
        let mut seen = HashSet::new();
        for package in self.imported_packages() {
            if !seen.insert(package.clone()) {
                continue;
            }
            let package_file = self
                .project_dir
                .join("packages")
                .join(format!("{package}.mfp"));
            if !package_file.is_file() {
                continue;
            }
            let Ok(exports) = bytecode::read_package_exports(&package_file) else {
                continue;
            };
            for export in exports {
                self.functions.insert(
                    format!("{package}.{}", export.name),
                    FunctionSig {
                        kind: match export.kind {
                            BytecodeExportKind::Func => FunctionKind::Func,
                            BytecodeExportKind::Sub => FunctionKind::Sub,
                        },
                        params: export
                            .params
                            .into_iter()
                            .map(|param| ParamSig {
                                type_: self.parse_type(&param.type_),
                                has_default: param.has_default,
                            })
                            .collect(),
                        return_type: self.parse_type(&export.return_type),
                        isolated: export.isolated,
                    },
                );
            }
        }
    }

    fn imported_packages(&self) -> HashSet<String> {
        self.ast
            .files
            .iter()
            .flat_map(|file| &file.imports)
            .filter_map(|import| import.module.split('.').next().map(str::to_string))
            .filter(|package| !builtins::is_builtin_import(package))
            .collect()
    }

    fn check(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                match item {
                    Item::Function(function) => self.check_function(file, function),
                    Item::Type(type_decl) => self.check_type_decl(file, type_decl),
                }
            }
        }
    }

    fn check_type_decl(&mut self, file: &AstFile, type_decl: &TypeDecl) {
        match type_decl.kind {
            TypeDeclKind::Type => {
                for field in &type_decl.fields {
                    let type_ = self.parse_type(&field.type_name);
                    self.check_type_reference(file, &type_, field.line);
                }
            }
            TypeDeclKind::Union => {
                for include in &type_decl.includes {
                    let type_ = self.parse_type(include);
                    self.check_type_reference(file, &type_, type_decl.line);
                    if let Some(kind) = self.user_type_kinds.get(include) {
                        if !matches!(kind, TypeDeclKind::Union) {
                            self.report(
                                "TYPE_UNION_INCLUDE_REQUIRES_UNION",
                                &format!(
                                    "UNION `{}` includes `{include}`, but `{include}` is not a UNION.",
                                    type_decl.name
                                ),
                                file,
                                type_decl.line,
                            );
                        }
                    }
                }

                for variant in &type_decl.variants {
                    for field in &variant.fields {
                        let type_ = self.parse_type(&field.type_name);
                        self.check_type_reference(file, &type_, field.line);
                    }
                }
            }
            TypeDeclKind::Enum => {
                if type_decl.members.is_empty() {
                    self.report(
                        "TYPE_ENUM_REQUIRES_MEMBER",
                        &format!(
                            "ENUM `{}` must declare at least one member.",
                            type_decl.name
                        ),
                        file,
                        type_decl.line,
                    );
                }
            }
        }
    }

    fn check_function(&mut self, file: &AstFile, function: &Function) {
        if function.isolated
            && (!matches!(function.kind, FunctionKind::Func)
                || !matches!(function.visibility, Visibility::Export))
        {
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "ISOLATED function `{}` must be an exported FUNC declaration.",
                    function.name
                ),
                file,
                function.line,
            );
        }

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
                    self.check_type_reference(file, &return_type, function.line);
                    if matches!(return_type, Type::Result(_)) {
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
            self.check_type_reference(file, &param_type, param.line);

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
                if !self.expression_compatible(&param_type, &default_type, Some(default)) {
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

            locals.insert(
                param.name.clone(),
                LocalInfo {
                    type_: param_type,
                    mutable: false,
                },
            );
        }

        let flow = self.check_block(file, &function.body, &expected_return, &mut locals);
        if let Some(trap) = &function.trap {
            let mut trap_locals = locals.clone();
            trap_locals.insert(
                trap.name.clone(),
                LocalInfo {
                    type_: Type::Error,
                    mutable: false,
                },
            );
            let trap_flow = self.check_block(file, &trap.body, &expected_return, &mut trap_locals);
            if trap_flow != Flow::AlwaysReturns {
                self.report(
                    "TYPE_TRAP_FALLTHROUGH",
                    &format!(
                        "TRAP `{}` must return, fail, recover, or propagate.",
                        trap.name
                    ),
                    file,
                    trap.line,
                );
            }
        }

        if matches!(function.kind, FunctionKind::Func) && flow != Flow::AlwaysReturns {
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

    fn check_block(
        &mut self,
        file: &AstFile,
        body: &[Statement],
        expected_return: &Type,
        locals: &mut HashMap<String, LocalInfo>,
    ) -> Flow {
        for statement in body {
            let flow = self.check_statement(file, statement, expected_return, locals);
            if flow == Flow::AlwaysReturns {
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    fn check_statement(
        &mut self,
        file: &AstFile,
        statement: &Statement,
        expected_return: &Type,
        locals: &mut HashMap<String, LocalInfo>,
    ) -> Flow {
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
                if let Some(declared) = &declared {
                    self.check_type_reference(file, declared, *line);
                }
                let inferred = value.as_ref().map(|value| {
                    self.infer_expression_with_expected(
                        file,
                        value,
                        locals,
                        *line,
                        declared.as_ref(),
                    )
                });

                if matches!(inferred, Some(Type::Unknown)) {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!("Initializer for binding `{name}` does not have a known type."),
                        file,
                        *line,
                    );
                }

                let reported_range_error =
                    declared
                        .as_ref()
                        .zip(value.as_ref())
                        .is_some_and(|(declared, value)| {
                            self.report_primitive_literal_range_error(declared, value, file, *line)
                        });

                match (&declared, &inferred) {
                    (Some(declared), Some(inferred))
                        if !reported_range_error
                            && !self.expression_compatible(declared, inferred, value.as_ref()) =>
                    {
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
                locals.insert(
                    name.clone(),
                    LocalInfo {
                        type_: binding_type,
                        mutable: *mutable,
                    },
                );
                Flow::FallsThrough
            }
            Statement::Return { value, line } => {
                let actual = value
                    .as_ref()
                    .map(|value| {
                        self.infer_expression_with_expected(
                            file,
                            value,
                            locals,
                            *line,
                            Some(expected_return),
                        )
                    })
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
                    return Flow::AlwaysReturns;
                }
                if !self.expression_compatible(expected_return, &actual, value.as_ref()) {
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
                Flow::AlwaysReturns
            }
            Statement::Fail { error, line } => {
                let actual = self.infer_expression(file, error, locals, *line);
                if !self.compatible(&Type::Error, &actual) {
                    self.report(
                        "TYPE_FAIL_REQUIRES_ERROR",
                        &format!("FAIL has type {}, expected Error.", self.type_name(&actual)),
                        file,
                        *line,
                    );
                }
                Flow::AlwaysReturns
            }
            Statement::Propagate { line } => {
                self.report(
                    "TYPE_PROPAGATE_REQUIRES_TRAP",
                    "PROPAGATE is valid only inside a TRAP.",
                    file,
                    *line,
                );
                Flow::AlwaysReturns
            }
            Statement::Recover { value, line } => {
                self.infer_expression(file, value, locals, *line);
                self.report(
                    "TYPE_RECOVER_REQUIRES_TRAP",
                    "RECOVER is not implemented for this trap context.",
                    file,
                    *line,
                );
                Flow::AlwaysReturns
            }
            Statement::Assign { name, value, line } => {
                let Some(local) = locals.get(name).cloned() else {
                    self.report(
                        "TYPE_UNKNOWN_VALUE",
                        &format!("Assignment target `{name}` is not a local binding."),
                        file,
                        *line,
                    );
                    return Flow::FallsThrough;
                };
                if !local.mutable {
                    self.report(
                        "TYPE_ASSIGN_REQUIRES_MUT",
                        &format!("Binding `{name}` is immutable and cannot be assigned."),
                        file,
                        *line,
                    );
                }
                let actual = self.infer_expression(file, value, locals, *line);
                let reported_range_error =
                    self.report_primitive_literal_range_error(&local.type_, value, file, *line);
                if !reported_range_error
                    && !self.expression_compatible(&local.type_, &actual, Some(value))
                {
                    self.report(
                        "TYPE_ASSIGNMENT_MISMATCH",
                        &format!(
                            "Assignment to `{name}` has type {}, expected {}.",
                            self.type_name(&actual),
                            self.type_name(&local.type_)
                        ),
                        file,
                        *line,
                    );
                }
                Flow::FallsThrough
            }
            Statement::Expression { expression, line } => {
                self.infer_expression(file, expression, locals, *line);
                Flow::FallsThrough
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                let condition_type = self.infer_expression(file, condition, locals, *line);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {
                    self.report(
                        "TYPE_CONDITION_REQUIRES_BOOLEAN",
                        &format!(
                            "IF condition has type {}, expected Boolean.",
                            self.type_name(&condition_type)
                        ),
                        file,
                        *line,
                    );
                }
                let mut then_locals = locals.clone();
                let then_flow =
                    self.check_block(file, then_body, expected_return, &mut then_locals);
                let mut else_locals = locals.clone();
                let else_flow =
                    self.check_block(file, else_body, expected_return, &mut else_locals);
                if then_flow == Flow::AlwaysReturns
                    && else_flow == Flow::AlwaysReturns
                    && !else_body.is_empty()
                {
                    Flow::AlwaysReturns
                } else {
                    Flow::FallsThrough
                }
            }
            Statement::Match {
                expression,
                cases,
                line,
            } => {
                let matched_type = self.infer_expression(file, expression, locals, *line);
                let mut has_else = false;
                let mut all_return = !cases.is_empty();
                let mut covered_cases = HashSet::new();
                for case in cases {
                    match &case.pattern {
                        MatchPattern::Else => has_else = true,
                        MatchPattern::Expression(pattern) => {
                            if let Some(name) = self.match_case_name(pattern) {
                                covered_cases.insert(name);
                            }
                            let pattern_type =
                                self.infer_match_pattern(file, pattern, locals, case.line);
                            if !self.expression_compatible(
                                &matched_type,
                                &pattern_type,
                                Some(pattern),
                            ) {
                                self.report(
                                    "TYPE_MATCH_PATTERN_MISMATCH",
                                    &format!(
                                        "CASE pattern has type {}, expected {}.",
                                        self.type_name(&pattern_type),
                                        self.type_name(&matched_type)
                                    ),
                                    file,
                                    case.line,
                                );
                            }
                        }
                    }
                    let mut case_locals = locals.clone();
                    if let Some((local_name, variant_name)) =
                        self.match_case_narrowing(expression, &case.pattern)
                    {
                        case_locals.insert(
                            local_name,
                            LocalInfo {
                                type_: Type::User(variant_name),
                                mutable: false,
                            },
                        );
                    }
                    let case_flow =
                        self.check_block(file, &case.body, expected_return, &mut case_locals);
                    all_return &= case_flow == Flow::AlwaysReturns;
                }
                if all_return
                    && (has_else || self.match_is_exhaustive(&matched_type, &covered_cases))
                {
                    Flow::AlwaysReturns
                } else {
                    Flow::FallsThrough
                }
            }
            Statement::Using {
                name,
                value,
                body,
                line,
            } => {
                let resource_type = self.infer_expression(file, value, locals, *line);
                let resource_type_name = self.type_name(&resource_type);
                if !builtins::is_resource_type(&resource_type_name) {
                    self.report(
                        "TYPE_USING_REQUIRES_RESOURCE",
                        &format!(
                            "USING binding `{name}` has type {}, expected resource.",
                            resource_type_name
                        ),
                        file,
                        *line,
                    );
                }
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: resource_type,
                        mutable: false,
                    },
                );
                self.check_block(file, body, expected_return, &mut nested)
            }
        }
    }

    fn infer_expression(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        self.infer_expression_with_expected(file, expression, locals, line, None)
    }

    fn infer_expression_with_expected(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        match expression {
            Expression::String(_) => Type::String,
            Expression::Boolean(_) => Type::Boolean,
            Expression::Number(value) => {
                if value.contains('.') {
                    Type::Float
                } else if value.parse::<i64>().is_ok() {
                    Type::Integer
                } else {
                    self.report(
                        "TYPE_INTEGER_LITERAL_OVERFLOW",
                        &format!("Integer literal `{value}` is outside the Integer range."),
                        file,
                        line,
                    );
                    Type::Integer
                }
            }
            Expression::Identifier(name) if name == "NOTHING" => Type::Nothing,
            Expression::Identifier(name) if builtins::math::is_math_constant(name) => {
                self.parse_type(builtins::math::constant_type_name(name).unwrap_or("Unknown"))
            }
            Expression::Identifier(name) => locals
                .get(name)
                .map(|local| local.type_.clone())
                .or_else(|| self.functions.get(name).map(function_type))
                .unwrap_or(Type::Unknown),
            Expression::Constructor {
                type_name,
                arguments,
            } => self.infer_constructor(file, type_name, arguments, locals, line, expected),
            Expression::WithUpdate { target, updates } => {
                self.infer_with_update(file, target, updates, locals, line)
            }
            Expression::MemberAccess { target, member } => {
                self.infer_member_access(file, target, member, locals, line)
            }
            Expression::Binary {
                left,
                operator,
                right,
            } => {
                let left_type = self.infer_expression(file, left, locals, line);
                let right_type = self.infer_expression(file, right, locals, line);
                self.infer_binary(file, operator, &left_type, &right_type, line)
            }
            Expression::Unary { operator, operand } => {
                if operator == "-" && !integer_literal_in_range(expression) {
                    if let Expression::Number(value) = operand.as_ref() {
                        self.report(
                            "TYPE_INTEGER_LITERAL_OVERFLOW",
                            &format!("Integer literal `-{value}` is outside the Integer range."),
                            file,
                            line,
                        );
                    }
                    return Type::Integer;
                }
                if operator == "-"
                    && matches!(operand.as_ref(), Expression::Number(value) if !value.contains('.'))
                {
                    return Type::Integer;
                }
                let operand_type = self.infer_expression(file, operand, locals, line);
                self.infer_unary(file, operator, &operand_type, line)
            }
            Expression::Call { callee, arguments } => {
                if builtins::is_builtin_call(callee) {
                    return self.check_builtin_call(file, callee, arguments, locals, line);
                }

                if let Some(sig) = self.functions.get(callee).cloned() {
                    self.check_call(file, callee, &sig, arguments, locals, line);
                    return sig.return_type;
                }

                if callee.contains('.') {
                    for argument in arguments {
                        self.infer_expression(file, argument, locals, line);
                    }
                    return Type::Unknown;
                }

                if let Some(local) = locals.get(callee).cloned() {
                    return self.check_function_value_call(
                        file,
                        callee,
                        &local.type_,
                        arguments,
                        locals,
                        line,
                    );
                }

                Type::Unknown
            }
            Expression::Lambda { params, body } => {
                self.infer_lambda(file, params, body, locals, line)
            }
            Expression::ListLiteral(values) => self.infer_list_literal(file, values, locals, line),
            Expression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => self.infer_map_literal(file, key_type, value_type, entries, locals, line),
        }
    }

    fn infer_match_pattern(
        &mut self,
        file: &AstFile,
        pattern: &Expression,
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        if let Expression::Identifier(name) = pattern {
            if let Some(variants) = self.variant_constructors.get(name).cloned() {
                if variants.len() == 1 {
                    return Type::User(variants[0].union_name.clone());
                }
            }
        }
        self.infer_expression(file, pattern, locals, line)
    }

    fn match_case_name(&self, pattern: &Expression) -> Option<String> {
        match pattern {
            Expression::Identifier(name) => Some(name.clone()),
            Expression::MemberAccess { target, member } => {
                if let Expression::Identifier(type_name) = target.as_ref() {
                    return Some(format!("{type_name}::{member}"));
                }
                None
            }
            _ => None,
        }
    }

    fn match_case_narrowing(
        &self,
        expression: &Expression,
        pattern: &MatchPattern,
    ) -> Option<(String, String)> {
        let Expression::Identifier(local_name) = expression else {
            return None;
        };
        let MatchPattern::Expression(Expression::Identifier(variant_name)) = pattern else {
            return None;
        };
        if self.variant_constructors.contains_key(variant_name) {
            Some((local_name.clone(), variant_name.clone()))
        } else {
            None
        }
    }

    fn match_is_exhaustive(&self, matched_type: &Type, covered_cases: &HashSet<String>) -> bool {
        let Type::User(type_name) = matched_type else {
            return false;
        };
        let Some(info) = self.type_infos.get(type_name) else {
            return false;
        };
        match info.kind {
            TypeDeclKind::Enum => info
                .members
                .iter()
                .all(|member| covered_cases.contains(&format!("{type_name}::{member}"))),
            TypeDeclKind::Union => info
                .variants
                .iter()
                .all(|variant| covered_cases.contains(&variant.name)),
            TypeDeclKind::Type => false,
        }
    }

    fn infer_list_literal(
        &mut self,
        file: &AstFile,
        values: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let Some(first) = values.first() else {
            return Type::List(Box::new(Type::Unknown));
        };
        let element_type = self.infer_expression(file, first, locals, line);
        for value in values.iter().skip(1) {
            let actual = self.infer_expression(file, value, locals, line);
            if !self.expression_compatible(&element_type, &actual, Some(value)) {
                self.report(
                    "TYPE_LIST_ELEMENT_MISMATCH",
                    &format!(
                        "List element has type {}, expected {}.",
                        self.type_name(&actual),
                        self.type_name(&element_type)
                    ),
                    file,
                    line,
                );
            }
        }
        Type::List(Box::new(element_type))
    }

    fn infer_map_literal(
        &mut self,
        file: &AstFile,
        key_type: &str,
        value_type: &str,
        entries: &[(Expression, Expression)],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let key_type = self.parse_type(key_type);
        let value_type = self.parse_type(value_type);
        self.check_type_reference(file, &key_type, line);
        self.check_type_reference(file, &value_type, line);
        for (key, value) in entries {
            let actual_key = self.infer_expression(file, key, locals, line);
            if !self.expression_compatible(&key_type, &actual_key, Some(key)) {
                self.report(
                    "TYPE_MAP_KEY_MISMATCH",
                    &format!(
                        "Map key has type {}, expected {}.",
                        self.type_name(&actual_key),
                        self.type_name(&key_type)
                    ),
                    file,
                    line,
                );
            }
            let actual_value = self.infer_expression(file, value, locals, line);
            if !self.expression_compatible(&value_type, &actual_value, Some(value)) {
                self.report(
                    "TYPE_MAP_VALUE_MISMATCH",
                    &format!(
                        "Map value has type {}, expected {}.",
                        self.type_name(&actual_value),
                        self.type_name(&value_type)
                    ),
                    file,
                    line,
                );
            }
        }
        Type::Map(Box::new(key_type), Box::new(value_type))
    }

    fn infer_constructor(
        &mut self,
        file: &AstFile,
        type_name: &str,
        arguments: &[ConstructorArg],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        if type_name == "Error" {
            let fields = vec![
                FieldInfo {
                    name: "code".to_string(),
                    type_: Type::Integer,
                    visibility: Visibility::Export,
                },
                FieldInfo {
                    name: "message".to_string(),
                    type_: Type::String,
                    visibility: Visibility::Export,
                },
            ];
            self.check_constructor_arguments(
                file, type_name, &fields, &file.path, arguments, locals, line,
            );
            return Type::Error;
        }

        if type_name == "Ok" {
            if arguments.len() != 1 {
                self.report(
                    "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
                    &format!(
                        "Constructor `Ok` has {} argument(s), expected 1.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            let success =
                self.infer_expression(file, constructor_arg_value(&arguments[0]), locals, line);
            return Type::Result(Box::new(success));
        }

        if type_name == "Err" {
            if arguments.len() != 1 {
                self.report(
                    "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
                    &format!(
                        "Constructor `Err` has {} argument(s), expected 1.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            let actual =
                self.infer_expression(file, constructor_arg_value(&arguments[0]), locals, line);
            if !self.compatible(&Type::Error, &actual) {
                self.report(
                    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                    &format!(
                        "Argument 1 for `Err` has type {}, expected Error.",
                        self.type_name(&actual)
                    ),
                    file,
                    line,
                );
            }
            return Type::Result(Box::new(Type::Unknown));
        }

        if let Some(info) = self.type_infos.get(type_name).cloned() {
            if !self.visible_from(file, info.visibility, &info.file_path) {
                self.report(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    &format!("Constructor `{type_name}` is not visible from this file."),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            if !matches!(info.kind, TypeDeclKind::Type) {
                self.report(
                    "TYPE_CONSTRUCTOR_REQUIRES_RECORD",
                    &format!(
                        "`{type_name}` is a {}, not a record TYPE.",
                        type_kind_name(info.kind)
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            self.check_constructor_arguments(
                file,
                type_name,
                &info.fields,
                &info.file_path,
                arguments,
                locals,
                line,
            );
            return Type::User(type_name.to_string());
        }

        if let Some(variants) = self.variant_constructors.get(type_name).cloned() {
            let selected = expected.and_then(|expected| {
                let Type::User(expected_name) = expected else {
                    return None;
                };
                variants
                    .iter()
                    .find(|variant| &variant.union_name == expected_name)
                    .cloned()
            });
            if variants.len() > 1 && selected.is_none() {
                self.report(
                    "TYPE_VARIANT_CONSTRUCTOR_AMBIGUOUS",
                    &format!(
                        "Variant constructor `{type_name}` is declared by more than one UNION."
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            let variant = selected.as_ref().unwrap_or(&variants[0]);
            if !self.visible_from(file, variant.visibility, &variant.file_path) {
                self.report(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    &format!("Variant constructor `{type_name}` is not visible from this file."),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            self.check_constructor_arguments(
                file,
                type_name,
                &variant.fields,
                &variant.file_path,
                arguments,
                locals,
                line,
            );
            return Type::User(variant.union_name.clone());
        }

        for argument in arguments {
            self.infer_expression(file, constructor_arg_value(argument), locals, line);
        }
        Type::Unknown
    }

    fn infer_with_update(
        &mut self,
        file: &AstFile,
        target: &Expression,
        updates: &[RecordUpdate],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let target_type = self.infer_expression(file, target, locals, line);
        let Type::User(type_name) = &target_type else {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "WITH update requires a record value, got {}.",
                    self.type_name(&target_type)
                ),
                file,
                line,
            );
            return Type::Unknown;
        };
        let Some(info) = self.type_infos.get(type_name).cloned() else {
            return Type::Unknown;
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "WITH update requires a record value, got {} `{type_name}`.",
                    type_kind_name(info.kind)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }
        let mut seen = HashSet::new();
        for update in updates {
            if !seen.insert(update.field.clone()) {
                self.report(
                    "TYPE_DUPLICATE_FIELD",
                    &format!("WITH update sets field `{}` more than once.", update.field),
                    file,
                    update.line,
                );
            }
            let Some(field) = info.fields.iter().find(|field| field.name == update.field) else {
                self.report(
                    "TYPE_UNKNOWN_FIELD",
                    &format!("TYPE `{type_name}` has no field `{}`.", update.field),
                    file,
                    update.line,
                );
                self.infer_expression(file, &update.value, locals, update.line);
                continue;
            };
            if !self.visible_from(file, field.visibility, &info.file_path) {
                self.report(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    &format!(
                        "Field `{type_name}::{}` is not visible from this file.",
                        update.field
                    ),
                    file,
                    update.line,
                );
            }
            let actual = self.infer_expression_with_expected(
                file,
                &update.value,
                locals,
                update.line,
                Some(&field.type_),
            );
            if !self.expression_compatible(&field.type_, &actual, Some(&update.value)) {
                self.report(
                    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                    &format!(
                        "WITH update for `{}` has type {}, expected {}.",
                        update.field,
                        self.type_name(&actual),
                        self.type_name(&field.type_)
                    ),
                    file,
                    update.line,
                );
            }
        }
        target_type
    }

    fn infer_member_access(
        &mut self,
        file: &AstFile,
        target: &Expression,
        member: &str,
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        if let Expression::Identifier(type_name) = target {
            if let Some(info) = self.type_infos.get(type_name).cloned() {
                if matches!(info.kind, TypeDeclKind::Enum) {
                    if !self.visible_from(file, info.visibility, &info.file_path) {
                        self.report(
                            "TYPE_MEMBER_NOT_VISIBLE",
                            &format!("ENUM `{type_name}` is not visible from this file."),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    }
                    if !info.members.contains(member) {
                        self.report(
                            "TYPE_UNKNOWN_ENUM_MEMBER",
                            &format!("ENUM `{type_name}` has no member `{member}`."),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    }
                    return Type::User(type_name.clone());
                }
            }
        }

        let target_type = self.infer_expression(file, target, locals, line);
        if let Type::Thread(_, output) = &target_type {
            if member == "result" {
                return Type::Result(output.clone());
            }
            self.report(
                "TYPE_UNKNOWN_FIELD",
                &format!("Thread value has no field `{member}`."),
                file,
                line,
            );
            return Type::Unknown;
        }
        let Type::User(type_name) = target_type else {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "Field access `{member}` requires a record value, got {}.",
                    self.type_name(&target_type)
                ),
                file,
                line,
            );
            return Type::Unknown;
        };
        let Some(info) = self.type_infos.get(&type_name).cloned() else {
            if let Some(variants) = self.variant_constructors.get(&type_name).cloned() {
                if let Some(field) = variants
                    .first()
                    .and_then(|variant| variant.fields.iter().find(|field| field.name == member))
                    .cloned()
                {
                    return field.type_;
                }
                self.report(
                    "TYPE_UNKNOWN_FIELD",
                    &format!("Variant `{type_name}` has no field `{member}`."),
                    file,
                    line,
                );
                return Type::Unknown;
            }
            if let Some(field_type) = builtins::io::builtin_type_fields(&type_name)
                .and_then(|fields| fields.iter().find(|(name, _)| *name == member))
                .map(|(_, type_name)| self.parse_type(type_name))
            {
                return field_type;
            }
            return Type::Unknown;
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
            self.report(
                "TYPE_FIELD_ACCESS_REQUIRES_RECORD",
                &format!(
                    "Field access `{member}` requires a record value, got {} `{type_name}`.",
                    type_kind_name(info.kind)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }
        let Some(field) = info
            .fields
            .iter()
            .find(|field| field.name == member)
            .cloned()
        else {
            self.report(
                "TYPE_UNKNOWN_FIELD",
                &format!("TYPE `{type_name}` has no field `{member}`."),
                file,
                line,
            );
            return Type::Unknown;
        };
        if !self.visible_from(file, field.visibility, &info.file_path) {
            self.report(
                "TYPE_MEMBER_NOT_VISIBLE",
                &format!("Field `{type_name}::{member}` is not visible from this file."),
                file,
                line,
            );
        }
        field.type_
    }

    fn check_constructor_arguments(
        &mut self,
        file: &AstFile,
        constructor: &str,
        fields: &[FieldInfo],
        owner_file_path: &str,
        arguments: &[ConstructorArg],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) {
        if arguments.len() != fields.len() {
            self.report(
                "TYPE_CONSTRUCTOR_ARITY_MISMATCH",
                &format!(
                    "Constructor `{constructor}` has {} argument(s), expected {}.",
                    arguments.len(),
                    fields.len()
                ),
                file,
                line,
            );
        }

        for field in fields {
            if !self.visible_from(file, field.visibility, owner_file_path) {
                self.report(
                    "TYPE_MEMBER_NOT_VISIBLE",
                    &format!(
                        "Constructor `{constructor}` cannot set hidden field `{}` from this file.",
                        field.name
                    ),
                    file,
                    line,
                );
            }
        }

        let mut seen_named = HashSet::new();
        for (index, argument) in arguments.iter().enumerate() {
            let (field, argument_value, argument_line) = match argument {
                ConstructorArg::Positional(value) => (fields.get(index), value, line),
                ConstructorArg::Named {
                    name,
                    value,
                    line: argument_line,
                } => {
                    if !seen_named.insert(name.clone()) {
                        self.report(
                            "TYPE_DUPLICATE_FIELD",
                            &format!(
                                "Constructor `{constructor}` sets field `{name}` more than once."
                            ),
                            file,
                            *argument_line,
                        );
                    }
                    (
                        fields.iter().find(|field| field.name == *name),
                        value,
                        *argument_line,
                    )
                }
            };
            let actual = self.infer_expression(file, argument_value, locals, argument_line);
            let Some(field) = field else {
                if let ConstructorArg::Named { name, .. } = argument {
                    self.report(
                        "TYPE_UNKNOWN_FIELD",
                        &format!("Constructor `{constructor}` has no field `{name}`."),
                        file,
                        argument_line,
                    );
                }
                continue;
            };
            if !self.expression_compatible(&field.type_, &actual, Some(argument_value)) {
                self.report(
                    "TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH",
                    &format!(
                        "Argument {} for `{constructor}` has type {}, expected {} for field `{}`.",
                        index + 1,
                        self.type_name(&actual),
                        self.type_name(&field.type_),
                        field.name
                    ),
                    file,
                    argument_line,
                );
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
        if matches!(operator, "AND" | "OR" | "XOR") {
            if self.compatible(&Type::Boolean, left) && self.compatible(&Type::Boolean, right) {
                return Type::Boolean;
            }
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `{operator}` requires Boolean operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }

        if matches!(operator, "=" | "<>") {
            if (self.is_numeric(left) && self.is_numeric(right))
                || self.compatible(left, right)
                || self.compatible(right, left)
            {
                return Type::Boolean;
            }
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `{operator}` requires compatible operands, got {} and {}.",
                    self.type_name(left),
                    self.type_name(right)
                ),
                file,
                line,
            );
            return Type::Unknown;
        }

        if matches!(operator, "<" | ">" | "<=" | ">=") {
            if self.is_numeric(left) && self.is_numeric(right) {
                return Type::Boolean;
            }
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
            return Type::Unknown;
        }

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
            numeric_binary_result_type(operator, left, right)
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

    fn infer_unary(&mut self, file: &AstFile, operator: &str, operand: &Type, line: usize) -> Type {
        match operator {
            "NOT" => {
                if self.compatible(&Type::Boolean, operand) {
                    Type::Boolean
                } else {
                    self.report(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        &format!(
                            "Operator `NOT` requires a Boolean operand, got {}.",
                            self.type_name(operand)
                        ),
                        file,
                        line,
                    );
                    Type::Unknown
                }
            }
            "-" => {
                if self.is_numeric(operand) {
                    operand.clone()
                } else {
                    self.report(
                        "TYPE_UNARY_OPERATOR_MISMATCH",
                        &format!(
                            "Unary `-` requires a numeric operand, got {}.",
                            self.type_name(operand)
                        ),
                        file,
                        line,
                    );
                    Type::Unknown
                }
            }
            _ => {
                self.report(
                    "TYPE_UNARY_OPERATOR_UNKNOWN",
                    &format!("Unknown unary operator `{operator}`."),
                    file,
                    line,
                );
                Type::Unknown
            }
        }
    }

    fn check_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        sig: &FunctionSig,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
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
            if !self.expression_compatible(&param.type_, &actual, Some(argument)) {
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

    fn check_function_value_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        type_: &Type,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let Type::Function {
            params,
            return_type,
            ..
        } = type_
        else {
            self.report(
                "SYMBOL_NOT_CALLABLE",
                &format!("Local binding or parameter `{callee}` is not callable."),
                file,
                line,
            );
            for argument in arguments {
                self.infer_expression(file, argument, locals, line);
            }
            return Type::Unknown;
        };

        if arguments.len() != params.len() {
            self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{callee}` has {} argument(s), expected {}.",
                    arguments.len(),
                    params.len()
                ),
                file,
                line,
            );
        }

        for (index, argument) in arguments.iter().enumerate() {
            let actual = self.infer_expression(file, argument, locals, line);
            let Some(expected) = params.get(index) else {
                continue;
            };
            if !self.expression_compatible(expected, &actual, Some(argument)) {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    &format!(
                        "Argument {} for `{callee}` has type {}, expected {}.",
                        index + 1,
                        self.type_name(&actual),
                        self.type_name(expected)
                    ),
                    file,
                    line,
                );
            }
        }

        *return_type.clone()
    }

    fn infer_lambda(
        &mut self,
        file: &AstFile,
        params: &[crate::ast::Param],
        body: &Expression,
        outer_locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let mut locals = outer_locals.clone();
        let mut param_types = Vec::new();
        for param in params {
            let type_ = param
                .type_name
                .as_deref()
                .map(|name| self.parse_type(name))
                .unwrap_or(Type::Unknown);
            if param.type_name.is_none() {
                self.report(
                    "TYPE_PARAMETER_MISSING",
                    &format!(
                        "Lambda parameter `{}` must declare an `AS` type.",
                        param.name
                    ),
                    file,
                    param.line,
                );
            }
            if param.default.is_some() {
                self.report(
                    "TYPE_DEFAULT_ARGUMENT_ORDER",
                    "Lambda parameters cannot declare default values.",
                    file,
                    param.line,
                );
            }
            locals.insert(
                param.name.clone(),
                LocalInfo {
                    type_: type_.clone(),
                    mutable: false,
                },
            );
            param_types.push(type_);
        }
        let param_names = params
            .iter()
            .map(|param| param.name.clone())
            .collect::<HashSet<_>>();
        let captures = captured_locals(body, outer_locals, &param_names);
        for capture in captures {
            if outer_locals
                .get(&capture)
                .is_some_and(|local| local.mutable)
            {
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures mutable local `{capture}`; mutable captures are invalid."
                    ),
                    file,
                    line,
                );
            } else {
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures local `{capture}`, but closure environments are not supported yet."
                    ),
                    file,
                    line,
                );
            }
        }
        let return_type = self.infer_expression(file, body, &locals, line);
        Type::Function {
            params: param_types,
            return_type: Box::new(return_type),
            isolated: false,
        }
    }

    fn check_builtin_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        if builtins::general::is_general_call(callee) {
            return self.check_general_builtin_call(file, callee, arguments, locals, line);
        }
        if builtins::strings::is_strings_call(callee) {
            return self.check_strings_builtin_call(file, callee, arguments, locals, line);
        }
        if builtins::math::is_math_call(callee) {
            return self.check_math_builtin_call(file, callee, arguments, locals, line);
        }
        if builtins::fs::is_fs_call(callee) {
            return self.check_fs_builtin_call(file, callee, arguments, locals, line);
        }
        if builtins::io::is_io_call(callee) {
            return self.check_io_builtin_call(file, callee, arguments, locals, line);
        }
        if builtins::thread::is_thread_call(callee) {
            return self.check_thread_builtin_call(file, callee, arguments, locals, line);
        }

        for argument in arguments {
            self.infer_expression(file, argument, locals, line);
        }
        Type::Unknown
    }

    fn check_fs_builtin_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::fs::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::fs::resolve_call(callee, &arg_types) else {
            let expected = builtins::fs::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_io_builtin_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::io::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    if min == 0 {
                        "0".to_string()
                    } else {
                        min.to_string()
                    }
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::io::resolve_call(callee, &arg_types) else {
            let expected = builtins::io::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_thread_builtin_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if callee == "thread.start" {
            let valid_entry = matches!(
                arguments.first(),
                Some(Expression::Identifier(name)) if name.contains('.')
            );
            if !valid_entry {
                self.report(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    "thread.start entry point must be an exported ISOLATED FUNC from an imported package.",
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        if let Some((min, max)) = builtins::thread::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::thread::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::thread::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_strings_builtin_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::strings::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::strings::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::strings::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_math_builtin_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::math::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::math::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::math::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_general_builtin_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[Expression],
        locals: &HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        if callee == "filter" && arguments.len() == 2 {
            if let Expression::Identifier(predicate) = &arguments[1] {
                if builtins::general::builtin_function_id(predicate).is_some() {
                    let collection_type = self.infer_expression(file, &arguments[0], locals, line);
                    let collection_type_name = self.type_name(&collection_type);
                    let predicate_type =
                        collection_type_name
                            .strip_prefix("List OF ")
                            .and_then(|element| {
                                builtins::general::filter_predicate_type(predicate, element)
                            });

                    let Some(predicate_type) = predicate_type else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `filter` has argument type(s) ({collection_type_name}, {predicate}), expected {}.",
                                builtins::general::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    let arg_types = vec![collection_type_name, predicate_type];
                    let Some(resolved) = builtins::general::resolve_call(callee, &arg_types) else {
                        self.report(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            &format!(
                                "Call to `filter` has argument type(s) ({}), expected {}.",
                                arg_types.join(", "),
                                builtins::general::expected_arguments(callee)
                                    .unwrap_or("supported overload")
                            ),
                            file,
                            line,
                        );
                        return Type::Unknown;
                    };

                    return self.parse_type(&resolved.return_type);
                }
            }
        }

        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::general::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::general::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::general::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn parse_type(&self, name: &str) -> Type {
        if let Some(rest) = name.strip_prefix("ISOLATED FUNC(") {
            return self.parse_function_type(rest, true);
        }
        if let Some(rest) = name.strip_prefix("FUNC(") {
            return self.parse_function_type(rest, false);
        }
        if let Some(element) = name.strip_prefix("List OF ") {
            return Type::List(Box::new(self.parse_type(element)));
        }
        if let Some(success) = name.strip_prefix("Result OF ") {
            return Type::Result(Box::new(self.parse_type(success)));
        }
        if let Some(rest) = name.strip_prefix("Thread OF ") {
            if let Some((message, output)) = rest.split_once(" TO ") {
                return Type::Thread(
                    Box::new(self.parse_type(message)),
                    Box::new(self.parse_type(output)),
                );
            }
        }
        if let Some(rest) = name.strip_prefix("Map OF ") {
            if let Some((key, value)) = rest.split_once(" TO ") {
                return Type::Map(
                    Box::new(self.parse_type(key)),
                    Box::new(self.parse_type(value)),
                );
            }
        }

        match name {
            "Boolean" => Type::Boolean,
            "Byte" => Type::Byte,
            "Error" => Type::Error,
            "Fixed" => Type::Fixed,
            "Float" => Type::Float,
            "Integer" => Type::Integer,
            "Nothing" => Type::Nothing,
            "String" => Type::String,
            "Unknown" => Type::Unknown,
            "Result" => Type::Result(Box::new(Type::Unknown)),
            other if builtins::is_builtin_type(other) => Type::User(other.to_string()),
            other if self.user_types.contains(other) => Type::User(other.to_string()),
            other => Type::User(other.to_string()),
        }
    }

    fn parse_function_type(&self, rest: &str, isolated: bool) -> Type {
        let Some((params, return_type)) = rest.split_once(") AS ") else {
            return Type::Unknown;
        };
        let params = if params.trim().is_empty() {
            Vec::new()
        } else {
            params
                .split(", ")
                .map(|param| self.parse_type(param))
                .collect()
        };
        Type::Function {
            params,
            return_type: Box::new(self.parse_type(return_type)),
            isolated,
        }
    }

    fn compatible(&self, expected: &Type, actual: &Type) -> bool {
        if matches!(expected, Type::Unknown) || matches!(actual, Type::Unknown) {
            return true;
        }
        match (expected, actual) {
            (Type::List(expected), Type::List(actual)) => self.compatible(expected, actual),
            (Type::Map(expected_key, expected_value), Type::Map(actual_key, actual_value)) => {
                self.compatible(expected_key, actual_key)
                    && self.compatible(expected_value, actual_value)
            }
            (Type::Result(expected), Type::Result(actual)) => self.compatible(expected, actual),
            (
                Type::Thread(expected_message, expected_output),
                Type::Thread(actual_message, actual_output),
            ) => {
                self.compatible(expected_message, actual_message)
                    && self.compatible(expected_output, actual_output)
            }
            (
                Type::Function {
                    params: expected_params,
                    return_type: expected_return,
                    isolated: expected_isolated,
                },
                Type::Function {
                    params: actual_params,
                    return_type: actual_return,
                    isolated: actual_isolated,
                },
            ) => {
                (!expected_isolated || *actual_isolated)
                    && expected_params.len() == actual_params.len()
                    && expected_params
                        .iter()
                        .zip(actual_params.iter())
                        .all(|(expected, actual)| self.compatible(expected, actual))
                    && self.compatible(expected_return, actual_return)
            }
            _ => expected == actual,
        }
    }

    fn expression_compatible(
        &self,
        expected: &Type,
        actual: &Type,
        expression: Option<&Expression>,
    ) -> bool {
        if self.compatible(expected, actual) {
            return true;
        }
        match (expected, actual, expression) {
            (Type::Byte, Type::Integer, Some(Expression::Number(value))) => value
                .parse::<u16>()
                .is_ok_and(|number| number <= u8::MAX as u16),
            (Type::Fixed, Type::Integer | Type::Float, Some(Expression::Number(_))) => true,
            (
                Type::Fixed,
                Type::Integer | Type::Float,
                Some(Expression::Unary { operator, operand }),
            ) if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) => true,
            (
                Type::List(expected_element),
                Type::List(_),
                Some(Expression::ListLiteral(values)),
            ) => values.iter().all(|value| {
                let Some(actual_element) = numeric_literal_type(value) else {
                    return false;
                };
                self.expression_compatible(expected_element, &actual_element, Some(value))
            }),
            _ => false,
        }
    }

    fn is_numeric(&self, type_: &Type) -> bool {
        matches!(
            type_,
            Type::Byte | Type::Fixed | Type::Float | Type::Integer | Type::Unknown
        )
    }

    fn visible_from(&self, file: &AstFile, visibility: Visibility, owner_file_path: &str) -> bool {
        match visibility {
            Visibility::Export | Visibility::Package => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    fn check_type_reference(&mut self, file: &AstFile, type_: &Type, line: usize) {
        match type_ {
            Type::List(element) => self.check_type_reference(file, element, line),
            Type::Map(key, value) => {
                self.check_type_reference(file, key, line);
                self.check_type_reference(file, value, line);
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.check_type_reference(file, param, line);
                }
                self.check_type_reference(file, return_type, line);
            }
            Type::Result(success) => self.check_type_reference(file, success, line),
            Type::Thread(message, output) => {
                self.check_type_reference(file, message, line);
                self.check_type_reference(file, output, line);
            }
            Type::User(name) => {
                let Some(info) = self.type_infos.get(name) else {
                    return;
                };
                if !self.visible_from(file, info.visibility, &info.file_path) {
                    self.report(
                        "TYPE_MEMBER_NOT_VISIBLE",
                        &format!("Type `{name}` is not visible from this file."),
                        file,
                        line,
                    );
                }
            }
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => {}
        }
    }

    fn type_name(&self, type_: &Type) -> String {
        match type_ {
            Type::Boolean => "Boolean".to_string(),
            Type::Byte => "Byte".to_string(),
            Type::Error => "Error".to_string(),
            Type::Fixed => "Fixed".to_string(),
            Type::Float => "Float".to_string(),
            Type::Integer => "Integer".to_string(),
            Type::List(element) => format!("List OF {}", self.type_name(element)),
            Type::Map(key, value) => {
                format!(
                    "Map OF {} TO {}",
                    self.type_name(key),
                    self.type_name(value)
                )
            }
            Type::Function {
                params,
                return_type,
                isolated,
            } => {
                let params = params
                    .iter()
                    .map(|param| self.type_name(param))
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{}FUNC({}) AS {}",
                    if *isolated { "ISOLATED " } else { "" },
                    params,
                    self.type_name(return_type)
                )
            }
            Type::Nothing => "Nothing".to_string(),
            Type::Result(success) => format!("Result OF {}", self.type_name(success)),
            Type::String => "String".to_string(),
            Type::Thread(message, output) => {
                format!(
                    "Thread OF {} TO {}",
                    self.type_name(message),
                    self.type_name(output)
                )
            }
            Type::User(name) => name.clone(),
            Type::Unknown => "Unknown".to_string(),
        }
    }

    fn report(&mut self, rule: &str, detail: &str, file: &AstFile, line: usize) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, &self.project_dir.join(&file.path), line, 1, 1);
    }

    fn report_primitive_literal_range_error(
        &mut self,
        expected: &Type,
        expression: &Expression,
        file: &AstFile,
        line: usize,
    ) -> bool {
        match expected {
            Type::Byte => {
                let Some(range_error) = byte_literal_range_error(expression) else {
                    return false;
                };
                match range_error {
                    ByteLiteralRangeError::Overflow(value) => self.report(
                        "TYPE_BYTE_LITERAL_OVERFLOW",
                        &format!("Integer literal `{value}` is outside the Byte range 0..255."),
                        file,
                        line,
                    ),
                    ByteLiteralRangeError::Underflow(value) => self.report(
                        "TYPE_BYTE_LITERAL_UNDERFLOW",
                        &format!("Integer literal `{value}` is outside the Byte range 0..255."),
                        file,
                        line,
                    ),
                }
                true
            }
            Type::Float => {
                let Some(range_error) = float_literal_range_error(expression) else {
                    return false;
                };
                match range_error {
                    SignedLiteralRangeError::Overflow(value) => self.report(
                        "TYPE_FLOAT_LITERAL_OVERFLOW",
                        &format!("Numeric literal `{value}` is outside the Float range."),
                        file,
                        line,
                    ),
                    SignedLiteralRangeError::Underflow(value) => self.report(
                        "TYPE_FLOAT_LITERAL_UNDERFLOW",
                        &format!("Numeric literal `{value}` is outside the Float range."),
                        file,
                        line,
                    ),
                }
                true
            }
            Type::Fixed => {
                let Some(range_error) = fixed_literal_range_error(expression) else {
                    return false;
                };
                match range_error {
                    SignedLiteralRangeError::Overflow(value) => self.report(
                        "TYPE_FIXED_LITERAL_OVERFLOW",
                        &format!("Numeric literal `{value}` is outside the Fixed range."),
                        file,
                        line,
                    ),
                    SignedLiteralRangeError::Underflow(value) => self.report(
                        "TYPE_FIXED_LITERAL_UNDERFLOW",
                        &format!("Numeric literal `{value}` is outside the Fixed range."),
                        file,
                        line,
                    ),
                }
                true
            }
            _ => false,
        }
    }
}

enum ByteLiteralRangeError<'a> {
    Overflow(&'a str),
    Underflow(String),
}

enum SignedLiteralRangeError {
    Overflow(String),
    Underflow(String),
}

fn byte_literal_range_error(expression: &Expression) -> Option<ByteLiteralRangeError<'_>> {
    match expression {
        Expression::Number(value) if !value.contains('.') => value
            .parse::<u16>()
            .map_or(Some(ByteLiteralRangeError::Overflow(value)), |number| {
                (number > u8::MAX as u16).then_some(ByteLiteralRangeError::Overflow(value))
            }),
        Expression::Unary { operator, operand } if operator == "-" => {
            let Expression::Number(value) = operand.as_ref() else {
                return None;
            };
            if value.contains('.') {
                return None;
            }
            let Ok(number) = value.parse::<u128>() else {
                return Some(ByteLiteralRangeError::Underflow(format!("-{value}")));
            };
            (number != 0).then_some(ByteLiteralRangeError::Underflow(format!("-{value}")))
        }
        _ => None,
    }
}

fn float_literal_range_error(expression: &Expression) -> Option<SignedLiteralRangeError> {
    let (text, negative) = signed_numeric_literal(expression)?;
    let parsed = text.parse::<f64>().ok()?;
    if parsed.is_finite() {
        return None;
    }
    if negative {
        Some(SignedLiteralRangeError::Underflow(format!("-{text}")))
    } else {
        Some(SignedLiteralRangeError::Overflow(text.to_string()))
    }
}

fn fixed_literal_range_error(expression: &Expression) -> Option<SignedLiteralRangeError> {
    let (text, negative) = signed_numeric_literal(expression)?;
    let parsed = text.parse::<f64>().ok()?;
    let value = if negative { -parsed } else { parsed };
    if value < -2147483648.0 {
        Some(SignedLiteralRangeError::Underflow(format!("-{text}")))
    } else if value >= 2147483648.0 {
        Some(SignedLiteralRangeError::Overflow(text.to_string()))
    } else {
        None
    }
}

fn signed_numeric_literal(expression: &Expression) -> Option<(&str, bool)> {
    match expression {
        Expression::Number(value) => Some((value.as_str(), false)),
        Expression::Unary { operator, operand } if operator == "-" => {
            let Expression::Number(value) = operand.as_ref() else {
                return None;
            };
            Some((value.as_str(), true))
        }
        _ => None,
    }
}

fn integer_literal_in_range(expression: &Expression) -> bool {
    match expression {
        Expression::Number(value) if !value.contains('.') => value.parse::<i64>().is_ok(),
        Expression::Unary { operator, operand } if operator == "-" => {
            let Expression::Number(value) = operand.as_ref() else {
                return true;
            };
            if value.contains('.') {
                return true;
            }
            value
                .parse::<u64>()
                .is_ok_and(|number| number <= (i64::MAX as u64) + 1)
        }
        _ => true,
    }
}

fn effective_field_visibility(
    declared: Option<Visibility>,
    containing_visibility: Visibility,
) -> Visibility {
    declared.unwrap_or(match containing_visibility {
        Visibility::Export => Visibility::Export,
        Visibility::Package | Visibility::Private => Visibility::Package,
    })
}

fn function_type(sig: &FunctionSig) -> Type {
    Type::Function {
        params: sig.params.iter().map(|param| param.type_.clone()).collect(),
        return_type: Box::new(sig.return_type.clone()),
        isolated: sig.isolated,
    }
}

fn captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, LocalInfo>,
    local_names: &HashSet<String>,
) -> HashSet<String> {
    let mut captures = HashSet::new();
    collect_captured_locals(expression, outer_locals, local_names, &mut captures);
    captures
}

fn collect_captured_locals(
    expression: &Expression,
    outer_locals: &HashMap<String, LocalInfo>,
    local_names: &HashSet<String>,
    captures: &mut HashSet<String>,
) {
    match expression {
        Expression::Identifier(name) => {
            if outer_locals.contains_key(name) && !local_names.contains(name) {
                captures.insert(name.clone());
            }
        }
        Expression::Call { callee, arguments } => {
            if outer_locals.contains_key(callee) && !local_names.contains(callee) {
                captures.insert(callee.clone());
            }
            for argument in arguments {
                collect_captured_locals(argument, outer_locals, local_names, captures);
            }
        }
        Expression::Lambda { .. } => {}
        Expression::Binary { left, right, .. } => {
            collect_captured_locals(left, outer_locals, local_names, captures);
            collect_captured_locals(right, outer_locals, local_names, captures);
        }
        Expression::Unary { operand, .. } => {
            collect_captured_locals(operand, outer_locals, local_names, captures);
        }
        Expression::Constructor { arguments, .. } => {
            for argument in arguments {
                collect_captured_locals(
                    constructor_arg_value(argument),
                    outer_locals,
                    local_names,
                    captures,
                );
            }
        }
        Expression::ListLiteral(values) => {
            for value in values {
                collect_captured_locals(value, outer_locals, local_names, captures);
            }
        }
        Expression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_captured_locals(key, outer_locals, local_names, captures);
                collect_captured_locals(value, outer_locals, local_names, captures);
            }
        }
        Expression::MemberAccess { target, .. } => {
            collect_captured_locals(target, outer_locals, local_names, captures);
        }
        Expression::WithUpdate { target, updates } => {
            collect_captured_locals(target, outer_locals, local_names, captures);
            for update in updates {
                collect_captured_locals(&update.value, outer_locals, local_names, captures);
            }
        }
        Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
    }
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}

fn type_kind_name(kind: TypeDeclKind) -> &'static str {
    match kind {
        TypeDeclKind::Type => "TYPE",
        TypeDeclKind::Union => "UNION",
        TypeDeclKind::Enum => "ENUM",
    }
}

fn numeric_literal_type(expression: &Expression) -> Option<Type> {
    match expression {
        Expression::Number(number) if number.contains('.') => Some(Type::Float),
        Expression::Number(_) => Some(Type::Integer),
        Expression::Unary { operator, operand }
            if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) =>
        {
            numeric_literal_type(operand)
        }
        _ => None,
    }
}

fn numeric_binary_result_type(operator: &str, left: &Type, right: &Type) -> Type {
    let Some(left) = numeric_type_name(left) else {
        return Type::Unknown;
    };
    let Some(right) = numeric_type_name(right) else {
        return Type::Unknown;
    };
    match numeric::binary_result_type(operator, left, right) {
        Some("Byte") => Type::Byte,
        Some("Fixed") => Type::Fixed,
        Some("Float") => Type::Float,
        Some("Integer") => Type::Integer,
        _ => Type::Unknown,
    }
}

fn numeric_type_name(type_: &Type) -> Option<&'static str> {
    match type_ {
        Type::Byte => Some(numeric::TYPE_BYTE),
        Type::Fixed => Some(numeric::TYPE_FIXED),
        Type::Float => Some(numeric::TYPE_FLOAT),
        Type::Integer => Some(numeric::TYPE_INTEGER),
        _ => None,
    }
}

fn variant_name_key(variant: &VariantConstructor) -> String {
    variant.name.clone()
}
