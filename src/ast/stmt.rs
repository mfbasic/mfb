use super::*;

impl<'a> FileParser<'a> {
    pub(super) fn parse_statement(&mut self) -> Option<Statement> {
        if self.check_keyword(Keyword::If) {
            return self.parse_if_statement();
        }

        if self.check_keyword(Keyword::Match) {
            return self.parse_match_statement();
        }

        if self.check_keyword(Keyword::For) {
            return self.parse_for_statement();
        }

        if self.check_keyword(Keyword::While) {
            return self.parse_while_statement();
        }

        if self.check_keyword(Keyword::Do) {
            return self.parse_do_statement();
        }

        self.parse_simple_statement(false)
    }

    pub(super) fn parse_simple_statement(
        &mut self,
        allow_else_terminator: bool,
    ) -> Option<Statement> {
        if self.check_keyword(Keyword::If)
            || self.check_keyword(Keyword::Match)
            || self.check_keyword(Keyword::For)
            || self.check_keyword(Keyword::While)
            || self.check_keyword(Keyword::Do)
        {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "Inline IF branches must use a simple statement.",
                &token,
            );
            return None;
        }

        if self.check_keyword(Keyword::Let)
            || self.check_keyword(Keyword::Mut)
            || self.check_keyword(Keyword::Res)
        {
            let keyword = self.advance().clone();
            let mutable = matches!(keyword.kind, TokenKind::Keyword(Keyword::Mut));
            let resource = matches!(keyword.kind, TokenKind::Keyword(Keyword::Res));
            let name = self.consume_identifier("Binding name must be an identifier.")?;
            let type_name = if self.match_keyword(Keyword::As) {
                self.parse_type_name()
            } else {
                None
            };
            let state_type = if resource {
                self.parse_optional_state()
            } else {
                None
            };
            let value = if self.match_kind(TokenKind::Equal) {
                match self.parse_expression() {
                    Some(expr) => self.maybe_attach_postfix_trap(expr, allow_else_terminator),
                    None => None,
                }
            } else {
                None
            };
            if !matches!(value, Some(Expression::Trapped { .. })) {
                self.consume_simple_statement_end(
                    "Expected end of statement after binding.",
                    allow_else_terminator,
                );
            }
            return Some(Statement::Let {
                mutable,
                resource,
                state_type,
                name,
                type_name,
                value,
                line: keyword.line,
            });
        }

        if self.match_keyword(Keyword::Return) {
            let token = self.previous().clone();
            let value = if self.is_statement_end()
                || (allow_else_terminator && self.check_keyword(Keyword::Else))
            {
                None
            } else {
                self.parse_expression()
            };
            self.consume_simple_statement_end(
                "Expected end of statement after RETURN.",
                allow_else_terminator,
            );
            return Some(Statement::Return {
                value,
                line: token.line,
            });
        }

