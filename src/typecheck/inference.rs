use super::*;
use super::helpers::*;

impl<'a> TypeChecker<'a> {
    pub(super) fn infer_expression(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        mode: ExprMode,
    ) -> Type {
        self.infer_expression_with_expected(file, expression, locals, line, None, mode)
    }

    pub(super) fn infer_expression_with_expected(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
        mode: ExprMode,
    ) -> Type {
        // A value-less `SUB` call is permitted only as the top expression of a
        // bare statement (or the inner call of an inline `TRAP` there). Consume
        // the permission here so it applies to this expression alone; nested
        // sub-expressions see it reset to false and reject `SUB` calls.
        let value_less_call_ok = self.allow_value_less_call;
        self.allow_value_less_call = false;
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
                } else if builtins::is_package_constant(&canonical_name) {
                    self.parse_type(
                        builtins::package_constant_type_name(&canonical_name).unwrap_or("Unknown"),
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
            Expression::Trapped {
                expression: inner,
                binding,
                handler,
                line: trap_line,
            } => {
                let trapped_callee = match inner.as_ref() {
                    Expression::Call { callee, .. } => {
                        Some(self.canonical_import_name(file, callee))
                    }
                    _ => None,
                };
                let fallible = match &trapped_callee {
                    Some(canonical) => !builtins::is_package_constant(canonical),
                    None => false,
                };
                // A failed `thread.send` returns ownership of the sent value to
                // the caller so the handler can release it. Capture it before
                // the call consumes it, then restore it into the handler scope.
                let send_failure_restore = self.thread_send_failure_restore(file, inner, locals);
                // A trapped `SUB` call as a bare statement is value-less too;
                // forward the permission to the inner call.
                self.allow_value_less_call = value_less_call_ok;
                let success_type =
                    self.infer_expression_with_expected(file, inner, locals, line, expected, mode);
                if !fallible {
                    self.report(
                        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                        "Inline TRAP requires a fallible call; this expression cannot fail.",
                        file,
                        *trap_line,
                    );
                }
                // An inline-lowered built-in (string/collection member, `bits::*`
                // op, or `len`/`toString`/`typeName`) has its code spliced in at
                // the call site and owns no callable symbol, so codegen's raw-TRAP
                // path cannot trap it — it would emit a `bl` to a missing symbol.
                // Reject it here with a located diagnostic and the workaround.
                // Report-and-continue so the rest of the expression still checks.
                if fallible {
                    if let Some(canonical) = &trapped_callee {
                        if builtins::inline_trap_unsupported(canonical) {
                            self.report(
                                "TYPE_INLINE_TRAP_ON_INLINED_BUILTIN",
                                &format!(
                                    "Inline TRAP is not supported on `{canonical}` (it is compiled inline). Move the call into a FUNC/SUB and TRAP on that call instead."
                                ),
                                file,
                                *trap_line,
                            );
                        }
                    }
                }
                let mut handler_locals = locals.clone();
                if let Some((name, info)) = send_failure_restore {
                    handler_locals.insert(name, info);
                }
                handler_locals.insert(
                    binding.clone(),
                    LocalInfo {
                        type_: Type::Error,
                        mutable: false,
                        ownership: OwnershipState::Available,
                        borrowed: false,
                        state_type: None,
                    },
                );
                self.inline_trap_types.push(success_type.clone());
                let current_return = self.current_return.clone();
                let handler_flow = self.check_block(
                    file,
                    handler,
                    &current_return,
                    &mut handler_locals,
                    Some(binding.as_str()),
                );
                self.inline_trap_types.pop();
                if handler_flow != Flow::AlwaysReturns {
                    self.report(
                        "TYPE_INLINE_TRAP_FALLS_THROUGH",
                        "Inline TRAP handler must end every path in RECOVER or a diverging statement (RETURN, FAIL, or PROPAGATE).",
                        file,
                        *trap_line,
                    );
                }
                success_type
            }
            Expression::Binary {
                left,
                operator,
                right,
                ..
            } => {
                let left_type = self.infer_expression(file, left, locals, line, ExprMode::Read);
                let right_type = self.infer_expression(file, right, locals, line, ExprMode::Read);
                self.infer_binary(file, operator, &left_type, &right_type, line)
            }
            Expression::Unary {
                operator, operand, ..
            } => {
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
            Expression::Call {
                callee, arguments, ..
            } => {
                let canonical_callee = self.canonical_import_name(file, callee);
                if builtins::is_package_constant(&canonical_callee) {
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
                        builtins::package_constant_type_name(&canonical_callee)
                            .unwrap_or("Unknown"),
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
                        expected,
                    );
                }

                if let Some(sig) = self
                    .lookup_visible_call_sig(file, callee, arguments, expected)
                    .cloned()
                    .or_else(|| {
                        self.lookup_visible_call_sig(file, &canonical_callee, arguments, expected)
                            .cloned()
                    })
                {
                    self.check_call(file, callee, &sig, arguments, locals, line);
                    if matches!(sig.kind, FunctionKind::Sub) && !value_less_call_ok {
                        self.report(
                            "TYPE_SUB_HAS_NO_VALUE",
                            &format!(
                                "SUB `{callee}` produces no value; its call is a statement, \
                                 not an expression."
                            ),
                            file,
                            line,
                        );
                    }
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
            Expression::Lambda {
                params,
                body,
                assign_target,
            } => self.infer_lambda(file, params, body, assign_target.as_deref(), locals, line),
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

    pub(super) fn infer_match_scrutinee(
        &mut self,
        file: &AstFile,
        expression: &Expression,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        // A call scrutinee auto-unwraps like every other call site (its `Ok`
        // value is matched). Local error handling now uses an inline `TRAP`
        // (§8.4); `MATCH` only matches enum/union/`Result` *values*. A
        // `Result`-typed value (a local or field) still infers to
        // `Type::Result(..)`, preserving `CASE Ok`/`CASE Error` matching.
        self.infer_expression(file, expression, locals, line, ExprMode::Read)
    }

    pub(super) fn thread_send_failure_restore(
        &self,
        file: &AstFile,
        expression: &Expression,
        locals: &HashMap<String, LocalInfo>,
    ) -> Option<(String, LocalInfo)> {
        let Expression::Call {
            callee, arguments, ..
        } = expression
        else {
            return None;
        };
        // Both `thread.send` and the resource-plane `thread.transfer` move on
        // success and return ownership to the sender on failure, so a `TRAP`
        // handler may use the binding again.
        let canonical = self.canonical_import_name(file, callee);
        if canonical != "thread.send" && canonical != "thread.transfer" {
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

    pub(super) fn check_match_pattern(
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
            MatchPattern::Union { type_name, binding } => {
                if matches!(type_name.as_str(), "Ok" | "Error" | "Err") {
                    // `Result`/`Ok` are internal: a user `MATCH` can never
                    // scrutinize a `Result`, so `CASE Ok`/`CASE Error` are not
                    // valid match arms. Failures are handled with inline `TRAP`.
                    self.report(
                        "TYPE_RESULT_NOT_MATCHABLE",
                        &format!(
                            "`CASE {type_name}` is not a valid match arm; \
                             handle failure with an inline `TRAP` instead."
                        ),
                        file,
                        line,
                    );
                    return;
                }
                match matched_type {
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
                                &format!(
                                    "CASE `{type_name}` is not a member of UNION `{union_name}`."
                                ),
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
                                borrowed: false,
                                state_type: None,
                            },
                        );
                    }
                    _ => self.report(
                        "TYPE_MATCH_PATTERN_MISMATCH",
                        &format!(
                            "CASE `{type_name}` requires a UNION value, got {}.",
                            self.type_name(matched_type)
                        ),
                        file,
                        line,
                    ),
                }
            }
        }
    }

    pub(super) fn match_case_name(&self, pattern: &MatchPattern) -> Option<String> {
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

    pub(super) fn match_is_exhaustive(&self, matched_type: &Type, covered_cases: &HashSet<String>) -> bool {
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

    pub(super) fn report_match_not_exhaustive(
        &mut self,
        file: &AstFile,
        line: usize,
        matched_type: &Type,
        covered_cases: &HashSet<String>,
    ) {
        let detail = match matched_type {
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

    pub(super) fn infer_list_literal(
        &mut self,
        file: &AstFile,
        values: &[Expression],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        expected: Option<&Type>,
    ) -> Type {
        if let Some(Type::List(expected_element)) = expected {
            if self.contains_thread(expected_element) {
                self.report_invalid_collection_element(file, line, "element", expected_element);
            }
            for value in values {
                let mode = self.collection_element_mode(value, locals);
                let actual = self.infer_expression_with_expected(
                    file,
                    value,
                    locals,
                    line,
                    Some(expected_element),
                    mode,
                );
                self.check_collection_resource_element(
                    file, line, "element", value, &actual, locals,
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
        let first_mode = self.collection_element_mode(first, locals);
        let element_type = self.infer_expression(file, first, locals, line, first_mode);
        if self.contains_thread(&element_type) {
            self.report_invalid_collection_element(file, line, "element", &element_type);
        }
        self.check_collection_resource_element(file, line, "element", first, &element_type, locals);
        for value in values.iter().skip(1) {
            let mode = self.collection_element_mode(value, locals);
            let actual = self.infer_expression(file, value, locals, line, mode);
            self.check_collection_resource_element(file, line, "element", value, &actual, locals);
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

    pub(super) fn infer_map_literal(
        &mut self,
        file: &AstFile,
        key_type: &str,
        value_type: &str,
        entries: &[(Expression, Expression)],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let key_type = self.parse_type(key_type);
        // The value may carry the `RES` ownership-axis marker (`Map OF K TO RES
        // File`, §15.6).
        let value_type = self.parse_collection_element_type(value_type);
        self.check_type_reference(file, &key_type, line);
        self.check_type_reference(file, strip_res(&value_type), line);
        if self.contains_resource_or_thread(&key_type) {
            self.report_invalid_collection_element(file, line, "key", &key_type);
        }
        self.require_comparable_type(file, line, "Map key type", &key_type);
        if self.contains_thread(&value_type) {
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
            let value_mode = self.collection_element_mode(value, locals);
            let actual_value = self.infer_expression(file, value, locals, line, value_mode);
            self.check_collection_resource_element(
                file,
                line,
                "value",
                value,
                &actual_value,
                locals,
            );
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

    pub(super) fn infer_constructor(
        &mut self,
        file: &AstFile,
        type_name: &str,
        arguments: &[ConstructorArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
        _expected: Option<&Type>,
    ) -> Type {
        // `Error` and `ErrorLoc` are read-only compiler/runtime-generated records.
        // Direct construction is rejected; user errors are created with the
        // `error(code, message)` built-in instead.
        if matches!(type_name, "Error" | "ErrorLoc") {
            self.report(
                "TYPE_READ_ONLY_RECORD_CONSTRUCTOR",
                &format!(
                    "`{type_name}` is a read-only built-in record and cannot be constructed; use `error(code, message)` to create an Error."
                ),
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
            return if type_name == "Error" {
                Type::Error
            } else {
                Type::ErrorLoc
            };
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

    pub(super) fn infer_with_update(
        &mut self,
        file: &AstFile,
        target: &Expression,
        updates: &[RecordUpdate],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        let target_type = self.infer_expression(file, target, locals, line, ExprMode::Transfer);
        if matches!(target_type, Type::Error | Type::ErrorLoc) {
            self.report(
                "TYPE_READ_ONLY_RECORD_UPDATE",
                &format!(
                    "`{}` is a read-only built-in record and cannot be updated.",
                    self.type_name(&target_type)
                ),
                file,
                line,
            );
            for update in updates {
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
            }
            return target_type;
        }
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

    pub(super) fn infer_member_access(
        &mut self,
        file: &AstFile,
        target: &Expression,
        member: &str,
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        // `s.state` on a `RES` binding/parameter yields its declared `STATE`
        // record. The owner and a borrower may both read it (and replace it via
        // `s.state = WITH s.state { ... }`).
        if member == "state" {
            if let Expression::Identifier(name) = target {
                if let Some(state) = locals.get(name).and_then(|info| info.state_type.clone()) {
                    if let Some(info) = locals.get(name).cloned() {
                        if !self.require_local_owned(file, line, name, &info) {
                            return Type::Unknown;
                        }
                    }
                    return self.parse_type(&state);
                }
            }
        }

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
        if let Type::Thread(..) = &target_type {
            if member == "result" {
                // The `t.result` field is removed: a worker outcome is retrieved
                // only through `thread::waitFor(t)`, which auto-unwraps the value
                // or auto-propagates the `Error` like any other call.
                self.report(
                    "TYPE_THREAD_RESULT_REMOVED",
                    "Thread values have no `result` field; use `thread::waitFor(t)` \
                     to retrieve the worker outcome.",
                    file,
                    line,
                );
                return Type::Unknown;
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
                "source" => Type::ErrorLoc,
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
        if matches!(target_type, Type::ErrorLoc) {
            return match member {
                "filename" => Type::String,
                "line" => Type::Integer,
                "char" => Type::Integer,
                _ => {
                    self.report(
                        "TYPE_UNKNOWN_FIELD",
                        &format!("ErrorLoc value has no field `{member}`."),
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
                .or_else(|| builtins::net::builtin_type_fields(&type_name))
                .or_else(|| builtins::term::builtin_type_fields(&type_name))
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

    pub(super) fn check_constructor_arguments(
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

    pub(super) fn infer_binary(
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
            // String is orderable: `<`, `>`, `<=`, `>=` compare two String operands
            // lexicographically by Unicode scalar value. Mixed String/numeric stays a
            // type error. Unknown is permissive on either side to avoid cascades.
            if self.is_orderable_string(left) && self.is_orderable_string(right) {
                return Type::Boolean;
            }
            self.report(
                "TYPE_BINARY_OPERATOR_MISMATCH",
                &format!(
                    "Operator `{operator}` requires numeric or String operands, got {} and {}.",
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

    pub(super) fn infer_unary(&mut self, file: &AstFile, operator: &str, operand: &Type, line: usize) -> Type {
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

    pub(super) fn check_call(
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
                self.call_argument_mode(callee, index, sig),
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
    }

    pub(super) fn check_function_value_call(
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

    pub(super) fn infer_lambda(
        &mut self,
        file: &AstFile,
        params: &[crate::ast::Param],
        body: &Expression,
        assign_target: Option<&str>,
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
                    borrowed: false,
                    state_type: None,
                },
            );
            param_types.push(type_);
        }
        // Consume the non-escaping callback licence so it applies only to this
        // lambda, never to a lambda nested inside its body.
        let nonescaping = self.nonescaping_callback;
        self.nonescaping_callback = false;
        let param_names = params
            .iter()
            .map(|param| param.name.clone())
            .collect::<HashSet<_>>();
        let mut captures = captured_locals(body, outer_locals, &param_names);
        // An assignment-bodied lambda mutates its target, so the target is a
        // capture too even when it never appears on the right-hand side (e.g.
        // `LAMBDA(x) -> total = x`). A target that is a lambda parameter is an
        // ordinary local, not a capture, and is rejected below as immutable.
        if let Some(target) = assign_target {
            if !param_names.contains(target)
                && !captures.iter().any(|capture| capture.name == target)
            {
                if let Some(local) = outer_locals.get(target) {
                    captures.push(CapturedLocal {
                        name: target.to_string(),
                        type_: local.type_.clone(),
                        mutable: local.mutable,
                    });
                }
            }
        }
        for capture in &captures {
            if capture.mutable && !nonescaping {
                // `MUT` capture is rejected by default: an ordinary closure would
                // observe a frozen copy, never the live binding. The
                // sole exception is a compiler-proven non-escaping callback
                // position, handled below.
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures mutable local `{}`; mutable captures are invalid.",
                        capture.name
                    ),
                    file,
                    line,
                );
            } else if capture.mutable && self.is_resource_type(&capture.type_) {
                // A non-escaping callback may borrow a `MUT` binding, but never a
                // resource: resource ownership rules are unchanged (§12.4).
                self.report(
                    "TYPE_LAMBDA_CAPTURE_UNSUPPORTED",
                    &format!(
                        "Lambda captures resource local `{}`; resource captures are invalid.",
                        capture.name
                    ),
                    file,
                    line,
                );
            } else if capture.mutable {
                // A permitted non-escaping `MUT` borrow: the binding is loaned to
                // the callback for the duration of the synchronous call and is the
                // outer binding's again once it returns.
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
        let return_type = match assign_target {
            Some(target) => {
                // `name = <body>`: validate the assignment the same way the
                // statement form does — the target must be a mutable binding and
                // the body type must match it — then yield `Nothing`.
                let target_type = match locals.get(target).cloned() {
                    Some(local) => {
                        if !local.mutable {
                            self.report(
                                "TYPE_ASSIGN_REQUIRES_MUT",
                                &format!("Binding `{target}` is immutable and cannot be assigned."),
                                file,
                                line,
                            );
                        }
                        Some(local.type_)
                    }
                    None => {
                        self.report(
                            "TYPE_UNKNOWN_VALUE",
                            &format!("Assignment target `{target}` is not a local binding."),
                            file,
                            line,
                        );
                        None
                    }
                };
                let actual =
                    self.infer_expression(file, body, &mut locals, line, ExprMode::Transfer);
                if let Some(target_type) = target_type {
                    let reported_range_error =
                        self.report_primitive_literal_range_error(&target_type, body, file, line);
                    if !reported_range_error
                        && !self.expression_compatible(&target_type, &actual, Some(body))
                    {
                        self.report(
                            "TYPE_ASSIGNMENT_MISMATCH",
                            &format!(
                                "Assignment to `{target}` has type {}, expected {}.",
                                self.type_name(&actual),
                                self.type_name(&target_type)
                            ),
                            file,
                            line,
                        );
                    }
                }
                Type::Nothing
            }
            None => self.infer_expression(file, body, &mut locals, line, ExprMode::Read),
        };
        Type::Function {
            params: param_types,
            return_type: Box::new(return_type),
            isolated: false,
        }
    }
}
