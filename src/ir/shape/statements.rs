//! plan-176-N: the shape checker's statements — blocks and their flow, statements, values,
//! handlers and match cases. The `Walker` struct and the free helpers live in
//! `ir/shape/mod.rs`.

use super::*;

impl<'a> Walker<'a> {
    /// Walk a block in a scope of its own — `lower_statement_block` clones the
    /// enclosing locals per block, so a binding never leaks out of it.
    pub(super) fn walk_block(
        &mut self,
        body: &[HirStatement],
        locals: &HashMap<String, ParameterType>,
    ) {
        self.walk_block_flow(body, locals);
    }

    /// Walk a block and report how it ends, the way the source checker's
    /// `check_block` did: the first diverging statement ends the walk (the
    /// checker checked nothing after it), and when that statement is an `EXIT
    /// FUNC`/`EXIT PROGRAM` every statement after it is UNREACHABLE_AFTER_EXIT
    /// — those two lower to nothing and to an `ExitProgram` op that
    /// `ir::verify`'s loop-exit form does not treat as an exit, so the IR cannot
    /// report them (the `EXIT
    /// FOR`/`DO`/`WHILE`/`CONTINUE` forms are verify's). Inside an inline-TRAP
    /// handler every exit form is shape's: `treeify_handler` drops the
    /// statements after any terminator before lowering sees them.
    fn walk_block_flow(
        &mut self,
        body: &[HirStatement],
        locals: &HashMap<String, ParameterType>,
    ) -> Flow {
        let mut nested = locals.clone();
        self.walk_statements_flow(body, &mut nested)
    }

