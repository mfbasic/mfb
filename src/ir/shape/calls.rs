//! plan-176-N: the shape checker's call shapes and typing — arity, call shape, initializers,
//! constructors, `WITH` updates, visibility, pattern binding, config literals, and the
//! bind/emit helpers. The `Walker` struct and the free helpers live in `ir/shape/mod.rs`.

use super::*;

impl<'a> Walker<'a> {
    /// TYPE_CALL_ARITY_MISMATCH, the builtin count form: the checker counted
    /// the normalized argument list (names bound, unknown names dropped,
    /// extras kept) against the registry's arity range. True when the count
    /// was out of range (the checker ended the call's checks there).
    pub(super) fn check_builtin_arity(
        &mut self,
        callee: &str,
        canonical: &str,
        count: usize,
        line: usize,
    ) -> bool {
        let Some((min, max)) = builtins::arity(canonical) else {
            return false;
        };
        if count < min || count > max {
            let expected = if min == max {
                min.to_string()
            } else {
                format!("{min} to {max}")
            };
            self.emit(
                "TYPE_CALL_ARITY_MISMATCH",
                format!("Call to `{callee}` has {count} argument(s), expected {expected}."),
                line,
            );
            self.call_typed_unknown = true;
            return true;
        }
        false
    }

    /// The call-shape rules — the ones lowering's argument normalization
    /// erases, so the evidence exists only in the HIR:
    ///
    /// - TYPE_UNKNOWN_ARGUMENT_NAME / TYPE_DUPLICATE_ARGUMENT_NAME: a name
    ///   that binds to no parameter (or binds one twice) is silently dropped by
    ///   lowering, so the lowered `IrValue::Call` carries no trace of it.
    /// - TYPE_CALL_ARITY_MISMATCH, the declared-function form: lowering drops
    ///   the positional arguments a signature cannot take and fills every
    ///   omitted slot from its default, so the count the source wrote is gone.
    /// - TYPE_CALL_ARITY_MISMATCH, the builtin and function-value count forms:
    ///   lowering pads a builtin's optional trailing arguments with their
    ///   defaults (the fixed-ABI runtime helpers take a full list) and appends
    ///   the extras, so the count the source wrote is gone from the IR; the
    ///   named-argument omission forms of a builtin call (a parameter left
    ///   unsupplied before a later named one, a name set no overload takes) go
    ///   the same way — lowering binds what it can and moves on.
    ///
    /// A builtin's argument TYPES survive lowering and are `ir::verify`'s.
    pub(super) fn check_call_shape(
        &mut self,
        callee: &str,
        arguments: &[HirCallArg],
        line: usize,
        locals: &HashMap<String, ParameterType>,
    ) {
        if crate::codegen::builtins_testing::is_testing_call(callee) {
            self.check_expect_call(callee, arguments, line, locals);
            return;
        }
        let has_named = arguments
            .iter()
            .any(|argument| matches!(argument, HirCallArg::Named { .. }));
        let Some(params) = self.callee_params(callee, locals) else {
            return;
        };
        // The builtin count rule reads the canonical name's arity table.
        let canonical = self.canonical_callee(callee);
        match params {
            CalleeParams::FunctionValue(params) => {
                // TYPE_CALL_ARGUMENT_MISMATCH, the function-value named form: a
                // callable type keeps no parameter names, so a name cannot bind
                // — and lowering discards the name, so only the HIR shows it.
                if has_named {
                    self.emit(
                        "TYPE_CALL_ARGUMENT_MISMATCH",
                        format!(
                            "Call to function value `{callee}` cannot use named arguments because the callable type does not preserve parameter names."
                        ),
                        line,
                    );
                }
                // A callable type carries no defaults: exactly its count.
                if arguments.len() != params.len() {
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{callee}` has {} argument(s), expected {}.",
                            arguments.len(),
                            params.len()
                        ),
                        line,
                    );
                }
                for (index, argument) in arguments.iter().enumerate() {
                    let value = match argument {
                        HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => value,
                    };
                    let Some(expected) = params.get(index) else {
                        continue;
                    };
                    let actual = self.type_of(value, locals).without_state();
                    // A lambda's `STATE T` parameter rides in its callable type
                    // (bug-1030). Whether the argument's state agrees is
                    // `ir::verify`'s `check_argument_state_agreement`, as for
                    // a declared FUNC; the resource itself is compared here.
                    let expected = &expected.without_state();
                    if !self.expression_compatible(expected, &actual, value) {
                        self.emit(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            format!(
                                "Argument {} for `{callee}` has type {}, expected {}.",
                                index + 1,
                                actual.name(),
                                expected.name()
                            ),
                            line,
                        );
                    }
                }
            }
            CalleeParams::BuiltinUnnamed => {
                // No parameter-name metadata: a name cannot bind at all (bug-173 B);
                // the arguments stay in source order for the count.
                for argument in arguments {
                    if let HirCallArg::Named { name, line, .. } = argument {
                        self.report_unknown_name(callee, name, *line);
                    }
                }
                let source_order = source_order(arguments);
                self.check_builtin_call(callee, &canonical, &source_order, line, locals);
            }
            CalleeParams::BuiltinOverloads(overloads) => {
                // Whichever way selection ends, the checker's normalized list has
                // every argument — bound in overload order when an overload is
                // selected, else left in source order.
                let source_order = source_order(arguments);
                if !has_named {
                    self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                    return;
                }
                // Overload selection needs a well-formed name set: the first
                // duplicate, else the first unknown name, ends the check.
                let named: Vec<(&String, usize)> = arguments
                    .iter()
                    .filter_map(|argument| match argument {
                        HirCallArg::Named { name, line, .. } => Some((name, *line)),
                        HirCallArg::Positional(_) => None,
                    })
                    .collect();
                for (index, (name, arg_line)) in named.iter().enumerate() {
                    if named[..index].iter().any(|(earlier, _)| earlier == name) {
                        self.report_duplicate_name(callee, name, *arg_line);
                        self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                        return;
                    }
                }
                if let Some((name, arg_line)) = named.iter().find(|(name, _)| {
                    !overloads
                        .iter()
                        .any(|params| params.contains(&name.as_str()))
                }) {
                    self.report_unknown_name(callee, name, *arg_line);
                    self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                    return;
                }
                let positionals = arguments.len() - named.len();
                let supplied_names: Vec<&str> =
                    named.iter().map(|(name, _)| name.as_str()).collect();
                let required =
                    builtins::call_param_name_overload_required(&canonical).unwrap_or_default();
                if let Some(params) = builtins::select_param_name_overload(
                    &overloads,
                    &required,
                    positionals,
                    &supplied_names,
                ) {
                    let positional_values: Vec<&HirExpression> = arguments
                        .iter()
                        .filter_map(|argument| match argument {
                            HirCallArg::Positional(value) => Some(value),
                            HirCallArg::Named { .. } => None,
                        })
                        .collect();
                    let ordered: Vec<&HirExpression> = params
                        .iter()
                        .enumerate()
                        .filter_map(|(index, param)| {
                            if index < positionals {
                                positional_values.get(index).copied()
                            } else {
                                arguments.iter().find_map(|argument| match argument {
                                    HirCallArg::Named { name, value, .. } if name == param => {
                                        Some(value)
                                    }
                                    _ => None,
                                })
                            }
                        })
                        .collect();
                    self.check_builtin_call(callee, &canonical, &ordered, line, locals);
                    return;
                }
                // Every supplied name exists, but no overload's arity and layout
                // accept this combination: report the first parameter left
                // unsupplied by the smallest overload that names them all
                // (`connectTcp(host:, timeoutMs:)` omits `port`).
                let covering = overloads
                    .iter()
                    .filter(|params| supplied_names.iter().all(|name| params.contains(name)))
                    .min_by_key(|params| params.len());
                if let Some(params) = covering {
                    let missing = params.iter().enumerate().find(|(index, param)| {
                        *index >= positionals && !supplied_names.contains(param)
                    });
                    if let Some((_, missing)) = missing {
                        self.emit(
                            "TYPE_CALL_ARITY_MISMATCH",
                            format!(
                                "Call to `{callee}` omits parameter `{missing}` before a later supplied argument."
                            ),
                            line,
                        );
                        self.check_builtin_call(callee, &canonical, &source_order, line, locals);
                        return;
                    }
                }
                self.emit(
                    "TYPE_CALL_ARITY_MISMATCH",
                    format!("Call to `{callee}` has no overload taking these arguments."),
                    line,
                );
                self.check_builtin_call(callee, &canonical, &source_order, line, locals);
            }
            CalleeParams::Builtin(aliases) => {
                if !has_named {
                    let normalized: Vec<&HirExpression> = arguments
                        .iter()
                        .map(|argument| match argument {
                            HirCallArg::Positional(value) | HirCallArg::Named { value, .. } => {
                                value
                            }
                        })
                        .collect();
                    // `thread::start`'s entry check precedes its count (the
                    // checker ended the call's checks on a bad entry).
                    if canonical == "thread.start"
                        && !self.thread_start_entry_valid(normalized.first().copied())
                    {
                        self.report_thread_entry(line);
                        return;
                    }
                    self.check_builtin_call(callee, &canonical, &normalized, line, locals);
                    return;
                }
                let mut ordered: Vec<Option<&HirExpression>> = vec![None; aliases.len()];
                let mut extras: Vec<&HirExpression> = Vec::new();
                let mut next_positional = 0usize;
                let mut saw_unknown_named = false;
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) => {
                            while next_positional < ordered.len()
                                && ordered[next_positional].is_some()
                            {
                                next_positional += 1;
                            }
                            if next_positional < ordered.len() {
                                ordered[next_positional] = Some(value);
                                next_positional += 1;
                            } else {
                                extras.push(value);
                            }
                        }
                        HirCallArg::Named { name, value, line } => {
                            let Some(index) = aliases
                                .iter()
                                .position(|aliases| aliases.iter().any(|alias| alias == name))
                            else {
                                self.report_unknown_name(callee, name, *line);
                                saw_unknown_named = true;
                                continue;
                            };
                            if ordered[index].is_some() {
                                // Reported under the parameter's canonical
                                // (first) alias, whichever alias the call wrote.
                                self.report_duplicate_name(callee, aliases[index][0], *line);
                                continue;
                            }
                            ordered[index] = Some(value);
                        }
                    }
                }
                // The checker's normalized list: the bound slots in parameter
                // order, then the extras; an unknown or duplicate name binds
                // nowhere and drops out of it.
                let normalized: Vec<&HirExpression> =
                    ordered.iter().flatten().copied().chain(extras).collect();
                // An unknown name has already disarmed the omission check: the
                // gap it left is its own diagnostic's.
                if !saw_unknown_named {
                    for (index, names) in aliases.iter().enumerate() {
                        if ordered[index].is_none()
                            && ordered[index + 1..].iter().any(|filled| filled.is_some())
                        {
                            self.emit(
                                "TYPE_CALL_ARITY_MISMATCH",
                                format!(
                                    "Call to `{callee}` omits parameter `{}` before a later supplied argument.",
                                    names[0]
                                ),
                                line,
                            );
                            break;
                        }
                    }
                }
                if canonical == "thread.start"
                    && !self.thread_start_entry_valid(normalized.first().copied())
                {
                    self.report_thread_entry(line);
                    return;
                }
                self.check_builtin_call(callee, &canonical, &normalized, line, locals);
            }
            CalleeParams::Declared(params) => {
                let mut ordered: Vec<Option<&HirExpression>> = vec![None; params.len()];
                let mut next_positional = 0usize;
                let mut supplied = 0usize;
                let mut arity_error = false;
                for argument in arguments {
                    match argument {
                        HirCallArg::Positional(value) => {
                            while next_positional < ordered.len()
                                && ordered[next_positional].is_some()
                            {
                                next_positional += 1;
                            }
                            if next_positional >= ordered.len() {
                                // An excess positional argument has no slot but is
                                // still one the call supplied: `f(1, 2)` against
                                // `f(x)` "has 2 argument(s)", not 1 (bug-653).
                                arity_error = true;
                                supplied += 1;
                                continue;
                            }
                            ordered[next_positional] = Some(value);
                            next_positional += 1;
                            supplied += 1;
                        }
                        HirCallArg::Named { name, value, line } => {
                            let Some(index) = params.iter().position(|param| param.name == *name)
                            else {
                                self.report_unknown_name(callee, name, *line);
                                continue;
                            };
                            if ordered[index].is_some() {
                                self.report_duplicate_name(callee, name, *line);
                                continue;
                            }
                            ordered[index] = Some(value);
                            supplied += 1;
                        }
                    }
                }
                let required = params.iter().filter(|param| !param.has_default).count();
                let missing_required = ordered
                    .iter()
                    .zip(params.iter())
                    .any(|(slot, param)| slot.is_none() && !param.has_default);
                if arity_error || supplied < required || supplied > params.len() || missing_required
                {
                    self.emit(
                        "TYPE_CALL_ARITY_MISMATCH",
                        format!(
                            "Call to `{callee}` has {supplied} argument(s), expected {required} to {}.",
                            params.len()
                        ),
                        line,
                    );
                }
                // TYPE_CALL_ARGUMENT_MISMATCH, the declared-function form over
                // the SUPPLIED arguments only: lowering fills every omitted slot
                // from its default, so the IR cannot tell a supplied argument
                // from a filled one (and a literal supplied argument is coerced
                // to the parameter type before the IR sees it).
                for (index, slot) in ordered.iter().enumerate() {
                    let Some(argument) = slot else {
                        continue;
                    };
                    let actual = self.type_of(argument, locals).without_state();
                    if !self.expression_compatible(&params[index].type_, &actual, argument) {
                        self.emit(
                            "TYPE_CALL_ARGUMENT_MISMATCH",
                            format!(
                                "Argument {} for `{callee}` has type {}, expected {}.",
                                index + 1,
                                actual.name(),
                                params[index].type_.name()
                            ),
                            line,
                        );
                    }
                }
            }
        }
    }

    /// A builtin argument-type rejection after which the checker typed the
    /// call `Unknown` (so a binding of it cascades TYPE_UNKNOWN_VALUE).
    pub(super) fn emit_call_typed_unknown(&mut self, detail: String, line: usize) {
        self.call_typed_unknown = true;
        self.emit("TYPE_CALL_ARGUMENT_MISMATCH", detail, line);
    }

    /// TYPE_UNKNOWN_VALUE, the initializer form — the source checker's cascade
    /// for a binding whose initializer it could not type. Its evidence is the
    /// checker's own typing verdict on the HIR: lowering stamps a lenient type
    /// on a failed builtin call and fills a `$`-temp for a trapped one, so the
    /// IR cannot tell the checker's `Unknown` from a typed value. `ir::verify`
    /// keeps the cascade for the values ITS rules poison (a mismatched
    /// operator, a bad constructor) — those the shape pass does not judge.
    pub(super) fn check_initializer_known(
        &mut self,
        name: &str,
        value: &HirExpression,
        locals: &HashMap<String, ParameterType>,
        expected: Option<&ParameterType>,
        line: usize,
    ) {
        if self.checker_types_unknown(value, locals, expected) {
            self.emit(
                "TYPE_UNKNOWN_VALUE",
                format!("Initializer for binding `{name}` does not have a known type."),
                line,
            );
        }
    }

    /// Whether the source checker would have typed `value` `Unknown` — its
    /// cascade condition — reconstructed from lowering's seam plus the few
    /// verdicts the checker made that the seam does not: a call its rules
    /// typed `Unknown`, a constructor or `WITH` of something it would not
    /// construct, an arithmetic on `Money` its lattice rejects or on a
    /// value-less call, `.state` on a plain `LET` of a stateful resource.
    /// (`ir::verify` cascades for the operator misuses its own rules poison.)
    pub(super) fn checker_types_unknown(
        &self,
        value: &HirExpression,
        locals: &HashMap<String, ParameterType>,
        expected: Option<&ParameterType>,
    ) -> bool {
        match value {
            HirExpression::Trapped { expression, .. } => {
                return self.checker_types_unknown(expression, locals, expected);
            }
            // A bare general built-in predicate in a value position types from
            // the expectation (`LET f AS FUNC(Integer) AS Boolean = isEven`,
            // bug-368) — lowering's seam has no expectation to type it by.
            HirExpression::Identifier(name)
                if crate::codegen::builtins::general::builtin_function_id(name).is_some() =>
            {
                if let Some(ParameterType::Func(params, returns, _)) = expected {
                    if params.len() == 1
                        && **returns == ParameterType::Boolean
                        && crate::codegen::builtins::general::filter_predicate_type_typed(
                            name, &params[0],
                        )
                        .is_some()
                    {
                        return false;
                    }
                }
            }
            HirExpression::Constructor { type_, .. } => return !self.constructor_typed(type_),
            HirExpression::WithUpdate { target, .. } => {
                return !self.with_update_typed(target, locals);
            }
            HirExpression::Call { callee, .. } => {
                let canonical = self.canonical_callee(callee);
                if builtins::is_package_constant(&canonical) {
                    return false;
                }
                if self
                    .call_verdicts
                    .get(&(value as *const HirExpression as usize))
                    .copied()
                    .unwrap_or(false)
                {
                    return true;
                }
                // A package-table builtin call the argument checker matched is
                // typed by the overload's DECLARED return type, even when an
                // argument's own type is unknown (`crypto::hash(Hash.Nope, s)`:
                // the unknown member is its own rule, and `Unknown` is
                // compatible with the `Hash` slot) — where lowering's exact
                // registry resolution answers `Unknown` and would cascade.
                if builtins::is_builtin_call(&canonical) && builtins::table_checked_call(&canonical)
                {
                    return false;
                }
            }
            HirExpression::Binary {
                left,
                operator,
                right,
                ..
            } if matches!(
                operator,
                BinaryOp::Add
                    | BinaryOp::Subtract
                    | BinaryOp::Multiply
                    | BinaryOp::Divide
                    | BinaryOp::IntDiv
                    | BinaryOp::Mod
                    | BinaryOp::Power
            ) =>
            {
                // A value-less or untyped operand: the checker's
                // promotion had nothing numeric to promote.
                let (left, right) = (self.type_of(left, locals), self.type_of(right, locals));
                if matches!(left, ParameterType::Nothing | ParameterType::Unknown)
                    || matches!(right, ParameterType::Nothing | ParameterType::Unknown)
                {
                    return true;
                }
                let (left_money, right_money) = (
                    matches!(left, ParameterType::Money),
                    matches!(right, ParameterType::Money),
                );
                if (left_money || right_money)
                    && crate::numeric::is_numeric(&left)
                    && crate::numeric::is_numeric(&right)
                    && crate::numeric::typed_money_result_type(*operator, left_money, right_money)
                        .is_none()
                {
                    return true;
                }
            }
            HirExpression::MemberAccess { target, member } if member == "state" => {
                if let HirExpression::Identifier(name) = target.as_ref() {
                    if self.state_dropped.contains(name) {
                        return true;
                    }
                }
            }
            _ => {}
        }
        matches!(
            lower::expression_type(value, locals, &self.context),
            None | Some(ParameterType::Unknown)
        )
    }

    /// Whether the checker typed a `T[…]` constructor: the three built-in
    /// nominals (rejected as read-only but typed), else a visible declared
    /// RECORD; `Ok`/`Result`, a compiler-owned record, a union, an enum or an
    /// unknown name typed `Unknown`.
    fn constructor_typed(&self, type_: &ParameterType) -> bool {
        // Nominal tests, not a render-and-match: every name here is a bare
        // `Named`, so `is_named` decides identically for every input while
        // dropping the `name()` render. (plan-111-D Correction D1 — a letter-B
        // site the arm scanner could not see until it learned to read a whole
        // pattern rather than only a leading spelling.)
        if type_.is_named("AttributedString")
            || type_.is_named("Error")
            || type_.is_named("ErrorLoc")
        {
            return true;
        }
        self.declared_record_constructible(type_)
    }

    /// A declared (or imported) record the calling file may construct — the
    /// one kind whose arguments the checker went on to check.
    pub(super) fn declared_record_constructible(&self, type_: &ParameterType) -> bool {
        if read_only_record(type_) {
            return false;
        }
        self.types
            .get(type_)
            .is_some_and(|info| info.is_record && self.type_visible(info))
    }

    /// Whether the checker typed a `WITH target { … }`: a built-in nominal or a
    /// map entry keeps its type (the read-only rejection is `ir::verify`'s);
    /// otherwise the target must be a declared record that is not compiler-owned.
    fn with_update_typed(
        &self,
        target: &HirExpression,
        locals: &HashMap<String, ParameterType>,
    ) -> bool {
        let target_type = self.type_of(target, locals);
        if matches!(target_type, ParameterType::MapEntryOf(..)) {
            return true;
        }
        let ParameterType::Named(_) = &target_type else {
            return false;
        };
        let name = target_type.name();
        if matches!(
            name.as_ref(),
            "AttributedString" | "Error" | "ErrorLoc" | "Scalar"
        ) {
            return true;
        }
        if read_only_record(&target_type) {
            return false;
        }
        self.types
            .get(&target_type)
            .is_some_and(|info| info.is_record)
    }

    /// The checker's visibility rule for a declared type: `PRIVATE` only from
    /// its own file.
    fn type_visible(&self, info: &TypeShape) -> bool {
        match info.visibility {
            Visibility::Export | Visibility::Public => true,
            Visibility::Private => info.file == self.file,
        }
    }

    /// Whether the checker bound a MATCH pattern's name (see `walk_match_case`).
    pub(super) fn checker_binds_pattern(
        &self,
        pattern: &crate::hir::HirMatchPattern,
        matched_type: &ParameterType,
    ) -> bool {
        let crate::hir::HirMatchPattern::Union { type_, .. } = pattern else {
            return false;
        };
        let variant = type_.name();
        if matches!(variant.as_ref(), "Error" | "Err") {
            return false;
        }
        // Correction G6: peel the STATE clause BEFORE asking "is this a
        // nominal?". Until plan-111-B a stateful resource union arrived here as
        // one opaque `Named("Stream STATE Cursor")` — it passed this guard, and
        // the string-splitting `without_state` below peeled the clause out of
        // the spelling. `Stateful` is a NEW variant, so the unpeeled value is no
        // longer a `Named`, the guard rejected every `CASE File(f)` over a
        // stateful union, `f` went unbound, and `f.state.pos` typed `Unknown`.
        // Peeled first, this decides exactly as it did before for every input.
        let union = matched_type.without_state();
        let ParameterType::Named(_) = union else {
            return false;
        };
        self.types
            .get(&union)
            .is_some_and(|info| info.is_union && info.variants.iter().any(|v| *v == variant))
    }

    /// `os::getConfigOr(key, default)`: each argument must be a literal (a
    /// negated number counts). Its type is the type rules' concern, and a call
    /// with the wrong argument count is the arity rule's.
    pub(super) fn check_config_call_literals(&mut self, arguments: &[HirCallArg], line: usize) {
        if arguments.len() != 2 {
            return;
        }
        let mut positional = 0;
        for argument in arguments {
            let (name, value) = match argument {
                HirCallArg::Named { name, value, .. } => (name.as_str(), value),
                HirCallArg::Positional(value) => {
                    positional += 1;
                    (if positional == 1 { "key" } else { "default" }, value)
                }
            };
            if !is_config_literal(value) {
                self.emit(
                    "OS_CONFIG_ARGUMENT_NOT_LITERAL",
                    format!(
                        "`os::getConfigOr` needs a literal `{name}`; the value is read when the program is built."
                    ),
                    line,
                );
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

    fn report_duplicate_name(&mut self, callee: &str, name: &str, line: usize) {
        self.emit(
            "TYPE_DUPLICATE_ARGUMENT_NAME",
            format!("Call to `{callee}` supplies parameter `{name}` more than once."),
            line,
        );
    }

    /// Bind `name` in `locals` at `type_`, exactly where lowering emits its
    /// `IrOp::Bind` for the same binding.
    pub(super) fn bind(
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
    pub(super) fn emit(&mut self, rule: &str, detail: String, line: usize) {
        if !self.sites.is_empty() {
            for site in &self.sites {
                self.diagnostics.push(PendingDiagnostic {
                    rule: rule.to_string(),
                    detail: site.describe(&detail),
                    path: self.project_dir.join(&site.file),
                    line: site.line,
                });
            }
            return;
        }
        self.diagnostics.push(PendingDiagnostic {
            rule: rule.to_string(),
            detail,
            path: self.project_dir.join(&self.file),
            line,
        });
    }
}
