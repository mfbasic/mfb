use super::*;

/// Maximum expression-nesting depth. Recursive-descent parsing turns each nesting
/// level into native stack frames, so an unbounded input (e.g. ~100k nested `(`,
/// or a long `NOT NOT …` / unary-minus / `^` chain) would overflow the stack with
/// a SIGSEGV before any diagnostic (bug-171 finding A). Matches the `MAX_DEPTH`
/// cap `ir::verify` uses for the same reason; no real source nests this deep.
const MAX_EXPR_DEPTH: usize = 256;

impl<'a> FileParser<'a> {
    /// Enter one expression-nesting level, reporting and returning `false` when
    /// the maximum depth is exceeded. On the `false` path the counter is already
    /// rewound, so the caller must simply bail (`return None`); otherwise it must
    /// pair a successful `enter_expr` with exactly one `leave_expr`.
    fn enter_expr(&mut self) -> bool {
        self.expr_depth += 1;
        if self.expr_depth > MAX_EXPR_DEPTH {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Expression nesting is too deep.",
                &token,
            );
            self.expr_depth -= 1;
            false
        } else {
            true
        }
    }

    fn leave_expr(&mut self) {
        self.expr_depth -= 1;
    }

    pub(super) fn parse_expression(&mut self) -> Option<Expression> {
        if !self.enter_expr() {
            return None;
        }
        let result = self.parse_pipeline();
        self.leave_expr();
        result
    }

    pub(super) fn parse_pipeline(&mut self) -> Option<Expression> {
        let mut expression = self.parse_or()?;
        while self.match_kind(TokenKind::PipeGreater) {
            let token = self.previous().clone();
            let right = self.parse_or()?;
            if !contains_placeholder(&right) {
                self.report(
                    "MFB_PARSE_PIPELINE_PLACEHOLDER_MISSING",
                    "Pipeline right-hand side must contain `_` as the input placeholder.",
                    &token,
                );
                return None;
            }
            expression = substitute_placeholder(right, &expression);
        }
        Some(expression)
    }

    pub(super) fn parse_or(&mut self) -> Option<Expression> {
        let mut expression = self.parse_and()?;
        while self.match_any_keywords(&[Keyword::Or, Keyword::Xor]) {
            let operator = match self.previous().kind {
                TokenKind::Keyword(Keyword::Or) => "OR",
                TokenKind::Keyword(Keyword::Xor) => "XOR",
                // coverage:off — the preceding match_any_keywords guarantees the
                // previous token is OR or XOR.
                _ => unreachable!(),
                // coverage:on
            };
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_and()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_and(&mut self) -> Option<Expression> {
        let mut expression = self.parse_not()?;
        while self.match_keyword(Keyword::And) {
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_not()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: "AND".to_string(),
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_not(&mut self) -> Option<Expression> {
        if self.match_keyword(Keyword::Not) {
            let (line, column) = (self.previous().line, self.previous().start);
            if !self.enter_expr() {
                return None;
            }
            let operand = self.parse_not();
            self.leave_expr();
            let operand = operand?;
            return Some(Expression::Unary {
                operator: "NOT".to_string(),
                operand: Box::new(operand),
                line,
                column,
            });
        }
        self.parse_comparison()
    }

    pub(super) fn parse_comparison(&mut self) -> Option<Expression> {
        let mut expression = self.parse_concat()?;
        while self.match_any(&[
            TokenKind::Equal,
            TokenKind::NotEqual,
            TokenKind::Less,
            TokenKind::LessEqual,
            TokenKind::Greater,
            TokenKind::GreaterEqual,
        ]) {
            let operator = match self.previous().kind {
                TokenKind::Equal => "=",
                TokenKind::NotEqual => "<>",
                TokenKind::Less => "<",
                TokenKind::LessEqual => "<=",
                TokenKind::Greater => ">",
                TokenKind::GreaterEqual => ">=",
                // coverage:off — the preceding match_any guarantees a comparison
                // operator token here.
                _ => unreachable!(),
                // coverage:on
            };
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_concat()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_concat(&mut self) -> Option<Expression> {
        let mut expression = self.parse_addition()?;
        while self.match_kind(TokenKind::Ampersand) {
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_addition()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: "&".to_string(),
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_addition(&mut self) -> Option<Expression> {
        let mut expression = self.parse_multiplication()?;
        while self.match_any(&[TokenKind::Plus, TokenKind::Minus]) {
            let operator = match self.previous().kind {
                TokenKind::Plus => "+",
                TokenKind::Minus => "-",
                // coverage:off — the preceding match_any guarantees `+` or `-`.
                _ => unreachable!(),
                // coverage:on
            };
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_multiplication()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_multiplication(&mut self) -> Option<Expression> {
        let mut expression = self.parse_power()?;
        while self.match_any(&[TokenKind::Star, TokenKind::Slash])
            || self.match_any_keywords(&[Keyword::Mod, Keyword::Div])
        {
            let operator = match self.previous().kind {
                TokenKind::Star => "*",
                TokenKind::Slash => "/",
                TokenKind::Keyword(Keyword::Mod) => "MOD",
                TokenKind::Keyword(Keyword::Div) => "DIV",
                // coverage:off — the preceding match guards guarantee `*`, `/`,
                // MOD, or DIV here.
                _ => unreachable!(),
                // coverage:on
            };
            let (line, column) = (self.previous().line, self.previous().start);
            let right = self.parse_power()?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: operator.to_string(),
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_power(&mut self) -> Option<Expression> {
        let mut expression = self.parse_unary()?;
        if self.match_kind(TokenKind::Caret) {
            let (line, column) = (self.previous().line, self.previous().start);
            if !self.enter_expr() {
                return None;
            }
            let right = self.parse_power();
            self.leave_expr();
            let right = right?;
            expression = Expression::Binary {
                left: Box::new(expression),
                operator: "^".to_string(),
                right: Box::new(right),
                line,
                column,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_unary(&mut self) -> Option<Expression> {
        if self.match_kind(TokenKind::Minus) {
            let (line, column) = (self.previous().line, self.previous().start);
            if !self.enter_expr() {
                return None;
            }
            let operand = self.parse_unary();
            self.leave_expr();
            let operand = operand?;
            return Some(Expression::Unary {
                operator: "-".to_string(),
                operand: Box::new(operand),
                line,
                column,
            });
        }
        if self.match_keyword(Keyword::With) {
            return self.parse_with_update();
        }
        self.parse_member_access()
    }

    pub(super) fn parse_with_update(&mut self) -> Option<Expression> {
        let target = self.parse_member_access()?;
        if !self.consume_kind(TokenKind::LBrace, "Expected `{` after WITH target.") {
            return None;
        }
        let mut updates = Vec::new();
        if !self.check_kind(&TokenKind::RBrace) {
            loop {
                let line = self.peek().line;
                let Some(field) =
                    self.consume_identifier("WITH update field must be an identifier.")
                else {
                    self.synchronize();
                    return None;
                };
                if !self.consume_kind(
                    TokenKind::ColonEqual,
                    "Expected `:=` between WITH update field and value.",
                ) {
                    return None;
                }
                let value = self.parse_expression()?;
                updates.push(RecordUpdate { field, value, line });
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.consume_kind(TokenKind::RBrace, "Expected `}` after WITH updates.") {
            return None;
        }
        Some(Expression::WithUpdate {
            target: Box::new(target),
            updates,
        })
    }

    pub(super) fn parse_member_access(&mut self) -> Option<Expression> {
        let mut expression = self.parse_call_or_constructor()?;
        while self.match_kind(TokenKind::Dot) {
            let member = self.consume_identifier("Expected identifier after `.`.")?;
            expression = Expression::MemberAccess {
                target: Box::new(expression),
                member,
            };
        }
        Some(expression)
    }

    pub(super) fn parse_call_or_constructor(&mut self) -> Option<Expression> {
        let start = self.peek().clone();
        let mut expression = self.parse_primary()?;
        loop {
            if self.match_kind(TokenKind::LParen) {
                let callee = match expression {
                    Expression::Identifier(value) => value,
                    _ => {
                        let token = self.previous().clone();
                        self.report(
                            "MFB_PARSE_EXPECTED_EXPRESSION",
                            "Only identifiers can be called by the current parser.",
                            &token,
                        );
                        return None;
                    }
                };
                let arguments = self.parse_argument_list(TokenKind::RParen)?;
                expression = Expression::Call {
                    callee,
                    arguments,
                    line: start.line,
                    column: start.start,
                };
            } else if self.match_kind(TokenKind::LBracket) {
                let type_name = match expression {
                    // A package-qualified built-in type used as a constructor
                    // (`http::Response[...]`) normalizes to its bare id, matching
                    // the type-position rule (plan-03-http.md §A.1/§B.2).
                    Expression::Identifier(value) => {
                        crate::builtins::qualified_builtin_type(&value).unwrap_or(value)
                    }
                    _ => {
                        let token = self.previous().clone();
                        self.report(
                            "MFB_PARSE_EXPECTED_EXPRESSION",
                            "Only identifiers can be used as constructors.",
                            &token,
                        );
                        return None;
                    }
                };
                let arguments = self.parse_constructor_argument_list(TokenKind::RBracket)?;
                expression = Expression::Constructor {
                    type_name,
                    arguments,
                };
            } else {
                break;
            }
        }
        Some(expression)
    }

    pub(super) fn parse_argument_list(&mut self, closing: TokenKind) -> Option<Vec<CallArg>> {
        let mut arguments = Vec::new();
        if !self.check_kind(&closing) {
            loop {
                if matches!(self.peek().kind, TokenKind::Identifier(_))
                    && self
                        .peek_next()
                        .is_some_and(|token| matches!(token.kind, TokenKind::ColonEqual))
                {
                    let line = self.peek().line;
                    let name =
                        self.consume_identifier("Call argument name must be an identifier.")?;
                    self.consume_kind(
                        TokenKind::ColonEqual,
                        "Expected `:=` between call argument name and value.",
                    );
                    let value = self.parse_expression()?;
                    arguments.push(CallArg::Named { name, value, line });
                } else {
                    arguments.push(CallArg::Positional(self.parse_expression()?));
                }
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        let detail = match closing {
            TokenKind::RParen => "Expected `)` after call arguments.",
            TokenKind::RBracket => "Expected `]` after constructor arguments.",
            _ => "Expected closing delimiter after arguments.",
        };
        if !self.consume_kind(closing, detail) {
            return None;
        }
        Some(arguments)
    }

    pub(super) fn parse_constructor_argument_list(
        &mut self,
        closing: TokenKind,
    ) -> Option<Vec<ConstructorArg>> {
        let mut arguments = Vec::new();
        if !self.check_kind(&closing) {
            loop {
                if matches!(self.peek().kind, TokenKind::Identifier(_))
                    && self
                        .peek_next()
                        .is_some_and(|token| matches!(token.kind, TokenKind::ColonEqual))
                {
                    let line = self.peek().line;
                    let name =
                        self.consume_identifier("Constructor field name must be an identifier.")?;
                    self.consume_kind(
                        TokenKind::ColonEqual,
                        "Expected `:=` between constructor field and value.",
                    );
                    let value = self.parse_expression()?;
                    arguments.push(ConstructorArg::Named { name, value, line });
                } else {
                    arguments.push(ConstructorArg::Positional(self.parse_expression()?));
                }
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        let detail = match closing {
            TokenKind::RBracket => "Expected `]` after constructor arguments.",
            _ => "Expected closing delimiter after constructor arguments.",
        };
        if !self.consume_kind(closing, detail) {
            return None;
        }
        Some(arguments)
    }

    pub(super) fn parse_primary(&mut self) -> Option<Expression> {
        // At end-of-input `advance()` does not move the cursor and re-yields the
        // token *before* Eof (an already-consumed `(`/`[`), which would re-enter
        // the grouped-expression / list-literal arms with zero progress and
        // recurse until the native stack overflows (bug-89). Treat Eof as a hard
        // parse error here instead of re-reading `previous()`.
        if self.is_at_end() {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_EXPECTED_EXPRESSION",
                "Expected an expression.",
                &token,
            );
            return None;
        }
        let token = self.advance().clone();
        match token.kind {
            TokenKind::String(value) => Some(Expression::String(value)),
            TokenKind::Number(value) => Some(Expression::Number(value)),
            TokenKind::Keyword(Keyword::True) => Some(Expression::Boolean(true)),
            TokenKind::Keyword(Keyword::False) => Some(Expression::Boolean(false)),
            TokenKind::Keyword(Keyword::Nothing) => {
                Some(Expression::Identifier("NOTHING".to_string()))
            }
            TokenKind::Keyword(Keyword::Lambda) => self.parse_lambda(),
            TokenKind::Identifier(value) => {
                if value.eq_ignore_ascii_case("Map") && self.check_identifier_ci("OF") {
                    self.advance();
                    let key_type = self.parse_type_name()?;
                    if !self.check_identifier_ci("TO") && !self.check_keyword(Keyword::To) {
                        let token = self.peek().clone();
                        self.report(
                            "MFB_PARSE_UNEXPECTED_TOKEN",
                            "Expected `TO` in map literal type.",
                            &token,
                        );
                        return None;
                    }
                    self.advance();
                    // A `Map OF K TO RES File { … }` literal carries the resource
                    // ownership-axis marker on its value type (§15.6).
                    let value_res = self.match_keyword(Keyword::Res);
                    let value_type = self.parse_type_name()?;
                    let value_type = if value_res {
                        format!("RES {value_type}")
                    } else {
                        value_type
                    };
                    return self.parse_map_literal(key_type, value_type);
                }
                let name = self.finish_qualified_name(value)?;
                Some(Expression::Identifier(name))
            }
            TokenKind::LParen => {
                let expression = self.parse_expression();
                self.consume_kind(TokenKind::RParen, "Expected `)` after expression.");
                expression
            }
            TokenKind::LBracket => self.parse_list_literal(),
            _ => {
                self.report(
                    "MFB_PARSE_EXPECTED_EXPRESSION",
                    "Expected an expression.",
                    &token,
                );
                None
            }
        }
    }

    pub(super) fn parse_qualified_name(&mut self, detail: &str) -> Option<String> {
        let name = self.consume_identifier(detail)?;
        self.finish_qualified_name(name)
    }

    pub(super) fn finish_qualified_name(&mut self, mut name: String) -> Option<String> {
        if self.match_kind(TokenKind::DoubleColon) {
            let part = self.consume_qualified_identifier_part()?;
            name.push('.');
            name.push_str(&part);
        }
        while self.match_kind(TokenKind::DoubleColon) {
            let token = self.previous().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Package-qualified names must have exactly two parts.",
                &token,
            );
            self.consume_qualified_identifier_part()?;
        }
        Some(name)
    }

    pub(super) fn parse_type_name(&mut self) -> Option<String> {
        if self.match_keyword(Keyword::Func) {
            return self.parse_function_type_name(false);
        }
        if self.match_keyword(Keyword::Isolated) {
            if self.consume_keyword(Keyword::Func, "ISOLATED type must be followed by FUNC.") {
                return self.parse_function_type_name(true);
            }
            return None;
        }
        if self.match_kind(TokenKind::LParen) {
            let name = self.parse_type_name()?;
            self.consume_kind(TokenKind::RParen, "Expected `)` after grouped type.");
            return Some(format!("({name})"));
        }
        let mut name = self.parse_type_base_name("Expected a type name.")?;
        if self.check_identifier_ci("OF") {
            self.advance();
            if name.eq_ignore_ascii_case("Thread") || name.eq_ignore_ascii_case("ThreadWorker") {
                return self.parse_thread_type_name(name);
            }

            if name.eq_ignore_ascii_case("Map") || name.eq_ignore_ascii_case("MapEntry") {
                let first = self.parse_type_name()?;
                if !self.check_identifier_ci("TO") && !self.check_keyword(Keyword::To) {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        "Expected `TO` in map type.",
                        &token,
                    );
                    return None;
                }
                self.advance();
                // A `RES` value marks a resource-transfer collection
                // (`Map OF K TO RES File`, §15.6): the value is a resource borrow
                // whose scope-ownership transfers across a function boundary.
                let value_res = self.match_keyword(Keyword::Res);
                let second = self.parse_type_name()?;
                name.push_str(" OF ");
                name.push_str(&first);
                name.push_str(" TO ");
                if value_res {
                    name.push_str("RES ");
                }
                name.push_str(&second);
                return Some(name);
            }

            if name.eq_ignore_ascii_case("List") || name.eq_ignore_ascii_case("Result") {
                // `List OF RES File` (§15.6): a resource-transfer list whose
                // element is a borrow whose scope-ownership transfers across a
                // function boundary. (`Result OF RES …` is not meaningful, but the
                // marker is harmless there and rejected later by type checking.)
                let element_res =
                    name.eq_ignore_ascii_case("List") && self.match_keyword(Keyword::Res);
                let arg = self.parse_type_name()?;
                name.push_str(" OF ");
                if element_res {
                    name.push_str("RES ");
                }
                name.push_str(&arg);
                return Some(name);
            }

            let mut args = vec![self.parse_type_name()?];
            while self.match_kind(TokenKind::Comma) {
                args.push(self.parse_type_name()?);
            }
            // coverage:off — `args` is seeded with one parsed type above, so it is
            // never empty here; this guard is defensive.
            if args.is_empty() {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    "Expected at least one template type argument.",
                    &token,
                );
                return None;
            }
            // coverage:on
            name.push_str(" OF ");
            name.push_str(&args.join(", "));
        }
        Some(name)
    }

    /// Parse a thread type body after `<kind> OF`, supporting the optional
    /// resource plane: `Thread OF Msg TO Out`, `Thread OF Msg RES Res TO Out`,
    /// or the resource-only `Thread OF RES Res TO Out` (message defaults to
    /// `Nothing`). `kind` is the leading `Thread`/`ThreadWorker` token.
    pub(super) fn parse_thread_type_name(&mut self, kind: String) -> Option<String> {
        let canonical = if kind.eq_ignore_ascii_case("ThreadWorker") {
            "ThreadWorker"
        } else {
            "Thread"
        };

        let mut message: Option<String> = None;
        let mut resource: Option<String> = None;

        if self.match_keyword(Keyword::Res) {
            resource = Some(self.parse_type_name()?);
        } else {
            message = Some(self.parse_type_name()?);
            if self.match_keyword(Keyword::Res) {
                resource = Some(self.parse_type_name()?);
            }
        }

        if !self.check_identifier_ci("TO") && !self.check_keyword(Keyword::To) {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Expected `TO` in thread type.",
                &token,
            );
            return None;
        }
        self.advance();
        let output = self.parse_type_name()?;

        let message = message.unwrap_or_else(|| "Nothing".to_string());
        Some(match resource {
            Some(resource) if message == "Nothing" => {
                format!("{canonical} OF RES {resource} TO {output}")
            }
            Some(resource) => format!("{canonical} OF {message} RES {resource} TO {output}"),
            None => format!("{canonical} OF {message} TO {output}"),
        })
    }

    pub(super) fn parse_function_type_name(&mut self, isolated: bool) -> Option<String> {
        if !self.consume_kind(TokenKind::LParen, "Function type must include `(`.") {
            return None;
        }
        let mut params = Vec::new();
        if !self.check_kind(&TokenKind::RParen) {
            loop {
                params.push(self.parse_type_name()?);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.consume_kind(TokenKind::RParen, "Function type must close with `)`.") {
            return None;
        }
        if !self.consume_keyword(Keyword::As, "Function type must include `AS`.") {
            return None;
        }
        let returns = self.parse_type_name()?;
        Some(format!(
            "{}FUNC({}) AS {}",
            if isolated { "ISOLATED " } else { "" },
            params.join(", "),
            returns
        ))
    }

    pub(super) fn parse_lambda(&mut self) -> Option<Expression> {
        if !self.consume_kind(TokenKind::LParen, "Lambda must include `(` after LAMBDA.") {
            return None;
        }
        let params = self.parse_params();
        if !self.consume_kind(TokenKind::RParen, "Lambda must close the parameter list.") {
            return None;
        }
        if !self.consume_kind(
            TokenKind::Arrow,
            "Lambda must include `->` before its body.",
        ) {
            return None;
        }
        // A lambda body of the form `name = <expr>` is an assignment (the same
        // `identifier =` lookahead the statement parser uses to tell assignment
        // from the `=` equality operator). It mutates `name` and yields Nothing;
        // this is the shape a non-escaping callback uses to update a captured
        // `MUT` binding.
        let assign_target = if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            if self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Equal))
            {
                self.advance();
                self.advance();
                Some(name)
            } else {
                None
            }
        } else {
            None
        };
        let body = self.parse_expression()?;
        Some(Expression::Lambda {
            params,
            body: Box::new(body),
            assign_target,
        })
    }

    pub(super) fn parse_type_base_name(&mut self, detail: &str) -> Option<String> {
        let name = match self.peek().kind.clone() {
            TokenKind::Identifier(value) => {
                self.advance();
                value
            }
            TokenKind::Keyword(Keyword::Nothing) => {
                self.advance();
                "Nothing".to_string()
            }
            _ => {
                let token = self.peek().clone();
                self.report("MFB_PARSE_INVALID_IDENTIFIER", detail, &token);
                return None;
            }
        };
        // A package-qualified built-in type (`net::Url`, `http::Response`) is
        // normalized to its bare internal id at parse time, so every downstream
        // stage sees only bare ids (plan-03-http.md §A.1/§B.2).
        self.finish_qualified_name(name).map(|qualified| {
            crate::builtins::qualified_builtin_type(&qualified).unwrap_or(qualified)
        })
    }

    pub(super) fn parse_list_literal(&mut self) -> Option<Expression> {
        let mut values = Vec::new();
        if !self.check_kind(&TokenKind::RBracket) {
            loop {
                values.push(self.parse_expression()?);
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume_kind(TokenKind::RBracket, "Expected `]` after list literal.");
        Some(Expression::ListLiteral(values))
    }

    pub(super) fn parse_map_literal(
        &mut self,
        key_type: String,
        value_type: String,
    ) -> Option<Expression> {
        if !self.consume_kind(TokenKind::LBrace, "Expected `{` after map literal type.") {
            return None;
        }
        let mut entries = Vec::new();
        if !self.check_kind(&TokenKind::RBrace) {
            loop {
                let key = self.parse_expression()?;
                if !self.consume_kind(
                    TokenKind::ColonEqual,
                    "Expected `:=` between map key and value.",
                ) {
                    return None;
                }
                let value = self.parse_expression()?;
                entries.push((key, value));
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume_kind(TokenKind::RBrace, "Expected `}` after map literal.");
        Some(Expression::MapLiteral {
            key_type,
            value_type,
            entries,
        })
    }
}
