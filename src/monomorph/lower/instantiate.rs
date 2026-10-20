//! plan-176-M: template instantiation — lowering bindings, functions and types, binding a
//! template's parameters to a call's argument types, and choosing the most specific
//! candidate. The driver lives in `monomorph/lower/mod.rs`.

use super::*;

/// Why a template's parameters do not bind a call's argument types (bug-913).
enum TemplateBindFailure {
    /// More arguments than the template has parameters.
    TooManyArguments(usize),
    /// A parameter without a default is not supplied.
    TooFewArguments,
    /// An argument's type does not fit its parameter's pattern.
    Mismatch(ParameterType),
    /// The named type parameters appear only where no argument binds them.
    Unbound(Vec<String>),
}

/// A template FUNC's overload identity — its ordered parameter types and its
/// return type (§6 "Overloading"), rendered for the instantiation key (bug-913).
fn template_identity(template: &HirFunction) -> String {
    let params = template
        .params
        .iter()
        .map(|param| param.type_.name().into_owned())
        .collect::<Vec<_>>()
        .join(",");
    format!("({params}){}", template.returns.name())
}

impl<'a> Monomorphizer<'a> {
    pub(super) fn lower_type(
        &mut self,
        mut type_decl: HirTypeDecl,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        concrete_name: Option<String>,
    ) -> HirTypeDecl {
        if let Some(name) = concrete_name {
            type_decl.name = name;
        }
        type_decl.template_params.clear();
        type_decl.includes = type_decl
            .includes
            .iter()
            .map(|include| self.concrete_type(include, substitutions))
            .collect();
        type_decl.fields = type_decl
            .fields
            .iter()
            .map(|field| self.lower_field(field, substitutions))
            .collect();
        type_decl.variants = type_decl
            .variants
            .iter()
            .map(|variant| crate::hir::HirUnionVariant {
                type_: self.concrete_type(&variant.type_, substitutions),
                line: variant.line,
            })
            .collect();
        type_decl
    }

    pub(super) fn lower_binding(&mut self, mut binding: HirTopLevelBinding) -> HirTopLevelBinding {
        if binding.explicit_type {
            let declared = binding.type_.clone();
            binding.type_ = self.concrete_type(&declared, &HashMap::new());
        }
        if let Some(value) = binding.value.take() {
            let mut context = FunctionContext::default();
            let expected = binding.explicit_type.then(|| binding.type_.clone());
            binding.value = Some(self.lower_expression(
                &value,
                &HashMap::new(),
                &mut context,
                expected.as_ref(),
                binding.line,
            ));
        }
        binding
    }

    pub(super) fn lower_function(
        &mut self,
        function: HirFunction,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        concrete_name: Option<String>,
    ) -> HirFunction {
        // Attribute any diagnostic raised while lowering this body to the file
        // the function was declared in, restoring the caller's file afterward so
        // a nested instantiation doesn't leak its file to the enclosing frame
        // (bug-107). The incoming `function.name` is the origin name (template
        // name for an instantiation, concrete name on the top-level pass).
        let saved_file = self.current_file.take();
        self.current_file = self
            .function_files
            .get(&function.name)
            .cloned()
            .or(saved_file.clone());
        let internal_origin = self
            .function_files
            .get(&function.name)
            .is_some_and(|path| self.internal_files.contains(path));
        let mut result = self.lower_function_inner(function, substitutions, concrete_name);
        result.internal_origin |= internal_origin;
        self.current_file = saved_file;
        result
    }

    fn lower_function_inner(
        &mut self,
        mut function: HirFunction,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
        concrete_name: Option<String>,
    ) -> HirFunction {
        if let Some(name) = concrete_name {
            function.name = name;
        }
        function.template_params.clear();
        for param in &mut function.params {
            if !matches!(param.type_, ParameterType::Unknown) {
                let declared = param.type_.clone();
                param.type_ = self.concrete_type(&declared, substitutions);
            }
        }
        if !matches!(function.returns, ParameterType::Unknown) {
            let declared = function.returns.clone();
            function.returns = self.concrete_type(&declared, substitutions);
        }

        let mut context = FunctionContext::default();
        context.enclosing_return = opt_type(&function.returns);
        for param in &function.params {
            context.bound.insert(param.name.clone());
            if !matches!(param.type_, ParameterType::Unknown) {
                // bug-671: a `RES … STATE S` parameter carries its payload in the
                // local's type, as the IR records it, so `h.state.f` types.
                let param_type = match &param.state_type {
                    Some(state) => param
                        .type_
                        .with_state(&self.concrete_type(state, substitutions)),
                    None => param.type_.clone(),
                };
                context.locals.insert(param.name.clone(), param_type);
            }
        }
        function.body = self.lower_statements(&function.body, substitutions, &mut context);
        if let Some(trap) = &mut function.trap {
            let mut trap_context = context.clone();
            trap_context.bound.insert(trap.name.clone());
            trap_context
                .locals
                .insert(trap.name.clone(), ParameterType::named("Error"));
            trap.body = self.lower_statements(&trap.body, substitutions, &mut trap_context);
        }
        function
    }

