//! bug-976: a parameter is a resource because its TYPE is one.
//!
//! There is no `RES` keyword on a parameter (the parser rejects it,
//! `MFB_PARSE_PARAMETER_RES`). The flag the keyword used to set —
//! [`HirParam::resource`](super::HirParam) and
//! [`HirLinkParam::resource`](super::HirLinkParam) — is derived here instead, from
//! the parameter's type, with the predicate `ir::verify` uses to decide that a
//! binding of a type needs `RES` (`TypeEnv::is_resource_or_resource_union`): a type
//! with a registered close op (a built-in resource, a parent `Thread`, a project
//! `RESOURCE`, an imported package's `RESOURCE_TABLE` row), or a union with a
//! variant that has one. A parameter that carries a `STATE` clause is a resource
//! claim too, exactly as the `RES p AS T STATE S` spelling was.
//!
//! The spelling alone cannot answer it — `db AS Db` is a resource only because
//! some file declares `RESOURCE Db` — so the derivation runs over a whole
//! elaborated project, after the parse. [`super::elaborate`] runs it with what the
//! sources declare; the build path runs it again on the concrete
//! (post-monomorph) HIR with the imported packages' resources folded in, so an
//! instantiated `x AS T` and an imported `pkg::Db` parameter are answered from
//! their concrete types. The consumers (`ir::resource_escape`,
//! `ir::lower_link`, the resolver's close-op signature rule) read the flag
//! unchanged.

use std::collections::{HashMap, HashSet};

use super::{
    HirCallArg, HirConstructorArg, HirExpression, HirFile, HirItem, HirMatchPattern, HirParam,
    HirProject, HirStatement,
};
use crate::ast::TypeDeclKind;
use crate::types::ParameterType;

/// The resource types a project can name, for deciding a parameter's resource-ness.
#[derive(Default)]
pub(crate) struct ResourceTypes {
    /// Declared resource types with a close op: the project's `RESOURCE`
    /// declarations and the imported packages' resources. Built-in resources are
    /// answered by the registry (`builtin_resource_close_function`), not stored.
    declared: HashSet<ParameterType>,
    /// Union type → its direct variant types (the shape `ir::verify`'s
    /// `UnionInfo::variants` holds), local and imported.
    unions: HashMap<ParameterType, Vec<ParameterType>>,
}

impl ResourceTypes {
    /// The resource types the project's own sources declare.
    pub(crate) fn from_project(project: &HirProject) -> Self {
        let mut types = Self::default();
        for file in &project.files {
            types.add_file(file);
        }
        types
    }

    fn from_file(file: &HirFile) -> Self {
        let mut types = Self::default();
        types.add_file(file);
        types
    }

    // Every name below is a NOMINAL identity — a `RESOURCE`/`UNION` declaration's
    // own identifier, or an imported package's qualified resource, union or
    // variant name — never a type spelling to decompose, so each is the
    // `ParameterType::named` it already is (as `resolver::packages` builds an
    // imported type's identity), not a `parse`.
    fn add_file(&mut self, file: &HirFile) {
        for item in &file.items {
            match item {
                HirItem::Resource(decl) => {
                    self.declared.insert(ParameterType::named(&decl.name));
                }
                HirItem::Type(decl) if matches!(decl.kind, TypeDeclKind::Union) => {
                    self.unions.insert(
                        ParameterType::named(&decl.name),
                        decl.variants
                            .iter()
                            .map(|variant| variant.type_.clone())
                            .collect(),
                    );
                }
                _ => {}
            }
        }
    }

    /// Fold in the imported packages' resources (their `RESOURCE_TABLE` rows) and
    /// unions. A project's own declaration of the same name wins, as in `ir::verify`.
    pub(crate) fn with_imported(
        mut self,
        resources: &[crate::ir::ImportedResource],
        types: &[crate::ir::ImportedTypeDef],
    ) -> Self {
        for resource in resources {
            self.declared
                .insert(ParameterType::named(&resource.type_name));
        }
        for imported in types {
            if matches!(imported.kind, crate::ir::ImportedTypeKind::Union) {
                self.unions
                    .entry(ParameterType::named(&imported.name))
                    .or_insert_with(|| {
                        imported
                            .variants
                            .iter()
                            .map(|variant| ParameterType::named(&variant.name))
                            .collect()
                    });
            }
        }
        self
    }

