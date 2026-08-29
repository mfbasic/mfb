//! The pre-lowering shape pass (plan-107-E).
//!
//! `ir::verify` checks the IR, on both the source path and the package path.
//! A handful of source rules cannot live there because lowering *erases* the
//! evidence they need — a named argument is normalized into positional order,
//! an omitted named argument is filled from the parameter's default, and so on.
//! Those rules run here, over the concrete `HirProject` lowering is about to
//! consume, in the build's first diagnostic stream (before `ir::verify`'s).
//!
//! The pass carries no type inference of its own. Where a rule needs the type
//! of an expression it asks lowering — [`lower::expression_type`] against a
//! [`lower::LowerContext`] built from the same [`lower::LowerFacts`] the build
//! path lowers with — and it tracks its local scopes exactly as
//! `lower_statement_block` does (parameters, `LET`/`MUT`/`RES` binds, `FOR` /
//! `FOR EACH` variables, `MATCH` case bindings, trap bindings, lambda
//! parameters), so a rule sees precisely the type the lowered `IrValue` will
//! carry. Total lowering (plan-20-D) already tolerates ill-typed input, so the
//! oracle never panics on the erroneous programs these rules exist for.
//!
//! Every rule this pass emits carries a one-line justification naming the
//! erased evidence that keeps it out of `ir::verify`.

use super::lower::{self, LowerContext, LowerFacts};
use super::{ExternalSignature, ImportedTypeDef};
use crate::ast::Visibility;
use crate::codegen::builtins;
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFile, HirFunction, HirItem, HirMatchCase,
    HirProject, HirStatement,
};
use crate::rules::PendingDiagnostic;
use crate::types::ParameterType;
use std::collections::HashMap;
use std::path::Path;

/// Run the shape pass over `hir` and return its diagnostics in traversal
/// (source) order, un-rendered, for the build path to merge with the other
/// streams. `external_signatures` and `imported_types` are the same inputs the
/// build path hands `lower_augmented_project`, so the typing seam sees exactly
/// the tables lowering will; `imported_signatures` is the UNFILTERED signature
/// table of every imported `.mfp` (the parameter-name source for a call into
/// an imported package — lowering keeps only the resource-returning subset).
pub(crate) fn collect_diagnostics(
    project_dir: &Path,
    hir: &HirProject,
    external_signatures: &HashMap<String, ExternalSignature>,
    imported_types: &[ImportedTypeDef],
    imported_signatures: &HashMap<String, ExternalSignature>,
) -> Vec<PendingDiagnostic> {
    let facts = lower::lower_facts(hir, external_signatures, imported_types);
    let mut walker = Walker::new(project_dir, &facts, hir, imported_signatures);
    walker.walk_project(hir);
    walker.diagnostics
}

/// Standalone form for callers that render rather than merge (`mfb audit`):
/// prints the diagnostics and reports whether any was an error.
pub(crate) fn check_project(
    project_dir: &Path,
    hir: &HirProject,
    imported_signatures: &HashMap<String, ExternalSignature>,
) -> Result<(), ()> {
    let diagnostics =
        collect_diagnostics(project_dir, hir, &HashMap::new(), &[], imported_signatures);
    let had_error = diagnostics.iter().any(|d| crate::rules::is_error(&d.rule));
    crate::rules::render_pending(diagnostics);
    if had_error {
        Err(())
    } else {
        Ok(())
    }
}

