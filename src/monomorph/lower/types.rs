//! plan-176-M: the monomorphizer's type queries — concrete and template views of a type,
//! union instantiations, record fields and an expression's type. The driver lives in
//! `monomorph/lower/mod.rs`.

use super::*;

impl<'a> Monomorphizer<'a> {
    /// The interned name of a NOMINAL leaf — a concrete nominal or a type
    /// variable — or `None` for anything with structure.
    ///
    /// This is where the `substitutions` probe belongs: its keys are always bare
    /// template-parameter names. `Var` and `Named` both appear because a name
    /// reaching monomorph through HIR was classified by `with_vars`, while one
    /// reaching it through a rendered spelling was not.
    fn leaf_symbol(type_: &ParameterType) -> Option<crate::intern::Symbol> {
        match type_ {
            ParameterType::Named(name) | ParameterType::Var(name) => Some(*name),
            // plan-113: same reason as `helpers::leaf_param_symbol` — a C ABI
            // spelling is a bare token, so a template parameter may be named
            // with one and used to be a `Named` here.
            ParameterType::C(ctype) => Some(crate::intern::Symbol::intern(ctype.name())),
            _ => None,
        }
    }

    /// Substitute a template's parameters through `type_`, instantiating any user
    /// generic it names, and return the concrete type.
    ///
    /// plan-106-E: was `concrete_type_name(&str) -> String`, which parsed its
    /// input, recursed by RE-RENDERING each child, and rebuilt the result with
    /// seven `format!("List OF {…}")`-family templates — a second renderer beside
    /// `ParameterType::name`. It is now a structural walk.
    ///
    /// Two behaviours the by-name recursion encoded are preserved explicitly:
    ///
    /// * the per-level grouped-type unwrap (bug-105) is no longer needed here —
    ///   plan-106-E made `ParameterType::parse` peel a `(T)` group at every level,
    ///   so a grouped spelling never reaches this walk as a junk nominal;
    /// * the `substitutions` probe is at the NOMINAL LEAVES. Its keys are always
    ///   bare template-parameter names (`Symbol::intern(param)` over
    ///   `template_params`, at `lower.rs:707` and `:861`), so probing the whole
    ///   spelling at every level — as the string form did — could only ever have
    ///   matched at a leaf anyway.
    pub(super) fn concrete_type(
        &mut self,
        type_: &ParameterType,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
    ) -> ParameterType {
        if let Some(symbol) = Self::leaf_symbol(type_) {
            if let Some(bound) = substitutions.get(&symbol) {
                // The bound value is already fully substituted, but it may itself be
                // an un-INSTANTIATED user generic: binding `T := Holder OF Integer`
                // while expanding `Holder OF Holder OF Integer` used to return that
                // spelling verbatim, so the inner `Holder OF Integer` template was
                // never lowered and the post-monomorph resolve pass reported
                // `SYMBOL_UNKNOWN_TYPE` against the template's own field. Walk the
                // bound value so its instantiation happens too.
                //
                // Walked with NO substitutions, deliberately: the value is already
                // substituted, and re-applying the map would loop forever on a
                // binding that names its own parameter. The comparison is on the
                // rendered names, as the string form's `bound != type_name` was, so
                // a `Var`/`Named` spelling difference is not mistaken for progress.
                if bound.name() != type_.name() {
                    return self.concrete_type(&bound.clone(), &HashMap::new());
                }
                return bound.clone();
            }
        }
        match type_ {
            ParameterType::ListOf(element) => {
                ParameterType::list_of(self.concrete_type(element, substitutions))
            }
            ParameterType::SetOf(element) => {
                ParameterType::set_of(self.concrete_type(element, substitutions))
            }
            ParameterType::ResultOf(success) => {
                ParameterType::result_of(self.concrete_type(success, substitutions))
            }
            ParameterType::MapOf(key, value) => ParameterType::map_of(
                self.concrete_type(key, substitutions),
                self.concrete_type(value, substitutions),
            ),
            ParameterType::MapEntryOf(key, value) => ParameterType::map_entry_of(
                self.concrete_type(key, substitutions),
                self.concrete_type(value, substitutions),
            ),
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => {
                // An absent resource plane is `Nothing` and stays `Nothing`, which
                // `name()` elides exactly as `format_thread_type` did.
                let res = match res.as_ref() {
                    ParameterType::Nothing => ParameterType::Nothing,
                    other => self.concrete_type(other, substitutions),
                };
                ParameterType::thread_handle(
                    *worker,
                    self.concrete_type(msg, substitutions),
                    res,
                    self.concrete_type(out, substitutions),
                )
            }
            ParameterType::Func(params, ret, isolated) => {
                let params = params
                    .iter()
                    .map(|param| self.concrete_type(param, substitutions))
                    .collect::<Vec<_>>();
                let ret = self.concrete_type(ret, substitutions);
                if *isolated {
                    ParameterType::func_isolated(params, ret)
                } else {
                    ParameterType::func(params, ret)
                }
            }
            // A user generic is the one arm that does more than rewrite: it also
            // INSTANTIATES the template, and yields the mangled concrete nominal.
            ParameterType::UserOf(name, args) => {
                let args = args
                    .iter()
                    .map(|arg| self.concrete_type(arg, substitutions))
                    .collect::<Vec<_>>();
                self.instantiate_type(name.resolve(), &args)
            }
            // A scalar, a concrete nominal, `RES`, `Unknown`, `Arg` — identity, as
            // the cascade's fall-through was.
            other => other.clone(),
        }
    }