    /// `ir::verify`'s `close_op_for(base).is_some()`.
    fn has_close_op(&self, base: &ParameterType) -> bool {
        self.declared.contains(base)
            || crate::codegen::resource::builtin_resource_close_function(base).is_some()
    }

    /// Whether `type_` is a resource or a resource union — `ir::verify`'s
    /// `is_resource_or_resource_union`, the rule that makes a binding of the type
    /// need `RES`.
    pub(crate) fn is_resource(&self, type_: &ParameterType) -> bool {
        let base = type_.without_state();
        self.has_close_op(&base)
            || self
                .unions
                .get(&base)
                .is_some_and(|variants| variants.iter().any(|variant| self.has_close_op(variant)))
    }

    /// A parameter's resource-ness: its type is a resource, or it carries `STATE`.
    pub(crate) fn is_resource_param(
        &self,
        type_: &ParameterType,
        state_type: Option<&ParameterType>,
    ) -> bool {
        state_type.is_some() || self.is_resource(type_)
    }
}

/// Derive every parameter's resource flag in `project` from its type.
pub(crate) fn derive_resource_params(project: &mut HirProject, types: &ResourceTypes) {
    for file in &mut project.files {
        derive_in_file(file, types);
    }
}

/// [`derive_resource_params`] for one file elaborated on its own (a registry
/// synthetic file), with the resource types that file declares.
pub(crate) fn derive_resource_params_in_file(file: &mut HirFile) {
    let types = ResourceTypes::from_file(file);
    derive_in_file(file, &types);
}

fn derive_in_file(file: &mut HirFile, types: &ResourceTypes) {
    for item in &mut file.items {
        match item {
            HirItem::Function(function) => {
                derive_in_params(&mut function.params, types);
                derive_in_statements(&mut function.body, types);
                if let Some(trap) = &mut function.trap {
                    derive_in_statements(&mut trap.body, types);
                }
            }
            HirItem::Link(link) => {
                for function in &mut link.functions {
                    for param in &mut function.params {
                        param.resource = param.state_type.is_some()
                            || param
                                .type_
                                .as_ref()
                                .is_some_and(|type_| types.is_resource(type_));
                        if let Some(default) = &mut param.default {
                            derive_in_expression(default, types);
                        }
                    }
                }
            }
            HirItem::Binding(binding) => {
                if let Some(value) = &mut binding.value {
                    derive_in_expression(value, types);
                }
            }
            HirItem::Type(_)
            | HirItem::Resource(_)
            | HirItem::FuncAlias(_)
            | HirItem::Doc(_)
            | HirItem::Testing(_) => {}
        }
    }
}

fn derive_in_params(params: &mut [HirParam], types: &ResourceTypes) {
    for param in params {
        param.resource = types.is_resource_param(&param.type_, param.state_type.as_ref());
        if let Some(default) = &mut param.default {
            derive_in_expression(default, types);
        }
    }
}

fn derive_in_statements(statements: &mut [HirStatement], types: &ResourceTypes) {
    for statement in statements {
        derive_in_statement(statement, types);
    }
}

