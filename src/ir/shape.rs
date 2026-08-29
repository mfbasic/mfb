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
use crate::hir::{
    HirCallArg, HirConstructorArg, HirExpression, HirFile, HirFunction, HirItem, HirMatchCase,
    HirProject, HirStatement,
};
use crate::rules::PendingDiagnostic;
use crate::types::ParameterType;
use std::collections::HashMap;

/// Run the shape pass over `hir` and return its diagnostics in traversal
/// (source) order, un-rendered, for the build path to merge with the other
/// streams. `external_signatures` and `imported_types` are the same inputs the
/// build path hands `lower_augmented_project`, so the typing seam sees exactly
/// the tables lowering will.
pub(crate) fn collect_diagnostics(
    hir: &HirProject,
    external_signatures: &HashMap<String, ExternalSignature>,
    imported_types: &[ImportedTypeDef],
) -> Vec<PendingDiagnostic> {
    let facts = lower::lower_facts(hir, external_signatures, imported_types);
    let mut walker = Walker::new(&facts);
    walker.walk_project(hir);
    walker.diagnostics
}

/// Standalone form for callers that render rather than merge (`mfb audit`):
/// prints the diagnostics and reports whether any was an error.
pub(crate) fn check_project(hir: &HirProject) -> Result<(), ()> {
    let diagnostics = collect_diagnostics(hir, &HashMap::new(), &[]);
    let had_error = diagnostics.iter().any(|d| crate::rules::is_error(&d.rule));
    crate::rules::render_pending(diagnostics);
    if had_error {
        Err(())
    } else {
        Ok(())
    }
}

/// The walk state: lowering's context (positioned per file / per function
/// exactly as lowering positions it) and the diagnostics collected so far.
struct Walker<'a> {
    context: LowerContext<'a>,
    diagnostics: Vec<PendingDiagnostic>,
    /// Every `LET`/`MUT`/`RES` binding's computed type in walk order — the
    /// seam-fidelity probe the unit tests compare against lowering's stamped
    /// `IrOp::Bind` types.
    #[cfg(test)]
    bound_types: Vec<(String, ParameterType)>,
}

impl<'a> Walker<'a> {
    fn new(facts: &'a LowerFacts) -> Self {
        Walker {
            context: facts.context(),
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
            HirExpression::Call { arguments, .. } => {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{parse_source, AstProject};
    use crate::ir::IrOp;
    use std::path::Path;

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
        let mut walker = Walker::new(&facts);
        walker.walk_project(&hir);
        assert!(walker.diagnostics.is_empty(), "the scaffold emits nothing");

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
        assert!(check_project(&hir).is_ok());
    }
}