    /// `walk_block_flow` over a scope the caller keeps (a function body's
    /// top-level locals feed its TRAP body).
    pub(super) fn walk_statements_flow(
        &mut self,
        body: &[HirStatement],
        locals: &mut HashMap<String, ParameterType>,
    ) -> Flow {
        for (index, statement) in body.iter().enumerate() {
            let flow = self.walk_statement(statement, locals);
            if flow == Flow::AlwaysReturns {
                let erased_exit = match statement {
                    HirStatement::Exit {
                        target: ExitTarget::Func | ExitTarget::Program,
                        ..
                    } => true,
                    HirStatement::Exit { .. } | HirStatement::Continue { .. } => {
                        self.handler_depth > 0
                    }
                    _ => false,
                };
                if erased_exit {
                    for unreachable in &body[index + 1..] {
                        self.emit(
                            "UNREACHABLE_AFTER_EXIT",
                            "Statement is unreachable after EXIT or CONTINUE.".to_string(),
                            statement_line(unreachable),
                        );
                    }
                }
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    /// The checker's verdict on whether a block always diverges — for the
    /// inline-TRAP handler rule and for the walk order above.
    fn statement_flow(&self, statement: &HirStatement) -> Flow {
        match statement {
            HirStatement::Return { .. }
            | HirStatement::Exit { .. }
            | HirStatement::Continue { .. }
            | HirStatement::Fail { .. }
            | HirStatement::Propagate { .. }
            | HirStatement::Recover { .. } => Flow::AlwaysReturns,
            HirStatement::If {
                then_body,
                else_body,
                ..
            } => {
                if self.block_flow(then_body) == Flow::AlwaysReturns
                    && self.block_flow(else_body) == Flow::AlwaysReturns
                {
                    Flow::AlwaysReturns
                } else {
                    Flow::FallsThrough
                }
            }
            HirStatement::Match { cases, .. } => {
                // Exhaustiveness is judged as the checker did: an unguarded
                // CASE ELSE, or every variant/member of the scrutinee's type
                // named by an unguarded case. The scrutinee type is not
                // needed here — a MATCH whose every case diverges but whose
                // coverage cannot be established falls through, which is
                // what the checker answered for an untyped scrutinee too.
                let all_return = !cases.is_empty()
                    && cases
                        .iter()
                        .all(|case| self.block_flow(&case.body) == Flow::AlwaysReturns);
                if all_return && self.match_covered(cases) {
                    Flow::AlwaysReturns
                } else {
                    Flow::FallsThrough
                }
            }
            _ => Flow::FallsThrough,
        }
    }

    fn block_flow(&self, body: &[HirStatement]) -> Flow {
        for statement in body {
            if self.statement_flow(statement) == Flow::AlwaysReturns {
                return Flow::AlwaysReturns;
            }
        }
        Flow::FallsThrough
    }

    /// Whether a MATCH's unguarded cases cover its scrutinee (a `CASE ELSE`,
    /// or every variant of the union / member of the enum the cases name).
    fn match_covered(&self, cases: &[HirMatchCase]) -> bool {
        use crate::hir::HirMatchPattern;
        let mut covered: HashSet<String> = HashSet::new();
        for case in cases {
            if case.guard.is_some() {
                continue;
            }
            match &case.pattern {
                HirMatchPattern::Else => return true,
                HirMatchPattern::Union { type_, .. } => {
                    covered.insert(type_.name().into_owned());
                }
                HirMatchPattern::Literal(HirExpression::MemberAccess { target, member }) => {
                    if let HirExpression::Identifier(type_name) = target.as_ref() {
                        covered.insert(format!("{type_name}::{member}"));
                    }
                }
                _ => {}
            }
        }
        // The cases name one type; its declaration decides the full set.
        let Some(first) = covered.iter().next() else {
            return false;
        };
        if let Some((type_name, _)) = first.split_once("::") {
            return self
                .types
                .get(&ParameterType::declared(type_name))
                .is_some_and(|info| {
                    !info.members.is_empty()
                        && info
                            .members
                            .iter()
                            .all(|member| covered.contains(&format!("{type_name}::{member}")))
                });
        }
        self.types.values().any(|info| {
            info.is_union
                && info
                    .variants
                    .iter()
                    .any(|variant| covered.contains(variant))
                && info
                    .variants
                    .iter()
                    .all(|variant| covered.contains(variant))
        })
    }

    fn walk_statement(
        &mut self,
        statement: &HirStatement,
        locals: &mut HashMap<String, ParameterType>,
    ) -> Flow {
        self.current_line = statement_line(statement);
        let flow = self.statement_flow(statement);
        match statement {
            HirStatement::Let {
                resource,
                state_type,
                name,
                type_,
                explicit_type,
                value,
                line,
                ..
            } => {
                let declared_type = explicit_type.then(|| type_.clone());
                if let Some(HirExpression::Trapped {
                    expression,
                    binding,
                    handler,
                    line: trap_line,
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
                    let trapped_type = self.type_of(expression, locals);
                    self.walk_handler(binding, handler, locals, trapped_type, *trap_line);
                    // The checker typed the binding by the trapped call.
                    self.check_initializer_known(
                        name,
                        expression,
                        locals,
                        declared_type.as_ref(),
                        *line,
                    );
                    self.bind(name, success_type, locals);
                    return flow;
                }
                let lowered_type = declared_type.clone().unwrap_or_else(|| {
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
                    self.check_initializer_known(
                        name,
                        value,
                        locals,
                        declared_type.as_ref(),
                        *line,
                    );
                }
                if !*resource && lowered_type.state().is_some() {
                    self.state_dropped.insert(name.clone());
                } else {
                    self.state_dropped.remove(name);
                }
                self.bind(name, lowered_type, locals);
            }
            HirStatement::Return { value, line } => {
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                    // TYPE_UNKNOWN_VALUE, the RETURN form (see
                    // `check_initializer_known`); the declared return type is the
                    // expectation the checker inferred the value under.
                    let expected = self.context.current_return_type.clone();
                    if self.checker_types_unknown(value, locals, expected.as_ref()) {
                        self.emit(
                            "TYPE_UNKNOWN_VALUE",
                            "RETURN value does not have a known type.".to_string(),
                            *line,
                        );
                    }
                }
            }
            HirStatement::Exit { target, code, line } => {
                match target {
                    // EXIT FOR/DO/WHILE outside a matching loop is `ir::verify`'s.
                    ExitTarget::For | ExitTarget::Do | ExitTarget::While => {}
                    // EXIT_FUNC_FORBIDDEN: `ExitTarget::Func` lowers to NOTHING —
                    // the fact does not exist in the IR.
                    ExitTarget::Func => {
                        self.emit(
                            "EXIT_FUNC_FORBIDDEN",
                            "Functions must RETURN a value; EXIT FUNC is not allowed.".to_string(),
                            *line,
                        );
                    }
                    ExitTarget::Program => {}
                }
                if let Some(code) = code {
                    self.walk_expression(code, locals);
                }
            }
            HirStatement::Continue { .. } | HirStatement::Propagate { .. } => {}
            HirStatement::Fail { error, .. } => self.walk_expression(error, locals),
            HirStatement::Recover { value, line } => {
                if let Some(value) = value {
                    self.walk_expression(value, locals);
                }
                // TYPE_RECOVER_OUTSIDE_INLINE_TRAP: a stray RECOVER lowers to a
                // `$recover_stray` bind (plan-107-B) — the statement is gone.
                let Some(recover_type) = self.inline_trap_types.last().cloned() else {
                    self.emit(
                        "TYPE_RECOVER_OUTSIDE_INLINE_TRAP",
                        "RECOVER is valid only inside an inline TRAP handler.".to_string(),
                        *line,
                    );
                    return flow;
                };
                // TYPE_RECOVER_TYPE_MISMATCH, the two count forms: lowering
                // stores a RECOVER value into the trap slot only when both exist
                // (a valueless RECOVER for a value-producing trap and a value for
                // a value-less one lower to nothing / an `Eval`), so the IR
                // keeps no trace of the mismatch; the value-TYPE form is verify's.
                let produces_value = !matches!(recover_type, ParameterType::Nothing);
                match (value, produces_value) {
                    (None, true) => self.emit(
                        "TYPE_RECOVER_TYPE_MISMATCH",
                        format!(
                            "RECOVER must supply a {} value for the trapped expression.",
                            recover_type.name()
                        ),
                        *line,
                    ),
                    (Some(_), false) => self.emit(
                        "TYPE_RECOVER_TYPE_MISMATCH",
                        "RECOVER must not supply a value for a value-less trapped expression."
                            .to_string(),
                        *line,
                    ),
                    _ => {}
                }
            }
            HirStatement::Assign { name, value, line } => {
                // TYPE_UNKNOWN_VALUE, the assignment-target form: a target that
                // is neither a local nor a top-level binding. Lowering emits an
                // `AssignGlobal` for any non-local name, so the IR does not
                // know the name resolved to nothing.
                if !locals.contains_key(name) && self.context.binding_type(name).is_none() {
                    self.emit(
                        "TYPE_UNKNOWN_VALUE",
                        format!("Assignment target `{name}` is not a local binding."),
                        *line,
                    );
                }
                self.walk_value(value, locals);
            }
            HirStatement::StateAssign {
                resource,
                value,
                line,
            } => {
                // TYPE_UNKNOWN_VALUE, the state-assignment form: `res.state = …`
                // needs a LOCAL resource binding (a file-PRIVATE top-level
                // resource arrives mangled and is reported by its source name).
                if !locals.contains_key(resource) {
                    self.emit(
                        "TYPE_UNKNOWN_VALUE",
                        format!(
                            "State assignment target `{}` is not a local binding.",
                            crate::internal_name::display_name(resource)
                        ),
                        *line,
                    );
                }
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
        flow
    }

    /// A statement-position value: an inline-TRAP form walks its handler in the
    /// binding's scope; anything else is an ordinary expression.
    fn walk_value(&mut self, value: &HirExpression, locals: &HashMap<String, ParameterType>) {
        if let HirExpression::Trapped {
            expression,
            binding,
            handler,
            line,
        } = value
        {
            self.walk_expression(expression, locals);
            let trapped_type = self.type_of(expression, locals);
            self.walk_handler(binding, handler, locals, trapped_type, *line);
            return;
        }
        self.walk_expression(value, locals);
    }

    /// TYPE_UNKNOWN_VALUE, the foreign-record field form (bug-466). Reading a
    /// field off a record a BUILTIN package declares requires that package's own
    /// `IMPORT` in this file. Imports are not transitive and a package cannot
    /// re-export another's types, so `tcp::localAddress`'s `net::Address` result
    /// has no field table in a file that imported only `tcp` — the read has no
    /// type, and the value it produces is `Unknown`.
    ///
    /// **Why here and not in `ir::verify`, and why not by asking whether the
    /// read typed.** The `Unknown` was already caught wherever it was BOUND
    /// (`LET p AS Integer = b.port`) or fed to an operator, but not where it was
    /// a call argument: a declared `Integer` parameter accepted it, and an
    /// overloaded builtin unified it against a candidate rather than failing
    /// resolution. It then survived to native lowering, which asserted its own
    /// invariant with no source location to attach — `native plan has no storage
    /// class for type 'Unknown'`, or, one call away from the mistake, `native
    /// code field access target 'Address' is not a record or variant while
    /// lowering eval call io.print`. Gating the FIELD ACCESS ITSELF makes the
    /// verdict independent of where the `Unknown` happens to land, so a position
    /// nobody thought to check cannot reopen the hole.
    ///
    /// And it keys on the file's own imports rather than on whether the read
    /// resolved, because whether it resolved was never a property of this file:
    /// the type table is populated by whichever companion sources got injected,
    /// and `udp`'s `Datagram` names a `net.Address`, so adding an unreferenced
    /// `IMPORT udp` used to drag `Address`'s declaration in and make the
    /// identical `tcp` program compile. Same program, different verdict, decided
    /// by an unrelated import. This asks the registry instead, so both spellings
    /// are refused.
    ///
    /// `.state` is excluded: it is not a record field, and its own rules
    /// (`ir::verify`'s TYPE_STATE_INVALID, and the `state_dropped` cascade
    /// below) already report it more precisely.
    pub(super) fn check_foreign_record_field(
        &mut self,
        target: &HirExpression,
        member: &str,
        locals: &HashMap<String, ParameterType>,
    ) {
        if self.current_file_internal || member == "state" {
            return;
        }
        let target_type = strip_res(&self.type_of(target, locals)).without_state();
        let Some(owner) = self.builtin_record_owner.get(&target_type) else {
            return;
        };
        if self.current_own_imports.contains(*owner) {
            return;
        }
        // The rendered name is already package-qualified (`net.Address`), so
        // interpolating it after `{owner}::` spelled the type `net::net.Address` —
        // a doubled qualifier no program can write. Strip the owning package's
        // prefix so the message names the type the way source does: `net::Address`.
        let rendered = target_type.name();
        let type_name = rendered
            .strip_prefix(&format!("{owner}."))
            .unwrap_or(rendered.as_ref());
        self.emit(
            "TYPE_UNKNOWN_VALUE",
            format!(
                "Field `{member}` belongs to `{owner}::{type_name}`, whose fields are not visible in this file. Imports are not transitive and a package cannot re-export another's types, so add `IMPORT {owner}`."
            ),
            self.current_line,
        );
    }

    /// An inline-TRAP handler block: the error binding is an `Error` local
    /// visible only inside it (`lower_inline_trap`).
    pub(super) fn walk_handler(
        &mut self,
        binding: &str,
        handler: &[HirStatement],
        locals: &HashMap<String, ParameterType>,
        trapped_type: ParameterType,
        trap_line: usize,
    ) {
        let mut handler_locals = locals.clone();
        handler_locals.insert(binding.to_string(), ParameterType::named("Error"));
        self.inline_trap_types.push(trapped_type);
        self.handler_depth += 1;
        let handler_flow = self.walk_block_flow(handler, &handler_locals);
        self.handler_depth -= 1;
        self.inline_trap_types.pop();
        // TYPE_INLINE_TRAP_FALLS_THROUGH: the handler's fall-through edge is a
        // source shape — lowering emits the handler as the `If`'s else arm and
        // nothing marks where a path ended without RECOVER.
        if handler_flow != Flow::AlwaysReturns {
            self.emit(
                "TYPE_INLINE_TRAP_FALLS_THROUGH",
                "Inline TRAP handler must end every path in RECOVER or a diverging statement (RETURN, FAIL, or PROPAGATE)."
                    .to_string(),
                trap_line,
            );
        }
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
        // The checker bound a variant pattern only when the scrutinee is a
        // union declaring that variant (a `CASE Ok`/`CASE Error` arm, a
        // non-union scrutinee or an unknown variant leaves the name unbound,
        // so a read of it types `Unknown` — the cascade those fixtures pin).
        if self.checker_binds_pattern(&case.pattern, matched_type) {
            if let Some((binding, binding_type, _)) =
                lower::match_case_binding(&case.pattern, "$match", matched_type)
            {
                case_locals.insert(binding, binding_type);
            }
        }
        if let Some(guard) = &case.guard {
            self.walk_expression(guard, &case_locals);
        }
        self.walk_block(&case.body, &case_locals);
    }
}