    /// Reorder `arg_types` (built in source/call order) into the callee template's
    /// declared-parameter order when the call uses named arguments, so generic
    /// instantiation binds each type-param against the type of the argument that
    /// actually fills its parameter slot (bug-196). Returns `None` for a positional
    /// call or an unknown callee (both already in order).
    fn arg_types_in_param_order(
        params: &[crate::hir::HirParam],
        arguments: &[HirCallArg],
        arg_types: &[ParameterType],
    ) -> Option<Vec<ParameterType>> {
        if !arguments
            .iter()
            .any(|argument| matches!(argument, HirCallArg::Named { .. }))
        {
            return None;
        }
        let mut ordered: Vec<Option<ParameterType>> = vec![None; params.len()];
        let mut next_positional = 0usize;
        for (index, argument) in arguments.iter().enumerate() {
            let arg_type = arg_types.get(index)?.clone();
            match argument {
                HirCallArg::Positional(_) => {
                    while next_positional < ordered.len() && ordered[next_positional].is_some() {
                        next_positional += 1;
                    }
                    if next_positional < ordered.len() {
                        ordered[next_positional] = Some(arg_type);
                        next_positional += 1;
                    }
                }
                HirCallArg::Named { name, .. } => {
                    if let Some(slot) = params.iter().position(|param| param.name == *name) {
                        ordered[slot] = Some(arg_type);
                    }
                }
            }
        }
        // Only reorder when every parameter slot is filled: an omitted defaulted
        // slot would have no actual type here, and padding it would feed a bogus
        // actual to `unify_type`. In that case fall back to the source-order types
        // (unchanged prior behavior); the named-arg reorder this fixes is the
        // fully-provided call (bug-196).
        ordered.into_iter().collect()
    }

    /// Claim `symbol` for the instantiation identified by the unambiguous `key`
    /// (`name<args>`). `mangle_name` is lossy, so two distinct type-argument tuples
    /// can mangle to the same symbol; when that happens, suffix the loser so each
    /// instantiation keeps its own symbol (bug-226). A symbol already claimed by
    /// this same key returns unchanged, so re-instantiation is stable and the
    /// common (collision-free) case emits exactly the symbol it always did.
    fn unique_concrete_symbol(&mut self, symbol: String, key: &str) -> String {
        if let Some(owner) = self.concrete_symbol_keys.get(&symbol) {
            if owner == key {
                return symbol;
            }
            let mut n = 2usize;
            loop {
                let candidate = format!("{symbol}${n}");
                match self.concrete_symbol_keys.get(&candidate) {
                    Some(owner) if owner == key => return candidate,
                    Some(_) => n += 1,
                    None => {
                        self.concrete_symbol_keys
                            .insert(candidate.clone(), key.to_string());
                        return candidate;
                    }
                }
            }
        }
        self.concrete_symbol_keys
            .insert(symbol.clone(), key.to_string());
        symbol
    }

