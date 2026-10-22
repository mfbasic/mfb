//! plan-176-N: the shape checker's expressions — callees, thread entry points, builtin calls,
//! `expect` and trap guards. The `Walker` struct and the free helpers live in
//! `ir/shape/mod.rs`.

use super::*;

impl<'a> Walker<'a> {
    pub(super) fn walk_expression(
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
            HirExpression::Binary {
                left,
                operator,
                right,
                ..
            } => {
                self.walk_expression(left, locals);
                self.walk_expression(right, locals);
                // MONEY_INEXACT_FLOAT_LITERAL (Warn, plan-29-F §4.6): scaling a
                // Money by a BARE decimal literal (`* 1.08` / `/ 1.08`) takes the
                // inexact Float path. The literal's spelling is the evidence —
                // lowering stamps the same `Float` const for `1.08` and `1.08f`.
                let money = |walker: &Self, value: &HirExpression| {
                    matches!(walker.type_of(value, locals), ParameterType::Money)
                };
                let bare_float = |walker: &Self, value: &HirExpression| {
                    matches!(walker.type_of(value, locals), ParameterType::Float)
                        && is_bare_decimal_float(value)
                };
                let culprit = match operator {
                    BinaryOp::Multiply => {
                        (money(self, left) && bare_float(self, right))
                            || (money(self, right) && bare_float(self, left))
                    }
                    BinaryOp::Divide => money(self, left) && bare_float(self, right),
                    // Only a scaling operator can silently make Money inexact:
                    // `+`/`-` require two Money operands and the rest are not
                    // Money operators at all.
                    BinaryOp::Or
                    | BinaryOp::Xor
                    | BinaryOp::And
                    | BinaryOp::Equal
                    | BinaryOp::NotEqual
                    | BinaryOp::Less
                    | BinaryOp::LessEqual
                    | BinaryOp::Greater
                    | BinaryOp::GreaterEqual
                    | BinaryOp::Concat
                    | BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Mod
                    | BinaryOp::IntDiv
                    | BinaryOp::Power => false,
                };
                if culprit {
                    self.emit(
                        "MONEY_INEXACT_FLOAT_LITERAL",
                        "scaling Money by a bare decimal literal uses inexact Float arithmetic; append `F` for exact fixed-point scaling, or `f` to confirm the Float is intentional.".to_string(),
                        self.current_line,
                    );
                }
            }
            HirExpression::Unary { operand, .. } => self.walk_expression(operand, locals),
            HirExpression::Call {
                callee,
                arguments,
                line,
                ..
            } => {
                // The call's own shape rules report before its arguments are
                // walked — the source checker normalized the argument list
                // before inferring any argument, so a nested call's rule follows
                // the enclosing call's.
                self.call_typed_unknown = false;
                self.check_call_shape(callee, arguments, *line, locals);
                // plan-167-B: OS_CONFIG_ARGUMENT_NOT_LITERAL. The fold after the
                // package merge replaces the call with a constant, so neither
                // argument may need evaluating. Lowering erases the distinction:
                // a negative literal and `0 - 5` both become IR operator trees.
                if !self.current_file_internal && self.canonical_callee(callee) == "os.getConfigOr"
                {
                    self.check_config_call_literals(arguments, *line);
                }
                if !self.current_file_internal && self.canonical_callee(callee) == "os.exit" {
                    self.check_exit_code_literal(arguments, *line);
                }
                self.call_verdicts.insert(
                    expression as *const HirExpression as usize,
                    self.call_typed_unknown,
                );
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                            self.walk_expression(value, locals)
                        }
                    }
                }
            }
            HirExpression::Lambda {
                params,
                body,
                assign_target,
            } => {
                // Lambda parameters shadow the enclosing scope for the body
                // (`expression_type`'s Lambda arm).
                let mut nested = locals.clone();
                for param in params {
                    self.check_stray_param_state(param, self.current_line);
                    nested.insert(param.name.clone(), lower::lambda_param_type(param));
                }
                // TYPE_UNKNOWN_VALUE, the assignment-bodied lambda's target form.
                if let Some(target) = assign_target {
                    if !nested.contains_key(target) {
                        self.emit(
                            "TYPE_UNKNOWN_VALUE",
                            format!("Assignment target `{target}` is not a local binding."),
                            self.current_line,
                        );
                    }
                }
                self.walk_expression(body, &nested);
            }
            HirExpression::Constructor { type_, arguments } => {
                // TYPE_READ_ONLY_RECORD_CONSTRUCTOR, the `Error`/`ErrorLoc` form:
                // lowering itself emits `Constructor{Error}` for the `error()`
                // builtin and the trap machinery, so on the IR a user `Error[..]`
                // is indistinguishable from a legitimate synthesized one (A
                // Corrections C-split-49). verify keeps the compiler-owned form
                // and the `AttributedString` form.
                let type_name = type_.name();
                if matches!(type_name.as_ref(), "Error" | "ErrorLoc") {
                    self.emit(
                        "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                        format!(
                            "`{type_name}` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error."
                        ),
                        self.current_line,
                    );
                }
                // TYPE_DUPLICATE_FIELD, the constructor form: lowering reorders
                // the named arguments into field order (the last spelling of a
                // repeated field wins), so the IR holds one value per field and
                // the repetition is gone. The checker checked it only for a
                // declared, visible record (`check_constructor_arguments` — the
                // built-in nominals and compiler-owned records return before
                // it); verify keeps the WITH form.
                if self.declared_record_constructible(type_) {
                    let mut seen = HashSet::new();
                    for argument in arguments {
                        if let HirConstructorArg::Named { name, line, .. } = argument {
                            if !seen.insert(name.as_str()) {
                                self.emit(
                                    "TYPE_DUPLICATE_FIELD",
                                    format!(
                                        "Constructor `{}` sets field `{name}` more than once.",
                                        type_.name()
                                    ),
                                    *line,
                                );
                            }
                        }
                    }
                }
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
            HirExpression::MemberAccess { target, member } => {
                self.walk_expression(target, locals);
                self.check_foreign_record_field(target, member, locals);
            }
            HirExpression::Trapped {
                expression,
                binding,
                handler,
                line,
            } => {
                self.walk_expression(expression, locals);
                let trapped_type = self.type_of(expression, locals);
                self.walk_handler(binding, handler, locals, trapped_type, *line);
            }
        }
    }

    /// The parameter names of the function a call names, resolved the way the
    /// source checker resolved a call target: a TESTING expectation or a package
    /// constant is not a function call; a builtin (by canonical name) comes
    /// before any declared function; a declared function must be visible from
    /// the calling file; an imported package's function is looked up under its
    /// canonical `package.member` name; last, a local or global binding of
    /// FUNC type is a function value. Anything else (an unresolved dotted name,
    /// a non-callable binding) has no signature to check against.
    pub(super) fn callee_params(
        &self,
        callee: &str,
        locals: &HashMap<String, ParameterType>,
    ) -> Option<CalleeParams> {
        if crate::codegen::builtins_testing::is_testing_call(callee) {
            return None;
        }
        let binding = callee.split_once('.').map(|(binding, _)| binding);
        let resolved_package =
            binding.and_then(|binding| self.context.current_imports.get(binding));
        let canonical = self.canonical_callee(callee);
        if builtins::is_package_constant(&canonical) {
            return None;
        }
        if builtins::is_builtin_call(&canonical) {
            if !builtins::checks_call_arguments(&canonical) {
                return None;
            }
            if let Some(overloads) = builtins::call_param_name_overloads(&canonical) {
                return Some(CalleeParams::BuiltinOverloads(overloads));
            }
            // Every argument-checked built-in carries names
            // (`ir::shape::tests::named_builtin_args::every_argument_checked_builtin_has_parameter_names`);
            // a name that is no member never gets here, because the resolver
            // refuses it first (SYMBOL_UNKNOWN_IDENTIFIER).
            return builtins::call_param_names(&canonical).map(CalleeParams::Builtin);
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
                        .map(|param| ShapeParam {
                            name: param.name.clone(),
                            type_: param.type_.clone(),
                            has_default: param.default.is_some(),
                        })
                        .collect(),
                ));
            }
        }
        // A dotted name that resolved to nothing is not a call to a binding.
        if binding.is_some() {
            return None;
        }
        let value_type = locals
            .get(callee)
            .or_else(|| self.context.binding_type(callee))?;
        match value_type {
            ParameterType::Func(params, _, _) => Some(CalleeParams::FunctionValue(params.clone())),
            _ => None,
        }
    }

    /// The canonical `package.member` spelling of a call target, through this
    /// file's import bindings — lowering's `canonical_import_name`.
    pub(super) fn canonical_callee(&self, callee: &str) -> String {
        let Some((binding, member)) = callee.split_once('.') else {
            return callee.to_string();
        };
        match self.context.current_imports.get(binding) {
            Some(package) => format!("{package}.{member}"),
            None => callee.to_string(),
        }
    }

    /// Whether a `thread::start` entry argument names an `ISOLATED FUNC` — that
    /// is the whole rule (plan-115). Either one this project declares, named by
    /// its bare identifier at any visibility, or an imported package's
    /// `EXPORT ISOLATED FUNC` named through an import binding (`pkg::f`, the
    /// `.mfp`'s export table). The checker resolved the NORMALIZED first
    /// argument, so `entry` is that.
    pub(super) fn thread_start_entry_valid(&self, entry: Option<&HirExpression>) -> bool {
        let Some(HirExpression::Identifier(name)) = entry else {
            return false;
        };
        // plan-115-A: an UNQUALIFIED name is now a valid entry when this project
        // declares it as an `ISOLATED FUNC`. This is the case the old predicate
        // never even considered — it bailed at `split_once('.')` — and it is the
        // whole point of the letter: `ISOLATED` is the sole marker, so an entry
        // no longer has to be reached through an import.
        //
        // Deliberately NOT filtered by visibility. Phase 2 made `ISOLATED`
        // orthogonal to visibility, and `PRIVATE` is file-local, so a `PRIVATE`
        // entry is nameable exactly where it is nameable at all — the ordinary
        // scoping rules already do that work, and re-checking it here would
        // reject the file-local case this plan exists to allow. It is also
        // kind-independent: the rule is the same in an executable and a package,
        // which is why `is_package` is NOT threaded in (plan §Open Decisions).
        let Some((binding, member)) = name.split_once('.') else {
            return self
                .functions
                .get(name)
                .is_some_and(|function| function.isolated);
        };
        match self.context.current_imports.get(binding) {
            Some(package) => self
                .imported_signatures
                .get(&format!("{package}.{member}"))
                .is_some_and(|signature| signature.isolated),
            None => false,
        }
    }

    /// TYPE_CALL_ARGUMENT_MISMATCH, the `thread.start` entry form — a source
    /// fact (a bare name and `pkg::name` both lower to one `FunctionRef`).
    pub(super) fn report_thread_entry(&mut self, line: usize) {
        self.call_typed_unknown = true;
        self.emit(
            "TYPE_CALL_ARGUMENT_MISMATCH",
            "thread.start entry point must name an ISOLATED FUNC.".to_string(),
            line,
        );
    }

    /// The builtin-call family on the source path — the former source checker's
    /// `check_builtin_call` over the HIR argument list: the count against the
    /// registry's arity range, then the argument types through the same
    /// arg-typed overload resolution the checker used (the `general`,
    /// `collections`, `term` and `thread` arms ahead of the package table).
    /// Every type here is lowering's `expression_type` of the argument the
    /// SOURCE wrote — the list lowering then pads with defaults and coerces
    /// literal by literal, which is why the IR cannot report this form with
    /// the checker's wording (`ir::verify` keeps the IR-level check for the
    /// package path).
    pub(super) fn check_builtin_call(
        &mut self,
        callee: &str,
        canonical: &str,
        normalized: &[&HirExpression],
        line: usize,
        locals: &HashMap<String, ParameterType>,
    ) {
        let registry = crate::codegen::registry::registry();
        let arg_types: Vec<ParameterType> = normalized
            .iter()
            .map(|argument| self.type_of(argument, locals).without_state())
            .collect();
        // A bare general built-in (`len(toString)`) has no value type, so lowering
        // types it `Unknown`; name it as written, as the package path does
        // (`bare_builtin_misfit`), rather than report "(Unknown)".
        let names: Vec<String> = arg_types
            .iter()
            .zip(normalized)
            .map(
                |(type_, argument)| match (type_, self.bare_general_builtin(argument, locals)) {
                    (ParameterType::Unknown, Some(name)) => name.to_string(),
                    _ => type_.name().into_owned(),
                },
            )
            .collect();
        let expected_overloads = || {
            builtins::expected_arguments(canonical)
                .unwrap_or_else(|| "supported overload".to_string())
        };
        let mismatch = |names: &[String], expected: String| {
            format!(
                "Call to `{callee}` has argument type(s) ({}), expected {expected}.",
                names.join(", ")
            )
        };

        if crate::codegen::builtins::general::is_general_call(canonical) {
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                return;
            }
            if builtins::resolve_call_return_type_with_kinds(
                canonical,
                &arg_types,
                true,
                &ShapeKinds(&self.types),
            )
            .is_none()
            {
                // A package-provided override may accept what the built-in
                // rejects (plan-01-overload §A.3.2) — never reject those.
                if crate::codegen::builtins::general::is_overridable(canonical)
                    && arg_types.len() == 1
                    && builtins::general_override_target(canonical, &arg_types[0]).is_some()
                {
                    return;
                }
                let detail = mismatch(&names, expected_overloads());
                self.emit_call_typed_unknown(detail, line);
            }
            return;
        }

        if registry.owning_package(canonical) == Some("collections") {
            // A bare general built-in predicate in the callback position
            // (bug-368): its type derives from the list's element type, and the
            // diagnostic quotes the predicate's NAME.
            //
            // A general built-in that is no predicate at all (`toString`, `len`)
            // takes this path too: it has no callable type to derive, so the call
            // is the same mismatch naming it, not an `Unknown` argument the
            // member's callback parameter silently absorbs.
            if crate::codegen::registry::callback_member(canonical) && normalized.len() == 2 {
                if let Some(predicate) = self.bare_general_builtin(normalized[1], locals) {
                    let collection = arg_types[0].clone();
                    // plan-111-C: the typed twin, so the predicate type is a
                    // `Func` rather than a `format!`ed spelling.
                    let predicate_type = match &arg_types[0] {
                        ParameterType::ListOf(element) => {
                            crate::codegen::builtins::general::filter_predicate_type_typed(
                                predicate, element,
                            )
                        }
                        _ => None,
                    };
                    let Some(predicate_type) = predicate_type else {
                        let detail = mismatch(
                            &[collection.name().into_owned(), predicate.to_string()],
                            expected_overloads(),
                        );
                        self.emit_call_typed_unknown(detail, line);
                        return;
                    };
                    let trial = vec![collection, predicate_type];
                    if builtins::resolve_call_return_type_with_kinds(
                        canonical,
                        &trial,
                        true,
                        &ShapeKinds(&self.types),
                    )
                    .is_none()
                    {
                        let names: Vec<String> =
                            trial.iter().map(|t| t.name().into_owned()).collect();
                        let detail = mismatch(&names, expected_overloads());
                        self.emit_call_typed_unknown(detail, line);
                    }
                    return;
                }
            }
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                return;
            }
            if let Some(names) = self.bare_builtin_misfit(canonical, &arg_types, normalized, locals)
            {
                let detail = mismatch(&names, expected_overloads());
                self.emit_call_typed_unknown(detail, line);
                return;
            }
            if builtins::resolve_call_return_type_with_kinds(
                canonical,
                &arg_types,
                true,
                &ShapeKinds(&self.types),
            )
            .is_none()
                && !self.selects_with_retyped_literals(canonical, &arg_types, normalized)
            {
                let detail = mismatch(&names, expected_overloads());
                self.emit_call_typed_unknown(detail, line);
            }
            return;
        }

        if registry.owning_package(canonical) == Some("term") {
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                // `term` types by name alone, so a count failure leaves the call typed.
                self.call_typed_unknown = false;
                return;
            }
            // `term::drawText` and `term::measureText` additionally accept an
            // `AttributedString`; their source-companion overloads use its visible
            // text (and drawText honours its attributes). Their bodies live in a bridge injected only when the
            // project imports `astrings`, so the import is required here — a
            // source fact the IR does not keep.
            let third_is_attributed = canonical == crate::codegen::builtins::term::DRAW_TEXT
                && names.len() == 3
                && names[2] == "AttributedString";
            let first_is_attributed = canonical == crate::codegen::builtins::term::MEASURE_TEXT
                && names.len() == 1
                && names[0] == "AttributedString";
            if (third_is_attributed || first_is_attributed) && !self.astrings_imported {
                self.emit(
                    "TYPE_CALL_ARGUMENT_MISMATCH",
                    format!(
                        "Call to `{callee}` with an `AttributedString` requires `IMPORT astrings`."
                    ),
                    line,
                );
                return;
            }
            // plan-111-B: `argument_types` is literally
            // `argument_types_typed(..).map(name())`, so the typed twin is exact
            // and the per-argument parse below disappears.
            let param_types: Vec<ParameterType> = if third_is_attributed {
                vec![
                    ParameterType::Integer,
                    ParameterType::Integer,
                    ParameterType::named("AttributedString"),
                ]
            } else if first_is_attributed {
                vec![ParameterType::named("AttributedString")]
            } else {
                builtins::argument_types_typed(canonical).unwrap_or_default()
            };
            let mismatched = param_types
                .iter()
                .zip(arg_types.iter())
                .zip(normalized.iter())
                .any(|((expected, actual), argument)| {
                    !self.expression_compatible(expected, actual, argument)
                });
            if mismatched {
                let expected = builtins::expected_arguments(canonical)
                    .unwrap_or_else(|| "no arguments".to_string());
                let detail = mismatch(&names, expected);
                self.emit("TYPE_CALL_ARGUMENT_MISMATCH", detail, line);
            }
            return;
        }

        if crate::codegen::builtins::thread::is_thread_call(canonical) {
            if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
                return;
            }
            if builtins::resolve_call_return_type_with_kinds(
                canonical,
                &arg_types,
                true,
                &ShapeKinds(&self.types),
            )
            .is_none()
                && !self.selects_with_retyped_literals(canonical, &arg_types, normalized)
            {
                let detail = mismatch(&names, expected_overloads());
                self.emit_call_typed_unknown(detail, line);
            }
            return;
        }

        // The shared package table: arity, then arg-typed overload resolution
        // with the literal→`Byte` retry the checker gave the table packages.
        if self.check_builtin_arity(callee, canonical, normalized.len(), line) {
            return;
        }
        if let Some(names) = self.bare_builtin_misfit(canonical, &arg_types, normalized, locals) {
            let detail = mismatch(&names, expected_overloads());
            self.emit_call_typed_unknown(detail, line);
            return;
        }
        if resolve_table_call_with_byte_literals(
            canonical,
            &arg_types,
            normalized,
            &ShapeKinds(&self.types),
        )
        .is_none()
            && !self.selects_with_retyped_literals(canonical, &arg_types, normalized)
        {
            let detail = mismatch(&names, expected_overloads());
            self.emit_call_typed_unknown(detail, line);
        }
    }

    /// A bare general built-in named as a value (`isEven`, `toString` written
    /// where a callable is expected) that no local, global or declared
    /// function shadows. Lowering types such a name only through the callable
    /// type its position expects (`builtin_function_id_for_type`), so on its
    /// own it is `Unknown` — which every compatibility check accepts. The
    /// shape pass therefore judges it by [`Self::bare_builtin_fits`] instead.
    pub(super) fn bare_general_builtin<'e>(
        &self,
        expression: &'e HirExpression,
        locals: &HashMap<String, ParameterType>,
    ) -> Option<&'e str> {
        let HirExpression::Identifier(name) = expression else {
            return None;
        };
        let shadowed = locals.contains_key(name)
            || self.functions.contains_key(name)
            || self.context.binding_type(name).is_some();
        (!shadowed && crate::codegen::builtins::general::is_general_call(name))
            .then_some(name.as_str())
    }

    /// Whether the bare general built-in `name` can stand where `expected` is
    /// required: only a predicate does (bug-368), as `FUNC(E) AS Boolean`
    /// over an element `E` it accepts — the type lowering selects for it.
    /// A non-predicate (`toString`, `len`) has no callable type at all.
    pub(super) fn bare_builtin_fits(&self, name: &str, expected: &ParameterType) -> bool {
        let ParameterType::Func(params, _, _) = expected else {
            return false;
        };
        let [element] = params.as_slice() else {
            return false;
        };
        crate::codegen::builtins::general::filter_predicate_type_typed(name, element)
            .is_some_and(|predicate| self.compatible(expected, &predicate))
    }

    /// The argument spellings of a registry built-in call in which a bare
    /// general built-in stands where its parameter's type — read in lowering's
    /// `call_argument_expected_type` order — is not one it can take, each
    /// misfit spelled by its NAME as the bug-368 mismatch spells a predicate;
    /// `None` when every such argument fits. `collections::reduce(xs, 0,
    /// toString)` is the mismatch `filter(xs, toString)` is, not an `Unknown`
    /// callback the call silently accepts.
    fn bare_builtin_misfit(
        &self,
        canonical: &str,
        arg_types: &[ParameterType],
        normalized: &[&HirExpression],
        locals: &HashMap<String, ParameterType>,
    ) -> Option<Vec<String>> {
        let mut names: Vec<String> = arg_types
            .iter()
            .map(|type_| type_.name().into_owned())
            .collect();
        let mut misfit = false;
        for (index, argument) in normalized.iter().enumerate() {
            let Some(name) = self.bare_general_builtin(argument, locals) else {
                continue;
            };
            let expected = builtins::argument_types_typed(canonical)
                .and_then(|params| params.get(index).cloned())
                .or_else(|| builtins::agreed_argument_type(canonical, index))
                .or_else(|| {
                    builtins::resolved_argument_type(
                        canonical,
                        index,
                        arg_types,
                        None,
                        &ShapeKinds(&self.types),
                    )
                });
            if !expected.is_some_and(|expected| self.bare_builtin_fits(name, &expected)) {
                names[index] = name.to_string();
                misfit = true;
            }
        }
        misfit.then_some(names)
    }

    /// Whether a generic built-in call whose exact-typed selection failed
    /// selects once its literal arguments are re-typed to their slots
    /// ([`lower::literal_retyped_argument_types`]) AND every re-typed literal
    /// fits its slot by `expression_compatible` — the rule a user `FUNC`
    /// parameter applies, so `collections::append(bytes, 7)` is accepted and
    /// `collections::append(bytes, 300)` is the same mismatch `takeB(300)` is.
    fn selects_with_retyped_literals(
        &self,
        canonical: &str,
        arg_types: &[ParameterType],
        normalized: &[&HirExpression],
    ) -> bool {
        let Some(retyped) = lower::literal_retyped_argument_types(
            canonical,
            arg_types,
            normalized,
            &ShapeKinds(&self.types),
        ) else {
            return false;
        };
        retyped
            .iter()
            .zip(arg_types)
            .zip(normalized)
            .all(|((slot, actual), argument)| {
                slot == actual || self.expression_compatible(slot, actual, argument)
            })
    }

    /// Lowering's type for an argument the source wrote; `Unknown` when it
    /// has none — the checker's own spelling for it. Callers strip a resource
    /// local's `STATE T` clause (`without_state`): the checker compared and
    /// printed the bare resource type, as the parameter tables spell it.
    pub(super) fn type_of(
        &self,
        expression: &HirExpression,
        locals: &HashMap<String, ParameterType>,
    ) -> ParameterType {
        lower::expression_type(expression, locals, &self.context).unwrap_or(ParameterType::Unknown)
    }

    /// TESTING_EXPECT_ARITY / TYPE_MISMATCH / INCOMPARABLE / NOT_PRINTABLE /
    /// CODE_TYPE / TRAP_REQUIRES_FALLIBLE: an assertion builtin's argument
    /// constraints (plan-18-B). Lowering expands `expectX(...)` into plain
    /// comparisons + FAIL, or a trap-guarded evaluation (`testing::expand_expect`,
    /// `lower_statement`), so the IR never sees the assertion — only the HIR
    /// call names it. The rules and their order are the source checker's
    /// `check_expect_call`.
    pub(super) fn check_expect_call(
        &mut self,
        callee: &str,
        arguments: &[HirCallArg],
        line: usize,
        locals: &HashMap<String, ParameterType>,
    ) {
        use crate::codegen::builtins_testing::{
            expect_arity, expect_operand_type, is_equality_assert, is_inequality_assert,
            EXPECT_NTRAP, EXPECT_TRAP,
        };
        if let Some((min, max)) = expect_arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                self.emit(
                    "TESTING_EXPECT_ARITY",
                    format!(
                        "`{callee}` expects {} argument(s), got {}.",
                        if min == max {
                            min.to_string()
                        } else {
                            format!("{min}\u{2013}{max}")
                        },
                        arguments.len()
                    ),
                    line,
                );
            }
        }
        let values = source_order(arguments);
        // The checker's verdict on an operand: its inferred type, or `Unknown`
        // where the checker could not type it (see `checker_types_unknown`).
        let operand = |walker: &Self, index: usize| {
            values.get(index).map_or(ParameterType::Unknown, |value| {
                if walker.checker_types_unknown(value, locals, None) {
                    ParameterType::Unknown
                } else {
                    walker.type_of(value, locals)
                }
            })
        };
        if is_equality_assert(callee) || is_inequality_assert(callee) {
            let left = operand(self, 0);
            let right = operand(self, 1);
            match expect_operand_type(callee) {
                // A typed assertion requires both operands to be exactly the
                // named type.
                Some(want) => {
                    for operand in [&left, &right] {
                        if !matches!(operand, ParameterType::Unknown) && operand.name() != want {
                            self.emit(
                                "TESTING_EXPECT_TYPE_MISMATCH",
                                format!(
                                    "`{callee}` operands must both be {want}; got {}.",
                                    operand.name()
                                ),
                                line,
                            );
                        }
                    }
                }
                // `expectEqual`/`expectNEqual` accept any `=`-comparable,
                // printable operands (the language's `=` acceptance).
                None => {
                    let comparable = (is_numeric(&left) && is_numeric(&right))
                        || ((self.compatible(&left, &right) || self.compatible(&right, &left))
                            && self.is_comparable(&left)
                            && self.is_comparable(&right));
                    if !comparable
                        && !matches!(left, ParameterType::Unknown)
                        && !matches!(right, ParameterType::Unknown)
                    {
                        self.emit(
                            "TESTING_EXPECT_INCOMPARABLE",
                            format!(
                                "`{callee}` operands must be comparable with `=`; got {} and {}.",
                                left.name(),
                                right.name()
                            ),
                            line,
                        );
                    }
                    for operand in [&left, &right] {
                        if !is_printable(operand) {
                            self.emit(
                                "TESTING_EXPECT_NOT_PRINTABLE",
                                format!(
                                    "`{callee}` operands must be printable (a scalar, String, Byte, or List OF Byte); got {}.",
                                    operand.name()
                                ),
                                line,
                            );
                        }
                    }
                }
            }
        } else if callee == EXPECT_TRAP {
            if let Some(value) = values.first() {
                self.check_trap_guardable(callee, value, line);
            }
            if values.get(1).is_some() {
                let code = operand(self, 1);
                if !self.compatible(&ParameterType::Integer, &code) {
                    self.emit(
                        "TESTING_EXPECT_CODE_TYPE",
                        format!(
                            "`{callee}` expected-code argument must be an Integer; got {}.",
                            code.name()
                        ),
                        line,
                    );
                }
            }
        } else if callee == EXPECT_NTRAP {
            if let Some(value) = values.first() {
                self.check_trap_guardable(callee, value, line);
            }
        }
    }

    /// `expectTrap`/`expectNTrap` evaluate their argument under a trap guard
    /// built on the inline-TRAP machinery, so the gate rejects exactly what an
    /// inline `TRAP` rejects (plan-26-C): a scrutinee with no runtime call to
    /// trap — a non-call, or a package constant.
    fn check_trap_guardable(&mut self, callee: &str, expression: &HirExpression, line: usize) {
        let HirExpression::Call {
            callee: inner_callee,
            ..
        } = expression
        else {
            self.emit(
                "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE",
                format!("`{callee}` requires a call to trap-guard (got a non-call)."),
                line,
            );
            return;
        };
        if builtins::is_package_constant(&self.canonical_callee(inner_callee)) {
            self.emit(
                "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE",
                format!(
                    "`{callee}` requires a call to trap-guard; a package constant is not a call."
                ),
                line,
            );
        }
    }
}