    /// The enum type `target.member` names when `target` is a declared enum and
    /// `member` one of its members (`Color.Blue`), else `None`.
    fn enum_member_type(&self, target: &HirExpression, member: &str) -> Option<ParameterType> {
        let HirExpression::Identifier(type_name) = target else {
            return None;
        };
        let enum_type = ParameterType::declared(type_name);
        match self.concrete_types.get(&enum_type) {
            Some(decl) => (matches!(decl.kind, TypeDeclKind::Enum)
                && decl
                    .members
                    .iter()
                    .any(|candidate| candidate.name == member))
            .then_some(enum_type),
            // An imported package's enum (`pkg::Suit.Clubs`): the package's IR
            // carries its members, and the IR lowering checks the member; here it
            // is enough that the type is one of the imported enums.
            None => {
                crate::codegen::builtins::TypeKinds::is_enum(self, &enum_type).then_some(enum_type)
            }
        }
    }

    /// The inverse of [`concrete_type`](Self::concrete_type): rewrite a mangled
    /// concrete type back to its template spelling, for diagnostics.
    ///
    /// plan-106-E: structural, like its twin. The user-generic arm stays a LOOKUP
    /// in `type_instantiations` — a mangled name is a plain nominal, so it can only
    /// be recognized that way, never by parsing.
    pub(super) fn template_view(&self, type_: &ParameterType) -> ParameterType {
        match type_ {
            ParameterType::ListOf(element) => ParameterType::list_of(self.template_view(element)),
            ParameterType::SetOf(element) => ParameterType::set_of(self.template_view(element)),
            ParameterType::ResultOf(success) => {
                ParameterType::result_of(self.template_view(success))
            }
            ParameterType::MapOf(key, value) => {
                ParameterType::map_of(self.template_view(key), self.template_view(value))
            }
            ParameterType::MapEntryOf(key, value) => {
                ParameterType::map_entry_of(self.template_view(key), self.template_view(value))
            }
            ParameterType::ThreadHandle {
                worker,
                msg,
                res,
                out,
            } => {
                let res = match res.as_ref() {
                    ParameterType::Nothing => ParameterType::Nothing,
                    other => self.template_view(other),
                };
                ParameterType::thread_handle(
                    *worker,
                    self.template_view(msg),
                    res,
                    self.template_view(out),
                )
            }
            other => {
                if let Some((template, args)) = self.type_instantiations.get(other) {
                    // plan-111-B: the recorded instantiation arguments are types,
                    // so the template view is rebuilt structurally — no render,
                    // no re-parse.
                    return ParameterType::user_of(
                        template,
                        args.iter().map(|arg| self.template_view(arg)).collect(),
                    );
                }
                other.clone()
            }
        }
    }