        if self.match_keyword(Keyword::Exit) {
            let token = self.previous().clone();
            let target = if self.match_keyword(Keyword::For) {
                ExitTarget::For
            } else if self.match_keyword(Keyword::Do) {
                ExitTarget::Do
            } else if self.match_keyword(Keyword::While) {
                ExitTarget::While
            } else if self.match_keyword(Keyword::Sub) {
                ExitTarget::Sub
            } else if self.match_keyword(Keyword::Func) {
                ExitTarget::Func
            } else if self.match_keyword(Keyword::Program) {
                ExitTarget::Program
            } else {
                let unexpected = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    "EXIT must be followed by FOR, DO, WHILE, SUB, FUNC, or PROGRAM.",
                    &unexpected,
                );
                return None;
            };
            let code = if matches!(target, ExitTarget::Program) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            self.consume_simple_statement_end(
                "Expected end of statement after EXIT.",
                allow_else_terminator,
            );
            return Some(Statement::Exit {
                target,
                code,
                line: token.line,
            });
        }

        if self.match_keyword(Keyword::Continue) {
            let token = self.previous().clone();
            let kind = if self.match_keyword(Keyword::For) {
                LoopKind::For
            } else if self.match_keyword(Keyword::Do) {
                LoopKind::Do
            } else if self.match_keyword(Keyword::While) {
                LoopKind::While
            } else {
                let unexpected = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    "CONTINUE must be followed by FOR, DO, or WHILE.",
                    &unexpected,
                );
                return None;
            };
            self.consume_simple_statement_end(
                "Expected end of statement after CONTINUE.",
                allow_else_terminator,
            );
            return Some(Statement::Continue {
                kind,
                line: token.line,
            });
        }

        if self.match_keyword(Keyword::Fail) {
            let token = self.previous().clone();
            let error = self.parse_expression()?;
            self.consume_simple_statement_end(
                "Expected end of statement after FAIL.",
                allow_else_terminator,
            );
            return Some(Statement::Fail {
                error,
                line: token.line,
            });
        }

        if self.match_keyword(Keyword::Propagate) {
            let token = self.previous().clone();
            self.consume_simple_statement_end(
                "Expected end of statement after PROPAGATE.",
                allow_else_terminator,
            );
            return Some(Statement::Propagate { line: token.line });
        }

        if self.match_keyword(Keyword::Recover) {
            let token = self.previous().clone();
            let value = if self.is_statement_end()
                || (allow_else_terminator && self.check_keyword(Keyword::Else))
            {
                None
            } else {
                self.parse_expression()
            };
            self.consume_simple_statement_end(
                "Expected end of statement after RECOVER.",
                allow_else_terminator,
            );
            return Some(Statement::Recover {
                value,
                line: token.line,
            });
        }

        // `resource.state = value` — the one member-target assignment, used to
        // replace a `RES` binding's `STATE` payload. The nested form
        // `resource.state.field = value` desugars to a `STATE` replacement with a
        // single-field `WITH` update, giving in-place field mutation (§4) while
        // reusing the one member-target assignment.
        if let TokenKind::Identifier(resource) = self.peek().kind.clone() {
            let on_state = self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Dot))
                && self.tokens.get(self.current + 2).is_some_and(|token| {
                    matches!(&token.kind, TokenKind::Identifier(member) if member == "state")
                });
            let state_assign = on_state
                && self
                    .tokens
                    .get(self.current + 3)
                    .is_some_and(|token| matches!(token.kind, TokenKind::Equal));
            // `resource.state.field =`
            let state_field_assign = on_state
                && self
                    .tokens
                    .get(self.current + 3)
                    .is_some_and(|token| matches!(token.kind, TokenKind::Dot))
                && self
                    .tokens
                    .get(self.current + 4)
                    .is_some_and(|token| matches!(token.kind, TokenKind::Identifier(_)))
                && self
                    .tokens
                    .get(self.current + 5)
                    .is_some_and(|token| matches!(token.kind, TokenKind::Equal));
            if state_assign || state_field_assign {
                let token = self.advance().clone(); // resource
                self.advance(); // .
                self.advance(); // state
                let field = if state_field_assign {
                    self.advance(); // .
                    let TokenKind::Identifier(field) = self.advance().kind.clone() else {
                        return None;
                    };
                    Some(field)
                } else {
                    None
                };
                self.advance(); // =
                let line = token.line;
                let value = self.parse_expression()?;
                let value = self.maybe_attach_postfix_trap(value, allow_else_terminator)?;
                if !matches!(value, Expression::Trapped { .. }) {
                    self.consume_simple_statement_end(
                        "Expected end of statement after assignment.",
                        allow_else_terminator,
                    );
                }
                // Desugar the nested-field form into a single-field `WITH` update
                // over the current state.
                let value = match field {
                    Some(field) => Expression::WithUpdate {
                        target: Box::new(Expression::MemberAccess {
                            target: Box::new(Expression::Identifier(resource.clone())),
                            member: "state".to_string(),
                        }),
                        updates: vec![RecordUpdate { field, value, line }],
                    },
                    None => value,
                };
                return Some(Statement::StateAssign {
                    resource,
                    value,
                    line,
                });
            }
        }

        if let TokenKind::Identifier(name) = self.peek().kind.clone() {
            if self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::Equal))
            {
                let token = self.advance().clone();
                self.advance();
                let value = self.parse_expression()?;
                let value = self.maybe_attach_postfix_trap(value, allow_else_terminator)?;
                if !matches!(value, Expression::Trapped { .. }) {
                    self.consume_simple_statement_end(
                        "Expected end of statement after assignment.",
                        allow_else_terminator,
                    );
                }
                return Some(Statement::Assign {
                    name,
                    value,
                    line: token.line,
                });
            }
        }

        let token = self.peek().clone();
        let expression = self.parse_expression();
        if expression.is_none() {
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "Statement is not recognized by the current parser.",
                &token,
            );
            return None;
        }
        let expression = self.maybe_attach_postfix_trap(
            expression.expect("checked expression"),
            allow_else_terminator,
        )?;
        if !matches!(expression, Expression::Trapped { .. }) {
            self.consume_simple_statement_end(
                "Expected end of statement after expression.",
                allow_else_terminator,
            );
        }
        Some(Statement::Expression {
            expression,
            line: token.line,
        })
    }

    /// Parse a postfix inline `TRAP(e) … END TRAP` if one immediately follows
    /// the just-parsed expression. Returns the expression wrapped in
    /// `Expression::Trapped` when a trap is attached, otherwise the expression
    /// unchanged. Inline traps are only legal at the top level of a binding,
    /// assignment, or bare-expression statement, so they are never attached
    /// inside an inline `IF` branch (`allow_else_terminator`).
    pub(super) fn maybe_attach_postfix_trap(
        &mut self,
        subject: Expression,
        allow_else_terminator: bool,
    ) -> Option<Expression> {
        if allow_else_terminator
            || !self.check_keyword(Keyword::Trap)
            || !self
                .tokens
                .get(self.current + 1)
                .is_some_and(|token| matches!(token.kind, TokenKind::LParen))
        {
            return Some(subject);
        }

        let token = self.advance().clone();
        self.advance();
        let binding = self.consume_identifier("TRAP must bind an error identifier.")?;
        if !self.consume_kind(TokenKind::RParen, "TRAP error binding must close with `)`.") {
            self.synchronize();
            return None;
        }
        self.consume_statement_end("Expected end of statement after TRAP header.");
        self.skip_separators();

        let mut handler = Vec::new();
        while !self.is_at_end() && !self.is_end_block(Keyword::Trap) {
            if let Some(statement) = self.parse_statement() {
                handler.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }
        if !self.consume_end_block(Keyword::Trap, "TRAP block must end with END TRAP.") {
            return None;
        }
        Some(Expression::Trapped {
            expression: Box::new(subject),
            binding,
            handler,
            line: token.line,
        })
    }

    pub(super) fn parse_if_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        let condition = self.parse_expression()?;
        if !self.consume_keyword(Keyword::Then, "IF statements must include THEN.") {
            return None;
        }

        if !self.is_statement_end() {
            let then_body = vec![self.parse_simple_statement(true)?];
            let else_body = if self.match_keyword(Keyword::Else) {
                vec![self.parse_simple_statement(false)?]
            } else {
                Vec::new()
            };
            return Some(Statement::If {
                condition,
                then_body,
                else_body,
                line: token.line,
            });
        }

        self.consume_statement_end("Expected end of statement after IF header.");
        self.skip_separators();
        let then_body = self.parse_statement_block(&[
            BlockTerminator::Else,
            BlockTerminator::ElseIf,
            BlockTerminator::EndIf,
        ]);
        let else_body = self.parse_if_tail();

        if !self.consume_end_block(Keyword::If, "IF block must end with END IF.") {
            return None;
        }

        Some(Statement::If {
            condition,
            then_body,
            else_body,
            line: token.line,
        })
    }

    pub(super) fn parse_if_tail(&mut self) -> Vec<Statement> {
        if self.match_keyword(Keyword::Else) {
            self.consume_statement_end("Expected end of statement after ELSE.");
            self.skip_separators();
            return self.parse_statement_block(&[BlockTerminator::EndIf]);
        }

        if self.match_keyword(Keyword::ElseIf) {
            let token = self.previous().clone();
            let Some(condition) = self.parse_expression() else {
                return Vec::new();
            };
            if !self.consume_keyword(Keyword::Then, "ELSEIF clauses must include THEN.") {
                return Vec::new();
            }
            self.consume_statement_end("Expected end of statement after ELSEIF header.");
            self.skip_separators();
            let then_body = self.parse_statement_block(&[
                BlockTerminator::Else,
                BlockTerminator::ElseIf,
                BlockTerminator::EndIf,
            ]);
            let else_body = self.parse_if_tail();
            return vec![Statement::If {
                condition,
                then_body,
                else_body,
                line: token.line,
            }];
        }

        Vec::new()
    }

    pub(super) fn parse_match_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        let expression = self.parse_expression()?;
        self.consume_statement_end("Expected end of statement after MATCH expression.");
        self.skip_separators();

        let mut cases = Vec::new();
        while !self.is_at_end() && !self.is_end_block(Keyword::Match) {
            if !self.match_keyword(Keyword::Case) {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_STATEMENT",
                    "MATCH blocks contain CASE clauses.",
                    &token,
                );
                self.synchronize();
                self.skip_separators();
                continue;
            }

            let case_token = self.previous().clone();
            let pattern = if self.match_keyword(Keyword::Else) {
                MatchPattern::Else
            } else {
                self.parse_match_pattern()?
            };
            let guard = if self.match_keyword(Keyword::When) {
                Some(self.parse_expression()?)
            } else {
                None
            };
            self.consume_statement_end("Expected end of statement after CASE pattern.");
            self.skip_separators();
            let body =
                self.parse_statement_block(&[BlockTerminator::Case, BlockTerminator::EndMatch]);
            cases.push(MatchCase {
                pattern,
                guard,
                body,
                line: case_token.line,
            });
        }

        if !self.consume_end_block(Keyword::Match, "MATCH block must end with END MATCH.") {
            return None;
        }

        Some(Statement::Match {
            expression,
            cases,
            line: token.line,
        })
    }

    pub(super) fn parse_match_pattern(&mut self) -> Option<MatchPattern> {
        if let Some(type_name) = self.try_parse_union_case_type() {
            if !self.consume_kind(
                TokenKind::LParen,
                "Union CASE patterns must bind one local with `(`.",
            ) {
                return None;
            }
            let binding =
                self.consume_identifier("Union CASE patterns must bind a local identifier.")?;
            if !self.consume_kind(
                TokenKind::RParen,
                "Union CASE pattern binding must close with `)`.",
            ) {
                return None;
            }
            return Some(MatchPattern::Union { type_name, binding });
        }

        let first = self.parse_expression()?;
        if !self.match_kind(TokenKind::Comma) {
            return Some(MatchPattern::Literal(first));
        }

        let mut patterns = vec![first];
        loop {
            patterns.push(self.parse_expression()?);
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }
        Some(MatchPattern::OneOf(patterns))
    }

    pub(super) fn try_parse_union_case_type(&mut self) -> Option<String> {
        if !matches!(self.peek().kind, TokenKind::Identifier(_)) {
            return None;
        }
        let saved = self.current;
        let name = self.parse_qualified_name("")?;
        if self.check_kind(&TokenKind::LParen) {
            Some(name)
        } else {
            self.current = saved;
            None
        }
    }

    pub(super) fn parse_for_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        if self.match_keyword(Keyword::Each) {
            return self.parse_for_each_statement(token);
        }
        let name = self.consume_identifier("FOR loop variable must be an identifier.")?;
        if !self.consume_kind(
            TokenKind::Equal,
            "FOR loop must assign the initial value with `=`.",
        ) {
            return None;
        }
        let start = self.parse_expression()?;
        if !self.consume_keyword(
            Keyword::To,
            "FOR loop must include TO before the end value.",
        ) {
            return None;
        }
        let end = self.parse_expression()?;
        let step = if self.match_keyword(Keyword::Step) {
            Some(self.parse_expression()?)
        } else {
            None
        };
        self.consume_statement_end("Expected end of statement after FOR header.");
        self.skip_separators();
        let body = self.parse_statement_block(&[BlockTerminator::Next]);
        if !self.consume_keyword(Keyword::Next, "FOR block must end with NEXT.") {
            return None;
        }
        self.consume_statement_end("Expected end of statement after NEXT.");
        Some(Statement::For {
            name,
            start,
            end,
            step,
            body,
            line: token.line,
        })
    }

    pub(super) fn parse_for_each_statement(&mut self, token: Token) -> Option<Statement> {
        let name = self.consume_identifier("FOR EACH loop variable must be an identifier.")?;
        if !self.consume_keyword(
            Keyword::In,
            "FOR EACH must include IN before the collection.",
        ) {
            return None;
        }
        let iterable = self.parse_expression()?;
        self.consume_statement_end("Expected end of statement after FOR EACH header.");
        self.skip_separators();
        let body = self.parse_statement_block(&[BlockTerminator::Next]);
        if !self.consume_keyword(Keyword::Next, "FOR EACH block must end with NEXT.") {
            return None;
        }
        self.consume_statement_end("Expected end of statement after NEXT.");
        Some(Statement::ForEach {
            name,
            iterable,
            body,
            line: token.line,
        })
    }

    pub(super) fn parse_while_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        let condition = self.parse_expression()?;
        self.consume_statement_end("Expected end of statement after WHILE header.");
        self.skip_separators();
        let body = self.parse_statement_block(&[BlockTerminator::Wend]);
        if !self.consume_keyword(Keyword::Wend, "WHILE block must end with WEND.") {
            return None;
        }
        self.consume_statement_end("Expected end of statement after WEND.");
        Some(Statement::While {
            kind: LoopKind::While,
            condition,
            body,
            line: token.line,
        })
    }

    pub(super) fn parse_do_statement(&mut self) -> Option<Statement> {
        let token = self.advance().clone();
        if self.match_keyword(Keyword::While) {
            let condition = self.parse_expression()?;
            self.consume_statement_end("Expected end of statement after DO WHILE header.");
            self.skip_separators();
            let body = self.parse_statement_block(&[BlockTerminator::Loop]);
            if !self.consume_keyword(Keyword::Loop, "DO WHILE block must end with LOOP.") {
                return None;
            }
            self.consume_statement_end("Expected end of statement after LOOP.");
            return Some(Statement::While {
                kind: LoopKind::Do,
                condition,
                body,
                line: token.line,
            });
        }

        self.consume_statement_end("Expected end of statement after DO.");
        self.skip_separators();
        let body = self.parse_statement_block(&[BlockTerminator::Loop]);
        if !self.consume_keyword(Keyword::Loop, "DO block must end with LOOP.") {
            return None;
        }
        if !self.consume_keyword(
            Keyword::Until,
            "DO blocks must end with LOOP UNTIL <condition>.",
        ) {
            return None;
        }
        let condition = self.parse_expression()?;
        self.consume_statement_end("Expected end of statement after LOOP UNTIL condition.");
        Some(Statement::DoUntil {
            body,
            condition,
            line: token.line,
        })
    }

    pub(super) fn parse_statement_block(
        &mut self,
        terminators: &[BlockTerminator],
    ) -> Vec<Statement> {
        let mut body = Vec::new();
        while !self.is_at_end() && !self.check_block_terminator(terminators) {
            if let Some(statement) = self.parse_statement() {
                body.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }
        body
    }

    pub(super) fn check_block_terminator(&self, terminators: &[BlockTerminator]) -> bool {
        terminators.iter().any(|terminator| match terminator {
            BlockTerminator::Case => self.check_keyword(Keyword::Case),
            BlockTerminator::Else => self.check_keyword(Keyword::Else),
            BlockTerminator::ElseIf => self.check_keyword(Keyword::ElseIf),
            BlockTerminator::EndIf => self.is_end_block(Keyword::If),
            BlockTerminator::EndMatch => self.is_end_block(Keyword::Match),
            BlockTerminator::Loop => self.check_keyword(Keyword::Loop),
            BlockTerminator::Next => self.check_keyword(Keyword::Next),
            BlockTerminator::Wend => self.check_keyword(Keyword::Wend),
        })
    }
}