    /// Instantiate the template FUNC a call to `callee` selects, returning its
    /// concrete symbol; `None` when `callee` names no template, or when the call
    /// selects none (a plain overload then resolves it).
    ///
    /// bug-913: a template overloads by signature exactly like a plain FUNC. A
    /// plain overload of the name that matches the call exactly wins; otherwise
    /// every template whose parameters bind the call's argument types is a
    /// candidate, and the most specific one is chosen ([`Self::most_specific`]).
    /// A name with one template and no plain overload reports why its call does
    /// not bind, exactly as before.
    pub(super) fn instantiate_function(
        &mut self,
        callee: &str,
        arguments: &[HirCallArg],
        arg_types: &[ParameterType],
        line: usize,
    ) -> Option<String> {
        let name = &self.template_name(callee).to_string();
        let templates = self.function_templates.get(name)?.clone();
        // Internal generic implementations (e.g. `collections::sort`) carry the
        // untypeable sigil. bug-642: name the member the user called
        // (`collections::sort`), falling back to the readable `__` form for an
        // internal helper no user calls directly.
        let display: std::borrow::Cow<'_, str> =
            match crate::codegen::registry::public_member_for_rewrite(name) {
                Some(public) => std::borrow::Cow::Owned(public),
                None => crate::internal_name::display_name(name),
            };
        let plain = self.function_overloads.get(callee);
        if plain.is_some_and(|set| set.iter().any(|function| params_match(function, arg_types))) {
            return None;
        }
        let lone = templates.len() == 1 && plain.is_none();
        let mut applicable = Vec::new();
        for (index, (template, _)) in templates.iter().enumerate() {
            let ordered = Self::arg_types_in_param_order(&template.params, arguments, arg_types);
            let types = ordered.as_deref().unwrap_or(arg_types);
            match self.bind_template(template, types, !lone) {
                Ok(args) => applicable.push((index, args)),
                Err(failure) => {
                    if lone {
                        self.report_binding_failure(&display, template, failure, line);
                        return None;
                    }
                }
            }
        }
        let (index, args) = match applicable.len() {
            0 => {
                if plain.is_none() {
                    self.report(
                        "TYPE_CALL_ARGUMENT_MISMATCH",
                        &format!(
                            "Call to `{display}` matches none of its {} template overloads.",
                            templates.len()
                        ),
                        line,
                    );
                }
                return None;
            }
            1 => applicable.pop()?,
            _ => match Self::most_specific(&templates, &applicable, arg_types.len()) {
                Some(chosen) => applicable.swap_remove(chosen),
                None => {
                    self.report(
                        "TYPE_OVERLOAD_AMBIGUOUS",
                        &format!(
                            "Call to `{display}` matches {} template overloads equally; \
                             no one of them is more specific than the others.",
                            applicable.len()
                        ),
                        line,
                    );
                    return None;
                }
            },
        };
        let (template, template_file) = templates[index].clone();
        // The mangled symbol is a lossy encoding (every non-alphanumeric collapses
        // to `$`), so two distinct type-argument tuples of the same arity can
        // produce the same symbol — the second instantiation would then overwrite
        // the first in `concrete_functions` and both call sites would be rewritten
        // to one shared, possibly-wrong symbol (bug-226). The `name<args>` key IS
        // unambiguous, so disambiguate the symbol whenever a different key already
        // claimed it. Existing single-instantiation symbols are unchanged.
        let arg_names: Vec<String> = args.iter().map(|a| a.name().into_owned()).collect();
        // bug-913: two templates of one name instantiated at the same arguments
        // are two instantiations, so a name with several keys each by the
        // template's identity — its ordered parameter types and return type.
        let key = if templates.len() == 1 {
            format!("{name}<{}>", arg_names.join(","))
        } else {
            format!(
                "{name}{}<{}>",
                template_identity(&template),
                arg_names.join(",")
            )
        };
        let concrete_name = self.unique_concrete_symbol(mangle_name(name, &arg_names), &key);
        if self.emitted_function_keys.insert(key) {
            if !self.charge_instantiation(&display, line) {
                return None;
            }
            if self.template_instantiation_depth >= MAX_TEMPLATE_INSTANTIATION_DEPTH {
                self.report_instantiation_too_deep(&display, line);
                // Halt the whole enumeration, not just this leaf: a wide fan-out
                // would otherwise keep hitting the depth cap on every one of its
                // (exponentially many) sibling paths (bug-399).
                self.instantiation_limit_reached = true;
                return None;
            }
            self.template_instantiation_depth += 1;
            let mut full_substitutions = HashMap::new();
            for (param, arg) in template.template_params.iter().zip(args.iter()) {
                full_substitutions.insert(crate::intern::Symbol::intern(param), arg.clone());
            }
            // bug-642: a built-in package's body is attributed to the user call
            // that instantiated it. Inside such a body the site is already set and
            // is inherited, so a helper it instantiates points at the same call.
            let site = self
                .current_instantiation_site
                .clone()
                .or_else(|| self.user_instantiation_site(name, &display, line));
            let saved_site = std::mem::replace(&mut self.current_instantiation_site, site.clone());
            let mut lowered =
                self.lower_function(template, &full_substitutions, Some(concrete_name.clone()));
            self.current_instantiation_site = saved_site;
            if lowered.internal_origin {
                lowered.instantiated_at.extend(site);
            }
            self.template_instantiation_depth -= 1;
            // bug-911: the instantiation belongs to the file that declares its
            // template; `into_project` emits it there.
            self.function_files
                .insert(concrete_name.clone(), template_file);
            self.concrete_functions
                .insert(concrete_name.clone(), lowered);
            self.function_instantiation_names
                .insert(concrete_name.clone(), display.to_string());
        } else if self.current_instantiation_site.is_none() {
            // bug-642: a later user call reusing the instantiation is a site of its
            // own — a rule that fails inside the body fails for this call too.
            if let Some(site) = self.user_instantiation_site(name, &display, line) {
                if let Some(existing) = self.concrete_functions.get_mut(&concrete_name) {
                    if existing.internal_origin && !existing.instantiated_at.contains(&site) {
                        existing.instantiated_at.push(site);
                    }
                }
            }
        }
        Some(concrete_name)
    }