    /// bug-697: the mangled-symbol -> template-spelling table for every user
    /// generic this run instantiated, keyed by exactly the string
    /// `ParameterType::name()` renders for the mangled nominal (so it is what a
    /// post-monomorph diagnostic's `Type::name()` interpolation actually
    /// contains), valued by [`Self::template_view`]'s spelling — fully
    /// recursive, so a nested instantiation renders fully
    /// (`Holder OF Holder OF Integer`, never a half-mangled mix). Read by
    /// `monomorphize_project`, which installs it at `rules::show_diagnostic`'s
    /// render boundary via `rules::set_template_spellings` before returning.
    ///
    /// bug-913: a template FUNC instantiation's symbol maps to the name the user
    /// called (`f$Integer` -> `f`), so an arity diagnostic about a call that
    /// instantiated it no longer leaks the mangled symbol.
    pub(in crate::monomorph) fn template_spellings(
        &self,
    ) -> std::collections::HashMap<String, String> {
        let mut spellings: std::collections::HashMap<String, String> = self
            .function_instantiation_names
            .iter()
            .map(|(symbol, name)| (symbol.clone(), name.clone()))
            .collect();
        spellings.extend(self.type_instantiations.keys().map(|concrete| {
            (
                concrete.name().into_owned(),
                self.template_view(concrete).name().into_owned(),
            )
        }));
        spellings
    }

    /// The declared fields of a concrete RECORD type, or `None` when `type_` names
    /// no record. The `TypeDeclKind::Type` filter is what the per-function
    /// snapshot applied when it populated `record_fields`, so an `ENUM`/`UNION`
    /// declaration stays invisible here exactly as it was then.
    ///
    /// plan-117 Phase 2: read live off `concrete_types` instead of through a
    /// snapshot rebuilt for every lowered function.
    ///
    /// bug-631: a record exported by an imported package is found in
    /// `imported_records` (under its package-qualifier-stripped spelling) when no
    /// local type claims the name. Without it an imported record's field typed
    /// as `Unknown`, which matched every nominal imported overload.
    /// The concrete type of a union `CASE` pattern.
    ///
    /// A pattern naming a `TYPE` template (bug-680) takes its arguments from the
    /// scrutinee: `CASE Some(s)` over an `Opt OF Integer` is the
    /// `Some OF Integer` instantiation that union carries. A template name is
    /// never a member on its own, so the scrutinee's union — its own members and
    /// every `INCLUDES`d union's — is the only place the arguments can come
    /// from. When that union carries no instantiation of the template, or more
    /// than one (so the name cannot say which), the pattern is reported here,
    /// in source spellings, rather than left for the post-monomorph resolve to
    /// misreport as an unknown type.
    ///
    /// Any other pattern (a concrete type, or a scrutinee that is not a known
    /// union) is substituted like every other type position.
    pub(super) fn union_pattern_type(
        &mut self,
        scrutinee: Option<&ParameterType>,
        pattern: &ParameterType,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        line: usize,
    ) -> ParameterType {
        let Some(union) = scrutinee.map(ParameterType::without_state) else {
            return self.concrete_type(pattern, substitutions);
        };
        let template = match pattern {
            ParameterType::Named(template)
                if self.type_templates.contains_key(pattern)
                    && self
                        .concrete_types
                        .get(&union)
                        .is_some_and(|decl| matches!(decl.kind, TypeDeclKind::Union)) =>
            {
                template.resolve()
            }
            _ => return self.concrete_type(pattern, substitutions),
        };
        let mut found = Vec::new();
        self.collect_union_instantiations(&union, template, &mut found, &mut HashSet::new());
        if let [only] = found.as_slice() {
            return only.clone();
        }
        let union_display = self.template_view(&union).name().replace('.', "::");
        let detail = if found.is_empty() {
            format!("CASE `{template}` is not a member of UNION `{union_display}`.")
        } else {
            let members = found
                .iter()
                .map(|member| format!("`{}`", self.template_view(member).name()))
                .collect::<Vec<_>>()
                .join(", ");
            format!(
                "CASE `{template}` is ambiguous in UNION `{union_display}`, which carries \
                 {members}; a CASE names a member by its type name, so a union may carry \
                 only one instantiation of a template."
            )
        };
        self.report("TYPE_MATCH_PATTERN_MISMATCH", &detail, line);
        pattern.clone()
    }

