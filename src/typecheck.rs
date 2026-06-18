use crate::ast::{
    AstFile, AstProject, CallArg, ConstructorArg, Expression, Function, FunctionKind, Item,
    MatchPattern, RecordUpdate, Statement, TopLevelBinding, TypeDecl, TypeDeclKind, TypeField,
    Visibility,
};
use crate::builtins;
use crate::bytecode::{
    self, BytecodeExportKind, BytecodeTypeExport, BytecodeTypeField, BytecodeTypeVariant,
    BytecodeTypeVisibility,
};
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
    ThreadWorker(Box<Type>, Box<Type>),
    User(String),
    Unknown,
}

#[derive(Clone)]
struct LocalInfo {
    type_: Type,
    mutable: bool,
    ownership: OwnershipState,
    scope_guard: ScopeGuard,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OwnershipState {
    Available,
    Moved,
    MaybeMoved,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScopeGuard {
    None,
    MustRemainOwned,
}

#[derive(Clone)]
struct CapturedLocal {
    name: String,
    type_: Type,
    mutable: bool,
}

#[derive(Clone)]
struct FunctionSig {
    kind: FunctionKind,
    params: Vec<ParamSig>,
    return_type: Type,
    isolated: bool,
    imported_package_export: bool,
    visibility: Visibility,
    owner_file_path: String,
}

#[derive(Clone)]
struct BindingSig {
    type_: Type,
    mutable: bool,
    visibility: Visibility,
    owner_file_path: String,
}

#[derive(Clone)]
struct ParamSig {
    name: String,
    type_: Type,
    has_default: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Flow {
    FallsThrough,
    AlwaysReturns,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExprMode {
    Read,
    Transfer,
    Borrow,
}

pub fn check_project(project_dir: &Path, ast: &AstProject) -> Result<(), ()> {
    let augmented = builtins::json::augmented_project(ast)?;
    let mut checker = TypeChecker::new(project_dir, &augmented);
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
    functions: HashMap<String, Vec<FunctionSig>>,
    bindings: HashMap<String, BindingSig>,
    user_types: HashSet<String>,
    user_type_kinds: HashMap<String, TypeDeclKind>,
    type_infos: HashMap<String, TypeInfo>,
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
            bindings: HashMap::new(),
            user_types: HashSet::new(),
            user_type_kinds: HashMap::new(),
            type_infos: HashMap::new(),
            had_error: false,
        };
        checker.collect_types();
        checker.collect_package_types();
        checker.collect_bindings();
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
    }

    fn collect_package_types(&mut self) {
        let mut seen = HashSet::new();
        for file in &self.ast.files {
            for import in &file.imports {
                let package = import.package_name().to_string();
                if !seen.insert(package.clone()) || builtins::is_builtin_import(&package) {
                    continue;
                }
                let package_file = self
                    .project_dir
                    .join("packages")
                    .join(format!("{package}.mfp"));
                if !package_file.is_file() {
                    continue;
                }
                let Ok(type_exports) = bytecode::read_package_type_exports(&package_file) else {
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{package}` has unreadable or invalid type metadata."
                        ),
                        file,
                        import.line,
                    );
                    continue;
                };
                for type_export in &type_exports {
                    self.install_package_type_info(&package_file, type_export.clone());
                }
                for type_export in type_exports {
                    self.validate_imported_package_type(
                        file,
                        import.line,
                        &package_file,
                        &type_export,
                    );
                }
            }
        }
    }

    fn validate_imported_package_type(
        &mut self,
        file: &AstFile,
        line: usize,
        package_file: &Path,
        type_export: &BytecodeTypeExport,
    ) {
        let mut seen = HashSet::new();
        match type_export.kind {
            BytecodeExportKind::Type => {
                let type_ = Type::User(type_export.name.clone());
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    &type_,
                    &format!("exported type `{}`", type_export.name),
                    &mut seen,
                );
            }
            BytecodeExportKind::Union => {
                let type_ = Type::User(type_export.name.clone());
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    &type_,
                    &format!("exported union `{}`", type_export.name),
                    &mut seen,
                );
            }
            BytecodeExportKind::Enum => {}
            BytecodeExportKind::Func | BytecodeExportKind::Sub => {}
        }
    }

    fn validate_package_metadata_type(
        &mut self,
        file: &AstFile,
        line: usize,
        package_file: &Path,
        type_: &Type,
        context: &str,
        seen: &mut HashSet<String>,
    ) {
        match type_ {
            Type::List(element) | Type::Result(element) => {
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    element,
                    context,
                    seen,
                );
            }
            Type::Map(key, value) => {
                self.validate_package_metadata_type(file, line, package_file, key, context, seen);
                self.validate_package_metadata_type(file, line, package_file, value, context, seen);
                if !self.is_comparable(key) {
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{}` has {context} with non-comparable map key type `{}`.",
                            package_file.display(),
                            self.type_name(key)
                        ),
                        file,
                        line,
                    );
                }
            }
            Type::Function {
                params,
                return_type,
                ..
            } => {
                for param in params {
                    self.validate_package_metadata_type(
                        file,
                        line,
                        package_file,
                        param,
                        context,
                        seen,
                    );
                }
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    return_type,
                    context,
                    seen,
                );
            }
            Type::Thread(message, output) | Type::ThreadWorker(message, output) => {
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    message,
                    context,
                    seen,
                );
                self.validate_package_metadata_type(
                    file,
                    line,
                    package_file,
                    output,
                    context,
                    seen,
                );
            }
            Type::User(name) => {
                if builtins::is_resource_type(name) || !seen.insert(name.clone()) {
                    return;
                }
                let Some(info) = self.type_infos.get(name).cloned() else {
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{}` has {context} that references unknown type `{name}`.",
                            package_file.display()
                        ),
                        file,
                        line,
                    );
                    return;
                };
                match info.kind {
                    TypeDeclKind::Enum => {}
                    TypeDeclKind::Type => {
                        for field in &info.fields {
                            self.validate_package_metadata_type(
                                file,
                                line,
                                package_file,
                                &field.type_,
                                context,
                                seen,
                            );
                        }
                    }
                    TypeDeclKind::Union => {
                        for variant in &info.variants {
                            for field in &variant.fields {
                                self.validate_package_metadata_type(
                                    file,
                                    line,
                                    package_file,
                                    &field.type_,
                                    context,
                                    seen,
                                );
                            }
                        }
                    }
                }
                seen.remove(name);
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

    fn collect_package_functions(&mut self) {
        let mut seen = HashSet::new();
        for file in &self.ast.files {
            for import in &file.imports {
                let binding = import.binding_name().to_string();
                let package = import.package_name().to_string();
                if !seen.insert(binding.clone()) || builtins::is_builtin_import(&package) {
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
                    self.report(
                        "PACKAGE_INVALID",
                        &format!(
                            "Imported package `{package}` has unreadable or invalid function metadata."
                        ),
                        file,
                        import.line,
                    );
                    continue;
                };
                for export in exports {
                    let sig = FunctionSig {
                        kind: match export.kind {
                            BytecodeExportKind::Func => FunctionKind::Func,
                            BytecodeExportKind::Sub => FunctionKind::Sub,
                            BytecodeExportKind::Type
                            | BytecodeExportKind::Union
                            | BytecodeExportKind::Enum => continue,
                        },
                        params: export
                            .params
                            .into_iter()
                            .map(|param| ParamSig {
                                name: param.name,
                                type_: self.parse_type(&param.type_),
                                has_default: param.has_default,
                            })
                            .collect(),
                        return_type: self.parse_type(&export.return_type),
                        isolated: export.isolated,
                        imported_package_export: true,
                        visibility: Visibility::Export,
                        owner_file_path: package_file.display().to_string(),
                    };
                    self.validate_imported_function_signature(
                        file,
                        import.line,
                        &package_file,
                        &export.name,
                        &sig,
                    );
                    self.functions
                        .entry(format!("{binding}.{}", export.name))
                        .or_default()
                        .push(sig);
                }
            }
        }
    }

    fn validate_imported_function_signature(
        &mut self,
        file: &AstFile,
        line: usize,
        package_file: &Path,
        function_name: &str,
        sig: &FunctionSig,
    ) {
        let mut seen = HashSet::new();
        for param in &sig.params {
            self.validate_package_metadata_type(
                file,
                line,
                package_file,
                &param.type_,
                &format!(
                    "exported function `{function_name}` parameter `{}`",
                    param.name
                ),
                &mut seen,
            );
        }
        self.validate_package_metadata_type(
            file,
            line,
            package_file,
            &sig.return_type,
            &format!("exported function `{function_name}` return type"),
            &mut seen,
        );
    }

    fn install_package_type_info(&mut self, package_file: &Path, type_export: BytecodeTypeExport) {
        let BytecodeTypeExport {
            name,
            kind,
            fields,
            variants,
            members,
        } = type_export;
        self.user_types.insert(name.clone());
        let kind = match kind {
            BytecodeExportKind::Type => TypeDeclKind::Type,
            BytecodeExportKind::Union => TypeDeclKind::Union,
            BytecodeExportKind::Enum => TypeDeclKind::Enum,
            BytecodeExportKind::Func | BytecodeExportKind::Sub => return,
        };
        self.user_type_kinds.insert(name.clone(), kind);
        self.type_infos.insert(
            name,
            TypeInfo {
                kind,
                visibility: Visibility::Export,
                file_path: package_file.display().to_string(),
                fields: fields
                    .into_iter()
                    .map(|field| self.package_field_info(field))
                    .collect(),
                variants: variants
                    .into_iter()
                    .map(|variant| self.package_variant_info(variant))
                    .collect(),
                members: members.into_iter().collect(),
            },
        );
    }

    fn package_field_info(&self, field: BytecodeTypeField) -> FieldInfo {
        FieldInfo {
            name: field.name,
            type_: self.parse_type(&field.type_),
            visibility: match field.visibility {
                BytecodeTypeVisibility::Private => Visibility::Private,
                BytecodeTypeVisibility::Package => Visibility::Package,
                BytecodeTypeVisibility::Export => Visibility::Export,
            },
        }
    }

    fn package_variant_info(&self, variant: BytecodeTypeVariant) -> VariantConstructor {
        VariantConstructor {
            name: variant.name,
            union_name: String::new(),
            visibility: Visibility::Export,
            file_path: String::new(),
            fields: variant
                .fields
                .into_iter()
                .map(|field| self.package_field_info(field))
                .collect(),
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
            variants.extend(info.variants.iter().map(|variant| {
                let mut expanded = variant.clone();
                expanded.fields = self
                    .type_infos
                    .get(&variant.name)
                    .filter(|member| matches!(member.kind, TypeDeclKind::Type))
                    .map(|member| member.fields.clone())
                    .unwrap_or_default();
                expanded
            }));
        }
        visiting.remove(union_name);
        variants
    }

    fn report_expanded_union_member_conflicts(&mut self, file: &AstFile, type_decl: &TypeDecl) {
        let mut included_members: HashMap<String, String> = HashMap::new();
        for include in &type_decl.includes {
            for variant in self.expanded_union_variants(include, &mut HashSet::new()) {
                if let Some(previous_include) =
                    included_members.insert(variant.name.clone(), include.clone())
                {
                    self.report(
                        "TYPE_DUPLICATE_VARIANT",
                        &format!(
                            "Member type `{}` in UNION `{}` is provided by both included UNION `{}` and included UNION `{}`.",
                            variant.name, type_decl.name, previous_include, include
                        ),
                        file,
                        type_decl.line,
                    );
                }
            }
        }

        for variant in &type_decl.variants {
            if let Some(include) = included_members.get(&variant.name) {
                self.report(
                    "TYPE_DUPLICATE_VARIANT",
                    &format!(
                        "Member type `{}` in UNION `{}` conflicts with a member included from UNION `{}`.",
                        variant.name, type_decl.name, include
                    ),
                    file,
                    variant.line,
                );
            }
        }
    }

    fn direct_record_successors(&self, name: &str) -> Vec<String> {
        let Some(info) = self.type_infos.get(name) else {
            return Vec::new();
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
            return Vec::new();
        }
        info.fields
            .iter()
            .filter_map(|field| match &field.type_ {
                Type::User(type_name)
                    if matches!(
                        self.type_infos.get(type_name).map(|info| info.kind),
                        Some(TypeDeclKind::Type)
                    ) =>
                {
                    Some(type_name.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn record_field_cycle(&self, start: &str) -> bool {
        let mut visited = HashSet::new();
        let mut stack = self.direct_record_successors(start);
        while let Some(node) = stack.pop() {
            if node == start {
                return true;
            }
            if !visited.insert(node.clone()) {
                continue;
            }
            stack.extend(self.direct_record_successors(&node));
        }
        false
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
                fields: Vec::new(),
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

    fn collect_bindings(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                if let Item::Binding(binding) = item {
                    let type_ = binding
                        .type_name
                        .as_deref()
                        .map(|name| self.parse_type(name))
                        .unwrap_or(Type::Unknown);
                    self.bindings.insert(
                        binding.name.clone(),
                        BindingSig {
                            type_,
                            mutable: binding.mutable,
                            visibility: binding.visibility,
                            owner_file_path: file.path.clone(),
                        },
                    );
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
                            name: param.name.clone(),
                            type_: param
                                .type_name
                                .as_deref()
                                .map(|name| self.parse_type(name))
                                .unwrap_or(Type::Unknown),
                            has_default: param.default.is_some(),
                        })
                        .collect();
                    self.functions
                        .entry(function.name.clone())
                        .or_default()
                        .push(FunctionSig {
                            kind: function.kind.clone(),
                            params,
                            return_type,
                            isolated: function.isolated,
                            imported_package_export: false,
                            visibility: function.visibility,
                            owner_file_path: file.path.clone(),
                        });
                }
            }
        }
    }

    fn canonical_import_name(&self, file: &AstFile, name: &str) -> String {
        let Some((binding, rest)) = name.split_once('.') else {
            return name.to_string();
        };
        let imports = file.import_bindings();
        let Some(package) = imports.get(binding) else {
            return name.to_string();
        };
        format!("{package}.{rest}")
    }

    fn visible_function_sigs<'b>(&'b self, file: &AstFile, name: &str) -> Vec<&'b FunctionSig> {
        self.functions
            .get(name)
            .into_iter()
            .flatten()
            .filter(|sig| self.visible_from(file, sig.visibility, &sig.owner_file_path))
            .collect()
    }

    fn lookup_visible_function<'b>(
        &'b self,
        file: &AstFile,
        name: &str,
    ) -> Option<&'b FunctionSig> {
        let visible = self.visible_function_sigs(file, name);
        if visible.len() == 1 {
            return visible.into_iter().next();
        }
        visible.into_iter().last()
    }

    fn lookup_visible_binding<'b>(&'b self, file: &AstFile, name: &str) -> Option<&'b BindingSig> {
        self.bindings
            .get(name)
            .filter(|sig| self.visible_from(file, sig.visibility, &sig.owner_file_path))
    }

    fn lookup_visible_call_sig<'b>(
        &'b self,
        file: &AstFile,
        name: &str,
        arguments: &[CallArg],
    ) -> Option<&'b FunctionSig> {
        let visible = self.visible_function_sigs(file, name);
        if visible.len() <= 1 {
            return visible.into_iter().next();
        }

        let matching = visible
            .into_iter()
            .filter(|sig| self.call_shape_matches_sig(arguments, sig))
            .collect::<Vec<_>>();
        if matching.len() == 1 {
            return matching.into_iter().next();
        }
        matching.into_iter().last()
    }

    fn call_shape_matches_sig(&self, arguments: &[CallArg], sig: &FunctionSig) -> bool {
        let positional = arguments
            .iter()
            .take_while(|argument| matches!(argument, CallArg::Positional(_)))
            .count();
        if positional > sig.params.len() {
            return false;
        }

        let required = sig.params.iter().filter(|param| !param.has_default).count();
        if arguments.len() < required || arguments.len() > sig.params.len() {
            return false;
        }

        let mut seen = HashSet::new();
        for argument in arguments {
            let CallArg::Named { name, .. } = argument else {
                continue;
            };
            if !seen.insert(name) {
                return false;
            }
            if !sig.params.iter().any(|param| param.name == *name) {
                return false;
            }
        }
        true
    }

    fn check(&mut self) {
        for file in &self.ast.files {
            for item in &file.items {
                match item {
                    Item::Binding(binding) => self.check_binding(file, binding),
                    Item::Function(function) => self.check_function(file, function),
                    Item::Type(type_decl) => self.check_type_decl(file, type_decl),
                }
            }
        }
    }

    fn check_binding(&mut self, file: &AstFile, binding: &TopLevelBinding) {
        let mut locals = HashMap::new();
        let declared = binding
            .type_name
            .as_deref()
            .map(|name| self.parse_type(name));
        if let Some(declared) = &declared {
            self.check_type_reference(file, declared, binding.line);
        }
        let inferred = binding.value.as_ref().map(|value| {
            self.infer_expression_with_expected(
                file,
                value,
                &mut locals,
                binding.line,
                declared.as_ref(),
                ExprMode::Read,
            )
        });
        self.check_binding_shape(
            file,
            &binding.name,
            binding.mutable,
            binding.line,
            declared.as_ref(),
            inferred.as_ref(),
            binding.value.as_ref(),
        );
        let binding_type = declared.or(inferred).unwrap_or(Type::Unknown);
        if let Some(sig) = self.bindings.get_mut(&binding.name) {
            sig.type_ = binding_type;
        }
    }

    fn check_type_decl(&mut self, file: &AstFile, type_decl: &TypeDecl) {
        match type_decl.kind {
            TypeDeclKind::Type => {
                for field in &type_decl.fields {
                    let type_ = self.parse_type(&field.type_name);
                    self.check_type_reference(file, &type_, field.line);
                }
                if self.record_field_cycle(&type_decl.name) {
                    self.report(
                        "TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION",
                        &format!(
                            "Record `{}` refers to itself without passing through a List, Map, or UNION; such a record has no base case and cannot be constructed.",
                            type_decl.name
                        ),
                        file,
                        type_decl.line,
                    );
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
                self.report_expanded_union_member_conflicts(file, type_decl);

                for variant in &type_decl.variants {
                    let type_ = self.parse_type(&variant.name);
                    self.check_type_reference(file, &type_, variant.line);
                    if let Some(kind) = self.user_type_kinds.get(&variant.name) {
                        if !matches!(kind, TypeDeclKind::Type) {
                            self.report(
                                "TYPE_UNION_MEMBER_REQUIRES_TYPE",
                                &format!(
                                    "UNION `{}` member `{}` must be a concrete TYPE.",
                                    type_decl.name, variant.name
                                ),
                                file,
                                variant.line,
                            );
                        }
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
                let default_type =
                    self.infer_expression(file, default, &mut locals, param.line, ExprMode::Read);
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
                    ownership: OwnershipState::Available,
                    scope_guard: ScopeGuard::None,
                },
            );
        }

        let flow = self.check_block(file, &function.body, &expected_return, &mut locals, None);
        if let Some(trap) = &function.trap {
            let mut trap_locals = locals.clone();
            trap_locals.insert(
                trap.name.clone(),
                LocalInfo {
                    type_: Type::Error,
                    mutable: false,
                    ownership: OwnershipState::Available,
                    scope_guard: ScopeGuard::None,
                },
            );
            let trap_flow = self.check_block(
                file,
                &trap.body,
                &expected_return,
                &mut trap_locals,
                Some(trap.name.as_str()),
            );
            if trap_flow != Flow::AlwaysReturns {
                self.report(
                    "TYPE_TRAP_FALLTHROUGH",
                    &format!("TRAP `{}` must return, fail, or propagate.", trap.name),
                    file,
                    trap.line,
                );
            }
            if flow != Flow::AlwaysReturns {
                self.report(
                    "TYPE_TRAP_FALLTHROUGH",
                    &format!(
                        "Normal flow in `{}` reaches TRAP `{}`; body paths before TRAP must end with RETURN or FAIL.",
                        function.name, trap.name
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
        trap_name: Option<&str>,
    ) -> Flow {
        for statement in body {
            let flow = self.check_statement(file, statement, expected_return, locals, trap_name);
            if flow == Flow::AlwaysReturns {
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    fn merge_branch_locals(
        &self,
        current: &mut HashMap<String, LocalInfo>,
        fallthrough_branches: Vec<HashMap<String, LocalInfo>>,
    ) {
        if fallthrough_branches.is_empty() {
            return;
        }
        let keys = current.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            let mut merged = current.get(&key).cloned();
            for branch in &fallthrough_branches {
                let Some(branch_info) = branch.get(&key) else {
                    continue;
                };
                merged =
                    merged.map(|current_info| self.merge_local_info(current_info, branch_info));
            }
            if let Some(merged) = merged {
                current.insert(key, merged);
            }
        }
    }

    fn merge_local_info(&self, left: LocalInfo, right: &LocalInfo) -> LocalInfo {
        let ownership = match (left.ownership, right.ownership) {
            (OwnershipState::Available, OwnershipState::Available) => OwnershipState::Available,
            (OwnershipState::Moved, OwnershipState::Moved) => OwnershipState::Moved,
            (OwnershipState::MaybeMoved, _) | (_, OwnershipState::MaybeMoved) => {
                OwnershipState::MaybeMoved
            }
            (OwnershipState::Available, OwnershipState::Moved)
            | (OwnershipState::Moved, OwnershipState::Available) => OwnershipState::MaybeMoved,
        };
        LocalInfo {
            type_: left.type_,
            mutable: left.mutable,
            ownership,
            scope_guard: left.scope_guard,
        }
    }

    fn require_local_owned(
        &mut self,
        file: &AstFile,
        line: usize,
        name: &str,
        info: &LocalInfo,
    ) -> bool {
        match info.ownership {
            OwnershipState::Available => true,
            OwnershipState::Moved => {
                self.report(
                    "TYPE_USE_AFTER_MOVE",
                    &format!("Binding `{name}` was moved and cannot be used again."),
                    file,
                    line,
                );
                false
            }
            OwnershipState::MaybeMoved => {
                self.report(
                    "TYPE_USE_AFTER_MOVE",
                    &format!(
                        "Binding `{name}` may have been moved on another control-flow path and cannot be used here."
                    ),
                    file,
                    line,
                );
                false
            }
        }
    }

    fn consume_local_if_needed(
        &mut self,
        file: &AstFile,
        line: usize,
        name: &str,
        locals: &mut HashMap<String, LocalInfo>,
    ) {
        let Some(info) = locals.get(name).cloned() else {
            return;
        };
        if !self.require_local_owned(file, line, name, &info) {
            return;
        }
        if !self.is_copyable_type(&info.type_) {
            if info.scope_guard == ScopeGuard::MustRemainOwned {
                self.report(
                    "TYPE_DOUBLE_DROP_PATH",
                    &format!(
                        "Binding `{name}` is cleaned up automatically at scope exit and cannot be consumed inside this scope."
                    ),
                    file,
                    line,
                );
                return;
            }
            if let Some(local) = locals.get_mut(name) {
                local.ownership = OwnershipState::Moved;
            }
        }
    }

    fn check_binding_shape(
        &mut self,
        file: &AstFile,
        name: &str,
        mutable: bool,
        line: usize,
        declared: Option<&Type>,
        inferred: Option<&Type>,
        value: Option<&Expression>,
    ) {
        if matches!(inferred, Some(Type::Unknown)) {
            self.report(
                "TYPE_UNKNOWN_VALUE",
                &format!("Initializer for binding `{name}` does not have a known type."),
                file,
                line,
            );
        }

        let reported_range_error = declared.zip(value).is_some_and(|(declared, value)| {
            self.report_primitive_literal_range_error(declared, value, file, line)
        });

        match (declared, inferred) {
            (Some(declared), Some(inferred))
                if !reported_range_error
                    && !self.expression_compatible(declared, inferred, value) =>
            {
                self.report(
                    "TYPE_BINDING_MISMATCH",
                    &format!(
                        "Binding `{name}` has initializer type {}, expected {}.",
                        self.type_name(inferred),
                        self.type_name(declared)
                    ),
                    file,
                    line,
                );
            }
            (None, None) => {
                self.report(
                    "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
                    &format!("Binding `{name}` needs a type annotation or initializer."),
                    file,
                    line,
                );
            }
            (Some(_), None) if !mutable => {
                self.report(
                    "TYPE_LET_REQUIRES_VALUE",
                    &format!("Immutable binding `{name}` must have an initializer."),
                    file,
                    line,
                );
            }
            (Some(declared), None) if mutable && !self.is_defaultable_type(declared) => {
                self.report(
                    "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
                    &format!(
                        "Mutable binding `{name}` cannot omit its initializer because type `{}` does not have a defined default value.",
                        self.type_name(declared)
                    ),
                    file,
                    line,
                );
            }
            _ => {}
        }
    }

    fn check_statement(
        &mut self,
        file: &AstFile,
        statement: &Statement,
        expected_return: &Type,
        locals: &mut HashMap<String, LocalInfo>,
        trap_name: Option<&str>,
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
                        ExprMode::Transfer,
                    )
                });

                self.check_binding_shape(
                    file,
                    name,
                    *mutable,
                    *line,
                    declared.as_ref(),
                    inferred.as_ref(),
                    value.as_ref(),
                );

                let binding_type = declared.or(inferred).unwrap_or(Type::Unknown);
                locals.insert(
                    name.clone(),
                    LocalInfo {
                        type_: binding_type,
                        mutable: *mutable,
                        ownership: OwnershipState::Available,
                        scope_guard: ScopeGuard::None,
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
                            ExprMode::Transfer,
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
                let actual = self.infer_expression(file, error, locals, *line, ExprMode::Transfer);
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
                if trap_name.is_none() {
                    self.report(
                        "TYPE_PROPAGATE_REQUIRES_TRAP",
                        "PROPAGATE is valid only inside a TRAP.",
                        file,
                        *line,
                    );
                }
                Flow::AlwaysReturns
            }
            Statement::Assign { name, value, line } => {
                let Some(local) = locals.get(name).cloned() else {
                    if let Some(binding) = self.lookup_visible_binding(file, name).cloned() {
                        if !binding.mutable {
                            self.report(
                                "TYPE_ASSIGN_REQUIRES_MUT",
                                &format!("Binding `{name}` is immutable and cannot be assigned."),
                                file,
                                *line,
                            );
                        }
                        let actual =
                            self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                        let reported_range_error = self.report_primitive_literal_range_error(
                            &binding.type_,
                            value,
                            file,
                            *line,
                        );
                        if !reported_range_error
                            && !self.expression_compatible(&binding.type_, &actual, Some(value))
                        {
                            self.report(
                                "TYPE_ASSIGNMENT_MISMATCH",
                                &format!(
                                    "Assignment to `{name}` has type {}, expected {}.",
                                    self.type_name(&actual),
                                    self.type_name(&binding.type_)
                                ),
                                file,
                                *line,
                            );
                        }
                        return Flow::FallsThrough;
                    }
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
                let actual = self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
                if !self.require_local_owned(file, *line, name, &local) {
                    return Flow::FallsThrough;
                }
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
                self.infer_expression(file, expression, locals, *line, ExprMode::Read);
                Flow::FallsThrough
            }
            Statement::If {
                condition,
                then_body,
                else_body,
                line,
            } => {
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
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
                let then_flow = self.check_block(
                    file,
                    then_body,
                    expected_return,
                    &mut then_locals,
                    trap_name,
                );
                let mut else_locals = locals.clone();
                let else_flow = self.check_block(
                    file,
                    else_body,
                    expected_return,
                    &mut else_locals,
                    trap_name,
                );
                let mut fallthroughs = Vec::new();
                if then_flow == Flow::FallsThrough {
                    fallthroughs.push(then_locals);
                }
                if else_flow == Flow::FallsThrough {
                    fallthroughs.push(else_locals);
                } else if else_body.is_empty() {
                    fallthroughs.push(locals.clone());
                }
                if then_flow == Flow::AlwaysReturns
                    && else_flow == Flow::AlwaysReturns
                    && !else_body.is_empty()
                {
                    Flow::AlwaysReturns
                } else {
                    self.merge_branch_locals(locals, fallthroughs);
                    Flow::FallsThrough
                }
            }
            Statement::Match {
                expression,
                cases,
                line,
            } => {
                let send_failure_restore =
                    self.thread_send_failure_restore(file, expression, locals);
                let matched_type = self.infer_match_scrutinee(file, expression, locals, *line);
                let mut has_unguarded_else = false;
                let mut all_return = !cases.is_empty();
                let mut covered_cases = HashSet::new();
                let mut fallthroughs = Vec::new();
                for case in cases {
                    if case.guard.is_none() {
                        if matches!(case.pattern, MatchPattern::Else) {
                            has_unguarded_else = true;
                        } else if let Some(name) = self.match_case_name(&case.pattern) {
                            covered_cases.insert(name);
                        }
                    }
                    let mut case_locals = locals.clone();
                    if matches!(
                        (&case.pattern, &send_failure_restore),
                        (
                            MatchPattern::Union { type_name, .. },
                            Some((_, _))
                        ) if type_name == "Error"
                    ) {
                        if let Some((name, info)) = &send_failure_restore {
                            case_locals.insert(name.clone(), info.clone());
                        }
                    }
                    self.check_match_pattern(
                        file,
                        &case.pattern,
                        &matched_type,
                        &mut case_locals,
                        case.line,
                    );
                    if let Some(guard) = &case.guard {
                        let guard_type = self.infer_expression(
                            file,
                            guard,
                            &mut case_locals,
                            case.line,
                            ExprMode::Read,
                        );
                        if !self.expression_compatible(&Type::Boolean, &guard_type, Some(guard)) {
                            self.report(
                                "TYPE_CONDITION_REQUIRES_BOOLEAN",
                                &format!(
                                    "WHEN guard has type {}, expected Boolean.",
                                    self.type_name(&guard_type)
                                ),
                                file,
                                case.line,
                            );
                        }
                    }
                    let case_flow = self.check_block(
                        file,
                        &case.body,
                        expected_return,
                        &mut case_locals,
                        trap_name,
                    );
                    all_return &= case_flow == Flow::AlwaysReturns;
                    if case_flow == Flow::FallsThrough {
                        fallthroughs.push(case_locals);
                    }
                }
                let exhaustive =
                    has_unguarded_else || self.match_is_exhaustive(&matched_type, &covered_cases);
                if !exhaustive && !matches!(matched_type, Type::Unknown) {
                    self.report_match_not_exhaustive(file, *line, &matched_type, &covered_cases);
                }
                if all_return && exhaustive {
                    Flow::AlwaysReturns
                } else {
                    if !exhaustive {
                        fallthroughs.push(locals.clone());
                    }
                    self.merge_branch_locals(locals, fallthroughs);
                    Flow::FallsThrough
                }
            }
            Statement::For {
                name,
                start,
                end,
                step,
                body,
                line,
            } => {
                let start_type = self.infer_expression(file, start, locals, *line, ExprMode::Read);
                let end_type = self.infer_expression(file, end, locals, *line, ExprMode::Read);
                let step_type = match step {
                    Some(step) => self.infer_expression(file, step, locals, *line, ExprMode::Read),
                    None => Type::Integer,
                };
                let numeric_types = [&start_type, &end_type, &step_type];
                let all_numeric = numeric_types.iter().all(|type_| self.is_numeric(type_));
                let loop_type = if all_numeric {
                    promote_loop_numeric_type(&start_type, &end_type, &step_type)
                } else {
                    for (label, type_) in [
                        ("start", &start_type),
                        ("end", &end_type),
                        ("step", &step_type),
                    ] {
                        if !self.is_numeric(type_) {
                            self.report(
                                "TYPE_FOR_REQUIRES_NUMERIC",
                                &format!(
                                    "FOR loop {label} value has type {}, expected numeric.",
                                    self.type_name(type_)
                                ),
                                file,
                                *line,
                            );
                        }
                    }
                    Type::Unknown
                };
                if let Some(step) = step {
                    if numeric_literal_is_zero(step) {
                        self.report(
                            "TYPE_FOR_STEP_ZERO",
                            "FOR loop STEP must not be zero.",
                            file,
                            *line,
                        );
                    }
                }
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: loop_type,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        scope_guard: ScopeGuard::None,
                    },
                );
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::ForEach {
                name,
                iterable,
                body,
                line,
            } => {
                let iterable_type =
                    self.infer_expression(file, iterable, locals, *line, ExprMode::Read);
                let element_type = match iterable_type {
                    Type::List(element) => *element,
                    Type::Map(key, value) => Type::User(format!(
                        "MapEntry OF {} TO {}",
                        self.type_name(&key),
                        self.type_name(&value)
                    )),
                    other => {
                        self.report(
                            "TYPE_FOR_EACH_REQUIRES_COLLECTION",
                            &format!(
                                "FOR EACH source has type {}, expected List or Map.",
                                self.type_name(&other)
                            ),
                            file,
                            *line,
                        );
                        Type::Unknown
                    }
                };
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    LocalInfo {
                        type_: element_type,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        scope_guard: ScopeGuard::None,
                    },
                );
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::While {
                condition,
                body,
                line,
            } => {
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {
                    self.report(
                        "TYPE_CONDITION_REQUIRES_BOOLEAN",
                        &format!(
                            "WHILE condition has type {}, expected Boolean.",
                            self.type_name(&condition_type)
                        ),
                        file,
                        *line,
                    );
                }
                let mut nested = locals.clone();
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::DoUntil {
                body,
                condition,
                line,
            } => {
                let mut nested = locals.clone();
                let body_flow =
                    self.check_block(file, body, expected_return, &mut nested, trap_name);
                let condition_type =
                    self.infer_expression(file, condition, locals, *line, ExprMode::Read);
                if !self.expression_compatible(&Type::Boolean, &condition_type, Some(condition)) {
                    self.report(
                        "TYPE_CONDITION_REQUIRES_BOOLEAN",
                        &format!(
                            "LOOP UNTIL condition has type {}, expected Boolean.",
                            self.type_name(&condition_type)
                        ),
                        file,
                        *line,
                    );
                }
                if body_flow == Flow::FallsThrough {
                    self.merge_branch_locals(locals, vec![nested]);
                }
                Flow::FallsThrough
            }
            Statement::Using {
                name,
                value,
                body,
                line,
            } => {
                let resource_type =
                    self.infer_expression(file, value, locals, *line, ExprMode::Transfer);
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
                        ownership: OwnershipState::Available,
                        scope_guard: ScopeGuard::MustRemainOwned,
                    },
                );
                let flow = self.check_block(file, body, expected_return, &mut nested, trap_name);
                if let Some(resource) = nested.get(name).cloned() {
                    self.require_local_owned(file, *line, name, &resource);
                }
                flow
            }
        }
    }

    fn infer_expression(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        mode: ExprMode,
    ) -> Type {
        self.infer_expression_with_expected(file, expression, locals, line, None, mode)
    }

    fn infer_expression_with_expected(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
        mode: ExprMode,
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
            Expression::Identifier(name) => {
                let canonical_name = self.canonical_import_name(file, name);
                if canonical_name == "NOTHING" {
                    Type::Nothing
                } else if builtins::math::is_math_constant(&canonical_name) {
                    self.parse_type(
                        builtins::math::constant_type_name(&canonical_name).unwrap_or("Unknown"),
                    )
                } else {
                    if let Some(local) = locals.get(name).cloned() {
                        self.require_local_owned(file, line, name, &local);
                        if matches!(mode, ExprMode::Transfer) {
                            self.consume_local_if_needed(file, line, name, locals);
                        }
                        local.type_
                    } else {
                        self.lookup_visible_function(file, name)
                            .map(function_type)
                            .or_else(|| {
                                self.lookup_visible_binding(file, name)
                                    .map(|binding| binding.type_.clone())
                            })
                            .or_else(|| {
                                self.lookup_visible_function(file, &canonical_name)
                                    .map(function_type)
                            })
                            .unwrap_or(Type::Unknown)
                    }
                }
            }
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
                let left_type = self.infer_expression(file, left, locals, line, ExprMode::Read);
                let right_type = self.infer_expression(file, right, locals, line, ExprMode::Read);
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
                let operand_type =
                    self.infer_expression(file, operand, locals, line, ExprMode::Read);
                self.infer_unary(file, operator, &operand_type, line)
            }
            Expression::Call { callee, arguments } => {
                let canonical_callee = self.canonical_import_name(file, callee);
                if builtins::math::is_math_constant(&canonical_callee) {
                    self.report(
                        "SYMBOL_NOT_CALLABLE",
                        &format!("Package constant `{callee}` is not callable."),
                        file,
                        line,
                    );
                    for argument in arguments {
                        self.infer_expression(
                            file,
                            call_arg_value(argument),
                            locals,
                            line,
                            ExprMode::Read,
                        );
                    }
                    return self.parse_type(
                        builtins::math::constant_type_name(&canonical_callee).unwrap_or("Unknown"),
                    );
                }
                if builtins::is_builtin_call(&canonical_callee) {
                    return self.check_builtin_call(
                        file,
                        callee,
                        &canonical_callee,
                        arguments,
                        locals,
                        line,
                    );
                }

                if let Some(sig) = self
                    .lookup_visible_call_sig(file, callee, arguments)
                    .cloned()
                    .or_else(|| {
                        self.lookup_visible_call_sig(file, &canonical_callee, arguments)
                            .cloned()
                    })
                {
                    self.check_call(file, callee, &sig, arguments, locals, line);
                    return sig.return_type;
                }

                if callee.contains('.') {
                    for argument in arguments {
                        self.infer_expression(
                            file,
                            call_arg_value(argument),
                            locals,
                            line,
                            ExprMode::Read,
                        );
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
            Expression::ListLiteral(values) => {
                self.infer_list_literal(file, values, locals, line, expected)
            }
            Expression::MapLiteral {
                key_type,
                value_type,
                entries,
            } => self.infer_map_literal(file, key_type, value_type, entries, locals, line),
        }
    }

    fn infer_match_scrutinee(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        if let Expression::Call { callee, arguments } = expression {
            let canonical_callee = self.canonical_import_name(file, callee);
            if builtins::math::is_math_constant(&canonical_callee) {
                self.report(
                    "SYMBOL_NOT_CALLABLE",
                    &format!("Package constant `{callee}` is not callable."),
                    file,
                    line,
                );
                for argument in arguments {
                    self.infer_expression(
                        file,
                        call_arg_value(argument),
                        locals,
                        line,
                        ExprMode::Read,
                    );
                }
                return self.parse_type(
                    builtins::math::constant_type_name(&canonical_callee).unwrap_or("Unknown"),
                );
            }
            if builtins::is_builtin_call(&canonical_callee) {
                let success = self.check_builtin_call(
                    file,
                    callee,
                    &canonical_callee,
                    arguments,
                    locals,
                    line,
                );
                return Type::Result(Box::new(success));
            }
            if let Some(sig) = self
                .lookup_visible_call_sig(file, callee, arguments)
                .cloned()
                .or_else(|| {
                    self.lookup_visible_call_sig(file, &canonical_callee, arguments)
                        .cloned()
                })
            {
                self.check_call(file, callee, &sig, arguments, locals, line);
                return Type::Result(Box::new(sig.return_type));
            }
            if let Some(local) = locals.get(callee).cloned() {
                let success = self.check_function_value_call(
                    file,
                    callee,
                    &local.type_,
                    arguments,
                    locals,
                    line,
                );
                return Type::Result(Box::new(success));
            }
        }
        self.infer_expression(file, expression, locals, line, ExprMode::Read)
    }

    fn thread_send_failure_restore(
        &self,
        file: &AstFile,
        expression: &Expression,
        locals: &HashMap<String, LocalInfo>,
    ) -> Option<(String, LocalInfo)> {
        let Expression::Call { callee, arguments } = expression else {
            return None;
        };
        if self.canonical_import_name(file, callee) != "thread.send" {
            return None;
        }
        let Some(argument) = arguments.get(1).map(call_arg_value) else {
            return None;
        };
        let Expression::Identifier(name) = argument else {
            return None;
        };
        let info = locals.get(name)?;
        if info.ownership != OwnershipState::Available || self.is_copyable_type(&info.type_) {
            return None;
        }
        Some((name.clone(), info.clone()))
    }

    fn check_match_pattern(
        &mut self,
        file: &AstFile,
        pattern: &MatchPattern,
        matched_type: &Type,
        case_locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) {
        match pattern {
            MatchPattern::Else => {}
            MatchPattern::Literal(expression) => {
                let pattern_type =
                    self.infer_expression(file, expression, case_locals, line, ExprMode::Read);
                if !self.expression_compatible(matched_type, &pattern_type, Some(expression)) {
                    self.report(
                        "TYPE_MATCH_PATTERN_MISMATCH",
                        &format!(
                            "CASE pattern has type {}, expected {}.",
                            self.type_name(&pattern_type),
                            self.type_name(matched_type)
                        ),
                        file,
                        line,
                    );
                }
            }
            MatchPattern::OneOf(expressions) => {
                for expression in expressions {
                    self.check_match_pattern(
                        file,
                        &MatchPattern::Literal(expression.clone()),
                        matched_type,
                        case_locals,
                        line,
                    );
                }
            }
            MatchPattern::Union { type_name, binding } => match matched_type {
                Type::User(union_name) => {
                    let Some(info) = self.type_infos.get(union_name) else {
                        return;
                    };
                    if !matches!(info.kind, TypeDeclKind::Union)
                        || !info
                            .variants
                            .iter()
                            .any(|variant| variant.name == *type_name)
                    {
                        self.report(
                            "TYPE_MATCH_PATTERN_MISMATCH",
                            &format!("CASE `{type_name}` is not a member of UNION `{union_name}`."),
                            file,
                            line,
                        );
                        return;
                    }
                    case_locals.insert(
                        binding.clone(),
                        LocalInfo {
                            type_: Type::User(type_name.clone()),
                            mutable: false,
                            ownership: OwnershipState::Available,
                            scope_guard: ScopeGuard::None,
                        },
                    );
                }
                Type::Result(success) => {
                    let binding_type = match type_name.as_str() {
                        "Ok" => *success.clone(),
                        "Error" => Type::Error,
                        _ => {
                            self.report(
                                "TYPE_MATCH_PATTERN_MISMATCH",
                                &format!("CASE `{type_name}` is not valid when matching a Result."),
                                file,
                                line,
                            );
                            return;
                        }
                    };
                    case_locals.insert(
                        binding.clone(),
                        LocalInfo {
                            type_: binding_type,
                            mutable: false,
                            ownership: OwnershipState::Available,
                            scope_guard: ScopeGuard::None,
                        },
                    );
                }
                _ => self.report(
                    "TYPE_MATCH_PATTERN_MISMATCH",
                    &format!(
                        "CASE `{type_name}` requires a UNION or direct call Result, got {}.",
                        self.type_name(matched_type)
                    ),
                    file,
                    line,
                ),
            },
        }
    }

    fn match_case_name(&self, pattern: &MatchPattern) -> Option<String> {
        match pattern {
            MatchPattern::Union { type_name, .. } => Some(type_name.clone()),
            MatchPattern::Literal(Expression::MemberAccess { target, member }) => {
                if let Expression::Identifier(type_name) = target.as_ref() {
                    Some(format!("{type_name}::{member}"))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn match_is_exhaustive(&self, matched_type: &Type, covered_cases: &HashSet<String>) -> bool {
        if matches!(matched_type, Type::Result(_)) {
            return covered_cases.contains("Ok") && covered_cases.contains("Error");
        }
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

    fn report_match_not_exhaustive(
        &mut self,
        file: &AstFile,
        line: usize,
        matched_type: &Type,
        covered_cases: &HashSet<String>,
    ) {
        let detail = match matched_type {
            Type::Result(_) => {
                let missing = ["Ok", "Error"]
                    .into_iter()
                    .filter(|name| !covered_cases.contains(*name))
                    .collect::<Vec<_>>();
                format!(
                    "MATCH on {} does not cover {}; add unguarded CASE arms or CASE ELSE.",
                    self.type_name(matched_type),
                    missing.join(", ")
                )
            }
            Type::User(type_name) => {
                let Some(info) = self.type_infos.get(type_name) else {
                    return;
                };
                match info.kind {
                    TypeDeclKind::Enum => {
                        let mut missing = info
                            .members
                            .iter()
                            .filter_map(|member| {
                                let case_name = format!("{type_name}::{member}");
                                if covered_cases.contains(&case_name) {
                                    None
                                } else {
                                    Some(format!("{type_name}.{member}"))
                                }
                            })
                            .collect::<Vec<_>>();
                        missing.sort();
                        format!(
                            "MATCH on enum `{type_name}` does not cover {}; add unguarded CASE arms or CASE ELSE.",
                            missing.join(", ")
                        )
                    }
                    TypeDeclKind::Union => {
                        let missing = info
                            .variants
                            .iter()
                            .filter_map(|variant| {
                                if covered_cases.contains(&variant.name) {
                                    None
                                } else {
                                    Some(variant.name.clone())
                                }
                            })
                            .collect::<Vec<_>>();
                        format!(
                            "MATCH on UNION `{type_name}` does not cover {}; add unguarded CASE arms or CASE ELSE.",
                            missing.join(", ")
                        )
                    }
                    TypeDeclKind::Type => format!(
                        "MATCH on open type {} requires an unguarded CASE ELSE.",
                        self.type_name(matched_type)
                    ),
                }
            }
            _ => format!(
                "MATCH on open type {} requires an unguarded CASE ELSE.",
                self.type_name(matched_type)
            ),
        };
        self.report("TYPE_MATCH_NOT_EXHAUSTIVE", &detail, file, line);
    }

    fn infer_list_literal(
        &mut self,
        file: &AstFile,
        values: &[Expression],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        if let Some(Type::List(expected_element)) = expected {
            if self.contains_resource_or_thread(expected_element) {
                self.report_invalid_collection_element(file, line, "element", expected_element);
            }
            for value in values {
                let actual = self.infer_expression_with_expected(
                    file,
                    value,
                    locals,
                    line,
                    Some(expected_element),
                    ExprMode::Transfer,
                );
                if !self.expression_compatible(expected_element, &actual, Some(value)) {
                    self.report(
                        "TYPE_LIST_ELEMENT_MISMATCH",
                        &format!(
                            "List element has type {}, expected {}.",
                            self.type_name(&actual),
                            self.type_name(expected_element)
                        ),
                        file,
                        line,
                    );
                }
            }
            return Type::List(expected_element.clone());
        }

        let Some(first) = values.first() else {
            return Type::List(Box::new(Type::Unknown));
        };
        let element_type = self.infer_expression(file, first, locals, line, ExprMode::Transfer);
        if self.contains_resource_or_thread(&element_type) {
            self.report_invalid_collection_element(file, line, "element", &element_type);
        }
        for value in values.iter().skip(1) {
            let actual = self.infer_expression(file, value, locals, line, ExprMode::Transfer);
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
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let key_type = self.parse_type(key_type);
        let value_type = self.parse_type(value_type);
        self.check_type_reference(file, &key_type, line);
        self.check_type_reference(file, &value_type, line);
        if self.contains_resource_or_thread(&key_type) {
            self.report_invalid_collection_element(file, line, "key", &key_type);
        }
        self.require_comparable_type(file, line, "Map key type", &key_type);
        if self.contains_resource_or_thread(&value_type) {
            self.report_invalid_collection_element(file, line, "value", &value_type);
        }
        for (key, value) in entries {
            let actual_key = self.infer_expression(file, key, locals, line, ExprMode::Transfer);
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
            let actual_value = self.infer_expression(file, value, locals, line, ExprMode::Transfer);
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
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        _expected: Option<&Type>,
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

        if matches!(type_name, "Ok" | "Result") {
            self.report(
                "TYPE_RESULT_IS_IMPLICIT",
                &format!("`{type_name}` is compiler-owned and cannot be constructed directly."),
                file,
                line,
            );
            for argument in arguments {
                self.infer_expression(
                    file,
                    constructor_arg_value(argument),
                    locals,
                    line,
                    ExprMode::Transfer,
                );
            }
            return Type::Unknown;
        }

        if read_only_record_type(type_name) {
            self.report(
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                &format!("TYPE `{type_name}` is compiler-owned and cannot be constructed."),
                file,
                line,
            );
            for argument in arguments {
                self.infer_expression(
                    file,
                    constructor_arg_value(argument),
                    locals,
                    line,
                    ExprMode::Transfer,
                );
            }
            return Type::Unknown;
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

        for argument in arguments {
            self.infer_expression(
                file,
                constructor_arg_value(argument),
                locals,
                line,
                ExprMode::Transfer,
            );
        }
        Type::Unknown
    }

    fn infer_with_update(
        &mut self,
        file: &AstFile,
        target: &Expression,
        updates: &[RecordUpdate],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let target_type = self.infer_expression(file, target, locals, line, ExprMode::Transfer);
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
        if read_only_record_type(type_name) {
            self.report(
                "TYPE_READ_ONLY_RECORD_UPDATE",
                &format!("TYPE `{type_name}` is read-only and cannot be updated."),
                file,
                line,
            );
            for update in updates {
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
            }
            return Type::Unknown;
        }
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
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
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
                ExprMode::Transfer,
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
        locals: &mut HashMap<String, LocalInfo>,
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

        let target_type = self.infer_expression(file, target, locals, line, ExprMode::Read);
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
        if matches!(target_type, Type::Error) {
            return match member {
                "code" => Type::Integer,
                "message" => Type::String,
                _ => {
                    self.report(
                        "TYPE_UNKNOWN_FIELD",
                        &format!("Error value has no field `{member}`."),
                        file,
                        line,
                    );
                    Type::Unknown
                }
            };
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
        if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
            if let Some((key, value)) = rest.split_once(" TO ") {
                return match member {
                    "key" => self.parse_type(key),
                    "value" => self.parse_type(value),
                    _ => {
                        self.report(
                            "TYPE_UNKNOWN_FIELD",
                            &format!("Map entry has no field `{member}`."),
                            file,
                            line,
                        );
                        Type::Unknown
                    }
                };
            }
        }
        let Some(info) = self.type_infos.get(&type_name).cloned() else {
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
        locals: &mut HashMap<String, LocalInfo>,
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
            let actual = if let Some(field) = field {
                self.infer_expression_with_expected(
                    file,
                    argument_value,
                    locals,
                    argument_line,
                    Some(&field.type_),
                    ExprMode::Transfer,
                )
            } else {
                self.infer_expression(
                    file,
                    argument_value,
                    locals,
                    argument_line,
                    ExprMode::Transfer,
                )
            };
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
            if self.is_numeric(left) && self.is_numeric(right) {
                return Type::Boolean;
            }
            if (self.compatible(left, right) || self.compatible(right, left))
                && self.is_comparable(left)
                && self.is_comparable(right)
            {
                return Type::Boolean;
            }
            self.report(
                if self.compatible(left, right) || self.compatible(right, left) {
                    "TYPE_REQUIRES_COMPARABLE"
                } else {
                    "TYPE_BINARY_OPERATOR_MISMATCH"
                },
                &format!(
                    "Operator `{operator}` requires compatible comparable operands, got {} and {}.",
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
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) {
        let arguments =
            self.normalize_named_arguments(file, callee, arguments, &sig.params, line, false);

        for (index, argument) in arguments.iter().enumerate() {
            let Some(argument) = argument else {
                continue;
            };
            let actual = self.infer_expression(
                file,
                argument,
                locals,
                line,
                self.argument_mode_for_type(&sig.params.get(index).map(|param| &param.type_)),
            );
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
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
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
                self.infer_expression(file, call_arg_value(argument), locals, line, ExprMode::Read);
            }
            return Type::Unknown;
        };

        if arguments
            .iter()
            .any(|argument| matches!(argument, CallArg::Named { .. }))
        {
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to function value `{callee}` cannot use named arguments because the callable type does not preserve parameter names."
                ),
                file,
                line,
            );
        }

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
            let argument = call_arg_value(argument);
            let actual = self.infer_expression(
                file,
                argument,
                locals,
                line,
                self.argument_mode_for_type(&params.get(index)),
            );
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
        outer_locals: &mut HashMap<String, LocalInfo>,
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
                    "TYPE_PARAM_REQUIRES_TYPE",
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
                    "TYPE_DEFAULT_ARG_ORDER",
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
                    ownership: OwnershipState::Available,
                    scope_guard: ScopeGuard::None,
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
            if capture.mutable {
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures mutable local `{}`; mutable captures are invalid.",
                        capture.name
                    ),
                    file,
                    line,
                );
            } else if self.is_resource_type(&capture.type_) {
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures resource local `{}`; resource captures are invalid.",
                        capture.name
                    ),
                    file,
                    line,
                );
            } else if !self.is_copyable_type(&capture.type_) {
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures non-copyable local `{}` of type `{}`; non-copyable captures are invalid.",
                        capture.name,
                        self.type_name(&capture.type_)
                    ),
                    file,
                    line,
                );
            }
        }
        let return_type = self.infer_expression(file, body, &mut locals, line, ExprMode::Read);
        Type::Function {
            params: param_types,
            return_type: Box::new(return_type),
            isolated: false,
        }
    }

    fn check_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        if builtins::general::is_general_call(callee) {
            return self.check_general_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::strings::is_strings_call(callee) {
            return self.check_strings_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::math::is_math_call(callee) {
            return self.check_math_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::fs::is_fs_call(callee) {
            return self.check_fs_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::io::is_io_call(callee) {
            return self.check_io_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::json::is_json_call(callee) {
            return self.check_json_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }
        if builtins::thread::is_thread_call(callee) {
            return self.check_thread_builtin_call(
                file,
                display_callee,
                callee,
                arguments,
                locals,
                line,
            );
        }

        for argument in arguments {
            self.infer_expression(file, call_arg_value(argument), locals, line, ExprMode::Read);
        }
        Type::Unknown
    }

    fn check_fs_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                let mode = if callee == "fs.close" && index == 0 {
                    ExprMode::Transfer
                } else {
                    ExprMode::Borrow
                };
                let type_ = self.infer_expression(file, argument, locals, line, mode);
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
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
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
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_types.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.parse_type(&resolved.return_type)
    }

    fn check_json_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
                self.type_name(&type_)
            })
            .collect::<Vec<_>>();

        if let Some((min, max)) = builtins::json::arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                let expected = if min == max {
                    min.to_string()
                } else {
                    format!("{min} to {max}")
                };
                self.report(
                    "TYPE_CALL_ARITY_MISMATCH",
                    &format!(
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::json::resolve_call(callee, &arg_types) else {
            let expected =
                builtins::json::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
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
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
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
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
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
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
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
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .enumerate()
            .map(|(index, argument)| {
                self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.thread_argument_mode(callee, index),
                )
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
            .collect::<Vec<_>>();

        if callee == "thread.start" {
            let valid_entry = match arguments.first() {
                Some(Expression::Identifier(name)) => {
                    let canonical_name = self.canonical_import_name(file, name);
                    self.lookup_visible_function(file, name)
                        .or_else(|| self.lookup_visible_function(file, &canonical_name))
                        .is_some_and(|sig| {
                            sig.imported_package_export
                                && matches!(sig.kind, FunctionKind::Func)
                                && sig.isolated
                        })
                }
                _ => false,
            };
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
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::thread::resolve_call(callee, &arg_type_names) else {
            let expected =
                builtins::thread::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        let return_type = self.parse_type(&resolved.return_type);
        self.check_thread_boundary_sendability(
            file,
            display_callee,
            callee,
            &arg_types,
            &return_type,
            line,
        );
        return_type
    }

    fn check_strings_builtin_call(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
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
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
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
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
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
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        let arg_types = arguments
            .iter()
            .map(|argument| {
                let type_ = self.infer_expression(file, argument, locals, line, ExprMode::Read);
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
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
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
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
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
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let arguments =
            self.normalize_builtin_call_arguments(file, display_callee, callee, arguments, line);
        if callee == "filter" && arguments.len() == 2 {
            if let Expression::Identifier(predicate) = &arguments[1] {
                if builtins::general::builtin_function_id(predicate).is_some() {
                    let collection_type =
                        self.infer_expression(file, &arguments[0], locals, line, ExprMode::Read);
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
                                "Call to `{display_callee}` has argument type(s) ({collection_type_name}, {predicate}), expected {}.",
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
                                "Call to `{display_callee}` has argument type(s) ({}), expected {}.",
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
            .enumerate()
            .map(|(index, argument)| {
                self.infer_expression(
                    file,
                    argument,
                    locals,
                    line,
                    self.general_argument_mode(callee, index),
                )
            })
            .collect::<Vec<_>>();
        let arg_type_names = arg_types
            .iter()
            .map(|type_| self.type_name(type_))
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
                        "Call to `{display_callee}` has {} argument(s), expected {expected}.",
                        arguments.len()
                    ),
                    file,
                    line,
                );
                return Type::Unknown;
            }
        }

        let Some(resolved) = builtins::general::resolve_call(callee, &arg_type_names) else {
            let expected =
                builtins::general::expected_arguments(callee).unwrap_or("supported overload");
            self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display_callee}` has argument type(s) ({}), expected {expected}.",
                    arg_type_names.join(", ")
                ),
                file,
                line,
            );
            return Type::Unknown;
        };

        self.check_general_builtin_comparability(file, display_callee, callee, &arg_types, line);

        self.parse_type(&resolved.return_type)
    }

    fn check_general_builtin_comparability(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arg_types: &[Type],
        line: usize,
    ) {
        match callee {
            "contains" | "replace" => {
                let Some(Type::List(element)) = arg_types.first() else {
                    return;
                };
                self.require_comparable_type(
                    file,
                    line,
                    &format!("Call to `{display_callee}`"),
                    element,
                );
            }
            "find" => {
                let Some(first) = arg_types.first() else {
                    return;
                };
                if let Type::List(element) = first {
                    self.require_comparable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}`"),
                        element,
                    );
                }
            }
            _ => {}
        }
    }

    fn normalize_builtin_call_arguments(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arguments: &[CallArg],
        line: usize,
    ) -> Vec<Expression> {
        if !arguments
            .iter()
            .any(|argument| matches!(argument, CallArg::Named { .. }))
        {
            return arguments
                .iter()
                .map(|argument| call_arg_value(argument).clone())
                .collect();
        }
        let Some(param_names) = builtins::call_param_names(callee) else {
            return arguments
                .iter()
                .map(|argument| call_arg_value(argument).clone())
                .collect();
        };
        let mut ordered = vec![None; param_names.len()];
        let mut next_positional = 0usize;
        let mut extras = Vec::new();
        let mut saw_unknown_named = false;
        for argument in arguments {
            match argument {
                CallArg::Positional(value) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional < ordered.len() {
                        ordered[next_positional] = Some(value.clone());
                        next_positional += 1;
                    } else {
                        extras.push(value.clone());
                    }
                }
                CallArg::Named { name, value, line } => {
                    let Some(index) = param_names
                        .iter()
                        .position(|aliases| aliases.iter().any(|alias| alias == name))
                    else {
                        self.report(
                            "TYPE_UNKNOWN_ARGUMENT_NAME",
                            &format!(
                                "Call to `{display_callee}` does not have a parameter named `{name}`."
                            ),
                            file,
                            *line,
                        );
                        saw_unknown_named = true;
                        continue;
                    };
                    if ordered[index].is_some() {
                        self.report(
                            "TYPE_DUPLICATE_ARGUMENT_NAME",
                            &format!(
                                "Call to `{display_callee}` supplies parameter `{}` more than once.",
                                param_names[index][0]
                            ),
                            file,
                            *line,
                        );
                        continue;
                    }
                    ordered[index] = Some(value.clone());
                }
            }
        }
        if !saw_unknown_named {
            for (index, aliases) in param_names.iter().enumerate() {
                if ordered[index].is_none()
                    && ordered
                        .iter()
                        .skip(index + 1)
                        .any(|argument| argument.is_some())
                {
                    self.report(
                        "TYPE_CALL_ARITY_MISMATCH",
                        &format!(
                            "Call to `{display_callee}` omits parameter `{}` before a later supplied argument.",
                            aliases[0]
                        ),
                        file,
                        line,
                    );
                    break;
                }
            }
        }
        let mut normalized = ordered.into_iter().flatten().collect::<Vec<_>>();
        normalized.extend(extras);
        normalized
    }

    fn normalize_named_arguments(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[CallArg],
        params: &[ParamSig],
        line: usize,
        allow_trailing_omission: bool,
    ) -> Vec<Option<Expression>> {
        let mut ordered = vec![None; params.len()];
        let mut next_positional = 0usize;
        let mut supplied = 0usize;
        let mut arity_error = false;

        for argument in arguments {
            match argument {
                CallArg::Positional(value) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional >= ordered.len() {
                        arity_error = true;
                        continue;
                    }
                    ordered[next_positional] = Some(value.clone());
                    next_positional += 1;
                    supplied += 1;
                }
                CallArg::Named { name, value, line } => {
                    let Some(index) = params.iter().position(|param| param.name == *name) else {
                        self.report(
                            "TYPE_UNKNOWN_ARGUMENT_NAME",
                            &format!(
                                "Call to `{callee}` does not have a parameter named `{name}`."
                            ),
                            file,
                            *line,
                        );
                        continue;
                    };
                    if ordered[index].is_some() {
                        self.report(
                            "TYPE_DUPLICATE_ARGUMENT_NAME",
                            &format!(
                                "Call to `{callee}` supplies parameter `{name}` more than once."
                            ),
                            file,
                            *line,
                        );
                        continue;
                    }
                    ordered[index] = Some(value.clone());
                    supplied += 1;
                }
            }
        }

        let required = params.iter().filter(|param| !param.has_default).count();
        let missing_required = ordered
            .iter()
            .zip(params.iter())
            .any(|(argument, param)| argument.is_none() && !param.has_default);
        let max_supplied = ordered
            .iter()
            .rposition(Option::is_some)
            .map(|index| index + 1)
            .unwrap_or(0);
        let has_internal_gap = allow_trailing_omission
            && ordered
                .iter()
                .zip(params.iter())
                .take(max_supplied)
                .any(|(argument, param)| argument.is_none() && !param.has_default);

        if arity_error
            || supplied < required
            || supplied > params.len()
            || missing_required
            || has_internal_gap
        {
            self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{callee}` has {} argument(s), expected {} to {}.",
                    supplied,
                    required,
                    params.len()
                ),
                file,
                line,
            );
        }

        ordered
    }

    fn parse_type(&self, name: &str) -> Type {
        let name = builtins::thread::strip_type_group(name);
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
        if let Some((kind, message, output)) = builtins::thread::thread_parts(name) {
            if kind == builtins::thread::THREAD_WORKER_TYPE {
                return Type::ThreadWorker(
                    Box::new(self.parse_type(message)),
                    Box::new(self.parse_type(output)),
                );
            }
            return Type::Thread(
                Box::new(self.parse_type(message)),
                Box::new(self.parse_type(output)),
            );
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
                Type::ThreadWorker(expected_message, expected_output),
                Type::ThreadWorker(actual_message, actual_output),
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
            (Type::User(expected_name), Type::User(actual_name)) => {
                expected_name == actual_name
                    || self.type_infos.get(expected_name).is_some_and(|info| {
                        matches!(info.kind, TypeDeclKind::Union)
                            && info
                                .variants
                                .iter()
                                .any(|variant| variant.name == *actual_name)
                    })
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

    fn is_comparable(&self, type_: &Type) -> bool {
        self.is_comparable_with_seen(type_, &mut HashSet::new())
    }

    fn is_comparable_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(_)
            | Type::Map(_, _)
            | Type::Function { .. }
            | Type::Result(_)
            | Type::Thread(_, _)
            | Type::ThreadWorker(_, _) => false,
            Type::User(name) => {
                if builtins::is_resource_type(name) || !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => true,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_comparable_with_seen(&field.type_, seen)),
                    TypeDeclKind::Union => false,
                };
                seen.remove(name);
                result
            }
        }
    }

    fn require_comparable_type(
        &mut self,
        file: &AstFile,
        line: usize,
        context: &str,
        type_: &Type,
    ) {
        if self.is_comparable(type_) {
            return;
        }
        self.report(
            "TYPE_REQUIRES_COMPARABLE",
            &format!(
                "{context} requires a comparable type, got `{}`.",
                self.type_name(type_)
            ),
            file,
            line,
        );
    }

    fn argument_mode_for_type(&self, expected: &Option<&Type>) -> ExprMode {
        match expected {
            Some(type_) if !self.is_copyable_type(type_) => ExprMode::Transfer,
            _ => ExprMode::Read,
        }
    }

    fn thread_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        match (callee, index) {
            ("thread.start", 1) | ("thread.send", 1) => ExprMode::Transfer,
            ("thread.start", _) | ("thread.send", _) => ExprMode::Borrow,
            _ => ExprMode::Borrow,
        }
    }

    fn general_argument_mode(&self, callee: &str, index: usize) -> ExprMode {
        if matches!(
            callee,
            "len"
                | "get"
                | "getOr"
                | "find"
                | "keys"
                | "values"
                | "hasKey"
                | "contains"
                | "forEach"
                | "transform"
                | "filter"
                | "reduce"
                | "sum"
        ) {
            return ExprMode::Read;
        }
        if matches!(
            callee,
            "removeAt" | "removeKey" | "replace" | "set" | "append" | "prepend" | "insert"
        ) {
            return if index == 0 {
                ExprMode::Transfer
            } else {
                ExprMode::Read
            };
        }
        ExprMode::Read
    }

    fn is_resource_type(&self, type_: &Type) -> bool {
        match type_ {
            Type::User(name) => builtins::is_resource_type(name),
            _ => false,
        }
    }

    fn contains_resource_or_thread(&self, type_: &Type) -> bool {
        self.contains_resource_or_thread_with_seen(type_, &mut HashSet::new())
    }

    fn contains_resource_or_thread_with_seen(
        &self,
        type_: &Type,
        seen: &mut HashSet<String>,
    ) -> bool {
        match type_ {
            Type::Thread(_, _) | Type::ThreadWorker(_, _) => true,
            Type::User(name) if builtins::is_resource_type(name) => true,
            Type::List(element) => self.contains_resource_or_thread_with_seen(element, seen),
            Type::Map(key, value) => {
                self.contains_resource_or_thread_with_seen(key, seen)
                    || self.contains_resource_or_thread_with_seen(value, seen)
            }
            Type::Result(success) => self.contains_resource_or_thread_with_seen(success, seen),
            Type::Function { .. } => false,
            Type::User(name) => {
                if !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return false;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => false,
                    TypeDeclKind::Type => info.fields.iter().any(|field| {
                        self.contains_resource_or_thread_with_seen(&field.type_, seen)
                    }),
                    TypeDeclKind::Union => info.variants.iter().any(|variant| {
                        variant.fields.iter().any(|field| {
                            self.contains_resource_or_thread_with_seen(&field.type_, seen)
                        })
                    }),
                };
                seen.remove(name);
                result
            }
            _ => false,
        }
    }

    fn report_invalid_collection_element(
        &mut self,
        file: &AstFile,
        line: usize,
        role: &str,
        type_: &Type,
    ) {
        self.report(
            "TYPE_COLLECTION_OWNERSHIP_VIOLATION",
            &format!(
                "Ordinary collections cannot store {role} values of type `{}` because they contain a resource or thread handle.",
                self.type_name(type_)
            ),
            file,
            line,
        );
    }

    fn is_copyable_type(&self, type_: &Type) -> bool {
        self.is_copyable_type_with_seen(type_, &mut HashSet::new())
    }

    fn is_thread_sendable_type(&self, type_: &Type) -> bool {
        self.is_thread_sendable_type_with_seen(type_, &mut HashSet::new())
    }

    fn is_defaultable_type(&self, type_: &Type) -> bool {
        self.is_defaultable_type_with_seen(type_, &mut HashSet::new())
    }

    fn is_defaultable_type_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(element) => self.is_defaultable_type_with_seen(element, seen),
            Type::Map(key, value) => {
                self.is_defaultable_type_with_seen(key, seen)
                    && self.is_defaultable_type_with_seen(value, seen)
            }
            Type::Function { .. }
            | Type::Result(_)
            | Type::Thread(_, _)
            | Type::ThreadWorker(_, _) => false,
            Type::User(name) => {
                if builtins::is_resource_type(name) {
                    return false;
                }
                if !seen.insert(name.clone()) {
                    return false;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return false;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum | TypeDeclKind::Union => false,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_defaultable_type_with_seen(&field.type_, seen)),
                };
                seen.remove(name);
                result
            }
        }
    }

    fn is_copyable_type_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(element) => self.is_copyable_type_with_seen(element, seen),
            Type::Map(key, value) => {
                self.is_copyable_type_with_seen(key, seen)
                    && self.is_copyable_type_with_seen(value, seen)
            }
            Type::Result(success) => self.is_copyable_type_with_seen(success, seen),
            Type::Function { .. } => true,
            Type::Thread(_, _) | Type::ThreadWorker(_, _) => false,
            Type::User(name) => {
                if builtins::is_resource_type(name) {
                    return false;
                }
                if !seen.insert(name.clone()) {
                    return true;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result = match info.kind {
                    TypeDeclKind::Enum => true,
                    TypeDeclKind::Type => info
                        .fields
                        .iter()
                        .all(|field| self.is_copyable_type_with_seen(&field.type_, seen)),
                    TypeDeclKind::Union => info.variants.iter().all(|variant| {
                        variant
                            .fields
                            .iter()
                            .all(|field| self.is_copyable_type_with_seen(&field.type_, seen))
                    }),
                };
                seen.remove(name);
                result
            }
        }
    }

    fn is_thread_sendable_type_with_seen(&self, type_: &Type, seen: &mut HashSet<String>) -> bool {
        match type_ {
            Type::Boolean
            | Type::Byte
            | Type::Error
            | Type::Fixed
            | Type::Float
            | Type::Integer
            | Type::Nothing
            | Type::String
            | Type::Unknown => true,
            Type::List(element) => self.is_thread_sendable_type_with_seen(element, seen),
            Type::Map(key, value) => {
                self.is_thread_sendable_type_with_seen(key, seen)
                    && self.is_thread_sendable_type_with_seen(value, seen)
            }
            Type::Result(success) => self.is_thread_sendable_type_with_seen(success, seen),
            Type::Function { .. } | Type::Thread(_, _) | Type::ThreadWorker(_, _) => false,
            Type::User(name) => {
                if builtins::is_resource_type(name) {
                    return builtins::is_thread_sendable_resource_type(name);
                }
                if !seen.insert(name.clone()) {
                    return true;
                }
                let Some(info) = self.type_infos.get(name) else {
                    return true;
                };
                let result =
                    match info.kind {
                        TypeDeclKind::Enum => true,
                        TypeDeclKind::Type => info.fields.iter().all(|field| {
                            self.is_thread_sendable_type_with_seen(&field.type_, seen)
                        }),
                        TypeDeclKind::Union => info.variants.iter().all(|variant| {
                            variant.fields.iter().all(|field| {
                                self.is_thread_sendable_type_with_seen(&field.type_, seen)
                            })
                        }),
                    };
                seen.remove(name);
                result
            }
        }
    }

    fn report_thread_type_not_sendable(
        &mut self,
        file: &AstFile,
        line: usize,
        context: &str,
        type_: &Type,
    ) {
        self.report(
            "TYPE_THREAD_NOT_SENDABLE",
            &format!(
                "{context} requires a thread-sendable type, got `{}`.",
                self.type_name(type_)
            ),
            file,
            line,
        );
    }

    fn require_thread_sendable_type(
        &mut self,
        file: &AstFile,
        line: usize,
        context: &str,
        type_: &Type,
    ) {
        if !self.is_thread_sendable_type(type_) {
            self.report_thread_type_not_sendable(file, line, context, type_);
        }
    }

    fn check_thread_boundary_sendability(
        &mut self,
        file: &AstFile,
        display_callee: &str,
        callee: &str,
        arg_types: &[Type],
        return_type: &Type,
        line: usize,
    ) {
        match callee {
            "thread.start" => {
                if let Some(input) = arg_types.get(1) {
                    self.require_thread_sendable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}` input"),
                        input,
                    );
                }
                if let Type::Thread(message, output) = return_type {
                    self.require_thread_sendable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}` message type"),
                        message,
                    );
                    self.require_thread_sendable_type(
                        file,
                        line,
                        &format!("Call to `{display_callee}` output type"),
                        output,
                    );
                }
            }
            "thread.send" => {
                if let Some(handle) = arg_types.first() {
                    match handle {
                        Type::Thread(message, _) | Type::ThreadWorker(message, _) => {
                            self.require_thread_sendable_type(
                                file,
                                line,
                                &format!("Call to `{display_callee}` message type"),
                                message,
                            );
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn visible_from(&self, file: &AstFile, visibility: Visibility, owner_file_path: &str) -> bool {
        match visibility {
            Visibility::Export | Visibility::Package => true,
            Visibility::Private => file.path == owner_file_path,
        }
    }

    fn check_type_reference(&mut self, file: &AstFile, type_: &Type, line: usize) {
        match type_ {
            Type::List(element) => {
                self.check_type_reference(file, element, line);
                if self.contains_resource_or_thread(element) {
                    self.report_invalid_collection_element(file, line, "element", element);
                }
            }
            Type::Map(key, value) => {
                self.check_type_reference(file, key, line);
                self.check_type_reference(file, value, line);
                if self.contains_resource_or_thread(key) {
                    self.report_invalid_collection_element(file, line, "key", key);
                }
                self.require_comparable_type(file, line, "Map key type", key);
                if self.contains_resource_or_thread(value) {
                    self.report_invalid_collection_element(file, line, "value", value);
                }
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
            Type::Thread(message, output) | Type::ThreadWorker(message, output) => {
                self.check_type_reference(file, message, line);
                self.check_type_reference(file, output, line);
                self.require_thread_sendable_type(file, line, "Thread message type", message);
                self.require_thread_sendable_type(file, line, "Thread output type", output);
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
                let message = self.thread_type_argument_name(message);
                let output = self.thread_type_argument_name(output);
                format!("Thread OF {message} TO {output}")
            }
            Type::ThreadWorker(message, output) => {
                let message = self.thread_type_argument_name(message);
                let output = self.thread_type_argument_name(output);
                format!("ThreadWorker OF {message} TO {output}")
            }
            Type::User(name) => name.clone(),
            Type::Unknown => "Unknown".to_string(),
        }
    }

    fn thread_type_argument_name(&self, type_: &Type) -> String {
        let name = self.type_name(type_);
        if name.contains(" TO ") {
            format!("({name})")
        } else {
            name
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
    outer_locals: &HashMap<String, LocalInfo>,
    local_names: &HashSet<String>,
    seen: &mut HashSet<String>,
    captures: &mut Vec<CapturedLocal>,
) {
    match expression {
        Expression::Identifier(name) => {
            if let Some(local) = outer_locals.get(name) {
                if !local_names.contains(name) && seen.insert(name.clone()) {
                    captures.push(CapturedLocal {
                        name: name.clone(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
                    });
                }
            }
        }
        Expression::Call { callee, arguments } => {
            if let Some(local) = outer_locals.get(callee) {
                if !local_names.contains(callee) && seen.insert(callee.clone()) {
                    captures.push(CapturedLocal {
                        name: callee.clone(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
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
        Expression::String(_) | Expression::Number(_) | Expression::Boolean(_) => {}
    }
}

fn constructor_arg_value(argument: &ConstructorArg) -> &Expression {
    match argument {
        ConstructorArg::Positional(value) => value,
        ConstructorArg::Named { value, .. } => value,
    }
}

fn call_arg_value(argument: &CallArg) -> &Expression {
    match argument {
        CallArg::Positional(value) => value,
        CallArg::Named { value, .. } => value,
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

fn numeric_literal_is_zero(expression: &Expression) -> bool {
    match expression {
        Expression::Number(value) => value.parse::<f64>().is_ok_and(|number| number == 0.0),
        Expression::Unary { operator, operand }
            if operator == "-" && matches!(operand.as_ref(), Expression::Number(_)) =>
        {
            numeric_literal_is_zero(operand)
        }
        _ => false,
    }
}

fn promote_loop_numeric_type(start: &Type, end: &Type, step: &Type) -> Type {
    let Some(start_name) = numeric_type_name(start) else {
        return Type::Unknown;
    };
    let Some(end_name) = numeric_type_name(end) else {
        return Type::Unknown;
    };
    let Some(step_name) = numeric_type_name(step) else {
        return Type::Unknown;
    };
    let first =
        numeric::binary_result_type("+", start_name, end_name).unwrap_or(numeric::TYPE_INTEGER);
    let second =
        numeric::binary_result_type("+", first, step_name).unwrap_or(numeric::TYPE_INTEGER);
    type_from_numeric_name(second)
}

fn type_from_numeric_name(type_name: &str) -> Type {
    match type_name {
        numeric::TYPE_BYTE => Type::Byte,
        numeric::TYPE_INTEGER => Type::Integer,
        numeric::TYPE_FIXED => Type::Fixed,
        numeric::TYPE_FLOAT => Type::Float,
        _ => Type::Unknown,
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

fn read_only_record_type(type_name: &str) -> bool {
    type_name == builtins::io::TERMINAL_SIZE_TYPE || type_name.starts_with("MapEntry OF ")
}