    /// Bind `template`'s type parameters from a call's argument types (already in
    /// declared-parameter order), without reporting. `complete` also requires every
    /// parameter without a default to be supplied — the applicability test of an
    /// overloaded name, where a template the call does not fill is not a
    /// candidate; a lone template's call leaves a missing argument to the checker,
    /// as it always has.
    fn bind_template(
        &self,
        template: &HirFunction,
        arg_types: &[ParameterType],
        complete: bool,
    ) -> Result<Vec<ParameterType>, TemplateBindFailure> {
        if arg_types.len() > template.params.len() {
            return Err(TemplateBindFailure::TooManyArguments(arg_types.len()));
        }
        if complete
            && template.params[arg_types.len()..]
                .iter()
                .any(|param| param.default.is_none())
        {
            return Err(TemplateBindFailure::TooFewArguments);
        }
        let param_symbols: HashSet<crate::intern::Symbol> = template
            .template_params
            .iter()
            .map(|param| crate::intern::Symbol::intern(param))
            .collect();
        let mut substitutions: HashMap<crate::intern::Symbol, ParameterType> = HashMap::new();
        for (param, actual) in template.params.iter().zip(arg_types.iter()) {
            // An absent (`Unknown`) parameter annotation contributes no pattern —
            // mirrors the old `opt_type_name(..) else continue`.
            if matches!(param.type_, ParameterType::Unknown) {
                continue;
            }
            // plan-111-B: `template_view` is structural, so the actual type
            // goes in and comes back as a type.
            let actual_view = self.template_view(actual);
            if !unify_type(
                &param.type_,
                &actual_view,
                &param_symbols,
                &mut substitutions,
            ) {
                return Err(TemplateBindFailure::Mismatch(actual_view));
            }
        }
        // plan-111-B: the bound arguments stay TYPES. They render once, for the
        // mangled symbol and the instantiation key — both symbols, which is a
        // sanctioned string sink — instead of being re-parsed to substitute them
        // back in.
        let unbound: Vec<String> = template
            .template_params
            .iter()
            .filter(|param| !substitutions.contains_key(&crate::intern::Symbol::intern(param)))
            .cloned()
            .collect();
        if !unbound.is_empty() {
            return Err(TemplateBindFailure::Unbound(unbound));
        }
        Ok(template
            .template_params
            .iter()
            .map(|param| substitutions[&crate::intern::Symbol::intern(param)].clone())
            .collect())
    }