    fn collect_union_instantiations(
        &self,
        union: &ParameterType,
        template: &str,
        found: &mut Vec<ParameterType>,
        visited: &mut HashSet<ParameterType>,
    ) {
        if !visited.insert(union.clone()) {
            return;
        }
        let Some(type_decl) = self
            .concrete_types
            .get(union)
            .filter(|type_decl| matches!(type_decl.kind, TypeDeclKind::Union))
        else {
            return;
        };
        for variant in &type_decl.variants {
            let is_instance = self
                .type_instantiations
                .get(&variant.type_)
                .is_some_and(|(name, _)| name == template);
            if is_instance && !found.contains(&variant.type_) {
                found.push(variant.type_.clone());
            }
        }
        for include in &type_decl.includes {
            self.collect_union_instantiations(include, template, found, visited);
        }
    }

    pub(super) fn record_fields(&self, type_: &ParameterType) -> Option<&Vec<HirTypeField>> {
        self.concrete_types
            .get(type_)
            .filter(|type_decl| matches!(type_decl.kind, TypeDeclKind::Type))
            .map(|type_decl| &type_decl.fields)
            .or_else(|| self.imported_records.get(type_))
            .or_else(|| self.imported_records.get(&self.normalize_type(type_)))
    }

    /// The return type of a builtin/package call, using the same per-package
    /// `resolve_call` resolvers that the former source checker dispatches through
    /// (`SyntaxChecker::check_builtin_call`). Argument types are resolved
    /// positionally, falling back to `Unknown` so a resolver that keys on arity
    /// still sees the right shape. Without this, `expression_type` returned `None`
    /// for every builtin call, so a builtin-call argument was silently dropped
    /// from a generic/overloaded call's argument list (bug-103).
    fn builtin_call_return_type(
        &self,
        callee: &str,
        arguments: &[HirCallArg],
        context: &FunctionContext,
    ) -> Option<ParameterType> {
        let arg_types = arguments
            .iter()
            .map(|argument| {
                self.expression_type(call_arg_value(argument), context)
                    .unwrap_or(ParameterType::Unknown)
            })
            .collect::<Vec<_>>();
        // plan-106-A: the TYPED registry entry (plan-104-C), so neither the
        // argument types nor the resolved return crosses a string.
        crate::codegen::builtins::resolve_call_return_type_typed(callee, &arg_types, false).or_else(
            || {
                // bug-988 / bug-989: a call that only resolves by union-variant
                // subsumption types as lowering types it, so a template argument
                // can still be inferred from it. A general built-in keeps the
                // kind-less answer.
                if crate::codegen::builtins::general::is_general_call(callee) {
                    return None;
                }
                crate::codegen::builtins::resolve_call_return_type_with_kinds(
                    callee, &arg_types, false, self,
                )
            },
        )
    }