/// A callee's parameter names as a rule needs them: the declared user/imported
/// function's list, or a builtin's per-position alias table.
enum CalleeParams {
    /// A user-declared or imported-package function: one name per position.
    Declared(Vec<String>),
    /// A builtin with a merged per-position alias table.
    Builtin(Vec<Vec<&'static str>>),
    /// A builtin whose overloads place a name at different positions, listed
    /// one overload at a time.
    BuiltinOverloads(Vec<Vec<&'static str>>),
    /// A builtin with no parameter-name metadata: names cannot bind at all.
    BuiltinUnnamed,
}

/// The walk state: lowering's context (positioned per file / per function
/// exactly as lowering positions it), the project's own function visibility
/// table, and the diagnostics collected so far.
struct Walker<'a> {
    project_dir: &'a Path,
    context: LowerContext<'a>,
    /// Every declared function's parameter names, visibility and owner file —
    /// the visibility filter the source checker applied to a call target
    /// (`PRIVATE` is callable from its own file only; an invisible target is
    /// simply not a function call to it).
    functions: HashMap<String, DeclaredFunction>,
    imported_signatures: &'a HashMap<String, ExternalSignature>,
    /// Project-relative path of the file being walked, for diagnostic paths.
    file: String,
    diagnostics: Vec<PendingDiagnostic>,
    /// Every `LET`/`MUT`/`RES` binding's computed type in walk order — the
    /// seam-fidelity probe the unit tests compare against lowering's stamped
    /// `IrOp::Bind` types.
    #[cfg(test)]
    bound_types: Vec<(String, ParameterType)>,
}

struct DeclaredFunction {
    params: Vec<String>,
    visibility: Visibility,
    owner_file: String,
}

impl<'a> Walker<'a> {
    fn new(
        project_dir: &'a Path,
        facts: &'a LowerFacts,
        hir: &HirProject,
        imported_signatures: &'a HashMap<String, ExternalSignature>,
    ) -> Self {
        let mut functions = HashMap::new();
        for file in &hir.files {
            for item in &file.items {
                if let HirItem::Function(function) = item {
                    functions.insert(
                        function.name.clone(),
                        DeclaredFunction {
                            params: function
                                .params
                                .iter()
                                .map(|param| param.name.clone())
                                .collect(),
                            visibility: function.visibility,
                            owner_file: file.path.clone(),
                        },
                    );
                }
            }
        }
        Walker {
            project_dir,
            context: facts.context(),
            functions,
            imported_signatures,
            file: String::new(),
            diagnostics: Vec::new(),
            #[cfg(test)]
            bound_types: Vec::new(),
        }
    }

    fn walk_project(&mut self, hir: &HirProject) {
        for file in &hir.files {
            self.walk_file(file);
        }
    }

    fn walk_file(&mut self, file: &HirFile) {
        self.context.current_imports = file.import_bindings();
        self.context.current_file = file.path.clone();
        self.file = file.path.clone();
        for item in &file.items {
            match item {
                HirItem::Binding(binding) => {
                    if let Some(value) = &binding.value {
                        let locals = HashMap::new();
                        self.walk_expression(value, &locals);
                    }
                }
                HirItem::Function(function) => self.walk_function(function),
                // Declarations without executable bodies: their rules are
                // `ir::verify`'s (types, LINK blocks, resources) or the parser's.
                HirItem::Type(_)
                | HirItem::Resource(_)
                | HirItem::FuncAlias(_)
                | HirItem::Link(_)
                | HirItem::Doc(_)
                | HirItem::Testing(_) => {}
            }
        }
    }

    fn walk_function(&mut self, function: &HirFunction) {
        // Parameter locals as `lower_function` seeds them: a `RES` parameter's
        // `STATE T` rides in its type.
        let mut locals = HashMap::new();
        for param in &function.params {
            let type_ = match &param.state_type {
                Some(state) => param.type_.with_state(state),
                None => param.type_.clone(),
            };
            if let Some(default) = &param.default {
                self.walk_expression(default, &locals);
            }
            locals.insert(param.name.clone(), type_);
        }
        let previous_return_type = self.context.current_return_type.take();
        self.context.current_return_type = Some(lower::function_return_type(function));
        self.walk_block(&function.body, &locals);
        if let Some(trap) = &function.trap {
            let mut trap_locals = locals.clone();
            trap_locals.insert(trap.name.clone(), ParameterType::named("Error"));
            self.walk_block(&trap.body, &trap_locals);
        }
        self.context.current_return_type = previous_return_type;
    }

    /// Walk a block in a scope of its own — `lower_statement_block` clones the
    /// enclosing locals per block, so a binding never leaks out of it.
    fn walk_block(&mut self, body: &[HirStatement], locals: &HashMap<String, ParameterType>) {
        let mut nested = locals.clone();
        for statement in body {
            self.walk_statement(statement, &mut nested);
        }
    }

