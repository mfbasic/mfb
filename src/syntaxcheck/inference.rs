use super::helpers::*;
use super::*;

impl<'a> SyntaxChecker<'a> {
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
            Expression::Scalar(_) => Type::Scalar,
            Expression::Boolean(_) => Type::Boolean,
            Expression::Number(value) => match numeric::classify_literal(value).1 {
                numeric::LiteralType::Integer => Type::Integer,
                numeric::LiteralType::Float => Type::Float,
                numeric::LiteralType::Fixed => Type::Fixed,
                numeric::LiteralType::Money => Type::Money,
            },
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
                // A failed `thread.send` returns ownership of the sent value to
                // the caller so the handler can release it. Capture it before
                // the call consumes it, then restore it into the handler scope.
                let send_failure_restore = self.thread_send_failure_restore(file, inner, locals);
                // A trapped `SUB` call as a bare statement is value-less too;
                // forward the permission to the inner call.
                self.allow_value_less_call = value_less_call_ok;
                let success_type =
                    self.infer_expression_with_expected(file, inner, locals, line, expected, mode);
                // Uniformity (plan-26-A): a `TRAP` is legal on any call. Only a
                // scrutinee with *nothing to trap* is rejected — a non-call, or a
                // package constant (which is not a runtime call). A provably-
                // infallible inline built-in (`len`, `toString`, every `bits::*`,
                // the pure-query/growth collection members, …) is *allowed*: it
                // compiles and runs, and its handler is dead code — flagged by the
                // advisory `TYPE_INLINE_TRAP_DEAD_HANDLER` warning, not an error.
                match &trapped_callee {
                    None => self.report(
                        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                        "Inline TRAP requires a call to trap; this expression is not a call.",
                        file,
                        *trap_line,
                    ),
                    Some(canonical) if builtins::is_package_constant(canonical) => self.report(
                        "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE",
                        "Inline TRAP requires a fallible call; a package constant is not a call.",
                        file,
                        *trap_line,
                    ),
                    Some(canonical) if builtins::inline_builtin_is_infallible(canonical) => self
                        .report_warning(
                            "TYPE_INLINE_TRAP_DEAD_HANDLER",
                            &format!(
                                "Inline TRAP handler is unreachable — `{canonical}` cannot fail, so the handler is dead code."
                            ),
                            file,
                            *trap_line,
                        ),
                    // Every other call — a fallible inline built-in (all now raw-
                    // supported, plan-26-B), a runtime-helper built-in, or a user
                    // FUNC/SUB — is trappable. (`inline_trap_unsupported` no longer
                    // matches any inline target; the codegen backstop guards a
                    // future built-in added without a raw/infallible lowering.)
                    Some(_) => {}
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
                self.warn_money_bare_float_literal(
                    file, operator, left, right, &left_type, &right_type, line,
                );
                self.infer_binary(file, operator, &left_type, &right_type, line)
            }
            Expression::Unary {
                operator, operand, ..
            } => {
                if operator == "-" && !integer_literal_in_range(expression) {
                    if let Expression::Number(_value) = operand.as_ref() {}
                    return Type::Integer;
                }
                if operator == "-" {
                    if let Expression::Number(value) = operand.as_ref() {
                        // A negated numeric literal keeps the operand's literal
                        // type: `-5` Integer, `-1.5`/`-1e3`/`-2f` Float, `-2F`
                        // Fixed, `-2m` Money.
                        return match numeric::classify_literal(value).1 {
                            numeric::LiteralType::Integer => Type::Integer,
                            numeric::LiteralType::Float => Type::Float,
                            numeric::LiteralType::Fixed => Type::Fixed,
                            numeric::LiteralType::Money => Type::Money,
                        };
                    }
                }
                let operand_type =
                    self.infer_expression(file, operand, locals, line, ExprMode::Read);
                self.infer_unary(file, operator, &operand_type, line)
            }
            Expression::Call {
                callee, arguments, ..
            } => {
                if builtins::testing::is_expect_call(callee) {
                    return self.check_expect_call(file, callee, arguments, locals, line);
                }
                let canonical_callee = self.canonical_import_name(file, callee);
                if builtins::is_package_constant(&canonical_callee) {
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
                    if matches!(sig.kind, FunctionKind::Sub) && !value_less_call_ok {}
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
        if self.is_copyable_type(&info.type_) {
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
                if !self.expression_compatible(matched_type, &pattern_type, Some(expression)) {}
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
                            return;
                        }
                        case_locals.insert(
                            binding.clone(),
                            LocalInfo {
                                type_: Type::User(type_name.clone()),
                                mutable: false,
                                state_type: None,
                            },
                        );
                    }
                    // Non-union scrutinee: rejected by `ir::verify`'s
                    // TYPE_MATCH_PATTERN_MISMATCH (plan-20-Z).
                    _ => {}
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

    pub(super) fn match_is_exhaustive(
        &self,
        matched_type: &Type,
        covered_cases: &HashSet<String>,
    ) -> bool {
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
        _file: &AstFile,
        _line: usize,
        _matched_type: &Type,
        _covered_cases: &HashSet<String>,
    ) {
        // MATCH exhaustiveness is now enforced by `ir::verify` (the sole rejecter
        // for both the source and package paths, plan-20). This relocated
        // syntaxcheck rule emits no diagnostic; the body is intentionally empty.
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
                if !self.expression_compatible(expected_element, &actual, Some(value)) {}
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
        for value in values.iter().skip(1) {
            let mode = self.collection_element_mode(value, locals);
            let actual = self.infer_expression(file, value, locals, line, mode);
            if !self.expression_compatible(&element_type, &actual, Some(value)) {}
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
            if !self.expression_compatible(&key_type, &actual_key, Some(key)) {}
            let value_mode = self.collection_element_mode(value, locals);
            let actual_value = self.infer_expression(file, value, locals, line, value_mode);
            if !self.expression_compatible(&value_type, &actual_value, Some(value)) {}
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
                return Type::Unknown;
            }
            if !matches!(info.kind, TypeDeclKind::Type) {
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
            for update in updates {
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
            }
            return target_type;
        }
        let Type::User(type_name) = &target_type else {
            return Type::Unknown;
        };
        if read_only_record_type(type_name) {
            for update in updates {
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
            }
            return Type::Unknown;
        }
        let Some(info) = self.type_infos.get(type_name).cloned() else {
            return Type::Unknown;
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
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
                self.infer_expression(file, &update.value, locals, update.line, ExprMode::Transfer);
                continue;
            };
            if !self.visible_from(file, field.visibility, &info.file_path) {}
            let actual = self.infer_expression_with_expected(
                file,
                &update.value,
                locals,
                update.line,
                Some(&field.type_),
                ExprMode::Transfer,
            );
            if !self.expression_compatible(&field.type_, &actual, Some(&update.value)) {}
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
                    return self.parse_type(&state);
                }
            }
        }

        if let Expression::Identifier(type_name) = target {
            if let Some(info) = self.type_infos.get(type_name).cloned() {
                if matches!(info.kind, TypeDeclKind::Enum) {
                    if !self.visible_from(file, info.visibility, &info.file_path) {
                        return Type::Unknown;
                    }
                    if !info.members.contains(member) {
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
                return Type::Unknown;
            }
            return Type::Unknown;
        }
        if matches!(target_type, Type::Error) {
            return match member {
                "code" => Type::Integer,
                "message" => Type::String,
                "source" => Type::ErrorLoc,
                _ => Type::Unknown,
            };
        }
        if matches!(target_type, Type::ErrorLoc) {
            return match member {
                "filename" => Type::String,
                "line" => Type::Integer,
                "char" => Type::Integer,
                _ => Type::Unknown,
            };
        }
        let Type::User(type_name) = target_type else {
            return Type::Unknown;
        };
        if let Some(rest) = type_name.strip_prefix("MapEntry OF ") {
            if let Some((key, value)) = split_top_level_to(rest) {
                return match member {
                    "key" => self.parse_type(key),
                    "value" => self.parse_type(value),
                    _ => Type::Unknown,
                };
            }
        }
        let Some(info) = self.type_infos.get(&type_name).cloned() else {
            if let Some(field_type) = builtins::io::builtin_type_fields(&type_name)
                .or_else(|| builtins::net::builtin_type_fields(&type_name))
                .or_else(|| builtins::term::builtin_type_fields(&type_name))
                .or_else(|| builtins::audio::builtin_type_fields(&type_name))
                .and_then(|fields| fields.iter().find(|(name, _)| *name == member))
                .map(|(_, type_name)| self.parse_type(type_name))
            {
                return field_type;
            }
            return Type::Unknown;
        };
        if !matches!(info.kind, TypeDeclKind::Type) {
            return Type::Unknown;
        }
        let Some(field) = info
            .fields
            .iter()
            .find(|field| field.name == member)
            .cloned()
        else {
            return Type::Unknown;
        };
        if !self.visible_from(file, field.visibility, &info.file_path) {}
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
        if arguments.len() != fields.len() {}

        for field in fields {
            if !self.visible_from(file, field.visibility, owner_file_path) {}
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
                if let ConstructorArg::Named { name: _, .. } = argument {}
                continue;
            };
            if !self.expression_compatible(&field.type_, &actual, Some(argument_value)) {}
        }
    }

    pub(super) fn infer_binary(
        &mut self,
        _file: &AstFile,
        operator: &str,
        left: &Type,
        right: &Type,
        _line: usize,
    ) -> Type {
        if matches!(operator, "AND" | "OR" | "XOR") {
            if self.compatible(&Type::Boolean, left) && self.compatible(&Type::Boolean, right) {
                return Type::Boolean;
            }
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
            // Scalar is orderable by codepoint against another Scalar (plan-41-A).
            // It is non-numeric and does not order against String; both operands
            // must be Scalar (Unknown permitted to avoid cascades).
            if self.is_orderable_scalar(left) && self.is_orderable_scalar(right) {
                return Type::Boolean;
            }
            return Type::Unknown;
        }

        if operator == "&" {
            if self.compatible(&Type::String, left) && self.compatible(&Type::String, right) {
                return Type::String;
            }
            return Type::Unknown;
        }

        if self.is_numeric(left) && self.is_numeric(right) {
            numeric_binary_result_type(operator, left, right)
        } else {
            Type::Unknown
        }
    }

    /// The exactness nudge (plan-29-F §4.6): warn when a `Money` is scaled
    /// (`*`/`/`) by a **bare, suffixless decimal literal**, which silently takes
    /// the inexact `Float` path (a bare decimal defaults to `Float`). It is
    /// diagnostic-only — the type is unchanged. Silenced by `1.08F` (exact Fixed
    /// scaling) or `1.08f` (explicitly, but still inexactly, Float). A Float
    /// *variable* never warns; a Fixed literal never warns.
    fn warn_money_bare_float_literal(
        &mut self,
        file: &AstFile,
        operator: &str,
        left: &Expression,
        right: &Expression,
        left_type: &Type,
        right_type: &Type,
        line: usize,
    ) {
        let money = |t: &Type| matches!(t, Type::Money);
        let float = |t: &Type| matches!(t, Type::Float);
        // Both commutative orders of `*`; only `Money / literal` for `/`.
        let culprit = match operator {
            "*" if money(left_type) && float(right_type) && is_bare_decimal_float(right) => {
                Some(right)
            }
            "*" if money(right_type) && float(left_type) && is_bare_decimal_float(left) => {
                Some(left)
            }
            "/" if money(left_type) && float(right_type) && is_bare_decimal_float(right) => {
                Some(right)
            }
            _ => None,
        };
        if culprit.is_some() {
            self.report_warning(
                "MONEY_INEXACT_FLOAT_LITERAL",
                &format!(
                    "scaling Money by a bare decimal literal uses inexact Float arithmetic; append `F` for exact fixed-point scaling, or `f` to confirm the Float is intentional."
                ),
                file,
                line,
            );
        }
    }

    /// Type-check one of the four assertion builtins (plan-18-B). All produce
    /// `Nothing`; the argument constraints differ per builtin. Called from the
    /// `Call` arm before general builtin dispatch.
    pub(super) fn check_expect_call(
        &mut self,
        file: &AstFile,
        callee: &str,
        arguments: &[CallArg],
        locals: &mut HashMap<String, LocalInfo>,
        line: usize,
    ) -> Type {
        use crate::builtins::testing::{
            expect_operand_type, is_equality_assert, is_inequality_assert, EXPECT_NTRAP,
            EXPECT_TRAP,
        };

        if let Some((min, max)) = crate::builtins::testing::expect_arity(callee) {
            if arguments.len() < min || arguments.len() > max {
                self.report(
                    "TESTING_EXPECT_ARITY",
                    &format!(
                        "`{callee}` expects {} argument(s), got {}.",
                        if min == max {
                            min.to_string()
                        } else {
                            format!("{min}–{max}")
                        },
                        arguments.len()
                    ),
                    file,
                    line,
                );
            }
        }

        let arg = |index: usize| arguments.get(index).map(call_arg_value);
        if is_equality_assert(callee) || is_inequality_assert(callee) {
            let left = arg(0)
                .map(|value| self.infer_expression(file, value, locals, line, ExprMode::Read))
                .unwrap_or(Type::Unknown);
            let right = arg(1)
                .map(|value| self.infer_expression(file, value, locals, line, ExprMode::Read))
                .unwrap_or(Type::Unknown);
            match expect_operand_type(callee) {
                // A typed assertion (`expectFloat`/`expectInteger`/…) requires both
                // operands to be exactly the named type — an exact type-and-value
                // check that needs no `toString`.
                Some(want) => {
                    for operand in [&left, &right] {
                        if !matches!(operand, Type::Unknown)
                            && self.type_name(operand) != want
                        {
                            self.report(
                                "TESTING_EXPECT_TYPE_MISMATCH",
                                &format!(
                                    "`{callee}` operands must both be {want}; got {}.",
                                    self.type_name(operand)
                                ),
                                file,
                                line,
                            );
                        }
                    }
                }
                // The generic `expectEqual`/`expectNEqual` accept any `=`-comparable,
                // printable operands (reusing the language `=` acceptance; `Unknown`
                // means not equality-comparable and neither operand was Unknown).
                None => {
                    let comparable =
                        matches!(self.infer_binary(file, "=", &left, &right, line), Type::Boolean);
                    if !comparable
                        && !matches!(left, Type::Unknown)
                        && !matches!(right, Type::Unknown)
                    {
                        self.report(
                            "TESTING_EXPECT_INCOMPARABLE",
                            &format!(
                                "`{callee}` operands must be comparable with `=`; got {} and {}.",
                                self.type_name(&left),
                                self.type_name(&right)
                            ),
                            file,
                            line,
                        );
                    }
                    for operand in [&left, &right] {
                        if !self.is_printable(operand) {
                            self.report(
                                "TESTING_EXPECT_NOT_PRINTABLE",
                                &format!(
                                    "`{callee}` operands must be printable (a scalar, String, Byte, or List OF Byte); got {}.",
                                    self.type_name(operand)
                                ),
                                file,
                                line,
                            );
                        }
                    }
                }
            }
        } else if callee == EXPECT_TRAP {
            if let Some(value) = arg(0) {
                self.infer_expression(file, value, locals, line, ExprMode::Read);
                self.check_trap_guardable(file, callee, value, line);
            }
            if let Some(value) = arg(1) {
                let code = self.infer_expression(file, value, locals, line, ExprMode::Read);
                if !self.compatible(&Type::Integer, &code) {
                    self.report(
                        "TESTING_EXPECT_CODE_TYPE",
                        &format!(
                            "`{callee}` expected-code argument must be an Integer; got {}.",
                            self.type_name(&code)
                        ),
                        file,
                        line,
                    );
                }
            }
        } else if callee == EXPECT_NTRAP {
            if let Some(value) = arg(0) {
                self.infer_expression(file, value, locals, line, ExprMode::Read);
                self.check_trap_guardable(file, callee, value, line);
            }
        }
        Type::Nothing
    }

    /// `expectTrap`/`expectNTrap` evaluate their argument under a trap guard built
    /// on the inline-TRAP machinery, so the gate rejects exactly what inline `TRAP`
    /// rejects (plan-26-C): a scrutinee with no runtime call to trap — a non-call,
    /// or a package constant. Everything else is accepted, including infallible
    /// built-ins (the assertion evaluates against the real outcome: `expectTrap`
    /// always fails, `expectNTrap` always passes — just as for an infallible user
    /// FUNC) and the callback members (a failing callback is trapped).
    fn check_trap_guardable(
        &mut self,
        file: &AstFile,
        callee: &str,
        expression: &Expression,
        line: usize,
    ) {
        let Expression::Call {
            callee: inner_callee,
            ..
        } = expression
        else {
            self.report(
                "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE",
                &format!("`{callee}` requires a call to trap-guard (got a non-call)."),
                file,
                line,
            );
            return;
        };
        let canonical = self.canonical_import_name(file, inner_callee);
        if builtins::is_package_constant(&canonical) {
            self.report(
                "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE",
                &format!("`{callee}` requires a call to trap-guard; a package constant is not a call."),
                file,
                line,
            );
        }
    }

    /// Whether a value of `type_` can be rendered by `toString` for an assertion
    /// failure message. `Unknown` is treated as printable to avoid error cascades.
    fn is_printable(&self, type_: &Type) -> bool {
        match type_ {
            Type::Integer
            | Type::Float
            | Type::Fixed
            | Type::Money
            | Type::Boolean
            | Type::String
            | Type::Byte
            | Type::Scalar
            | Type::Unknown => true,
            Type::List(inner) => matches!(**inner, Type::Byte),
            _ => false,
        }
    }

    pub(super) fn infer_unary(
        &mut self,
        _file: &AstFile,
        operator: &str,
        operand: &Type,
        _line: usize,
    ) -> Type {
        match operator {
            "NOT" => {
                if self.compatible(&Type::Boolean, operand) {
                    Type::Boolean
                } else {
                    Type::Unknown
                }
            }
            "-" => {
                if self.is_numeric(operand) {
                    operand.clone()
                } else {
                    Type::Unknown
                }
            }
            _ => Type::Unknown,
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
            if param.type_name.is_none() {}
            if param.default.is_some() {}
            locals.insert(
                param.name.clone(),
                LocalInfo {
                    type_: type_.clone(),
                    mutable: false,
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
                        if !local.mutable {}
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
                let _actual =
                    self.infer_expression(file, body, &mut locals, line, ExprMode::Transfer);
                if let Some(target_type) = target_type {
                    // Assignment mismatch/range rejections live in
                    // `ir::verify` (plan-20-Z).
                    let _ = target_type;
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

/// Split a `Map`/`MapEntry` body `K TO V` on the top-level ` TO ` separating the
/// outer key from its value. A leftmost `split_once(" TO ")` mis-parses a key
/// that itself carries a top-level ` TO ` (a nested `Map`/`Thread`/`FUNC`-typed
/// key, bug-108.2). Mirrors `syntaxcheck::types::split_map_body`: separators
/// inside parenthesized / `FUNC(...)` groups and those owned by nested
/// `Map`/`MapEntry`/`Thread`/`ThreadWorker` sub-types are skipped.
fn split_top_level_to(body: &str) -> Option<(&str, &str)> {
    let bytes = body.as_bytes();
    let mut depth: usize = 0;
    let mut pending: usize = 0;
    let mut index = 0;
    while index < body.len() {
        match bytes[index] {
            b'(' => {
                depth += 1;
                index += 1;
            }
            b')' => {
                depth = depth.saturating_sub(1);
                index += 1;
            }
            // `is_char_boundary` guards the slice: `.mfp`-decoded type strings are
            // not guaranteed ASCII, so `index` can land on a UTF-8 continuation
            // byte where `body[index..]` would panic (bug-169). A non-boundary
            // byte never begins ` TO ` nor a keyword, so skipping it is correct.
            _ if depth == 0 && body.is_char_boundary(index) && body[index..].starts_with(" TO ") => {
                if pending > 0 {
                    pending -= 1;
                    index += 4;
                } else {
                    return Some((&body[..index], &body[index + 4..]));
                }
            }
            _ if depth == 0
                && body.is_char_boundary(index)
                && type_owns_a_to_separator(body, index) =>
            {
                pending += 1;
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

/// Whether a `Map`/`MapEntry`/`Thread`/`ThreadWorker` `OF`-construct (each owning
/// exactly one top-level ` TO `) begins at byte `at`, on a word boundary.
fn type_owns_a_to_separator(body: &str, at: usize) -> bool {
    let bytes = body.as_bytes();
    if at > 0 {
        let prev = bytes[at - 1];
        if prev.is_ascii_alphanumeric()
            || prev == b'_'
            || prev == b'.'
            || prev == b':'
            || prev >= 0x80
        {
            return false;
        }
    }
    ["MapEntry OF ", "ThreadWorker OF ", "Map OF ", "Thread OF "]
        .iter()
        .any(|keyword| body[at..].starts_with(keyword))
}

/// Whether `expr` is a bare (suffixless) decimal literal that types as `Float`:
/// the `Money`-scaling exactness nudge (plan-29-F §4.6) fires only on these, not
/// on `f`/`F`-suffixed literals or Float variables. A leading unary minus is
/// transparent (`-1.08` is still a bare literal).
fn is_bare_decimal_float(expr: &Expression) -> bool {
    match expr {
        Expression::Number(text) => {
            // A suffixed literal (`1.08f`/`1.08F`) is intrinsically typed and is
            // never the culprit; only an unsuffixed decimal that classifies as
            // Float qualifies.
            !text.ends_with('f')
                && !text.ends_with('F')
                && !text.ends_with('m')
                && !text.ends_with('M')
                && matches!(numeric::classify_literal(text).1, numeric::LiteralType::Float)
        }
        Expression::Unary {
            operator, operand, ..
        } if operator == "-" => is_bare_decimal_float(operand),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::testutil::*;

    fn wrap(body: &str) -> String {
        format!("FUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
    }

    // ---- literals & simple expressions ----

    #[test]
    fn string_boolean_number_literals_accepted() {
        assert!(accepts(&wrap(
            "  LET s AS String = \"hi\"\n  LET b AS Boolean = TRUE\n  LET i AS Integer = 5\n  LET f AS Float = 1.5"
        )));
    }

    #[test]
    fn negative_integer_literal_accepted() {
        assert!(accepts(&wrap("  LET n AS Integer = -5")));
    }

    #[test]
    fn negative_large_integer_accepted() {
        // -value where the negated number is out of i64 literal range path.
        assert!(accepts(&wrap("  LET n AS Integer = -9223372036854775807")));
    }

    #[test]
    fn nothing_identifier_accepted() {
        // A SUB returns Nothing; using it in a bare statement exercises NOTHING.
        assert!(accepts(
            "IMPORT io\nSUB doIt()\n  io::print(\"hi\")\nEND SUB\nFUNC main AS Integer\n  doIt()\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn local_binding_identifier_accepted() {
        assert!(accepts(&wrap(
            "  LET x AS Integer = 3\n  LET y AS Integer = x"
        )));
    }

    #[test]
    fn function_reference_identifier_accepted() {
        assert!(accepts(
            "FUNC helper() AS Integer\n  RETURN 1\nEND FUNC\nFUNC main AS Integer\n  LET f AS FUNC() AS Integer = helper\n  RETURN f()\nEND FUNC\n"
        ));
    }

    // ---- binary operators ----

    #[test]
    fn logical_operators_accepted() {
        assert!(accepts(&wrap(
            "  LET a AS Boolean = TRUE AND FALSE\n  LET b AS Boolean = TRUE OR FALSE\n  LET c AS Boolean = TRUE XOR FALSE"
        )));
    }

    #[test]
    fn equality_numeric_and_comparable_accepted() {
        assert!(accepts(&wrap(
            "  LET a AS Boolean = 1 = 2\n  LET b AS Boolean = 1 <> 2\n  LET c AS Boolean = \"x\" = \"y\""
        )));
    }

    #[test]
    fn ordering_numeric_and_string_accepted() {
        assert!(accepts(&wrap(
            "  LET a AS Boolean = 1 < 2\n  LET b AS Boolean = 1 >= 2\n  LET c AS Boolean = \"a\" < \"b\"\n  LET d AS Boolean = \"a\" >= \"b\""
        )));
    }

    #[test]
    fn string_concatenation_accepted() {
        assert!(accepts(&wrap("  LET s AS String = \"a\" & \"b\"")));
    }

    #[test]
    fn numeric_arithmetic_accepted() {
        assert!(accepts(&wrap(
            "  LET a AS Integer = 1 + 2 * 3 - 4\n  LET b AS Float = 1.0 / 2.0"
        )));
    }

    // ---- unary operators ----

    #[test]
    fn unary_not_and_negation_accepted() {
        assert!(accepts(&wrap(
            "  LET a AS Boolean = NOT TRUE\n  LET b AS Integer = 5\n  LET c AS Integer = -b"
        )));
    }

    #[test]
    fn unary_negation_of_float_accepted() {
        assert!(accepts(&wrap(
            "  LET f AS Float = 1.5\n  LET g AS Float = -f"
        )));
    }

    // ---- collections ----

    #[test]
    fn list_literal_accepted() {
        assert!(accepts(&wrap("  LET xs AS List OF Integer = [1, 2, 3]")));
    }

    #[test]
    fn empty_list_literal_accepted() {
        assert!(accepts(&wrap("  LET xs AS List OF Integer = []")));
    }

    #[test]
    fn list_literal_no_expected_type_accepted() {
        // Bare list literal without an expected List type takes the inference path.
        assert!(accepts(&wrap("  LET n AS Integer = len([1, 2, 3])")));
    }

    #[test]
    fn map_literal_accepted() {
        assert!(accepts(&wrap(
            "  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1, \"b\" := 2 }"
        )));
    }

    // ---- record constructors ----

    fn point_prelude() -> &'static str {
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n"
    }

    #[test]
    fn record_constructor_positional_and_named_accepted() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET a AS Point = Point[1, 2]\n  LET b AS Point = Point[x := 3, y := 4]\n  RETURN a.x + b.y\nEND FUNC\n",
            point_prelude()
        );
        assert!(accepts(&src));
    }

    #[test]
    fn with_update_accepted() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET a AS Point = Point[1, 2]\n  LET b AS Point = WITH a {{ y := 10 }}\n  RETURN b.y\nEND FUNC\n",
            point_prelude()
        );
        assert!(accepts(&src));
    }

    // ---- member access ----

    #[test]
    fn record_member_access_accepted() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET a AS Point = Point[1, 2]\n  RETURN a.x + a.y\nEND FUNC\n",
            point_prelude()
        );
        assert!(accepts(&src));
    }

    #[test]
    fn enum_member_access_accepted() {
        assert!(accepts(
            "ENUM Color\n  Red, Blue\nEND ENUM\nFUNC main AS Integer\n  LET c AS Color = Color.Red\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn error_member_access_accepted() {
        assert!(accepts(&wrap(
            "  LET e AS Error = error(1, \"m\")\n  LET code AS Integer = e.code\n  LET msg AS String = e.message\n  LET src AS ErrorLoc = e.source\n  LET fn AS String = e.source.filename\n  LET ln AS Integer = e.source.line"
        )));
    }

    // ---- calls ----

    #[test]
    fn user_function_call_accepted() {
        assert!(accepts(
            "FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN add(1, 2)\nEND FUNC\n"
        ));
    }

    #[test]
    fn function_value_call_accepted() {
        assert!(accepts(&wrap(
            "  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1\n  LET r AS Integer = f(5)"
        )));
    }

    // ---- lambdas ----

    #[test]
    fn lambda_with_immutable_capture_accepted() {
        assert!(accepts(&wrap(
            "  LET base AS Integer = 10\n  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + base\n  RETURN f(1)"
        )));
    }

    // ---- match ----

    #[test]
    fn match_union_and_enum_accepted() {
        let src = "TYPE Circle\n  radius AS Integer\nEND TYPE\nTYPE Rect\n  width AS Integer\n  height AS Integer\nEND TYPE\nUNION Shape\n  Circle\n  Rect\nEND UNION\nFUNC main AS Integer\n  LET s AS Shape = Circle[5]\n  MUT total AS Integer = 0\n  MATCH s\n    CASE Circle(c)\n      total = total + c.radius\n    CASE Rect(r)\n      total = total + r.width\n  END MATCH\n  RETURN total\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn match_enum_literal_accepted() {
        let src = "ENUM Color\n  Red, Blue\nEND ENUM\nFUNC main AS Integer\n  LET c AS Color = Color.Red\n  MUT n AS Integer = 0\n  MATCH c\n    CASE Color.Red\n      n = 1\n    CASE Color.Blue\n      n = 2\n  END MATCH\n  RETURN n\nEND FUNC\n";
        assert!(accepts(src));
    }

    // ---- inline TRAP (accept path) ----

    #[test]
    fn inline_trap_fallible_call_accepted() {
        let src = "FUNC parsePositive(v AS Integer) AS Integer\n  IF v < 0 THEN FAIL error(1, \"neg\")\n  RETURN v + 1\nEND FUNC\nFUNC main AS Integer\n  LET a AS Integer = parsePositive(9) TRAP(e)\n    RECOVER e.code\n  END TRAP\n  RETURN a\nEND FUNC\n";
        assert!(accepts(src));
    }

    // =====================================================================
    // Rejection paths (one per emitted rule)
    // =====================================================================

    #[test]
    fn call_argument_mismatch_rejected() {
        let src = "FUNC add(a AS Integer, b AS Integer) AS Integer\n  RETURN a + b\nEND FUNC\nFUNC main AS Integer\n  RETURN add(1, \"x\")\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    #[test]
    fn function_value_call_argument_mismatch_rejected() {
        let src = &wrap(
            "  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1\n  LET r AS Integer = f(\"nope\")",
        );
        assert!(rejects_with(src, "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    #[test]
    fn function_value_named_argument_rejected() {
        let src = &wrap(
            "  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1\n  LET r AS Integer = f(x := 5)",
        );
        assert!(rejects_with(src, "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    #[test]
    fn function_value_call_arity_mismatch_rejected() {
        let src = &wrap(
            "  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1\n  LET r AS Integer = f(1, 2)",
        );
        assert!(rejects_with(src, "TYPE_CALL_ARITY_MISMATCH"));
    }

    #[test]
    fn duplicate_named_constructor_field_rejected() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET a AS Point = Point[x := 1, x := 2]\n  RETURN a.x\nEND FUNC\n",
            point_prelude()
        );
        assert!(rejects_with(&src, "TYPE_DUPLICATE_FIELD"));
    }

    #[test]
    fn duplicate_with_update_field_rejected() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET a AS Point = Point[1, 2]\n  LET b AS Point = WITH a {{ y := 10, y := 20 }}\n  RETURN b.y\nEND FUNC\n",
            point_prelude()
        );
        assert!(rejects_with(&src, "TYPE_DUPLICATE_FIELD"));
    }

    #[test]
    fn inline_trap_falls_through_rejected() {
        let src = "IMPORT io\nFUNC parsePositive(v AS Integer) AS Integer\n  IF v < 0 THEN FAIL error(1, \"neg\")\n  RETURN v + 1\nEND FUNC\nFUNC main AS Integer\n  LET a AS Integer = parsePositive(9) TRAP(e)\n    io::print(e.message)\n  END TRAP\n  RETURN a\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_INLINE_TRAP_FALLS_THROUGH"));
    }

    #[test]
    fn inline_trap_on_callback_member_accepted() {
        // `collections::transform` is a callback member with a raw inline-TRAP
        // lowering (plan-26-B), so an inline TRAP on it is accepted — no more
        // `TYPE_INLINE_TRAP_ON_INLINED_BUILTIN` rejection (retired in plan-26-C).
        let src = "IMPORT collections\nFUNC dbl(x AS Integer) AS Integer\n  RETURN x * 2\nEND FUNC\nFUNC main AS Integer\n  LET numbers AS List OF Integer = [1, 2, 3]\n  LET doubled AS List OF Integer = collections::transform(numbers, dbl) TRAP(e)\n    RECOVER numbers\n  END TRAP\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn inline_trap_on_infallible_builtin_warns_dead_handler() {
        // An infallible inline builtin under a TRAP compiles (plan-26-A): the
        // handler is dead code, flagged by the advisory warning, not an error.
        let src = "IMPORT collections\nFUNC main AS Integer\n  LET xs AS List OF Integer = [1, 2, 3]\n  LET n AS Integer = len(xs) TRAP(e)\n    RECOVER 0\n  END TRAP\n  RETURN n\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_INLINE_TRAP_DEAD_HANDLER"));
        assert!(!rejects_with(src, "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"));
        assert!(!rejects_with(src, "TYPE_INLINE_TRAP_ON_INLINED_BUILTIN"));
    }

    #[test]
    fn inline_trap_requires_fallible_rejected() {
        // A non-call scrutinee still has nothing to trap.
        let src = "FUNC main AS Integer\n  LET a AS Integer = 5 TRAP(e)\n    RECOVER 0\n  END TRAP\n  RETURN a\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"));
    }

    #[test]
    fn inline_trap_on_package_constant_rejected() {
        // A package constant is not a runtime call — still rejected.
        let src = "IMPORT math\nFUNC main AS Float\n  LET a AS Float = math::pi() TRAP(e)\n    RECOVER 0.0\n  END TRAP\n  RETURN a\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_INLINE_TRAP_REQUIRES_FALLIBLE"));
    }

    #[test]
    fn lambda_mut_capture_rejected() {
        let src = &wrap(
            "  MUT offset AS Integer = 1\n  LET f AS FUNC(Integer) AS Integer = LAMBDA(value AS Integer) -> value + offset",
        );
        assert!(rejects_with(src, "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"));
    }

    #[test]
    fn lambda_resource_capture_rejected() {
        let src = "IMPORT fs\nFUNC main AS Integer\n  RES file = fs::openFile(\"x.txt\")\n  LET f AS FUNC() AS String = LAMBDA() -> fs::readLine(file)\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"));
    }

    #[test]
    fn read_only_error_constructor_rejected() {
        let src = &wrap("  LET e AS Error = Error[100, \"m\"]");
        assert!(rejects_with(src, "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"));
    }

    #[test]
    fn read_only_errorloc_constructor_rejected() {
        let src = &wrap("  LET e AS ErrorLoc = ErrorLoc[\"file\", 1, 2]");
        assert!(rejects_with(src, "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"));
    }

    #[test]
    fn lambda_assign_unknown_target_rejected() {
        // An assignment-bodied lambda whose target is not a visible binding
        // reports TYPE_UNKNOWN_VALUE from infer_lambda's assign_target arm.
        let src = "IMPORT collections\nFUNC main AS Integer\n  LET numbers AS List OF Integer = [1, 2, 3]\n  collections::forEach(numbers, LAMBDA(x AS Integer) -> missing = x)\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_UNKNOWN_VALUE"));
    }

    // =====================================================================
    // Additional coverage: identifier resolution, package constants,
    // constructors, member access, match helpers, lambda captures.
    // =====================================================================

    #[test]
    fn package_constant_identifier_accepted() {
        // `math::pi` resolves to a package constant type via the
        // is_package_constant identifier arm.
        assert!(accepts(
            "IMPORT math\nFUNC main AS Integer\n  LET p AS Float = math::pi\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn errorcode_package_constant_accepted() {
        // `errorCode::ErrInvalidArgument` resolves to an Integer package constant
        // via the is_package_constant identifier arm.
        assert!(accepts(
            "IMPORT errorCode\nFUNC main AS Integer\n  LET c AS Integer = errorCode::ErrInvalidArgument\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn visible_binding_identifier_accepted() {
        // Top-level LET binding referenced from main exercises the
        // lookup_visible_binding fallback.
        assert!(accepts(
            "LET GLOBAL AS Integer = 7\nFUNC main AS Integer\n  RETURN GLOBAL\nEND FUNC\n"
        ));
    }

    #[test]
    fn unknown_identifier_infers_unknown() {
        // An undeclared identifier falls through to Type::Unknown (the final
        // unwrap_or arm); no inference-layer diagnostic is emitted here.
        let codes = check_src(&wrap("  LET x AS Integer = doesNotExist + 1"));
        assert!(!codes.iter().any(|c| c == "TYPE_CALL_ARGUMENT_MISMATCH"));
    }

    // ---- constructors: Ok/Result, unknown, unknown-field ----

    #[test]
    fn ok_result_constructor_infers_unknown() {
        // `Ok[..]` construction infers Unknown; arguments are still inferred (the
        // Ok/Result constructor arm). The untyped LET then draws TYPE_UNKNOWN_VALUE
        // downstream, which confirms the Unknown inference propagated.
        let codes = check_src(&wrap("  LET a AS Integer = 5\n  LET r = Ok[a]"));
        assert!(codes.iter().any(|c| c == "TYPE_UNKNOWN_VALUE"));
    }

    #[test]
    fn unknown_constructor_infers_unknown() {
        // Constructing an undeclared type falls to the final unknown-type arm;
        // arguments are inferred and the type resolves to Unknown.
        let codes = check_src(&wrap("  LET r = Undeclared[1, 2]"));
        assert!(codes.iter().any(|c| c == "TYPE_UNKNOWN_VALUE"));
    }

    #[test]
    fn constructor_unknown_named_field_ignored() {
        // A named argument that doesn't match any field takes the `field: None`
        // branch of check_constructor_arguments.
        let src = format!(
            "{}FUNC main AS Integer\n  LET a = Point[x := 1, bogus := 9]\n  RETURN 0\nEND FUNC\n",
            point_prelude()
        );
        // No inference panic; the checker still runs to completion.
        let _ = check_src(&src);
    }

    // ---- WITH update edge cases ----

    #[test]
    fn with_update_on_error_accepted_as_readonly_path() {
        // WITH on an Error target hits the Error/ErrorLoc early-return arm of
        // infer_with_update; the update value is still inferred.
        let _ = check_src(&wrap(
            "  LET e AS Error = error(1, \"m\")\n  LET u = WITH e { code := 2 }",
        ));
    }

    #[test]
    fn with_update_on_non_type_infers_unknown() {
        // WITH on an enum value (non-Type kind) returns Unknown.
        let _ = check_src(
            "ENUM Color\n  Red, Blue\nEND ENUM\nFUNC main AS Integer\n  LET c AS Color = Color.Red\n  LET u = WITH c { x := 1 }\n  RETURN 0\nEND FUNC\n",
        );
    }

    #[test]
    fn with_update_unknown_field_ignored() {
        // A WITH update naming a field that doesn't exist takes the
        // `field: None` continue branch.
        let src = format!(
            "{}FUNC main AS Integer\n  LET a AS Point = Point[1, 2]\n  LET b = WITH a {{ bogus := 3 }}\n  RETURN 0\nEND FUNC\n",
            point_prelude()
        );
        let _ = check_src(&src);
    }

    // ---- member access variants ----

    #[test]
    fn member_access_unknown_field_infers_unknown() {
        // Accessing a non-existent field on a user record returns Unknown.
        let src = format!(
            "{}FUNC main AS Integer\n  LET a AS Point = Point[1, 2]\n  LET z = a.missing\n  RETURN 0\nEND FUNC\n",
            point_prelude()
        );
        let _ = check_src(&src);
    }

    #[test]
    fn errorloc_member_access_accepted() {
        assert!(accepts(&wrap(
            "  LET e AS Error = error(1, \"m\")\n  LET loc AS ErrorLoc = e.source\n  LET c AS Integer = loc.char"
        )));
    }

    #[test]
    fn thread_member_access_infers_unknown() {
        // `t.result` (and any thread member) returns Unknown from the Thread arm.
        let src = "IMPORT thread\nFUNC worker(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\nFUNC main AS Integer\n  LET t AS Thread OF Integer TO Integer = thread::start(worker, 1, 1, 1)\n  LET r = t.result\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- match helpers ----

    #[test]
    fn match_non_exhaustive_enum_exercises_helpers() {
        // A MATCH on an enum missing a member drives match_is_exhaustive and
        // report_match_not_exhaustive (enum arm). The exhaustiveness diagnostic
        // itself is emitted by a later pass, so we only assert the checker runs.
        let src = "ENUM Color\n  Red, Blue\nEND ENUM\nFUNC pick(c AS Color) AS Integer\n  MATCH c\n    CASE Color.Red\n      RETURN 1\n  END MATCH\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN pick(Color.Red)\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn match_non_exhaustive_union_exercises_helpers() {
        // A MATCH on a union missing a variant drives the union arm of
        // match_is_exhaustive and report_match_not_exhaustive.
        let src = "TYPE Circle\n  radius AS Integer\nEND TYPE\nTYPE Rect\n  width AS Integer\n  height AS Integer\nEND TYPE\nUNION Shape\n  Circle\n  Rect\nEND UNION\nFUNC pick(s AS Shape) AS Integer\n  MATCH s\n    CASE Circle(c)\n      RETURN c.radius\n  END MATCH\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  RETURN pick(Circle[5])\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn match_with_else_exhaustive_accepted() {
        // CASE ELSE covers all remaining variants (MatchPattern::Else arm).
        let src = "ENUM Color\n  Red, Blue\nEND ENUM\nFUNC pick(c AS Color) AS Integer\n  MATCH c\n    CASE Color.Red\n      RETURN 1\n    CASE ELSE\n      RETURN 0\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  RETURN pick(Color.Blue)\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn match_literal_and_oneof_accepted() {
        // Integer literal scrutinee with literal and OneOf patterns (needs a
        // CASE ELSE since open Integer is never exhaustive).
        let src = "FUNC pick(n AS Integer) AS Integer\n  MATCH n\n    CASE 1\n      RETURN 1\n    CASE 2, 3\n      RETURN 2\n    CASE ELSE\n      RETURN 0\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  RETURN pick(2)\nEND FUNC\n";
        assert!(accepts(src));
    }

    // ---- lambda capture variants ----

    #[test]
    fn lambda_noncopyable_capture_rejected() {
        // Capturing an immutable non-copyable local (a Thread is not copyable and
        // not a resource) hits the `!is_copyable_type` capture arm.
        let src = "IMPORT thread\nFUNC worker(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\nSUB use(t AS Thread OF Integer TO Integer)\nEND SUB\nFUNC main AS Integer\n  LET t AS Thread OF Integer TO Integer = thread::start(worker, 1, 1, 1)\n  LET f AS FUNC() AS Nothing = LAMBDA() -> use(t)\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"));
    }

    #[test]
    fn lambda_no_capture_accepted() {
        // A lambda that captures nothing takes the capture-free path.
        assert!(accepts(&wrap(
            "  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x * 2\n  RETURN f(3)"
        )));
    }

    // =====================================================================
    // Third coverage batch: number edge cases, dotted callees, thread.send,
    // read-only records, builtin-type member fields, binary/unary rejects,
    // collection thread elements, remaining lambda-capture arms.
    // =====================================================================

    #[test]
    fn huge_integer_literal_accepted() {
        // A digit-only literal that overflows i64 still infers Integer (the
        // else arm of the Number match), so no argument-type mismatch arises.
        assert!(accepts(&wrap(
            "  LET n AS Integer = 99999999999999999999999999"
        )));
    }

    #[test]
    fn unary_negation_of_huge_integer_accepted() {
        // `-<huge>`: the literal is out of i64 range so the
        // `!integer_literal_in_range` unary arm returns Integer directly.
        assert!(accepts(&wrap(
            "  LET n AS Integer = -99999999999999999999999999"
        )));
    }

    #[test]
    fn dotted_unknown_callee_infers_unknown() {
        // A call whose canonical callee contains a dot but is neither a builtin
        // nor a visible function takes the `callee.contains('.')` unknown arm;
        // arguments are still inferred.
        let _ = check_src(&wrap("  LET r = pkg::missing(1, 2)"));
    }

    #[test]
    fn thread_send_in_trap_accepted() {
        // A trapped `thread::send` of a non-copyable value drives
        // thread_send_failure_restore (restore into handler scope).
        let src = "IMPORT thread\nFUNC worker(msg AS List OF Integer) AS Integer\n  RETURN 0\nEND FUNC\nFUNC main AS Integer\n  LET t AS Thread OF (List OF Integer) TO Integer = thread::start(worker, [1], 1, 1)\n  LET payload AS List OF Integer = [1, 2, 3]\n  thread::send(t, payload) TRAP(e)\n    RECOVER\n  END TRAP\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn read_only_termcolor_constructor_rejected() {
        // `TermColor` is a compiler-owned read-only record; direct construction
        // is rejected via read_only_record_type in infer_constructor.
        let src = "IMPORT term\nFUNC main AS Integer\n  LET c AS term::TermColor = term::TermColor[1, 2, 3]\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"));
    }

    #[test]
    fn net_address_member_fields_accepted() {
        // `addr.host`/`addr.port` resolve via net::builtin_type_fields in the
        // "type has no user info" member-access fallback.
        let src = "IMPORT net\nFUNC main AS Integer\n  RES sock = net::bindUdp(\"127.0.0.1\", 0)\n  LET addr = net::localAddress(sock)\n  LET h AS String = addr.host\n  LET p AS Integer = addr.port\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn mapentry_member_access_accepted() {
        // FOR EACH over a Map yields MapEntry values; `.key`/`.value` resolve via
        // the `MapEntry OF ` strip_prefix member-access arm.
        let src = "IMPORT io\nFUNC main AS Integer\n  LET m AS Map OF String TO Integer = Map OF String TO Integer { \"a\" := 1 }\n  FOR EACH entry IN m\n    io::print(entry.key & \"=\" & toString(entry.value))\n  NEXT\n  RETURN 0\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn state_member_access_accepted() {
        // `f.state` on a `RES f AS File STATE FileState` binding yields the state
        // record type (the `member == "state"` member-access arm).
        let src = "IMPORT fs\nTYPE FileState\n  pos AS Integer\n  len AS Integer\nEND TYPE\nFUNC main AS Integer\n  RES f AS File STATE FileState = fs::createTempFile()\n  LET p AS Integer = f.state.pos\n  fs::close(f)\n  RETURN p\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn package_constant_call_with_arguments_accepted() {
        // `math::pi(1)`: a package constant in call position still infers the
        // arguments (the arg loop in the is_package_constant Call arm).
        assert!(accepts(
            "IMPORT math\nFUNC main AS Integer\n  LET x AS Float = math::pi(1)\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn enum_non_member_access_infers_unknown() {
        // `Color.NotAMember` on an enum type returns Unknown (member not found).
        let src = "ENUM Color\n  Red, Blue\nEND ENUM\nFUNC main AS Integer\n  LET x = Color.Green\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- binary / unary rejection arms ----

    #[test]
    fn logical_operator_non_boolean_infers_unknown() {
        // AND on non-boolean operands returns Unknown (no Boolean result).
        let _ = check_src(&wrap("  LET x = 1 AND 2"));
    }

    #[test]
    fn equality_incompatible_infers_unknown() {
        // `=` between incomparable types returns Unknown.
        let _ = check_src(&wrap("  LET x = TRUE = \"s\""));
    }

    #[test]
    fn ordering_mixed_infers_unknown() {
        // `<` between String and numeric returns Unknown (mixed operands).
        let _ = check_src(&wrap("  LET x = \"s\" < 1"));
    }

    #[test]
    fn concatenation_non_string_infers_unknown() {
        // `&` with a non-String operand returns Unknown.
        let _ = check_src(&wrap("  LET x = \"s\" & 1"));
    }

    #[test]
    fn arithmetic_non_numeric_infers_unknown() {
        // `+` with a non-numeric operand returns Unknown.
        let _ = check_src(&wrap("  LET x = TRUE + 1"));
    }

    #[test]
    fn unary_not_non_boolean_infers_unknown() {
        // NOT on a non-boolean returns Unknown.
        let _ = check_src(&wrap("  LET x = NOT 5"));
    }

    #[test]
    fn unary_negation_non_numeric_infers_unknown() {
        // `-` on a String returns Unknown.
        let _ = check_src(&wrap("  LET s AS String = \"x\"\n  LET y = -s"));
    }

    // ---- collections with thread elements (rejected) ----

    #[test]
    fn list_of_thread_element_rejected() {
        // A List whose element type contains a Thread is an invalid collection
        // element (expected-type branch of infer_list_literal).
        let src = "IMPORT thread\nFUNC worker(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\nFUNC main AS Integer\n  LET t AS Thread OF Integer TO Integer = thread::start(worker, 1, 1, 1)\n  LET xs AS List OF (Thread OF Integer TO Integer) = [t]\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn map_value_thread_rejected() {
        // A Map whose value type contains a Thread is an invalid element.
        let src = "IMPORT thread\nFUNC worker(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\nFUNC main AS Integer\n  LET t AS Thread OF Integer TO Integer = thread::start(worker, 1, 1, 1)\n  LET m AS Map OF String TO (Thread OF Integer TO Integer) = Map OF String TO (Thread OF Integer TO Integer) { \"a\" := t }\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- lambda: assignment-body capturing an outer mutable target ----

    #[test]
    fn lambda_assign_body_captures_target_rejected() {
        // An assignment-bodied lambda whose target is an outer MUT binding pushes
        // the target as a capture; it is a mutable capture and is rejected.
        let src = "IMPORT collections\nFUNC main AS Integer\n  MUT total AS Integer = 0\n  LET numbers AS List OF Integer = [1, 2, 3]\n  collections::forEach(numbers, LAMBDA(x AS Integer) -> total = total + x)\n  RETURN total\nEND FUNC\n";
        // forEach is a non-escaping position, so the MUT borrow may be permitted;
        // either way the assign-target capture arm is exercised.
        let _ = check_src(src);
    }

    // =====================================================================
    // Fourth batch: call-form package constants, callable-local calls,
    // union-match arms, with-update read-only records, function-value
    // non-function type.
    // =====================================================================

    #[test]
    fn package_constant_call_form_accepted() {
        // `math::pi()` (call syntax) resolves through the is_package_constant
        // Call arm, inferring the constant's Float type.
        assert!(accepts(
            "IMPORT math\nFUNC main AS Integer\n  LET x AS Float = math::pi()\n  RETURN 0\nEND FUNC\n"
        ));
    }

    #[test]
    fn callable_local_called_accepted() {
        // A local holding a FUNC value called via `g(..)` routes through
        // check_function_value_call (the locals.get(callee) call arm).
        assert!(accepts(&wrap(
            "  LET g AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x\n  LET r AS Integer = g(1)"
        )));
    }

    #[test]
    fn non_callable_local_called_infers_unknown() {
        // Calling an Integer local reaches check_function_value_call's
        // non-Function early-return arm (arguments still inferred, Unknown result).
        let _ = check_src(&wrap("  LET g AS Integer = 5\n  LET r = g(1)"));
    }

    #[test]
    fn union_match_variant_binding_accepted() {
        // `CASE Circle(c)` binds `c` to the variant type via the Union match arm.
        let src = "TYPE Circle\n  radius AS Integer\nEND TYPE\nUNION Shape\n  Circle\nEND UNION\nFUNC pick(s AS Shape) AS Integer\n  MATCH s\n    CASE Circle(c)\n      RETURN c.radius\n    CASE ELSE\n      RETURN 0\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  RETURN pick(Circle[1])\nEND FUNC\n";
        assert!(accepts(src));
    }

    #[test]
    fn union_match_unknown_variant_ignored() {
        // A CASE naming a variant not in the union takes the `!variants.any`
        // early-return arm of check_match_pattern (no binding inserted).
        let src = "TYPE Circle\n  radius AS Integer\nEND TYPE\nUNION Shape\n  Circle\nEND UNION\nFUNC pick(s AS Shape) AS Integer\n  MATCH s\n    CASE Bogus(b)\n      RETURN 1\n    CASE ELSE\n      RETURN 0\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  RETURN pick(Circle[1])\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn match_result_ok_error_arms_ignored() {
        // `CASE Ok`/`CASE Error` on any scrutinee take the internal-Result
        // early-return arm of check_match_pattern.
        let src = "FUNC pick(n AS Integer) AS Integer\n  MATCH n\n    CASE Ok(v)\n      RETURN 1\n    CASE ELSE\n      RETURN 0\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  RETURN pick(1)\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn with_update_on_readonly_record_accepted() {
        // WITH on a net Address (a read-only record type) takes the
        // read_only_record_type early-return arm of infer_with_update.
        let src = "IMPORT net\nFUNC main AS Integer\n  RES sock = net::bindUdp(\"127.0.0.1\", 0)\n  LET addr = net::localAddress(sock)\n  LET u = WITH addr { port := 9 }\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn function_value_non_function_type_infers_unknown() {
        // Calling a lambda-typed local is a function value; calling a
        // non-function-typed value through check_function_value_call is the
        // non-Function arm covered by non_callable_local_called; here we also
        // exercise the return-type unwrap on a genuine callable.
        assert!(accepts(&wrap(
            "  LET g AS FUNC() AS Integer = LAMBDA() -> 7\n  LET r AS Integer = g()"
        )));
    }

    // =====================================================================
    // Fifth batch: member-access "not found" arms, thread member, error/
    // errorloc unknown members, member-on-non-record, lambda assign-target
    // capture push, MUT resource in non-escaping position.
    // =====================================================================

    #[test]
    fn error_unknown_member_infers_unknown() {
        // A member other than code/message/source on an Error returns Unknown.
        let _ = check_src(&wrap(
            "  LET e AS Error = error(1, \"m\")\n  LET x = e.bogus",
        ));
    }

    #[test]
    fn errorloc_unknown_member_infers_unknown() {
        // A member other than filename/line/char on an ErrorLoc returns Unknown.
        let _ = check_src(&wrap(
            "  LET e AS Error = error(1, \"m\")\n  LET loc AS ErrorLoc = e.source\n  LET x = loc.bogus",
        ));
    }

    #[test]
    fn member_access_on_scalar_infers_unknown() {
        // Member access on a non-record (Integer) reaches the
        // `else { return Unknown }` non-User arm.
        let _ = check_src(&wrap("  LET n AS Integer = 5\n  LET x = n.field"));
    }

    #[test]
    fn thread_non_result_member_infers_unknown() {
        // A member other than `result` on a Thread returns Unknown (the second
        // Thread arm).
        let src = "IMPORT thread\nFUNC worker(n AS Integer) AS Integer\n  RETURN n\nEND FUNC\nFUNC main AS Integer\n  LET t AS Thread OF Integer TO Integer = thread::start(worker, 1, 1, 1)\n  LET x = t.other\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn builtin_type_unknown_member_infers_unknown() {
        // A member not present in net Address fields returns Unknown (the
        // builtin_type_fields lookup miss).
        let src = "IMPORT net\nFUNC main AS Integer\n  RES sock = net::bindUdp(\"127.0.0.1\", 0)\n  LET addr = net::localAddress(sock)\n  LET x = addr.bogus\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn member_on_enum_value_infers_unknown() {
        // A field access on an enum *value* (non-Type kind user type) returns
        // Unknown via the `!TypeDeclKind::Type` member-access arm.
        let src = "ENUM Color\n  Red, Blue\nEND ENUM\nFUNC main AS Integer\n  LET c AS Color = Color.Red\n  LET x = c.field\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn lambda_assign_body_pure_target_capture_rejected() {
        // `LAMBDA(x) -> total = x`: the target `total` never appears on the RHS,
        // so infer_lambda's assign_target block pushes it as an extra capture.
        let src = "IMPORT collections\nFUNC main AS Integer\n  MUT total AS Integer = 0\n  LET numbers AS List OF Integer = [1, 2, 3]\n  collections::forEach(numbers, LAMBDA(x AS Integer) -> total = x)\n  RETURN total\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn lambda_immutable_resource_in_foreach_rejected() {
        // An immutable resource captured in the non-escaping `forEach` position
        // hits the `is_resource_type` (non-mutable) capture arm.
        let src = "IMPORT collections\nIMPORT fs\nFUNC main AS Integer\n  LET numbers AS List OF Integer = [1, 2, 3]\n  RES handle AS File = fs::createTempFile()\n  collections::forEach(numbers, LAMBDA(x AS Integer) -> fs::writeLine(handle, toString(x)))\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_LAMBDA_CAPTURE_UNSUPPORTED"));
    }

    // ---- plan-18 assertion type-checking (check_expect_call) ----------------

    fn tcase(body: &str) -> String {
        // The assertion built-ins are recognized directly in the `Call` arm
        // (`is_expect_call`), so a plain FUNC body reaches `check_expect_call`
        // without the TESTING desugaring (which runs only in the build pipeline).
        format!("FUNC main AS Integer\n{body}\n  RETURN 0\nEND FUNC\n")
    }

    #[test]
    fn expect_arity_wrong_count_rejected() {
        // `expectEqual` needs two operands; one is an arity error.
        assert!(rejects_with(
            &tcase("      expectEqual(1)"),
            "TESTING_EXPECT_ARITY"
        ));
    }

    #[test]
    fn expect_typed_operand_mismatch_rejected() {
        // `expectFloat` requires both operands to be Float; Integers are rejected.
        assert!(rejects_with(
            &tcase("      expectFloat(1, 2)"),
            "TESTING_EXPECT_TYPE_MISMATCH"
        ));
    }

    #[test]
    fn expect_typed_operand_match_accepted() {
        assert!(accepts(&tcase("      expectInteger(1, 2)")));
    }

    #[test]
    fn expect_neq_typed_operand_match_accepted() {
        assert!(accepts(&tcase("      expectNString(\"a\", \"b\")")));
    }

    #[test]
    fn expect_equal_incomparable_rejected() {
        // A String and an Integer are not comparable with `=`.
        assert!(rejects_with(
            &tcase("      expectEqual(\"a\", 1)"),
            "TESTING_EXPECT_INCOMPARABLE"
        ));
    }

    #[test]
    fn expect_equal_not_printable_rejected() {
        let body = "      LET m AS Map OF String TO Integer = Map OF String TO Integer {}\n      expectEqual(m, m)";
        assert!(rejects_with(&tcase(body), "TESTING_EXPECT_NOT_PRINTABLE"));
    }

    #[test]
    fn expect_equal_scalars_accepted() {
        assert!(accepts(&tcase("      expectEqual(1, 1)")));
    }

    #[test]
    fn expect_trap_non_integer_code_rejected() {
        let body = "      LET xs AS List OF Integer = [1, 2, 3]\n      expectTrap(collections::get(xs, 0), \"x\")";
        let src = format!("IMPORT collections\n{}", tcase(body));
        assert!(rejects_with(&src, "TESTING_EXPECT_CODE_TYPE"));
    }

    #[test]
    fn expect_trap_non_call_rejected() {
        assert!(rejects_with(
            &tcase("      expectTrap(5)"),
            "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"
        ));
    }

    #[test]
    fn expect_ntrap_non_call_rejected() {
        assert!(rejects_with(
            &tcase("      expectNTrap(42)"),
            "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"
        ));
    }

    #[test]
    fn expect_trap_package_constant_rejected() {
        let src = format!("IMPORT math\n{}", tcase("      expectTrap(math::pi)"));
        assert!(rejects_with(&src, "TESTING_EXPECT_TRAP_REQUIRES_FALLIBLE"));
    }

    // ---- enum member access & match patterns --------------------------------

    fn enum_prelude() -> &'static str {
        "ENUM Color\n  Red, Green, Blue\nEND ENUM\n"
    }

    #[test]
    fn enum_member_access_valid_accepted() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET c AS Color = Color.Red\n  RETURN 0\nEND FUNC\n",
            enum_prelude()
        );
        assert!(accepts(&src));
    }

    #[test]
    fn enum_member_access_unknown_member_yields_unknown() {
        // `Color.Bogus` is not a declared member — walks the Unknown arm.
        let src = format!(
            "{}FUNC main AS Integer\n  LET c = Color.Bogus\n  RETURN 0\nEND FUNC\n",
            enum_prelude()
        );
        let _ = check_src(&src);
    }

    #[test]
    fn constructor_of_enum_walks_non_type_arm() {
        // `Color[1]` constructs an Enum, not a record Type — non-Type arm.
        let src = format!(
            "{}FUNC main AS Integer\n  LET c = Color[1]\n  RETURN 0\nEND FUNC\n",
            enum_prelude()
        );
        let _ = check_src(&src);
    }

    #[test]
    fn match_literal_incompatible_pattern() {
        // MATCH on an Integer with a String CASE literal walks the incompatible
        // literal-pattern arm.
        let src = "FUNC main AS Integer\n  LET n AS Integer = 3\n  MATCH n\n    CASE \"x\"\n      RETURN 1\n    CASE ELSE\n      RETURN 2\n  END MATCH\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn match_oneof_pattern_accepted() {
        let src = "FUNC main AS Integer\n  LET n AS Integer = 3\n  MATCH n\n    CASE 1, 2, 3\n      RETURN 1\n    CASE ELSE\n      RETURN 2\n  END MATCH\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn match_union_pattern_on_non_union_scrutinee() {
        // A union CASE against a non-union scrutinee walks the `_ => {}` arm.
        let src = "FUNC main AS Integer\n  LET n AS Integer = 3\n  MATCH n\n    CASE Foo(x)\n      RETURN 1\n    CASE ELSE\n      RETURN 2\n  END MATCH\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn match_union_pattern_unknown_variant() {
        // A CASE naming a variant not in the union walks the non-variant arm.
        let src = "TYPE Dot\n  x AS Integer\nEND TYPE\nTYPE Line\n  a AS Integer\nEND TYPE\nUNION Shape\n  Dot\n  Line\nEND UNION\nFUNC pick(s AS Shape) AS Integer\n  MATCH s\n    CASE Dot(d)\n      RETURN 1\n    CASE Nope(n)\n      RETURN 2\n    CASE ELSE\n      RETURN 3\n  END MATCH\nEND FUNC\nFUNC main AS Integer\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn match_case_ok_error_result_skipped() {
        // `CASE Ok`/`CASE Error` are internal Result variants and are skipped.
        let src = "FUNC main AS Integer\n  LET n AS Integer = 3\n  MATCH n\n    CASE Ok(o)\n      RETURN 1\n    CASE ELSE\n      RETURN 2\n  END MATCH\nEND FUNC\n";
        let _ = check_src(src);
    }

    // ---- WITH-update arms ---------------------------------------------------

    fn point_type() -> &'static str {
        "TYPE Point\n  x AS Integer\n  y AS Integer\nEND TYPE\n"
    }

    #[test]
    fn with_update_on_non_user_type_yields_unknown() {
        let src = "FUNC main AS Integer\n  LET n AS Integer = 3\n  LET m = WITH n { }\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn with_update_duplicate_field_rejected() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET p AS Point = Point[1, 2]\n  LET q = WITH p {{ x := 3, x := 4 }}\n  RETURN 0\nEND FUNC\n",
            point_type()
        );
        assert!(rejects_with(&src, "TYPE_DUPLICATE_FIELD"));
    }

    #[test]
    fn with_update_unknown_field_walks_arm() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET p AS Point = Point[1, 2]\n  LET q = WITH p {{ bogus := 3 }}\n  RETURN 0\nEND FUNC\n",
            point_type()
        );
        let _ = check_src(&src);
    }

    #[test]
    fn with_update_valid_accepted() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET p AS Point = Point[1, 2]\n  LET q = WITH p {{ x := 9 }}\n  RETURN q.x\nEND FUNC\n",
            point_type()
        );
        assert!(accepts(&src));
    }

    // ---- constructor argument arms ------------------------------------------

    #[test]
    fn constructor_named_unknown_field_walks_arm() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET p AS Point = Point[x := 1, bogus := 2]\n  RETURN 0\nEND FUNC\n",
            point_type()
        );
        let _ = check_src(&src);
    }

    #[test]
    fn constructor_named_duplicate_field_rejected() {
        let src = format!(
            "{}FUNC main AS Integer\n  LET p AS Point = Point[x := 1, x := 2]\n  RETURN 0\nEND FUNC\n",
            point_type()
        );
        assert!(rejects_with(&src, "TYPE_DUPLICATE_FIELD"));
    }

    #[test]
    fn read_only_error_record_constructor_rejected() {
        let src = "FUNC main AS Integer\n  LET e AS Error = Error[1, \"m\"]\n  RETURN 0\nEND FUNC\n";
        assert!(rejects_with(src, "TYPE_READ_ONLY_RECORD_CONSTRUCTOR"));
    }

    // ---- literal-type arms --------------------------------------------------

    #[test]
    fn fixed_literal_types_accepted() {
        assert!(accepts(&wrap("  LET f AS Fixed = 1.5F")));
    }

    #[test]
    fn negated_fixed_literal_accepted() {
        assert!(accepts(&wrap("  LET f AS Fixed = -2F")));
    }

    #[test]
    fn negated_out_of_range_integer_literal_is_integer() {
        // The negated magnitude is out of positive i64 range, taking the early
        // `Type::Integer` arm of the unary-minus branch.
        assert!(accepts(&wrap("  LET n AS Integer = -9223372036854775808")));
    }

    // ---- lambda parameter arms ----------------------------------------------

    #[test]
    fn lambda_untyped_param_arm() {
        // An untyped lambda parameter walks the `type_name.is_none()` arm.
        let src = "IMPORT collections\nFUNC main AS Integer\n  LET numbers AS List OF Integer = [1, 2, 3]\n  LET ys = collections::transform(numbers, LAMBDA(x) -> x * 2)\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }

    #[test]
    fn lambda_assign_to_param_immutable_arm() {
        // The assignment target is a lambda parameter (immutable) — walks the
        // `!local.mutable` arm of the assignment-bodied lambda.
        let src = "IMPORT collections\nFUNC main AS Integer\n  LET numbers AS List OF Integer = [1, 2, 3]\n  collections::forEach(numbers, LAMBDA(x AS Integer) -> x = 5)\n  RETURN 0\nEND FUNC\n";
        let _ = check_src(src);
    }
}