    /// Monomorph's type oracle: the (pre-instantiation) type of an expression,
    /// or `None` when the environment cannot decide it.
    ///
    /// plan-106-A: returns a [`ParameterType`], built structurally. Every arm
    /// that used to `format!` a spelling (`List OF …`, `Map OF … TO …`,
    /// `FUNC(…) AS …`) now constructs the variant directly, and the scalar arms
    /// are variants rather than `"Integer".to_string()`.
    pub(super) fn expression_type(
        &self,
        expression: &HirExpression,
        context: &FunctionContext,
    ) -> Option<ParameterType> {
        match expression {
            HirExpression::String(_) => Some(ParameterType::String),
            HirExpression::Number(value) => Some(match crate::numeric::classify_literal(value).1 {
                crate::numeric::LiteralType::Integer => ParameterType::Integer,
                crate::numeric::LiteralType::Float => ParameterType::Float,
                crate::numeric::LiteralType::Fixed => ParameterType::Fixed,
                crate::numeric::LiteralType::Money => ParameterType::Money,
            }),
            HirExpression::Scalar(_) => Some(ParameterType::named("Scalar")),
            HirExpression::Boolean(_) => Some(ParameterType::Boolean),
            HirExpression::Identifier(value) if value == "NOTHING" => Some(ParameterType::Nothing),
            HirExpression::Identifier(value) => context
                .locals
                .get(value)
                .cloned()
                .or_else(|| {
                    self.concrete_functions
                        .get(value)
                        .map(|function| function_signature_types(function).1)
                })
                .or_else(|| self.globals.get(value).cloned()),
            // plan-111-B: `Error` is a nominal, and the record table is keyed by
            // the type.
            HirExpression::Constructor { type_, .. } => {
                if type_.is_named("Error") {
                    Some(ParameterType::named("Error"))
                } else if self.record_fields(type_).is_some() {
                    Some(type_.clone())
                } else {
                    None
                }
            }
            HirExpression::WithUpdate { target, .. } => self.expression_type(target, context),
            HirExpression::ListLiteral(values) => Some(ParameterType::ListOf(Box::new(
                values
                    .first()
                    .and_then(|value| self.expression_type(value, context))
                    .unwrap_or(ParameterType::Unknown),
            ))),
            HirExpression::SetLiteral { element_type, .. } => {
                Some(ParameterType::SetOf(Box::new(element_type.clone())))
            }
            HirExpression::MapLiteral {
                key_type,
                value_type,
                ..
            } => Some(ParameterType::MapOf(
                Box::new(key_type.clone()),
                Box::new(value_type.clone()),
            )),
            HirExpression::MemberAccess { target, member } => {
                // `Color.Blue`: a member of a declared enum is a value of that enum,
                // mirroring the IR's `expression_type`. Without this arm the member
                // had no type, so an inferred `[Color.Blue, …]` bound a template's
                // `T` to `Unknown`.
                if let Some(enum_type) = self.enum_member_type(target, member) {
                    return Some(enum_type);
                }
                let target_type = self.expression_type(target, context)?;
                // bug-671: `h.state` is the `RES` handle's payload record,
                // mirroring the IR's `expression_type`.
                if member == "state" {
                    if let Some(state) = target_type.state() {
                        return Some(state);
                    }
                }
                self.record_fields(&target_type)?
                    .iter()
                    .find(|field| field.name == *member)
                    .map(|field| field.type_.clone())
            }
            HirExpression::Call {
                callee, arguments, ..
            } => self
                .concrete_functions
                .get(callee)
                .map(|function| function_signature_types(function).0)
                .or_else(|| self.builtin_call_return_type(callee, arguments, context)),
            HirExpression::Lambda {
                params,
                body,
                assign_target,
            } => {
                let mut nested = context.clone();
                let param_types = params
                    .iter()
                    .map(|param| {
                        nested
                            .locals
                            .insert(param.name.clone(), param.type_.clone());
                        param.type_.clone()
                    })
                    .collect::<Vec<_>>();
                // An assignment-bodied lambda yields `Nothing`; otherwise its
                // result type is the body expression's type.
                let returns = if assign_target.is_some() {
                    ParameterType::Nothing
                } else {
                    self.expression_type(body, &nested)?
                };
                // A lambda is never `ISOLATED` (that marker is only written on a
                // declared FUNC), reproducing the un-prefixed `FUNC(…) AS …`
                // spelling this arm used to `format!`.
                Some(ParameterType::Func(param_types, Box::new(returns), false))
            }
            HirExpression::Binary {
                operator,
                left,
                right,
                ..
            } => {
                if operator.is_comparison()
                    || matches!(operator, BinaryOp::And | BinaryOp::Or | BinaryOp::Xor)
                {
                    return Some(ParameterType::Boolean);
                }
                if *operator == BinaryOp::Concat {
                    return Some(ParameterType::String);
                }
                let left = self.expression_type(left, context)?;
                let right = self.expression_type(right, context)?;
                Some(
                    numeric::typed_binary_result_type(*operator, &left, &right)
                        .unwrap_or(ParameterType::Integer),
                )
            }
            HirExpression::Unary {
                operator, operand, ..
            } => {
                if *operator == UnaryOp::Not {
                    Some(ParameterType::Boolean)
                } else {
                    self.expression_type(operand, context)
                }
            }
            HirExpression::Trapped { expression, .. } => self.expression_type(expression, context),
        }
    }
}

/// The element type a `List`/`Set` literal's members are expected to have, given the
/// literal's own expected type — or `None` when the context expects something that is
/// not that container.
///
/// plan-105-B: routed through the canonical grammar rather than a
/// `strip_prefix("List OF ")` / `strip_prefix("Set OF ")` pair. `set` picks which
/// container the caller is lowering, so a `List` literal in a `Set`-typed context
/// still (correctly) gets no expected element type.
pub(super) fn expected_element_type(
    expected_type: Option<&ParameterType>,
    set: bool,
) -> Option<ParameterType> {
    match expected_type? {
        ParameterType::SetOf(element) if set => Some((**element).clone()),
        ParameterType::ListOf(element) if !set => Some((**element).clone()),
        _ => None,
    }
}