    fn walk_statement(
        &mut self,
        statement: &HirStatement,
        locals: &mut HashMap<String, ParameterType>,
    ) {
        match statement {
            HirStatement::Let {
                state_type,
                name,
                type_,
                explicit_type,
                value,
                ..
            } => {
                let declared_type = explicit_type.then(|| type_.clone());
                if let Some(HirExpression::Trapped {
                    expression,
                    binding,
                    handler,
                    ..
                }) = value
                {
                    // The inline-TRAP form: the binding takes the trapped
                    // expression's success type (`lower_inline_trap`).
                    let success_type = declared_type
                        .clone()
                        .or_else(|| lower::expression_type(expression, locals, &self.context))
                        .unwrap_or(ParameterType::Unknown);
                    let success_type = match (&declared_type, state_type) {
                        (Some(declared_type), Some(state)) => declared_type.with_state(state),
                        _ => success_type,
                    };
                    self.walk_expression(expression, locals);
                    self.walk_handler(binding, handler, locals);
                    self.bind(name, success_type, locals);
                    return;
                }
                let lowered_type = declared_type.unwrap_or_else(|| {
                    value
                        .as_ref()
                        .and_then(|value| lower::expression_type(value, locals, &self.context))
                        .unwrap_or(ParameterType::Unknown)
                });
                let lowered_type = match state_type {
                    Some(state) => lowered_type.with_state(state),
                    None => lowered_type,
                };
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                }
                self.bind(name, lowered_type, locals);
            }
            HirStatement::Return { value, .. } => {
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                }
            }
            HirStatement::Exit { code, .. } => {
                if let Some(code) = code {
                    self.walk_expression(code, locals);
                }
            }
            HirStatement::Continue { .. } | HirStatement::Propagate { .. } => {}
            HirStatement::Fail { error, .. } => self.walk_expression(error, locals),
            HirStatement::Recover { value, .. } => {
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                }
            }
            HirStatement::Assign { value, .. } | HirStatement::StateAssign { value, .. } => {
                self.walk_value(value, locals);
            }
            HirStatement::Expression { expression, .. } => {
                self.walk_value(expression, locals);
            }
            HirStatement::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                self.walk_expression(condition, locals);
                self.walk_block(then_body, locals);
                self.walk_block(else_body, locals);
            }
            HirStatement::Match {
                expression, cases, ..
            } => {
                let matched_type = lower::match_expression_type(expression, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                self.walk_expression(expression, locals);
                for case in cases {
                    self.walk_match_case(case, &matched_type, locals);
                }
            }
            HirStatement::For {
                name,
                start,
                end,
                step,
                body,
                ..
            } => {
                // The loop variable's type is the promoted numeric type of the
                // three bounds, as lowering computes it.
                let start_type = lower::expression_type(start, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                let end_type = lower::expression_type(end, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                let step_type = step
                    .as_ref()
                    .and_then(|value| lower::expression_type(value, locals, &self.context))
                    .unwrap_or(ParameterType::Integer);
                let loop_type = crate::numeric::typed_promote_loop_numeric_type(
                    &start_type,
                    &end_type,
                    &step_type,
                );
                self.walk_expression(start, locals);
                self.walk_expression(end, locals);
                if let Some(step) = step {
                    self.walk_expression(step, locals);
                }
                let mut nested = locals.clone();
                self.bind(name, loop_type, &mut nested);
                self.walk_block(body, &nested);
            }
            HirStatement::ForEach {
                name,
                iterable,
                body,
                ..
            } => {
                let iterable_type = lower::expression_type(iterable, locals, &self.context)
                    .unwrap_or(ParameterType::Unknown);
                let element_type = lower::collection_iteration_type(&iterable_type)
                    .unwrap_or(ParameterType::Unknown);
                self.walk_expression(iterable, locals);
                let mut nested = locals.clone();
                self.bind(name, element_type, &mut nested);
                self.walk_block(body, &nested);
            }
            HirStatement::While {
                condition, body, ..
            } => {
                self.walk_expression(condition, locals);
                self.walk_block(body, locals);
            }
            HirStatement::DoUntil {
                body, condition, ..
            } => {
                self.walk_block(body, locals);
                self.walk_expression(condition, locals);
            }
        }
    }

    /// A statement-position value: an inline-TRAP form walks its handler in the
    /// binding's scope; anything else is an ordinary expression.
    fn walk_value(&mut self, value: &HirExpression, locals: &HashMap<String, ParameterType>) {
        if let HirExpression::Trapped {
            expression,
            binding,
            handler,
            ..
        } = value
        {
            self.walk_expression(expression, locals);
            self.walk_handler(binding, handler, locals);
            return;
        }
        self.walk_expression(value, locals);
    }

    /// An inline-TRAP handler block: the error binding is an `Error` local
    /// visible only inside it (`lower_inline_trap`).
    fn walk_handler(
        &mut self,
        binding: &str,
        handler: &[HirStatement],
        locals: &HashMap<String, ParameterType>,
    ) {
        let mut handler_locals = locals.clone();
        handler_locals.insert(binding.to_string(), ParameterType::named("Error"));
        self.walk_block(handler, &handler_locals);
    }

    fn walk_match_case(
        &mut self,
        case: &HirMatchCase,
        matched_type: &ParameterType,
        locals: &HashMap<String, ParameterType>,
    ) {
        use crate::hir::HirMatchPattern;
        match &case.pattern {
            HirMatchPattern::Else | HirMatchPattern::Union { .. } => {}
            HirMatchPattern::Literal(expression) => self.walk_expression(expression, locals),
            HirMatchPattern::OneOf(expressions) => {
                for expression in expressions {
                    self.walk_expression(expression, locals);
                }
            }
        }
        let mut case_locals = locals.clone();
        // The matched local's name is irrelevant to the binding's TYPE (it only
        // names the extract's source); the pattern and scrutinee type decide it.
        if let Some((binding, binding_type, _)) =
            lower::match_case_binding(&case.pattern, "$match", matched_type)
        {
            case_locals.insert(binding, binding_type);
        }
        if let Some(guard) = &case.guard {
            self.walk_expression(guard, &case_locals);
        }
        self.walk_block(&case.body, &case_locals);
    }

    fn walk_expression(
        &mut self,
        expression: &HirExpression,
        locals: &HashMap<String, ParameterType>,
    ) {
        match expression {
            HirExpression::String(_)
            | HirExpression::Number(_)
            | HirExpression::Scalar(_)
            | HirExpression::Boolean(_)
            | HirExpression::Identifier(_) => {}
            HirExpression::Binary { left, right, .. } => {
                self.walk_expression(left, locals);
                self.walk_expression(right, locals);
            }
            HirExpression::Unary { operand, .. } => self.walk_expression(operand, locals),
            HirExpression::Call {
                callee, arguments, ..
            } => {
                // The call's own shape rules report before its arguments are
                // walked — the source checker normalized the argument list
                // before inferring any argument, so a nested call's rule follows
                // the enclosing call's.
                self.check_named_arguments(callee, arguments);
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                            self.walk_expression(value, locals)
                        }
                    }
                }
            }
            HirExpression::Lambda { params, body, .. } => {
                // Lambda parameters shadow the enclosing scope for the body
                // (`expression_type`'s Lambda arm).
                let mut nested = locals.clone();
                for param in params {
                    nested.insert(param.name.clone(), param.type_.clone());
                }
                self.walk_expression(body, &nested);
            }
            HirExpression::Constructor { arguments, .. } => {
                for argument in arguments {
                    match argument {
                        HirConstructorArg::Positional(value)
                        | HirConstructorArg::Named { value, .. } => {
                            self.walk_expression(value, locals)
                        }
                    }
                }
            }
            HirExpression::WithUpdate { target, updates } => {
                self.walk_expression(target, locals);
                for update in updates {
                    self.walk_expression(&update.value, locals);
                }
            }
            HirExpression::ListLiteral(elements) | HirExpression::SetLiteral { elements, .. } => {
                for element in elements {
                    self.walk_expression(element, locals);
                }
            }
            HirExpression::MapLiteral { entries, .. } => {
                for (key, value) in entries {
                    self.walk_expression(key, locals);
                    self.walk_expression(value, locals);
                }
            }
            HirExpression::MemberAccess { target, .. } => self.walk_expression(target, locals),
            HirExpression::Trapped {
                expression,
                binding,
                handler,
                ..
            } => {
                self.walk_expression(expression, locals);
                self.walk_handler(binding, handler, locals);
            }
        }
    }

    /// The parameter names of the function a call names, resolved the way the
    /// source checker resolved a call target: a TESTING expectation or a package
    /// constant is not a function call; a builtin (by canonical name) comes
    /// before any declared function; a declared function must be visible from
    /// the calling file; an imported package's function is looked up under its
    /// canonical `package.member` name. Anything else (a function value, an
    /// unresolved dotted name) has no parameter names to bind against.
    fn callee_params(&self, callee: &str) -> Option<CalleeParams> {
        if crate::codegen::builtins_testing::is_testing_call(callee) {
            return None;
        }
        let (binding, member) = match callee.split_once('.') {
            Some((binding, member)) => (Some(binding), member),
            None => (None, callee),
        };
        let resolved_package =
            binding.and_then(|binding| self.context.current_imports.get(binding));
        let canonical = match (resolved_package, binding) {
            // `IMPORT self` binds the package's own exports under their bare names.
            (Some(package), _) if package == crate::ast::SELF_IMPORT => member.to_string(),
            (Some(package), _) => format!("{package}.{member}"),
            _ => callee.to_string(),
        };
        if builtins::is_package_constant(&canonical) {
            return None;
        }
        if builtins::is_builtin_call(&canonical) {
            if !crate::syntaxcheck::checks_builtin_call_arguments(&canonical) {
                return None;
            }
            if let Some(overloads) = builtins::call_param_name_overloads(&canonical) {
                return Some(CalleeParams::BuiltinOverloads(overloads));
            }
            return Some(match builtins::call_param_names(&canonical) {
                Some(names) => CalleeParams::Builtin(names),
                None => CalleeParams::BuiltinUnnamed,
            });
        }
        let declared = self
            .functions
            .get(callee)
            .or_else(|| self.functions.get(&canonical))
            .filter(|function| match function.visibility {
                Visibility::Export | Visibility::Public => true,
                Visibility::Private => function.owner_file == self.file,
            });
        if let Some(function) = declared {
            return Some(CalleeParams::Declared(function.params.clone()));
        }
        // An imported package's function: only through an import binding of
        // this file (a dotted name whose prefix is not an import is not a call
        // into a package, whatever the dependency table happens to hold).
        if resolved_package.is_some() {
            if let Some(signature) = self.imported_signatures.get(&canonical) {
                return Some(CalleeParams::Declared(
                    signature
                        .params
                        .iter()
                        .map(|param| param.name.clone())
                        .collect(),
                ));
            }
        }
        None
    }

    /// TYPE_UNKNOWN_ARGUMENT_NAME / TYPE_DUPLICATE_ARGUMENT_NAME.
    ///
    /// Shape-pass rules: lowering normalizes named arguments into positional
    /// order and silently drops a name that binds to no parameter (or binds a
    /// parameter twice), so the lowered `IrValue::Call` carries no trace of the
    /// name the source wrote — the evidence exists only in the HIR.
    fn check_named_arguments(&mut self, callee: &str, arguments: &[HirCallArg]) {
        if !arguments
            .iter()
            .any(|argument| matches!(argument, HirCallArg::Named { .. }))
        {
            return;
        }
        let Some(params) = self.callee_params(callee) else {
            return;
        };
        match params {
            CalleeParams::BuiltinUnnamed => {
                // No parameter-name metadata: a name cannot bind at all (bug-173 B).
                for argument in arguments {
                    if let HirCallArg::Named { name, line, .. } = argument {
                        self.report_unknown_name(callee, name, *line);
                    }
                }
            }
            CalleeParams::BuiltinOverloads(overloads) => {
                // Overload selection needs a well-formed name set: the first
                // duplicate, else the first unknown name, ends the check.
                let named: Vec<(&String, usize)> = arguments
                    .iter()
                    .filter_map(|argument| match argument {
                        HirCallArg::Named { name, line, .. } => Some((name, *line)),
                        HirCallArg::Positional(_) => None,
                    })
                    .collect();
                // (The duplicate itself is still the source checker's
                // TYPE_DUPLICATE_ARGUMENT_NAME until its own landing.)
                for (index, (name, _)) in named.iter().enumerate() {
                    if named[..index].iter().any(|(earlier, _)| earlier == name) {
                        return;
                    }
                }
                if let Some((name, line)) = named.iter().find(|(name, _)| {
                    !overloads
                        .iter()
                        .any(|params| params.contains(&name.as_str()))
                }) {
                    self.report_unknown_name(callee, name, *line);
                }
            }
            CalleeParams::Builtin(aliases) => {
                let mut ordered = vec![false; aliases.len()];
                let mut next_positional = 0usize;
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(_) => {
                            while next_positional < ordered.len() && ordered[next_positional] {
                                next_positional += 1;
                            }
                            if next_positional < ordered.len() {
                                ordered[next_positional] = true;
                                next_positional += 1;
                            }
                        }
                        HirCallArg::Named { name, line, .. } => {
                            let Some(index) = aliases
                                .iter()
                                .position(|aliases| aliases.iter().any(|alias| alias == name))
                            else {
                                self.report_unknown_name(callee, name, *line);
                                continue;
                            };
                            if ordered[index] {
                                // A duplicate is still the source checker's
                                // TYPE_DUPLICATE_ARGUMENT_NAME until its own landing.
                                continue;
                            }
                            ordered[index] = true;
                        }
                    }
                }
            }
            CalleeParams::Declared(params) => {
                let mut ordered = vec![false; params.len()];
                let mut next_positional = 0usize;
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(_) => {
                            while next_positional < ordered.len() && ordered[next_positional] {
                                next_positional += 1;
                            }
                            if next_positional >= ordered.len() {
                                continue;
                            }
                            ordered[next_positional] = true;
                            next_positional += 1;
                        }
                        HirCallArg::Named { name, line, .. } => {
                            let Some(index) = params.iter().position(|param| param == name) else {
                                self.report_unknown_name(callee, name, *line);
                                continue;
                            };
                            if ordered[index] {
                                // A duplicate is still the source checker's
                                // TYPE_DUPLICATE_ARGUMENT_NAME until its own landing.
                                continue;
                            }
                            ordered[index] = true;
                        }
                    }
                }
            }
        }
    }

    fn report_unknown_name(&mut self, callee: &str, name: &str, line: usize) {
        self.emit(
            "TYPE_UNKNOWN_ARGUMENT_NAME",
            format!("Call to `{callee}` does not have a parameter named `{name}`."),
            line,
        );
    }

    /// Bind `name` in `locals` at `type_`, exactly where lowering emits its
    /// `IrOp::Bind` for the same binding.
    fn bind(
        &mut self,
        name: &str,
        type_: ParameterType,
        locals: &mut HashMap<String, ParameterType>,
    ) {
        #[cfg(test)]
        self.bound_types.push((name.to_string(), type_.clone()));
        locals.insert(name.to_string(), type_);
    }

    /// Record a diagnostic at `line` of the current file.
    fn emit(&mut self, rule: &str, detail: String, line: usize) {
        self.diagnostics.push(PendingDiagnostic {
            rule: rule.to_string(),
            detail,
            path: self.project_dir.join(&self.file),
            line,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse_source, AstProject};
    use crate::ir::IrOp;

    /// Parse `src` as `main.mfb`, augment it with the builtin package sources
    /// (the same chain the build path runs before lowering), and return the
    /// concrete HIR.
    fn hir_from(src: &str) -> HirProject {
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src)
            .expect("test source must lex+parse");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        crate::resolver::augment_hir_project(&crate::hir::elaborate(&project))
            .expect("builtin augmentation must succeed")
    }

    /// The shape pass's rule codes for `src`, in traversal order.
    fn shape_codes(src: &str) -> Vec<String> {
        collect_diagnostics(
            Path::new("/proj"),
            &hir_from(src),
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .into_iter()
        .map(|d| d.rule)
        .collect()
    }

    fn rejects_with(src: &str, rule: &str) -> bool {
        shape_codes(src).iter().any(|r| r == rule)
    }

    fn accepts(src: &str) -> bool {
        shape_codes(src).is_empty()
    }

    fn wrap_import(import: &str, body: &str) -> String {
        format!("IMPORT {import}\nFUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
    }

    /// Every named `Bind` in the lowered IR (the `$` temps lowering mints have
    /// no HIR counterpart), in emission order.
    fn lowered_binds(ops: &[IrOp], out: &mut Vec<(String, ParameterType)>) {
        for op in ops {
            match op {
                IrOp::Bind { name, type_, .. } => {
                    if !name.starts_with('$') {
                        out.push((name.clone(), type_.clone()));
                    }
                }
                IrOp::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    lowered_binds(then_body, out);
                    lowered_binds(else_body, out);
                }
                IrOp::Match { cases, .. } => {
                    for case in cases {
                        lowered_binds(&case.body, out);
                    }
                }
                // The FOR EACH element is bound by the loop op itself, not by a
                // `Bind` inside its body.
                IrOp::ForEach {
                    name, type_, body, ..
                } => {
                    out.push((name.clone(), type_.clone()));
                    lowered_binds(body, out);
                }
                IrOp::For { body, .. }
                | IrOp::While { body, .. }
                | IrOp::DoUntil { body, .. }
                | IrOp::Trap { body, .. } => lowered_binds(body, out),
                _ => {}
            }
        }
    }

    // The seam-fidelity proof: the walker's scope tracking (parameters, LET
    // with/without annotation, inline-TRAP LET, FOR with numeric promotion,
    // FOR EACH element, MATCH case binding, trap binding, lambda parameter)
    // yields, for every binding, the type lowering stamps on its `IrOp::Bind`.
    #[test]
    fn walker_types_bindings_exactly_as_lowering_does() {
        let src = "IMPORT collections\n\
                   TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n\
                   UNION Shape\n  Point\nEND UNION\n\
                   FUNC helper(n AS Integer) AS Float\n  RETURN n * 1.5\nEND FUNC\n\
                   FUNC main AS Integer\n\
                   \x20 LET a = 1\n\
                   \x20 LET b AS Float = 2.5\n\
                   \x20 LET c = helper(a)\n\
                   \x20 LET items = [1, 2, 3]\n\
                   \x20 FOR i = a TO 10 STEP 0.5\n\
                   \x20   LET d = i\n\
                   \x20 NEXT\n\
                   \x20 FOR EACH item IN items\n\
                   \x20   LET e = item\n\
                   \x20 NEXT\n\
                   \x20 LET s AS Shape = Point(1, 2)\n\
                   \x20 MATCH s\n\
                   \x20   CASE Point(p)\n\
                   \x20     LET f = p\n\
                   \x20 END MATCH\n\
                   \x20 LET g = collections::map(items, LAMBDA(v AS Integer) -> v * 2.0)\n\
                   \x20 LET h = helper(a) TRAP(err)\n\
                   \x20   LET i2 = err\n\
                   \x20   RECOVER 0.0\n\
                   \x20 END TRAP\n\
                   \x20 RETURN 0\n\
                   TRAP(e2)\n\
                   \x20 LET j = e2\n\
                   \x20 RETURN 1\n\
                   END TRAP\n\
                   END FUNC\n";
        let hir = hir_from(src);
        let facts = lower::lower_facts(&hir, &HashMap::new(), &[]);
        let no_imports = HashMap::new();
        let mut walker = Walker::new(Path::new("/proj"), &facts, &hir, &no_imports);
        walker.walk_project(&hir);
        assert!(
            walker.diagnostics.is_empty(),
            "a clean program emits nothing"
        );

        let ir = lower::lower_augmented_project(&hir, None, &HashMap::new(), &[]);
        let mut lowered = Vec::new();
        for function in ir.functions.iter().filter(|f| f.name == "main") {
            lowered_binds(&function.body, &mut lowered);
        }
        // Lowering emits the MATCH case binding and the FOR loop variable as
        // `Bind`s of their own; the walker records them through `bind` too, so
        // the two sequences line up name-for-name.
        let walked: Vec<_> = walker
            .bound_types
            .iter()
            .filter(|(name, _)| !name.starts_with('$'))
            .cloned()
            .collect();
        let lowered_by_name: HashMap<_, _> = lowered.iter().cloned().collect();
        for (name, type_) in &walked {
            let stamped = lowered_by_name
                .get(name)
                .unwrap_or_else(|| panic!("lowering emitted no Bind for `{name}`"));
            assert_eq!(stamped, type_, "binding `{name}` typed differently");
        }
        // Coverage of the scope forms the walker mirrors.
        let names: Vec<_> = walked.iter().map(|(n, _)| n.as_str()).collect();
        for expected in [
            "a", "b", "c", "items", "i", "d", "item", "e", "s", "f", "g", "h", "i2", "j",
        ] {
            assert!(names.contains(&expected), "walker never bound `{expected}`");
        }
        // The promoted FOR type and the inline-TRAP success type are the
        // non-trivial computations; pin them so a silent fallback to `Unknown`
        // on both sides cannot pass.
        assert_eq!(lowered_by_name["i"], ParameterType::Float);
        assert_eq!(lowered_by_name["h"], ParameterType::Float);
        assert_eq!(lowered_by_name["e"], ParameterType::Integer);
    }

    // The standalone entry renders nothing and passes on a clean program.
    #[test]
    fn check_project_accepts_clean_source() {
        let hir = hir_from("FUNC main AS Integer\n  RETURN 0\nEND FUNC\n");
        assert!(check_project(Path::new("/proj"), &hir, &HashMap::new()).is_ok());
    }

    // ---- named arguments: user functions ----------------------------------

    #[test]
    fn user_named_argument_valid() {
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, a := 1)\nEND FUNC\n"
        ));
    }

    #[test]
    fn user_named_argument_unknown_name() {
        assert!(rejects_with(
            "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(z := 1)\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn user_named_positional_after_named_walk() {
        // A positional after a named argument fills the first free slot.
        assert!(accepts(
            "FUNC g(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN g(b := 2, 1)\nEND FUNC\n"
        ));
    }

    #[test]
    fn user_private_function_in_another_file_is_not_a_call_target() {
        // The source checker never bound names against a function invisible
        // from the calling file; the rule stays silent there too.
        let main = parse_source(
            Path::new("main.mfb"),
            "main.mfb",
            "FUNC main AS Integer\n  RETURN g(z := 1)\nEND FUNC\n",
        )
        .expect("parses");
        let other = parse_source(
            Path::new("other.mfb"),
            "other.mfb",
            "PRIVATE FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\n",
        )
        .expect("parses");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![main, other],
        };
        let hir = crate::resolver::augment_hir_project(&crate::hir::elaborate(&project))
            .expect("augments");
        let codes: Vec<_> = collect_diagnostics(
            Path::new("/proj"),
            &hir,
            &HashMap::new(),
            &[],
            &HashMap::new(),
        )
        .into_iter()
        .map(|d| d.rule)
        .collect();
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn imported_package_function_named_arguments() {
        // A `.mfp` function's parameter names come from the imported-signature
        // table; the call is spelled through the file's import binding.
        let mut imported = HashMap::new();
        imported.insert(
            "shapes.area".to_string(),
            ExternalSignature {
                params: vec![
                    crate::ir::ExternalFunctionParam {
                        name: "width".to_string(),
                        type_: ParameterType::Integer,
                    },
                    crate::ir::ExternalFunctionParam {
                        name: "height".to_string(),
                        type_: ParameterType::Integer,
                    },
                ],
                returns: ParameterType::Integer,
                isolated: false,
            },
        );
        let src = "IMPORT shapes AS sh\nFUNC main AS Integer\n  LET a = sh::area(width := 1, depth := 2)\n  RETURN a\nEND FUNC\n";
        let file = parse_source(Path::new("main.mfb"), "main.mfb", src).expect("parses");
        let project = AstProject {
            name: "test".to_string(),
            files: vec![file],
        };
        let hir = crate::hir::elaborate(&project);
        let diagnostics =
            collect_diagnostics(Path::new("/proj"), &hir, &HashMap::new(), &[], &imported);
        let codes: Vec<_> = diagnostics.iter().map(|d| d.rule.as_str()).collect();
        assert_eq!(codes, ["TYPE_UNKNOWN_ARGUMENT_NAME"]);
        assert_eq!(
            diagnostics[0].detail,
            "Call to `sh.area` does not have a parameter named `depth`."
        );
        assert_eq!(diagnostics[0].line, 3);
        assert_eq!(diagnostics[0].path, Path::new("/proj/main.mfb"));
    }

    // ---- named arguments: builtins ----------------------------------------

    #[test]
    fn builtin_named_argument_valid() {
        assert!(accepts(
            "IMPORT json\nIMPORT io\nFUNC main AS Integer\n  io::print(json::stringify(json::parse(value := \"null\")))\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_named_argument_unknown_name() {
        assert!(rejects_with(
            "IMPORT json\nFUNC main AS Integer\n  LET x = json::parse(nope := \"null\")\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn builtin_named_argument_matching_parameter_name_accepted() {
        // Every registry builtin carries its parameter names (`math::abs`'s is
        // `value`), so a matching name binds; the source checker's old test of
        // this call assumed a name-less fallback that no builtin reaches today.
        assert!(accepts(
            "IMPORT math\nFUNC main AS Integer\n  LET x = math::abs(value := -1)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_named_then_positional_reorders() {
        assert!(accepts(
            "IMPORT strings\nFUNC main AS Integer\n  LET b = strings::startsWith(prefix := \"a\", \"abc\")\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn builtin_general_call_named_argument() {
        // `general` members dispatch through their own arm in the source
        // checker; the rule reaches them all the same.
        assert!(rejects_with(
            "FUNC main AS Integer\n  LET s = toString(v := 1)\n  RETURN 0\nEND FUNC\n",
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn overloaded_named_unknown_argument_rejected() {
        assert!(rejects_with(
            &wrap_import("datetime", "  LET z = datetime::fixedOffset(bogus := 1)"),
            "TYPE_UNKNOWN_ARGUMENT_NAME"
        ));
    }

    #[test]
    fn overloaded_duplicate_ends_the_check_before_unknown_names() {
        // A duplicate (the source checker's rule for now) ends overload
        // selection before any unknown name is considered.
        let codes = shape_codes(&wrap_import(
            "datetime",
            "  LET z = datetime::fixedOffset(hours := 1, hours := 2, bogus := 3)",
        ));
        assert!(codes.is_empty(), "{codes:?}");
    }

    #[test]
    fn overloaded_named_valid_selection_accepted() {
        assert!(accepts(&wrap_import(
            "datetime",
            "  LET z = datetime::fixedOffset(hours := 1, mins := 2)",
        )));
    }

    #[test]
    fn nested_call_rule_follows_enclosing_call_rule() {
        // Outer `g(z := ...)` reports before the inner `g(y := 1)` argument.
        let diagnostics = collect_diagnostics(
            Path::new("/proj"),
            &hir_from(
                "FUNC g(a AS Integer) AS Integer\n  RETURN a\nEND FUNC\nFUNC main AS Integer\n  RETURN g(z := g(y := 1))\nEND FUNC\n",
            ),
            &HashMap::new(),
            &[],
            &HashMap::new(),
        );
        let details: Vec<_> = diagnostics.iter().map(|d| d.detail.as_str()).collect();
        assert_eq!(
            details,
            [
                "Call to `g` does not have a parameter named `z`.",
                "Call to `g` does not have a parameter named `y`.",
            ]
        );
    }
}