fn derive_in_statement(statement: &mut HirStatement, types: &ResourceTypes) {
    match statement {
        HirStatement::Let { value, .. }
        | HirStatement::Return { value, .. }
        | HirStatement::Recover { value, .. } => {
            if let Some(value) = value {
                derive_in_expression(value, types);
            }
        }
        HirStatement::Exit { code, .. } => {
            if let Some(code) = code {
                derive_in_expression(code, types);
            }
        }
        HirStatement::Continue { .. } | HirStatement::Propagate { .. } => {}
        HirStatement::Fail { error, .. } => derive_in_expression(error, types),
        HirStatement::Assign { value, .. }
        | HirStatement::StateAssign { value, .. }
        | HirStatement::Expression {
            expression: value, ..
        } => derive_in_expression(value, types),
        HirStatement::If {
            condition,
            then_body,
            else_body,
            ..
        } => {
            derive_in_expression(condition, types);
            derive_in_statements(then_body, types);
            derive_in_statements(else_body, types);
        }
        HirStatement::Match {
            expression, cases, ..
        } => {
            derive_in_expression(expression, types);
            for case in cases {
                match &mut case.pattern {
                    HirMatchPattern::Literal(value) => derive_in_expression(value, types),
                    HirMatchPattern::OneOf(values) => {
                        for value in values {
                            derive_in_expression(value, types);
                        }
                    }
                    HirMatchPattern::Else | HirMatchPattern::Union { .. } => {}
                }
                if let Some(guard) = &mut case.guard {
                    derive_in_expression(guard, types);
                }
                derive_in_statements(&mut case.body, types);
            }
        }
        HirStatement::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            derive_in_expression(start, types);
            derive_in_expression(end, types);
            if let Some(step) = step {
                derive_in_expression(step, types);
            }
            derive_in_statements(body, types);
        }
        HirStatement::ForEach { iterable, body, .. } => {
            derive_in_expression(iterable, types);
            derive_in_statements(body, types);
        }
        HirStatement::While {
            condition, body, ..
        }
        | HirStatement::DoUntil {
            body, condition, ..
        } => {
            derive_in_expression(condition, types);
            derive_in_statements(body, types);
        }
    }
}

