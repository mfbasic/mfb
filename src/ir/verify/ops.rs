use super::*;

impl TypeEnv {
    // 2. `check_ops` — per-op structural + type checks (one large dispatch)
    // ===========================================================================

    pub(super) fn check_ops(
        &self,
        ops: &[IrOp],
        locals: &mut HashMap<String, String>,
        muts: &mut HashMap<String, bool>,
        closure_slots: Option<usize>,
        depth: usize,
    ) {
        if depth > MAX_DEPTH {
            self.emit(
                VERIFY_TYPE,
                format!("statement nesting exceeds the {MAX_DEPTH} level limit"),
            );
            return;
        }
        // `$`-temp name → its numeric-literal bind value, for rules that read a
        // literal through a synthesized temp (a FOR loop's STEP is always bound
        // to a `$for` temp immediately before its For op in the same op list).
        let mut temp_consts: HashMap<&str, &IrValue> = HashMap::new();
        let mut exited_at: Option<usize> = None;
        for (op_index, op) in ops.iter().enumerate() {
            let line = op.loc().line;
            self.current_line.set(line);
            // Anything after an EXIT/CONTINUE in the same block is unreachable
            // (syntaxcheck reports each following statement, then stops).
            if let Some(exit_index) = exited_at {
                if op_index > exit_index {
                    self.emit(
                        "UNREACHABLE_AFTER_EXIT",
                        "Statement is unreachable after EXIT or CONTINUE.".to_string(),
                    );
                    continue;
                }
            }
            if matches!(op, IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. }) {
                exited_at = Some(op_index);
            }
            match op {
                IrOp::Bind {
                    mutable,
                    name,
                    type_,
                    value,
                    explicit_type,
                    ..
                } => {
                    if let Some(value) = value {
                        self.poisoned.set(false);
                        self.check_value_captures(value, closure_slots);
                        // A `$`-temp bind is the trap machinery capturing a
                        // *statement-position* call result — a SUB call is
                        // legal there (`doEffect(v) TRAP(e)`).
                        if name.starts_with('$') {
                            self.allow_sub_call.set(true);
                        }
                        self.check_value(value, locals);
                        self.allow_sub_call.set(false);
                        // syntaxcheck's cascade: an initializer whose type could
                        // not be determined *because it is erroneous* also gets
                        // TYPE_UNKNOWN_VALUE. Gate on a poisoning rule having
                        // fired for this very value, so a clean-but-untypable
                        // initializer (an external LINK call the lowering has
                        // no signature for) is never rejected.
                        if self.value_type_poisoned(value, locals) {
                            self.emit(
                                "TYPE_UNKNOWN_VALUE",
                                format!(
                                    "Initializer for binding `{name}` does not have a known type."
                                ),
                            );
                        }
                        let before = self.diags.borrow().len();
                        self.check_literal_range(resource_base_type(type_), value);
                        let range_errored = self.diags.borrow().len() > before;
                        // Only an explicit `AS T` annotation can disagree with
                        // the initializer; an inferred type is the initializer's
                        // type by construction (matches syntaxcheck).
                        if !range_errored && *explicit_type {
                            self.check_binding_type(name, type_, value, locals);
                        }
                    }
                    // A declared map type's key must be comparable; the
                    // inferred case is covered at its MapLiteral (checking it
                    // here too would double-report).
                    if *explicit_type {
                        self.check_map_key_comparable(type_);
                        self.check_collection_res_axis(resource_base_type(type_));
                    }
                    // The RES ownership axis (syntaxcheck's
                    // check_resource_declaration): a resource-typed binding
                    // must be RES-declared (else its close obligation is
                    // untracked — a leak/UAF on decoded IR), and RES may only
                    // mark a resource. RES-ness on the IR = membership in the
                    // function's resource-owner table.
                    //
                    // bug-376: this deliberately does NOT gate on
                    // `explicit_type`. The gate above at check_binding_type is
                    // right to — only an `AS T` annotation can *disagree* with
                    // the initializer. But this block compares nothing: it asks
                    // whether `type_` is a resource and whether `name` is an
                    // owner, and `type_` is populated for inferred bindings by
                    // construction. Gating here silently exempted
                    // `LET f = fs::open(...)`, dropping the close obligation
                    // along with the annotation.
                    //
                    // Ungating exposes the *synthesized* binds that also carry
                    // `explicit_type: false`, so they are excluded by shape.
                    // The exclusion list is exhaustive over the emitters in
                    // `src/ir/lower.rs` (audited for bug-376):
                    //   - `$`-prefixed temps ($match, $for_iter, $trap_res,
                    //     $trap_val): the guard below. Load-bearing — a
                    //     `$trap_val` for `RES f = open() TRAP` is typed with
                    //     the resource itself.
                    //   - `UnionExtract`: the `CASE File(f)` match-arm binding
                    //     (lower.rs match_case_binding). There is nowhere to
                    //     write `RES` in a case pattern, so the name is
                    //     legitimately absent from the owner table.
                    //   - `ResultValue`/`ResultError`: the match-arm siblings on
                    //     the `Result OF` path, and the `TRAP(e)` handler
                    //     binding.
                    //   - `Capture`: the lambda-capture prologue, whose
                    //     synthesized function has an empty owner table by
                    //     construction. Exempted rather than owner-listed:
                    //     adding every capture to `current_owners` would make
                    //     ordinary data captures trip TYPE_RES_REQUIRES_RESOURCE
                    //     below. This leaves no hole — a genuine resource
                    //     capture is already rejected, more precisely, by
                    //     TYPE_LAMBDA_CAPTURE_UNSUPPORTED.
                    // The remaining `explicit_type: false` emitters (the FOR
                    // loop variable, the `TRAP(e)` binding) are structurally
                    // non-resource: numeric and `Error` respectively.
                    let synthesized_bind = matches!(
                        value,
                        Some(
                            IrValue::UnionExtract { .. }
                                | IrValue::ResultValue { .. }
                                | IrValue::ResultError { .. }
                                | IrValue::Capture { .. }
                        )
                    );
                    let base = resource_base_type(type_);
                    let is_resource = self.is_resource_or_resource_union(base);
                    if !synthesized_bind && !name.starts_with('$') {
                        let is_res_declared = self.current_owners.borrow().contains(name.as_str());
                        if is_resource && !is_res_declared {
                            self.emit(
                                "TYPE_RESOURCE_REQUIRES_RES",
                                format!(
                                    "binding `{name}` holds resource `{base}`; bind it with `RES`, not `LET`/`MUT`."
                                ),
                            );
                        } else if is_res_declared && !is_resource && self.provably_data_type(base) {
                            // Only a POSITIVELY known data type rejects: an
                            // unknown name may be an external package's
                            // resource (e.g. sqlite3's Db), which the source
                            // lowering has no table for.
                            self.emit(
                                "TYPE_RES_REQUIRES_RESOURCE",
                                format!(
                                    "binding `{name}` is declared `RES` but `{base}` is not a resource type; use `LET`/`MUT`."
                                ),
                            );
                        }
                    }
                    // STATE is undefined on a resource union (varies by
                    // tag), and a STATE payload type must be defaultable.
                    // `state_type_name` peels only a *top-level* STATE, so a
                    // thread handle whose plane carries `STATE` (`Thread OF RES
                    // File STATE Cursor TO Out`, plan-54) is not misread here.
                    //
                    // bug-376 widened only the RES axis above; these keep the
                    // `explicit_type` gate they have always had. A STATE clause
                    // can only be *written*, so an inferred binding has nothing
                    // new for them to check.
                    if *explicit_type && !name.starts_with('$') {
                        if let Some(state_type) = crate::builtins::resource::state_type_name(type_)
                        {
                            if self.unions.contains_key(base) {
                                self.emit(
                                    "TYPE_UNION_STATE_FORBIDDEN",
                                    format!(
                                        "binding `{name}` attaches STATE to resource union `{base}`; a resource union carries no STATE — use a concrete stateful resource."
                                    ),
                                );
                            }
                            if !self.is_defaultable(state_type, &mut HashSet::new()) {
                                self.emit(
                                    "TYPE_STATE_INVALID",
                                    format!(
                                        "binding `{name}` STATE type `{state_type}` must be a copyable, defaultable data type."
                                    ),
                                );
                            }
                        }
                        if is_resource {
                            self.check_binding_state_agreement(name, type_, value, locals);
                        }
                    }
                    // plan-59-E: RES-binding a collection element used to be
                    // rejected here (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`, retired).
                    // Under scope ownership a `RES` is a pointer to the one
                    // resource, and an element is such a pointer like any other,
                    // so the binding is legal and closes exactly once — by
                    // whichever scope ends up owning it.
                    // An initializer-less binding must be annotated, immutable
                    // ones must have a value, and MUT needs a defaultable type
                    // (syntaxcheck's check_binding_shape None-value arms).
                    // Synthesized `$` temps are the compiler's own.
                    if value.is_none() && !name.starts_with('$') {
                        if !*explicit_type {
                            self.emit(
                                "TYPE_BINDING_REQUIRES_TYPE_OR_VALUE",
                                format!("Binding `{name}` needs a type annotation or initializer."),
                            );
                        } else if !*mutable {
                            self.emit(
                                "TYPE_LET_REQUIRES_VALUE",
                                format!("Immutable binding `{name}` must have an initializer."),
                            );
                        } else if !self.is_defaultable(type_, &mut HashSet::new()) {
                            self.emit(
                                "TYPE_MUT_REQUIRES_DEFAULTABLE_TYPE",
                                format!(
                                    "Mutable binding `{name}` cannot omit its initializer because type `{type_}` does not have a defined default value."
                                ),
                            );
                        }
                    }
                    locals.insert(name.clone(), type_.clone());
                    // A capture bind's `mutable` reflects the by-ref/non-escaping
                    // proof, not the outer binding's MUTness — syntaxcheck judges
                    // assignments to captures at the lambda site (as
                    // TYPE_LAMBDA_CAPTURE_UNSUPPORTED when escaping), so leave
                    // the capture's mutability unknown here.
                    if !matches!(value, Some(IrValue::Capture { .. })) {
                        muts.insert(name.clone(), *mutable);
                    }
                    if name.starts_with('$') {
                        if let Some(value) = value {
                            temp_consts.insert(name.as_str(), value);
                        }
                    }
                }
                IrOp::Assign { name, value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    // Synthesized `$`-temp targets are not user assignments —
                    // but an assign into the RECOVER slot (`$trap_val*`) is the
                    // lowered RECOVER value, which must match the trapped
                    // expression's success type (TYPE_RECOVER_TYPE_MISMATCH).
                    if name.starts_with('$') {
                        if name.starts_with("$trap_val") {
                            if let (Some(expected), Some(actual)) =
                                (locals.get(name), self.infer_type(value, locals))
                            {
                                // Both sides are stripped of any ` STATE T`
                                // suffix, or neither: the slot's declared type
                                // and the value's inferred type carry the state
                                // in the same shape, so stripping only one made
                                // a stateful native resource compare unequal to
                                // itself (bug-372).
                                let expected = resource_base_type(expected);
                                let actual = resource_base_type(&actual).to_string();
                                if !expected.is_empty()
                                    && expected != "Unknown"
                                    && expected != "Nothing"
                                    && !self.expression_compatible(expected, &actual, value)
                                {
                                    self.emit(
                                        "TYPE_RECOVER_TYPE_MISMATCH",
                                        format!("RECOVER has type {actual}, expected {expected}."),
                                    );
                                }
                            }
                        }
                        continue;
                    }
                    if muts.get(name) == Some(&false) {
                        self.emit(
                            "TYPE_ASSIGN_REQUIRES_MUT",
                            format!("Binding `{name}` is immutable and cannot be assigned."),
                        );
                    }
                    if let Some(t) = locals.get(name).cloned() {
                        let before = self.diags.borrow().len();
                        self.check_literal_range(resource_base_type(&t), value);
                        let range_errored = self.diags.borrow().len() > before;
                        if !range_errored {
                            self.check_assignment_type(name, &t, value, locals);
                        }
                    }
                }
                IrOp::AssignGlobal { name, value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    if self.global_muts.get(name) == Some(&false) {
                        self.emit(
                            "TYPE_ASSIGN_REQUIRES_MUT",
                            format!("Binding `{name}` is immutable and cannot be assigned."),
                        );
                    }
                    if let Some(t) = self.globals.get(name).cloned() {
                        let before = self.diags.borrow().len();
                        self.check_literal_range(resource_base_type(&t), value);
                        let range_errored = self.diags.borrow().len() > before;
                        if !range_errored {
                            self.check_assignment_type(name, &t, value, locals);
                        }
                    }
                }
                IrOp::StateAssign {
                    resource, value, ..
                } => {
                    self.check_value_captures(value, closure_slots);
                    // The `WITH s.state { … }` target reads `s.state`; the arm below
                    // diagnoses a missing STATE for this statement precisely, so
                    // suppress the generic `.state`-read rule inside the value.
                    self.checking_state_assign.set(true);
                    self.check_value(value, locals);
                    self.checking_state_assign.set(false);
                    // `res.state = value` must match the declared `STATE T` type,
                    // carried in the local's type string (`File STATE T`); a
                    // resource declared without STATE has nothing to assign.
                    if let Some(t) = locals.get(resource) {
                        let declared_state = crate::builtins::resource::state_type_name(t);
                        if declared_state.is_none()
                            && self.is_resource_or_resource_union(resource_base_type(t))
                        {
                            self.emit(
                                "TYPE_STATE_INVALID",
                                format!(
                                    "`{resource}` has no STATE to assign; declare the resource with `STATE T`."
                                ),
                            );
                        }
                        if let Some(state_type) = declared_state.map(str::to_string) {
                            if let Some(actual) = self.infer_type(value, locals) {
                                if !self.expression_compatible(&state_type, &actual, value) {
                                    self.emit(
                                        "TYPE_ASSIGNMENT_MISMATCH",
                                        format!(
                                            "State assignment to `{resource}.state` has type {actual}, expected {state_type}."
                                        ),
                                    );
                                }
                            }
                        }
                    }
                }
                IrOp::Eval { value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    // Statement position: a value-less SUB call is legal here.
                    self.allow_sub_call.set(true);
                    self.check_value(value, locals);
                    self.allow_sub_call.set(false);
                }
                IrOp::ExitProgram { code: value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    // The exit code must be an Integer, and a constant code
                    // must fit the host's 0..255 exit-status range.
                    if let Some(actual) = self.infer_type(value, locals) {
                        if !self.expression_compatible("Integer", &actual, value) {
                            self.emit(
                                "TYPE_EXIT_PROGRAM_REQUIRES_INTEGER",
                                format!("EXIT PROGRAM code has type {actual}, expected Integer."),
                            );
                        }
                    }
                    if let Some(code) = integer_constant_value(value) {
                        if !(0..=255).contains(&code) {
                            self.emit(
                                "EXIT_PROGRAM_CODE_OUT_OF_RANGE",
                                "EXIT PROGRAM constant exit code must be in the host range 0..255."
                                    .to_string(),
                            );
                        }
                    }
                }
                IrOp::Fail { error: value, .. } => {
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    // `PROPAGATE` outside a TRAP lowers to `Fail(Local("$error"))`
                    // with the sentinel unbound; inside a trap the real error
                    // binding is used (syntaxcheck's TYPE_PROPAGATE_REQUIRES_TRAP).
                    if matches!(value, IrValue::Local(n) if n == "$error")
                        && !locals.contains_key("$error")
                    {
                        self.emit(
                            "TYPE_PROPAGATE_REQUIRES_TRAP",
                            "PROPAGATE is valid only inside a TRAP.".to_string(),
                        );
                    } else if let Some(actual) = self.infer_type(value, locals) {
                        // FAIL carries an Error (syntaxcheck's TYPE_FAIL_REQUIRES_ERROR).
                        if !self.compatible("Error", &actual) {
                            self.emit(
                                "TYPE_FAIL_REQUIRES_ERROR",
                                format!("FAIL has type {actual}, expected Error."),
                            );
                        }
                    }
                }
                IrOp::Return { value, .. } => {
                    // A SUB produces no value; lowering keeps a SUB's `RETURN
                    // <value>` so the rejection survives to the IR.
                    if value.is_some() && *self.current_kind.borrow() == "sub" {
                        self.emit(
                            "SUB_RETURN_FORBIDDEN",
                            "A SUB returns no value; use `EXIT SUB`.".to_string(),
                        );
                    }
                    if let Some(value) = value {
                        self.poisoned.set(false);
                        self.check_value_captures(value, closure_slots);
                        self.check_value(value, locals);
                        // Cascade: an erroneous RETURN value with no
                        // determinable type (see the Bind arm).
                        if self.value_type_poisoned(value, locals) {
                            self.emit(
                                "TYPE_UNKNOWN_VALUE",
                                "RETURN value does not have a known type.".to_string(),
                            );
                        }
                        // plan-59-E: returning a collection element used to be
                        // rejected here (`TYPE_RESOURCE_ELEMENT_NOT_OWNER`,
                        // retired). Under scope ownership the element is a pointer
                        // to the one resource and returning it hands that pointer
                        // to the caller, whose scope becomes the outermost one
                        // touching it.
                        // plan-59-C: returning a bare `RES` parameter under a
                        // CONCRETE `STATE` is the `launder` shape — an unprovable
                        // narrowing. The declared return names a STATE the checker
                        // cannot show the opaque value carries.
                        if self.is_opaque_state_value(value) {
                            let ret = self.current_return.borrow().clone();
                            if let Some(declared) = crate::builtins::resource::state_type_name(&ret)
                            {
                                self.emit(
                                    "TYPE_STATE_OPAQUE_NARROWING",
                                    format!(
                                        "RETURN declares `STATE {declared}`, but the returned value is a bare `RES` parameter whose STATE is opaque — it carries some state or none, and the compiler cannot prove it is a `{declared}`."
                                    ),
                                );
                            }
                        }
                        self.check_return_type(value, locals);
                        let ret = self.current_return.borrow().clone();
                        self.check_literal_range(&ret, value);
                    }
                }
                IrOp::ExitLoop { kind, .. } => {
                    if !self.loop_stack.borrow().iter().any(|k| k == kind) {
                        self.emit(
                            "EXIT_NO_MATCHING_LOOP",
                            format!(
                                "EXIT {} has no matching enclosing loop.",
                                loop_kind_keyword(*kind)
                            ),
                        );
                    }
                }
                IrOp::ContinueLoop { kind, .. } => {
                    if !self.loop_stack.borrow().iter().any(|k| k == kind) {
                        self.emit(
                            "CONTINUE_NO_MATCHING_LOOP",
                            format!(
                                "CONTINUE {} has no matching enclosing loop.",
                                loop_kind_keyword(*kind)
                            ),
                        );
                    }
                }
                IrOp::If {
                    condition,
                    then_body,
                    else_body,
                    ..
                } => {
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
                    self.check_condition_boolean("IF condition", condition, locals);
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.check_ops(
                        then_body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.check_ops(
                        else_body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                }
                IrOp::Match { value, cases, .. } => {
                    if cases.is_empty() {
                        self.emit(
                            VERIFY_MATCH,
                            "MATCH has no cases (not exhaustive)".to_string(),
                        );
                    }
                    self.check_value_captures(value, closure_slots);
                    self.check_value(value, locals);
                    self.check_match_exhaustive(value, cases, locals);
                    self.check_match_patterns(value, cases, locals);
                    self.current_line.set(line);
                    for case in cases {
                        match &case.pattern {
                            super::super::IrMatchPattern::Else => {}
                            // bug-297: the scrutinee is capture-checked above, but
                            // these pattern values were checked with `check_value`
                            // alone -- whose `Capture` arm is a no-op. An
                            // out-of-range `Capture` here passed verification and
                            // lowered to an env-relative load, an OOB read a
                            // crafted `.mfp` could steer.
                            super::super::IrMatchPattern::Value(v) => {
                                self.check_value_captures(v, closure_slots);
                                self.check_value(v, locals);
                            }
                            super::super::IrMatchPattern::OneOf(vs) => {
                                for v in vs {
                                    self.check_value_captures(v, closure_slots);
                                    self.check_value(v, locals);
                                }
                            }
                        }
                        let mut case_locals = locals.clone();
                        let mut case_muts = muts.clone();
                        if let Some(guard) = &case.guard {
                            // A guard may reference the leading union-extract
                            // binds; register those first (mirrors validate.rs).
                            for op in &case.body {
                                let IrOp::Bind { name, type_, .. } = op else {
                                    break;
                                };
                                case_locals.insert(name.clone(), type_.clone());
                            }
                            // bug-297: same omission as the pattern values above.
                            self.check_value_captures(guard, closure_slots);
                            self.check_value(guard, &case_locals);
                            self.current_line.set(case.loc.line);
                            self.check_condition_boolean("WHEN guard", guard, &case_locals);
                            self.current_line.set(line);
                            case_locals = locals.clone();
                        }
                        self.check_ops(
                            &case.body,
                            &mut case_locals,
                            &mut case_muts,
                            closure_slots,
                            depth + 1,
                        );
                        self.current_line.set(line);
                    }
                }
                IrOp::While {
                    kind,
                    condition,
                    body,
                    ..
                } => {
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
                    self.check_condition_boolean("WHILE condition", condition, locals);
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.loop_stack.borrow_mut().push(*kind);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    self.loop_stack.borrow_mut().pop();
                }
                IrOp::For {
                    name,
                    type_,
                    start,
                    end,
                    step,
                    body,
                    ..
                } => {
                    for value in [start, end, step] {
                        self.check_value_captures(value, closure_slots);
                        self.check_value(value, locals);
                    }
                    // The end/step values are bound to `$for` temps just before
                    // this op (the temp's own type is the promoted loop type,
                    // not the original expression's), so resolve each bound
                    // through `temp_consts` to judge the user's expression.
                    fn resolve<'v>(
                        v: &'v IrValue,
                        temp_consts: &HashMap<&str, &'v IrValue>,
                    ) -> Option<&'v IrValue> {
                        match v {
                            IrValue::Local(n) if n.starts_with('$') => {
                                temp_consts.get(n.as_str()).copied()
                            }
                            other => Some(other),
                        }
                    }
                    // A provably non-numeric bound cannot drive the counter.
                    for (label, bound) in [("start", start), ("end", end), ("step", step)] {
                        let Some(bound) = resolve(bound, &temp_consts) else {
                            continue;
                        };
                        let Some(actual) = self.infer_type(bound, locals) else {
                            continue;
                        };
                        // A local the lowering could not type carries the
                        // literal "Unknown" through the locals map — skip it
                        // like any other unreconstructable type.
                        if actual.is_empty() || actual == "Unknown" {
                            continue;
                        }
                        if !matches!(actual.as_str(), "Integer" | "Float" | "Byte" | "Fixed") {
                            self.emit(
                                "TYPE_FOR_REQUIRES_NUMERIC",
                                format!(
                                    "FOR loop {label} value has type {actual}, expected numeric."
                                ),
                            );
                        }
                    }
                    // A literal STEP of zero never advances the counter (a
                    // non-literal step is left alone, matching syntaxcheck).
                    if resolve(step, &temp_consts).is_some_and(numeric_literal_is_zero) {
                        self.emit(
                            "TYPE_FOR_STEP_ZERO",
                            "FOR loop STEP must not be zero.".to_string(),
                        );
                    }
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    branch.insert(name.clone(), type_.clone());
                    // The loop counter is immutable inside the body (syntaxcheck
                    // registers it `mutable: false`).
                    branch_muts.insert(name.clone(), false);
                    self.loop_stack.borrow_mut().push(crate::ast::LoopKind::For);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    self.loop_stack.borrow_mut().pop();
                }
                IrOp::DoUntil {
                    body, condition, ..
                } => {
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    self.loop_stack.borrow_mut().push(crate::ast::LoopKind::Do);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    self.loop_stack.borrow_mut().pop();
                    // The trailing condition is reported at the loop's own line.
                    self.current_line.set(line);
                    self.check_value_captures(condition, closure_slots);
                    self.check_value(condition, locals);
                    self.check_condition_boolean("LOOP UNTIL condition", condition, locals);
                }
                IrOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                    ..
                } => {
                    self.check_value_captures(iterable, closure_slots);
                    self.check_value(iterable, locals);
                    // Only a List or Map can be iterated. (`MapEntry OF …` does
                    // not match the `Map OF ` prefix.)
                    if let Some(actual) = self.infer_type(iterable, locals) {
                        let base = resource_base_type(&actual);
                        // A local the lowering could not type carries the
                        // literal "Unknown" through the locals map — skip it.
                        if !base.is_empty()
                            && base != "Unknown"
                            && !base.starts_with("List OF ")
                            && !base.starts_with("Map OF ")
                        {
                            self.emit(
                                "TYPE_FOR_EACH_REQUIRES_COLLECTION",
                                format!("FOR EACH source has type {actual}, expected List or Map."),
                            );
                        }
                    }
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    branch.insert(name.clone(), type_.clone());
                    // The element binding is an immutable, non-owning view.
                    branch_muts.insert(name.clone(), false);
                    self.loop_stack.borrow_mut().push(crate::ast::LoopKind::For);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    self.loop_stack.borrow_mut().pop();
                }
                IrOp::Trap { name, body, .. } => {
                    let mut branch = locals.clone();
                    let mut branch_muts = muts.clone();
                    branch.insert(name.clone(), "Error".to_string());
                    branch_muts.insert(name.clone(), false);
                    self.check_ops(
                        body,
                        &mut branch,
                        &mut branch_muts,
                        closure_slots,
                        depth + 1,
                    );
                    // A function-level TRAP block must leave the function on
                    // every path (syntaxcheck's TYPE_TRAP_FALLTHROUGH).
                    self.current_line.set(line);
                    if !self.block_always_returns(body, &branch) {
                        // A bare `TRAP` synthesizes a `#`-sentinel name the user
                        // never wrote; keep it out of diagnostics.
                        let trap_label = if name == crate::ast::SYNTHETIC_TRAP_BINDING {
                            "the TRAP handler".to_string()
                        } else {
                            format!("TRAP `{name}`")
                        };
                        self.emit(
                            "TYPE_TRAP_FALLTHROUGH",
                            format!("{trap_label} must return, fail, or propagate."),
                        );
                    }
                }
            }
        }
    }

    // ===========================================================================
}