    /// Report why a lone template's call does not bind.
    fn report_binding_failure(
        &mut self,
        display: &str,
        template: &HirFunction,
        failure: TemplateBindFailure,
        line: usize,
    ) {
        match failure {
            TemplateBindFailure::TooManyArguments(count) => self.report(
                "TYPE_CALL_ARITY_MISMATCH",
                &format!(
                    "Call to `{display}` has {count} argument(s), expected at most {}.",
                    template.params.len()
                ),
                line,
            ),
            // `bind_template` checks completeness only for an overloaded name,
            // never for the lone template this reports for.
            TemplateBindFailure::TooFewArguments => {}
            // `actual.name()` is a message-formatting sink, not a decision.
            TemplateBindFailure::Mismatch(actual) => self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display}` cannot infer template arguments from `{}`.",
                    actual.name()
                ),
                line,
            ),
            // A type-param the arguments cannot pin down (it appears only in the
            // return type, e.g. `FUNC make OF T() AS T`). Previously this returned
            // None silently and the call was left as the bare template name,
            // surfacing later as a confusing "unknown function" (bug-226).
            TemplateBindFailure::Unbound(missing) => self.report(
                "TYPE_CALL_ARGUMENT_MISMATCH",
                &format!(
                    "Call to `{display}` cannot infer template argument(s) `{}` from \
                     its arguments (they appear only in the return type). Supply them at a \
                     position the arguments determine.",
                    missing.join(", ")
                ),
                line,
            ),
        }
    }

    /// Of several templates that all bind a call, the one more specific than
    /// every other (bug-913): template `a` is at least as specific as `b` when
    /// `b`'s parameter patterns bind `a`'s own parameter types over the call's
    /// first `arity` parameters — every call `a` accepts, `b` accepts too. `None`
    /// when no single template is strictly the most specific.
    fn most_specific(
        templates: &[(HirFunction, String)],
        applicable: &[(usize, Vec<ParameterType>)],
        arity: usize,
    ) -> Option<usize> {
        let at_least_as_specific = |a: &HirFunction, b: &HirFunction| {
            let symbols: HashSet<crate::intern::Symbol> = b
                .template_params
                .iter()
                .map(|param| crate::intern::Symbol::intern(param))
                .collect();
            let mut substitutions = HashMap::new();
            a.params
                .iter()
                .zip(b.params.iter())
                .take(arity)
                .all(|(pa, pb)| unify_type(&pb.type_, &pa.type_, &symbols, &mut substitutions))
        };
        let mut chosen = None;
        for (position, (index, _)) in applicable.iter().enumerate() {
            let a = &templates[*index].0;
            let beats_all = applicable.iter().all(|(other, _)| {
                other == index || {
                    let b = &templates[*other].0;
                    at_least_as_specific(a, b) && !at_least_as_specific(b, a)
                }
            });
            if beats_all {
                if chosen.is_some() {
                    return None;
                }
                chosen = Some(position);
            }
        }
        chosen
    }

    /// bug-642: the site a user-frame call to internal template `name` is
    /// attributed to, or `None` when `name` is not a built-in package's template.
    fn user_instantiation_site(
        &self,
        name: &str,
        display: &str,
        line: usize,
    ) -> Option<crate::hir::InstantiationSite> {
        let internal = self
            .function_files
            .get(name)
            .is_some_and(|path| self.internal_files.contains(path));
        let file = self.current_file.clone()?;
        internal.then(|| crate::hir::InstantiationSite {
            file,
            line,
            callee: display.to_string(),
        })
    }

    pub(super) fn instantiate_type(&mut self, name: &str, args: &[ParameterType]) -> ParameterType {
        // The mangled symbol is a lossy encoding (every non-alphanumeric collapses
        // to `$`), so two distinct same-arity type-argument tuples can produce the
        // same symbol; without disambiguation the second instantiation would
        // overwrite the first in `concrete_types` and both use-sites would rewrite
        // to one shared, possibly-wrong concrete type. The `name<args>` key IS
        // unambiguous, so claim the symbol against it exactly as
        // `instantiate_function` does (bug-226 fixed the function half but left this
        // one unguarded — bug-400). A symbol claimed by only one key is unchanged.
        //
        // plan-111-B: the arguments are types; the KEY and the MANGLED SYMBOL
        // are symbols, so they render here — a symbol sink, which plan-111-A
        // §Rejected alternatives sanctions alongside diagnostics and wire
        // encode. What comes back is the mangled nominal as a type, so the
        // caller no longer re-parses it.
        let arg_names: Vec<String> = args.iter().map(|a| a.name().into_owned()).collect();
        let key = format!("{name}<{}>", arg_names.join(","));
        let concrete_name = self.unique_concrete_symbol(mangle_name(name, &arg_names), &key);
        let concrete = ParameterType::named(&concrete_name);
        self.type_instantiations
            .insert(concrete.clone(), (name.to_string(), args.to_vec()));
        if !self.emitted_type_keys.insert(key) {
            return concrete;
        }
        let Some(template) = self
            .type_templates
            .get(&ParameterType::declared(name))
            .cloned()
        else {
            return concrete;
        };
        if !self.charge_instantiation(name, 1) {
            return concrete;
        }
        if self.template_instantiation_depth >= MAX_TEMPLATE_INSTANTIATION_DEPTH {
            self.report_instantiation_too_deep(name, 1);
            // Halt the whole enumeration — see the `instantiate_function` twin.
            self.instantiation_limit_reached = true;
            return concrete;
        }
        self.template_instantiation_depth += 1;
        let mut substitutions = HashMap::new();
        for (param, arg) in template.template_params.iter().zip(args.iter()) {
            substitutions.insert(crate::intern::Symbol::intern(param), arg.clone());
        }
        let lowered = self.lower_type(template, &substitutions, Some(concrete_name.clone()));
        self.template_instantiation_depth -= 1;
        self.concrete_types.insert(concrete.clone(), lowered);
        concrete
    }

    fn lower_field(
        &mut self,
        field: &HirTypeField,
        substitutions: &HashMap<crate::intern::Symbol, crate::types::ParameterType>,
    ) -> HirTypeField {
        let mut lowered = field.clone();
        lowered.type_ = self.concrete_type(&field.type_, substitutions);
        lowered
    }
}