fn derive_in_expression(expression: &mut HirExpression, types: &ResourceTypes) {
    match expression {
        HirExpression::String(_)
        | HirExpression::Number(_)
        | HirExpression::Scalar(_)
        | HirExpression::Boolean(_)
        | HirExpression::Identifier(_) => {}
        HirExpression::Binary { left, right, .. } => {
            derive_in_expression(left, types);
            derive_in_expression(right, types);
        }
        HirExpression::Unary { operand, .. } => derive_in_expression(operand, types),
        HirExpression::Call { arguments, .. } => {
            for argument in arguments {
                match argument {
                    HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                        derive_in_expression(value, types)
                    }
                }
            }
        }
        HirExpression::Lambda { params, body, .. } => {
            derive_in_params(params, types);
            derive_in_expression(body, types);
        }
        HirExpression::Constructor { arguments, .. } => {
            for argument in arguments {
                match argument {
                    HirConstructorArg::Positional(value)
                    | HirConstructorArg::Named { value, .. } => derive_in_expression(value, types),
                }
            }
        }
        HirExpression::WithUpdate { target, updates } => {
            derive_in_expression(target, types);
            for update in updates {
                derive_in_expression(&mut update.value, types);
            }
        }
        HirExpression::ListLiteral(values)
        | HirExpression::SetLiteral {
            elements: values, ..
        } => {
            for value in values {
                derive_in_expression(value, types);
            }
        }
        HirExpression::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                derive_in_expression(key, types);
                derive_in_expression(value, types);
            }
        }
        HirExpression::MemberAccess { target, .. } => derive_in_expression(target, types),
        HirExpression::Trapped {
            expression,
            handler,
            ..
        } => {
            derive_in_expression(expression, types);
            derive_in_statements(handler, types);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{derive_resource_params, ResourceTypes};
    use crate::hir::{elaborate, HirExpression, HirItem, HirProject, HirStatement};

    fn project(source: &str) -> HirProject {
        let file = crate::ast::parse_source(Path::new("main.mfb"), "main.mfb", source)
            .expect("test source should parse");
        elaborate(&crate::ast::AstProject {
            name: "test".to_string(),
            files: vec![file],
        })
    }

    /// `(function, parameter) -> resource` for every FUNC parameter.
    fn func_params(hir: &HirProject) -> Vec<(String, String, bool)> {
        hir.files
            .iter()
            .flat_map(|file| &file.items)
            .filter_map(|item| match item {
                HirItem::Function(function) => Some(function),
                _ => None,
            })
            .flat_map(|function| {
                function
                    .params
                    .iter()
                    .map(|param| (function.name.clone(), param.name.clone(), param.resource))
            })
            .collect()
    }

    fn resource_of(hir: &HirProject, function: &str, param: &str) -> bool {
        func_params(hir)
            .into_iter()
            .find(|(f, p, _)| f == function && p == param)
            .unwrap_or_else(|| panic!("{function}({param}) present"))
            .2
    }

    /// The decision table: a parameter is a resource exactly when its type is one
    /// (built-in, a parent `Thread`, a project `RESOURCE`, a resource union) or it
    /// carries `STATE`; a data type, a record, a data union and a worker view (no
    /// close op today) are not.
    #[test]
    fn a_parameter_is_a_resource_exactly_when_its_type_is_one() {
        let hir = project(
            "IMPORT fs\nIMPORT tcp\n\
             TYPE Cur\n  pos AS Integer\nEND TYPE\n\
             UNION Stream\n  fs::File\n  tcp::Socket\nEND UNION\n\
             UNION Shape\n  Cur\nEND UNION\n\
             RESOURCE Db CLOSE BY l::close\n\
             LINK \"x\" AS l\n  FUNC close(db AS Db) AS Nothing\n    SYMBOL \"x_close\"\n    ABI (db CPtr) AS st CInt32\n  END FUNC\nEND LINK\n\
             FUNC f(a AS fs::File, b AS fs::File STATE Cur, c AS Stream, d AS Db, \
             e AS tcp::Socket, n AS Integer, r AS Cur, u AS Shape, \
             t AS Thread OF Integer TO Integer, w AS ThreadWorker OF Integer TO Integer, \
             s AS Integer STATE Cur) AS Nothing\nEND FUNC\n",
        );
        for (param, expected) in [
            ("a", true),
            ("b", true),
            ("c", true),
            ("d", true),
            ("e", true),
            ("n", false),
            ("r", false),
            ("u", false),
            ("t", true),
            ("w", false),
            // A STATE clause is a resource claim, exactly as `RES s AS Integer
            // STATE Cur` was: the derivation keeps what the keyword meant.
            ("s", true),
        ] {
            assert_eq!(
                resource_of(&hir, "f", param),
                expected,
                "parameter `{param}`"
            );
        }
        let link_param = hir
            .files
            .iter()
            .flat_map(|file| &file.items)
            .find_map(|item| match item {
                HirItem::Link(link) => link.functions[0].params.first(),
                _ => None,
            })
            .expect("LINK close param present");
        assert!(
            link_param.resource,
            "a LINK close op's `db AS Db` parameter is a resource by its type"
        );
    }

    /// A LAMBDA parameter is derived the same way as a FUNC parameter.
    #[test]
    fn a_lambda_parameter_is_derived_from_its_type() {
        let hir = project(
            "IMPORT fs\nFUNC main AS Integer\n  LET p = LAMBDA(x AS fs::File) -> TRUE\n  LET q = LAMBDA(n AS Integer) -> TRUE\n  RETURN 0\nEND FUNC\n",
        );
        let lambdas: Vec<bool> = hir
            .files
            .iter()
            .flat_map(|file| &file.items)
            .filter_map(|item| match item {
                HirItem::Function(function) => Some(function),
                _ => None,
            })
            .flat_map(|function| &function.body)
            .filter_map(|statement| match statement {
                HirStatement::Let {
                    value: Some(HirExpression::Lambda { params, .. }),
                    ..
                } => Some(params[0].resource),
                _ => None,
            })
            .collect();
        assert_eq!(lambdas, vec![true, false]);
    }

    /// An imported package's resource is only known once the importer's package
    /// tables are folded in — the build path's second derivation.
    #[test]
    fn an_imported_resource_counts_once_folded_in() {
        let mut hir = project("IMPORT demo\nFUNC f(db AS demo::Db) AS Nothing\nEND FUNC\n");
        assert!(
            !resource_of(&hir, "f", "db"),
            "unknown to the sources alone"
        );
        let types = ResourceTypes::from_project(&hir).with_imported(
            &[crate::ir::ImportedResource {
                type_name: "demo.Db".to_string(),
                close_function: "demo.close".to_string(),
                sendable: false,
            }],
            &[],
        );
        derive_resource_params(&mut hir, &types);
        assert!(resource_of(&hir, "f", "db"));
    }
}
