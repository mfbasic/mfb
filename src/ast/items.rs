use super::*;

impl<'a> FileParser<'a> {
    pub(super) fn parse_top_level_binding(
        &mut self,
        visibility: Visibility,
    ) -> Option<TopLevelBinding> {
        let keyword = self.advance().clone();
        let mutable = matches!(keyword.kind, TokenKind::Keyword(Keyword::Mut));
        let resource = matches!(keyword.kind, TokenKind::Keyword(Keyword::Res));
        let Some(name) = self.consume_identifier("Binding name must be an identifier.") else {
            self.synchronize();
            return None;
        };
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
            self.parse_expression()
        } else {
            None
        };
        self.consume_statement_end("Expected end of statement after binding.");
        Some(TopLevelBinding {
            visibility,
            mutable,
            resource,
            state_type,
            name,
            type_name,
            value,
            line: keyword.line,
        })
    }

    pub(super) fn parse_function(&mut self) -> Option<Function> {
        let isolated = self.match_keyword(Keyword::Isolated);
        let kind_token = self.advance().clone();
        let kind = if matches!(kind_token.kind, TokenKind::Keyword(Keyword::Sub)) {
            FunctionKind::Sub
        } else {
            FunctionKind::Func
        };
        if isolated && !matches!(kind, FunctionKind::Func) {
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "ISOLATED is valid only on FUNC declarations.",
                &kind_token,
            );
        }

        let Some(name) = self.consume_identifier("Function name must be an identifier.") else {
            self.synchronize();
            return None;
        };
        let template_params = self.parse_template_params();

        let params = if self.match_kind(TokenKind::LParen) {
            let params = self.parse_params();
            if !self.consume_kind(
                TokenKind::RParen,
                "Function declarations must close the parameter list.",
            ) {
                self.synchronize();
                return None;
            }
            params
        } else {
            Vec::new()
        };

        let (return_type, return_resource, return_state_type) =
            if matches!(kind, FunctionKind::Func) && self.match_keyword(Keyword::As) {
                let return_resource = self.match_keyword(Keyword::Res);
                let return_type = self.parse_type_name();
                let return_state_type = if return_resource {
                    self.parse_optional_state()
                } else {
                    None
                };
                (return_type, return_resource, return_state_type)
            } else {
                (None, false, None)
            };

        self.consume_statement_end("Expected end of function header.");
        self.skip_separators();

        let mut body = Vec::new();
        let mut trap = None;
        while !self.is_at_end() {
            if self.check_keyword(Keyword::Trap) {
                if trap.is_some() {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_STATEMENT",
                        "Each function may declare at most one TRAP.",
                        &token,
                    );
                    self.parse_trap();
                } else {
                    trap = self.parse_trap();
                }
                self.skip_separators();
                continue;
            }
            if self.check_keyword(Keyword::End) {
                self.advance();
                let expected = match kind {
                    FunctionKind::Func => Keyword::Func,
                    FunctionKind::Sub => Keyword::Sub,
                };
                if !self.consume_keyword(expected, "END must name the block kind it closes.") {
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END.");
                return Some(Function {
                    kind,
                    visibility: Visibility::Private,
                    isolated,
                    name,
                    template_params,
                    params,
                    return_type,
                    return_resource,
                    return_state_type,
                    body,
                    trap,
                    line: kind_token.line,
                });
            }

            if trap.is_some() {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_STATEMENT",
                    "TRAP must appear at the bottom of the function after normal flow.",
                    &token,
                );
            }

            if let Some(statement) = self.parse_statement() {
                body.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }

        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "Function block reached end-of-file before its END statement.",
            &kind_token,
        );
        None
    }

    pub(super) fn parse_trap(&mut self) -> Option<Trap> {
        let token = self.advance().clone();
        if !self.consume_kind(
            TokenKind::LParen,
            "TRAP must bind an error identifier with `TRAP(name)`.",
        ) {
            self.synchronize();
            return None;
        }
        let Some(name) = self.consume_identifier("TRAP must bind an error identifier.") else {
            self.synchronize();
            return None;
        };
        if !self.consume_kind(TokenKind::RParen, "TRAP error binding must close with `)`.") {
            self.synchronize();
            return None;
        }
        self.consume_statement_end("Expected end of statement after TRAP header.");
        self.skip_separators();

        let mut body = Vec::new();
        while !self.is_at_end() && !self.is_end_block(Keyword::Trap) {
            if let Some(statement) = self.parse_statement() {
                body.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }
        if !self.consume_end_block(Keyword::Trap, "TRAP block must end with END TRAP.") {
            return None;
        }
        Some(Trap {
            name,
            body,
            line: token.line,
        })
    }

    pub(super) fn parse_type_decl(&mut self) -> Option<TypeDecl> {
        let kind_token = self.advance().clone();
        let (kind, end_keyword) = match kind_token.kind {
            TokenKind::Keyword(Keyword::Type) => (TypeDeclKind::Type, Keyword::Type),
            TokenKind::Keyword(Keyword::Union) => (TypeDeclKind::Union, Keyword::Union),
            TokenKind::Keyword(Keyword::Enum) => (TypeDeclKind::Enum, Keyword::Enum),
            _ => unreachable!(),
        };
        let Some(name) = self.consume_identifier("Type declaration name must be an identifier.")
        else {
            self.synchronize();
            return None;
        };
        let template_params = if matches!(kind, TypeDeclKind::Enum) {
            Vec::new()
        } else {
            self.parse_template_params()
        };

        let includes =
            if matches!(kind, TypeDeclKind::Union) && self.check_identifier_ci("INCLUDES") {
                self.advance();
                self.parse_union_includes()
            } else {
                Vec::new()
            };

        self.consume_statement_end("Expected end of type declaration header.");
        self.skip_separators();

        let mut fields = Vec::new();
        let mut variants = Vec::new();
        let mut members = Vec::new();
        while !self.is_at_end() {
            if self.match_keyword(Keyword::End) {
                if !self
                    .consume_keyword(end_keyword, "END must name the type block kind it closes.")
                {
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END.");
                return Some(TypeDecl {
                    kind,
                    visibility: Visibility::Private,
                    name,
                    template_params,
                    fields,
                    includes,
                    variants,
                    members,
                    line: kind_token.line,
                });
            }

            match kind {
                TypeDeclKind::Type => {
                    if let Some(field) = self.parse_type_field() {
                        fields.push(field);
                    } else {
                        self.synchronize();
                    }
                }
                TypeDeclKind::Union => {
                    if let Some(variant) = self.parse_union_variant() {
                        variants.push(variant);
                    } else {
                        self.synchronize();
                    }
                }
                TypeDeclKind::Enum => {
                    let parsed = self.parse_enum_members();
                    if parsed.is_empty() {
                        self.synchronize();
                    }
                    members.extend(parsed);
                }
            }
            self.skip_separators();
        }

        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "Type block reached end-of-file before its END statement.",
            &kind_token,
        );
        None
    }

    pub(super) fn parse_union_includes(&mut self) -> Vec<String> {
        let mut includes = Vec::new();
        loop {
            if let Some(name) = self.parse_qualified_name("Expected a union name after INCLUDES.") {
                includes.push(name);
            }
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }
        includes
    }

    pub(super) fn parse_template_params(&mut self) -> Vec<String> {
        if !self.check_identifier_ci("OF") {
            return Vec::new();
        }
        self.advance();
        let mut params = Vec::new();
        loop {
            if let Some(name) =
                self.consume_identifier("Expected template parameter name after OF.")
            {
                params.push(name);
            } else {
                self.synchronize();
                break;
            }
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }
        params
    }

    pub(super) fn parse_type_field(&mut self) -> Option<TypeField> {
        let line = self.peek().line;
        let visibility = self.parse_visibility();
        let name = self.consume_identifier("Field name must be an identifier.")?;
        if !self.consume_keyword(Keyword::As, "Field declarations must include an `AS` type.") {
            return None;
        }
        let type_name = self.parse_type_name()?;
        self.consume_statement_end("Expected end of statement after field declaration.");
        Some(TypeField {
            visibility,
            name,
            type_name,
            line,
        })
    }

    pub(super) fn parse_union_variant(&mut self) -> Option<UnionVariant> {
        let line = self.peek().line;
        let name = self.parse_qualified_name("Union member type must be a type name.")?;
        self.consume_statement_end("Expected end of statement after union member type.");
        Some(UnionVariant { name, line })
    }

    pub(super) fn parse_enum_members(&mut self) -> Vec<EnumMember> {
        let mut members = Vec::new();
        loop {
            let line = self.peek().line;
            let Some(name) = self.consume_identifier("Enum member name must be an identifier.")
            else {
                break;
            };
            members.push(EnumMember { name, line });
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }
        self.consume_statement_end("Expected end of statement after enum member declaration.");
        members
    }

    pub(super) fn parse_params(&mut self) -> Vec<Param> {
        let mut params = Vec::new();
        if self.check_kind(&TokenKind::RParen) {
            return params;
        }

        loop {
            let line = self.peek().line;
            let resource = self.match_keyword(Keyword::Res);
            let Some(name) = self.consume_identifier("Parameter name must be an identifier.")
            else {
                self.synchronize();
                return params;
            };
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
            let default = if self.match_kind(TokenKind::Equal) {
                self.parse_expression()
            } else {
                None
            };
            params.push(Param {
                name,
                type_name,
                resource,
                state_type,
                default,
                line,
            });
            if !self.match_kind(TokenKind::Comma) {
                break;
            }
        }

        params
    }

    pub(super) fn parse_visibility(&mut self) -> Option<Visibility> {
        if self.match_keyword(Keyword::Private) {
            Some(Visibility::Private)
        } else if self.match_keyword(Keyword::Public) {
            Some(Visibility::Public)
        } else if self.match_keyword(Keyword::Export) {
            Some(Visibility::Export)
        } else {
            None
        }
    }

    pub(super) fn check_top_level_item_start(&self) -> bool {
        self.check_keyword(Keyword::Sub)
            || self.check_keyword(Keyword::Func)
            || (self.check_keyword(Keyword::Isolated)
                && self
                    .tokens
                    .get(self.current + 1)
                    .is_some_and(|token| matches!(token.kind, TokenKind::Keyword(Keyword::Func))))
            || (self.check_visibility()
                && self.tokens.get(self.current + 1).is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenKind::Keyword(Keyword::Sub)
                            | TokenKind::Keyword(Keyword::Func)
                            | TokenKind::Keyword(Keyword::Isolated)
                    )
                }))
    }

    pub(super) fn check_top_level_type_start(&self) -> bool {
        self.check_keyword(Keyword::Type)
            || self.check_keyword(Keyword::Union)
            || self.check_keyword(Keyword::Enum)
            || (self.check_visibility()
                && self.tokens.get(self.current + 1).is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenKind::Keyword(Keyword::Type)
                            | TokenKind::Keyword(Keyword::Union)
                            | TokenKind::Keyword(Keyword::Enum)
                    )
                }))
    }

    pub(super) fn check_top_level_binding_start(&self) -> bool {
        self.check_keyword(Keyword::Let)
            || self.check_keyword(Keyword::Mut)
            || self.check_keyword(Keyword::Res)
            || (self.check_visibility()
                && self.tokens.get(self.current + 1).is_some_and(|token| {
                    matches!(
                        token.kind,
                        TokenKind::Keyword(Keyword::Let)
                            | TokenKind::Keyword(Keyword::Mut)
                            | TokenKind::Keyword(Keyword::Res)
                    )
                }))
    }

    pub(super) fn check_top_level_resource_start(&self) -> bool {
        self.check_identifier_ci("RESOURCE")
            || (self.check_visibility()
                && self.peek_next().is_some_and(|token| {
                    matches!(&token.kind, TokenKind::Identifier(value) if value.eq_ignore_ascii_case("RESOURCE"))
                }))
    }

    pub(super) fn check_top_level_link_start(&self) -> bool {
        self.check_identifier_ci("LINK")
    }

    /// Detect a function-alias item: `[vis] FUNC name AS qualified::func`. The
    /// `::`-qualified target distinguishes the alias (plan-link-update.md §5a)
    /// from an ordinary function declaration with a body.
    pub(super) fn check_top_level_func_alias(&self) -> bool {
        let mut index = self.current;
        if matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Keyword(
                Keyword::Private | Keyword::Public | Keyword::Export
            ))
        ) {
            index += 1;
        }
        if !matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Keyword(Keyword::Func))
        ) {
            return false;
        }
        index += 1;
        if !matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Identifier(_))
        ) {
            return false;
        }
        index += 1;
        if !matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Keyword(Keyword::As))
        ) {
            return false;
        }
        index += 1;
        if !matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::Identifier(_))
        ) {
            return false;
        }
        index += 1;
        matches!(
            self.tokens.get(index).map(|token| &token.kind),
            Some(TokenKind::DoubleColon)
        )
    }

    pub(super) fn parse_top_level_resource(&mut self) -> Option<ResourceDecl> {
        let visibility = self.parse_visibility().unwrap_or(Visibility::Public);
        let keyword = self.advance().clone(); // the `RESOURCE` contextual keyword
        let Some(name) = self.consume_identifier("Resource name must be an identifier.") else {
            self.synchronize();
            return None;
        };
        if !self.consume_contextual(
            "CLOSE",
            "RESOURCE declaration requires `CLOSE BY <closeFn>`.",
        ) {
            self.synchronize();
            return None;
        }
        if !self.consume_contextual("BY", "RESOURCE `CLOSE` must be followed by `BY`.") {
            self.synchronize();
            return None;
        }
        let Some(close_fn) = self.parse_qualified_name("Expected a close op after `CLOSE BY`.")
        else {
            self.synchronize();
            return None;
        };
        let thread_sendable = self.match_identifier_ci("THREAD_SENDABLE");
        self.consume_statement_end("Expected end of statement after RESOURCE declaration.");
        Some(ResourceDecl {
            visibility,
            name,
            close_fn,
            thread_sendable,
            line: keyword.line,
        })
    }

    pub(super) fn parse_top_level_func_alias(&mut self) -> Option<FuncAlias> {
        let visibility = self.parse_visibility().unwrap_or(Visibility::Public);
        let func_token = self.advance().clone(); // FUNC
        let Some(name) = self.consume_identifier("Function alias name must be an identifier.")
        else {
            self.synchronize();
            return None;
        };
        if !self.consume_keyword(Keyword::As, "Function alias requires `AS qualified::func`.") {
            self.synchronize();
            return None;
        }
        let Some(target) = self.parse_qualified_name("Expected `qualified::func` after `AS`.")
        else {
            self.synchronize();
            return None;
        };
        self.consume_statement_end("Expected end of statement after function alias.");
        Some(FuncAlias {
            visibility,
            name,
            target,
            line: func_token.line,
        })
    }

    pub(super) fn parse_link_block(&mut self) -> Option<LinkBlock> {
        let keyword = self.advance().clone(); // the `LINK` contextual keyword
        let library = match self.peek().kind.clone() {
            TokenKind::String(value) => {
                self.advance();
                value
            }
            _ => {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    "LINK requires a native library name string, e.g. `LINK \"sqlite3\" AS ...`.",
                    &token,
                );
                self.synchronize();
                return None;
            }
        };
        if !self.consume_keyword(Keyword::As, "LINK requires `AS <alias>`.") {
            self.synchronize();
            return None;
        }
        let Some(alias) = self.consume_identifier("Expected a LINK alias name after `AS`.") else {
            self.synchronize();
            return None;
        };
        self.consume_statement_end("Expected end of statement after LINK header.");
        self.skip_separators();

        let mut functions = Vec::new();
        while !self.is_at_end() {
            if self.is_end_link() {
                self.advance(); // END
                self.advance(); // LINK
                self.consume_statement_end("Expected end of statement after END LINK.");
                return Some(LinkBlock {
                    library,
                    alias,
                    functions,
                    line: keyword.line,
                });
            }
            if self.check_keyword(Keyword::Func) {
                if let Some(function) = self.parse_link_function() {
                    functions.push(function);
                } else {
                    self.synchronize();
                }
                self.skip_separators();
                continue;
            }
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "A LINK block may only contain native FUNC declarations.",
                &token,
            );
            self.synchronize();
            self.skip_separators();
        }
        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "LINK block reached end-of-file before its END LINK statement.",
            &keyword,
        );
        None
    }

    pub(super) fn parse_link_function(&mut self) -> Option<LinkFunction> {
        let func_token = self.advance().clone(); // FUNC
                                                 // A native function may be named after a keyword (e.g. `step`, which
                                                 // collides with `STEP`); accept a keyword token in this name position.
        let Some(name) =
            self.consume_name_or_keyword("Native function name must be an identifier.")
        else {
            self.synchronize();
            return None;
        };
        let params = if self.match_kind(TokenKind::LParen) {
            let params = self.parse_params();
            if !self.consume_kind(
                TokenKind::RParen,
                "Native function declarations must close the parameter list.",
            ) {
                self.synchronize();
                return None;
            }
            params
        } else {
            Vec::new()
        };
        let (return_type, return_resource) = if self.match_keyword(Keyword::As) {
            let return_resource = self.match_keyword(Keyword::Res);
            (self.parse_type_name(), return_resource)
        } else {
            (None, false)
        };
        self.consume_statement_end("Expected end of native function header.");
        self.skip_separators();

        let mut symbol: Option<String> = None;
        let mut abi: Option<AbiSpec> = None;
        let mut consts = Vec::new();
        let mut success_on: Option<Expression> = None;
        let mut result: Option<Expression> = None;
        let mut free: Option<FreeSpec> = None;

        while !self.is_at_end() {
            if self.check_keyword(Keyword::End) {
                self.advance(); // END
                if !self.consume_keyword(
                    Keyword::Func,
                    "END must name the block kind it closes (END FUNC).",
                ) {
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END FUNC.");
                break;
            }
            if self.match_identifier_ci("SYMBOL") {
                symbol = self.parse_string_literal("SYMBOL requires a native symbol name string.");
                self.consume_statement_end("Expected end of statement after SYMBOL.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("ABI") {
                abi = self.parse_abi_spec();
                self.consume_statement_end("Expected end of statement after ABI.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("CONST") {
                if let Some(pin) = self.parse_const_pin() {
                    consts.push(pin);
                }
                self.consume_statement_end("Expected end of statement after CONST.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("SUCCESS_ON") {
                success_on = self.parse_expression();
                self.consume_statement_end("Expected end of statement after SUCCESS_ON.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("ERROR_ON") {
                // ERROR_ON is the De Morgan complement of SUCCESS_ON; store the
                // negation so downstream stages see a single success condition.
                let (error_line, error_column) = (self.previous().line, self.previous().start);
                if let Some(expr) = self.parse_expression() {
                    success_on = Some(Expression::Unary {
                        operator: "NOT".to_string(),
                        operand: Box::new(expr),
                        line: error_line,
                        column: error_column,
                    });
                }
                self.consume_statement_end("Expected end of statement after ERROR_ON.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("RESULT") {
                result = self.parse_expression();
                self.consume_statement_end("Expected end of statement after RESULT.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("FREE") {
                free = self.parse_free_block();
                self.skip_separators();
                continue;
            }
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "A native FUNC body may only contain SYMBOL, ABI, CONST, SUCCESS_ON, ERROR_ON, RESULT, or FREE clauses.",
                &token,
            );
            self.synchronize();
            self.skip_separators();
        }

        let Some(symbol) = symbol else {
            self.report(
                "MFB_PARSE_MISSING_NATIVE_SYMBOL",
                "A native FUNC must declare its native SYMBOL.",
                &func_token,
            );
            return None;
        };
        let Some(abi) = abi else {
            self.report(
                "MFB_PARSE_MISSING_NATIVE_ABI",
                "A native FUNC must declare its ABI signature.",
                &func_token,
            );
            return None;
        };

        Some(LinkFunction {
            name,
            params,
            return_type,
            return_resource,
            symbol,
            abi,
            consts,
            success_on,
            result,
            free,
            line: func_token.line,
        })
    }

    /// Parse a `FREE <slot> SYMBOL "…" ABI (ptr CPtr) AS <ctype> END FREE` block.
    /// The opening `FREE` keyword has already been consumed.
    pub(super) fn parse_free_block(&mut self) -> Option<FreeSpec> {
        let line = self.previous().line;
        let slot = self.parse_abi_slot_name()?;
        self.consume_statement_end("Expected end of statement after FREE <slot>.");
        self.skip_separators();

        let mut symbol: Option<String> = None;
        let mut param: Option<(String, String)> = None;
        let mut return_ctype: Option<String> = None;

        while !self.is_at_end() {
            if self.check_keyword(Keyword::End) {
                self.advance(); // END
                if !self.match_identifier_ci("FREE") {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        "END must name the block kind it closes (END FREE).",
                        &token,
                    );
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END FREE.");
                break;
            }
            if self.match_identifier_ci("SYMBOL") {
                symbol = self.parse_string_literal("SYMBOL requires a native symbol name string.");
                self.consume_statement_end("Expected end of statement after SYMBOL.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("ABI") {
                if !self.consume_kind(
                    TokenKind::LParen,
                    "FREE ABI requires a `(` to open its slot.",
                ) {
                    self.synchronize();
                    return None;
                }
                let param_name = self.parse_abi_slot_name()?;
                let param_ctype = self.parse_c_type_name()?;
                if !self.consume_kind(TokenKind::RParen, "FREE ABI slot must close with `)`.") {
                    self.synchronize();
                    return None;
                }
                if !self.consume_keyword(
                    Keyword::As,
                    "FREE ABI requires `AS <ctype>` for the deallocator return.",
                ) {
                    self.synchronize();
                    return None;
                }
                return_ctype = self.parse_c_type_name();
                param = Some((param_name, param_ctype));
                self.consume_statement_end("Expected end of statement after FREE ABI.");
                self.skip_separators();
                continue;
            }
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "A FREE block may only contain SYMBOL and ABI clauses.",
                &token,
            );
            self.synchronize();
            self.skip_separators();
        }

        let symbol = symbol?;
        let (param_name, param_ctype) = param?;
        let return_ctype = return_ctype?;
        Some(FreeSpec {
            slot,
            symbol,
            param_name,
            param_ctype,
            return_ctype,
            line,
        })
    }

    pub(super) fn parse_abi_spec(&mut self) -> Option<AbiSpec> {
        let line = self.previous().line;
        if !self.consume_kind(
            TokenKind::LParen,
            "ABI requires a `(` to open its slot list.",
        ) {
            self.synchronize();
            return None;
        }
        let mut slots = Vec::new();
        if !self.check_kind(&TokenKind::RParen) {
            loop {
                let slot_line = self.peek().line;
                let name = self.parse_abi_slot_name()?;
                let is_out = self.match_identifier_ci("OUT");
                let ctype = self.parse_c_type_name()?;
                slots.push(AbiSlot {
                    name,
                    ctype,
                    is_out,
                    line: slot_line,
                });
                if !self.match_kind(TokenKind::Comma) {
                    break;
                }
            }
        }
        if !self.consume_kind(TokenKind::RParen, "ABI slot list must close with `)`.") {
            self.synchronize();
            return None;
        }
        if !self.consume_keyword(
            Keyword::As,
            "ABI requires `AS <name> <ctype>` for the native return.",
        ) {
            self.synchronize();
            return None;
        }
        let return_name = self.parse_abi_slot_name()?;
        let return_ctype = self.parse_c_type_name()?;
        Some(AbiSpec {
            slots,
            return_name,
            return_ctype,
            line,
        })
    }

    /// Parse an ABI slot name: an identifier, or the `return` keyword (the
    /// wrapper-result marker, plan-link-update.md §5b).
    pub(super) fn parse_abi_slot_name(&mut self) -> Option<String> {
        if self.match_keyword(Keyword::Return) {
            return Some("return".to_string());
        }
        self.consume_identifier("Expected an ABI slot name.")
    }

    pub(super) fn parse_c_type_name(&mut self) -> Option<String> {
        self.consume_identifier("Expected an ABI slot C type (e.g. CPtr, CString, CInt32).")
    }

    pub(super) fn parse_const_pin(&mut self) -> Option<ConstPin> {
        let line = self.peek().line;
        let Some(slot) = self.consume_identifier("CONST requires an ABI slot name.") else {
            self.synchronize();
            return None;
        };
        if !self.consume_kind(TokenKind::Equal, "CONST requires `= <value>`.") {
            self.synchronize();
            return None;
        }
        let value = self.parse_expression()?;
        Some(ConstPin { slot, value, line })
    }

    pub(super) fn parse_string_literal(&mut self, detail: &str) -> Option<String> {
        match self.peek().kind.clone() {
            TokenKind::String(value) => {
                self.advance();
                Some(value)
            }
            _ => {
                let token = self.peek().clone();
                self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
                None
            }
        }
    }

    /// Parse an optional `STATE T` clause that follows a `RES` type. `STATE` is a
    /// contextual keyword (so `state` remains usable as an identifier).
    pub(super) fn parse_optional_state(&mut self) -> Option<String> {
        if self.check_identifier_ci("STATE") {
            self.advance();
            self.parse_type_name()
        } else {
            None
        }
    }

    /// Structurally parse a `DOC` block captured by the lexer. Semantic checks
    /// (name resolution, duplicate/context rules, attribute validity) belong to
    /// the resolver; this only shapes the raw lines into a `DocBlock`.
    pub(super) fn parse_doc_block(&mut self, raw: lexer::DocRaw) -> Option<DocBlock> {
        let lexer::DocRaw { line, attrs, lines } = raw;

        // The header is the first non-blank body line.
        let header_index = lines.iter().position(|l| !l.text.trim().is_empty());
        let Some(header_index) = header_index else {
            self.report_at(
                "DOC_BAD_HEADER",
                "DOC block has no header line; expected FUNC, SUB, TYPE, UNION, ENUM, or PACKAGE.",
                line,
            );
            return None;
        };
        let header_line = lines[header_index].line;
        let (head_kw, head_rest) = split_first_word(lines[header_index].text.trim());
        let header_kind = match head_kw.to_ascii_uppercase().as_str() {
            "FUNC" => DocHeaderKind::Func,
            "SUB" => DocHeaderKind::Sub,
            "TYPE" => DocHeaderKind::Type,
            "UNION" => DocHeaderKind::Union,
            "ENUM" => DocHeaderKind::Enum,
            "PACKAGE" => DocHeaderKind::Package,
            _ => {
                self.report_at(
                    "DOC_BAD_HEADER",
                    &format!(
                        "`{head_kw}` is not a valid DOC header; expected FUNC, SUB, TYPE, UNION, ENUM, or PACKAGE."
                    ),
                    header_line,
                );
                return None;
            }
        };
        // FUNC/SUB headers may carry a parenthesized parameter-type list to pick a
        // specific overload, e.g. `FUNC query(Db, String, List OF String)`.
        let callable = matches!(header_kind, DocHeaderKind::Func | DocHeaderKind::Sub);
        let (header_name, header_params) = if callable {
            parse_header_signature(head_rest.trim())
        } else {
            (head_rest.trim().to_string(), None)
        };
        if header_kind == DocHeaderKind::Package {
            if !header_name.is_empty() {
                self.report_at(
                    "DOC_BAD_HEADER",
                    "A PACKAGE doc header takes no name.",
                    header_line,
                );
                return None;
            }
        } else if header_name.is_empty() {
            self.report_at(
                "DOC_BAD_HEADER",
                &format!(
                    "A {} doc header must name a declaration.",
                    header_kind.keyword()
                ),
                header_line,
            );
            return None;
        }

        let mut desc: Vec<DocProse> = Vec::new();
        // Accumulator for the current prose block: its kind plus the lines joined
        // so far. Consecutive lines of the same kind concatenate; a blank line, a
        // different prose kind, or any structured line flushes it.
        let mut current: Option<(DocProseKind, Vec<String>)> = None;
        let mut deprecated = Vec::new();
        let mut groups = Vec::new();
        let mut args = Vec::new();
        let mut rets = Vec::new();
        let mut errors = Vec::new();
        let mut props = Vec::new();
        let mut examples = Vec::new();

        let flush = |current: &mut Option<(DocProseKind, Vec<String>)>,
                     desc: &mut Vec<DocProse>| {
            if let Some((kind, parts)) = current.take() {
                if !parts.is_empty() {
                    desc.push(DocProse {
                        kind,
                        text: parts.join(" "),
                    });
                }
            }
        };

        let mut index = header_index + 1;
        while index < lines.len() {
            let line_no = lines[index].line;
            let raw_text = &lines[index].text;
            let trimmed = raw_text.trim();
            if trimmed.is_empty() {
                flush(&mut current, &mut desc);
                index += 1;
                continue;
            }
            let (kw, rest) = split_first_word(trimmed);
            if let Some(kind) = DocProseKind::from_keyword(kw) {
                // A prose keyword (DESC/WARN/INFO/SEC). Switching kinds or a blank
                // line ends the current block.
                if current.as_ref().is_some_and(|(k, _)| *k != kind) {
                    flush(&mut current, &mut desc);
                }
                if rest.trim().is_empty() {
                    flush(&mut current, &mut desc);
                } else {
                    current
                        .get_or_insert_with(|| (kind, Vec::new()))
                        .1
                        .push(rest.trim().to_string());
                }
                index += 1;
                continue;
            }
            match kw.to_ascii_uppercase().as_str() {
                "DEPRECATED" => {
                    flush(&mut current, &mut desc);
                    deprecated.push((rest.trim().to_string(), line_no));
                }
                "GROUP" => {
                    flush(&mut current, &mut desc);
                    groups.push((rest.trim().to_string(), line_no));
                }
                "RET" => {
                    flush(&mut current, &mut desc);
                    rets.push((rest.trim().to_string(), line_no));
                }
                "ARG" => {
                    flush(&mut current, &mut desc);
                    let (name, adesc) = split_first_word(rest.trim());
                    if name.is_empty() {
                        self.report_at(
                            "DOC_UNKNOWN_LINE",
                            "ARG line must name a parameter.",
                            line_no,
                        );
                    } else {
                        args.push(DocNamed {
                            name: name.to_string(),
                            desc: adesc.trim().to_string(),
                            line: line_no,
                        });
                    }
                }
                "PROP" => {
                    flush(&mut current, &mut desc);
                    let (name, pdesc) = split_first_word(rest.trim());
                    if name.is_empty() {
                        self.report_at(
                            "DOC_UNKNOWN_LINE",
                            "PROP line must name a member.",
                            line_no,
                        );
                    } else {
                        props.push(DocNamed {
                            name: name.to_string(),
                            desc: pdesc.trim().to_string(),
                            line: line_no,
                        });
                    }
                }
                "ERROR" => {
                    flush(&mut current, &mut desc);
                    let (code, edesc) = split_first_word(rest.trim());
                    if code.is_empty() {
                        self.report_at(
                            "DOC_UNKNOWN_LINE",
                            "ERROR line must name an error code.",
                            line_no,
                        );
                    } else {
                        errors.push(DocError {
                            code: code.to_string(),
                            desc: edesc.trim().to_string(),
                            line: line_no,
                        });
                    }
                }
                "EXAMPLE" => {
                    flush(&mut current, &mut desc);
                    // Collect verbatim lines until `END EXAMPLE`.
                    let mut body: Vec<&str> = Vec::new();
                    let mut closed = false;
                    index += 1;
                    while index < lines.len() {
                        let t = lines[index].text.trim();
                        let words: Vec<&str> = t.split_whitespace().collect();
                        if words.len() == 2
                            && words[0].eq_ignore_ascii_case("END")
                            && words[1].eq_ignore_ascii_case("EXAMPLE")
                        {
                            closed = true;
                            break;
                        }
                        body.push(&lines[index].text);
                        index += 1;
                    }
                    if !closed {
                        self.report_at(
                            "DOC_EXAMPLE_UNTERMINATED",
                            "EXAMPLE block reached END DOC before its `END EXAMPLE` line.",
                            line_no,
                        );
                    }
                    examples.push((dedent(&body), line_no));
                }
                _ => {
                    self.report_at(
                        "DOC_UNKNOWN_LINE",
                        &format!(
                            "`{kw}` is not a valid DOC line; expected DESC, WARN, INFO, SEC, DEPRECATED, GROUP, ARG, RET, ERROR, PROP, or EXAMPLE."
                        ),
                        line_no,
                    );
                }
            }
            index += 1;
        }
        flush(&mut current, &mut desc);

        Some(DocBlock {
            line,
            attrs,
            header_kind,
            header_name,
            header_params,
            header_line,
            desc,
            deprecated,
            groups,
            args,
            rets,
            errors,
            props,
            examples,
        })
    }
}

/// Parse a FUNC/SUB doc header's name and optional parenthesized parameter-type
/// disambiguator: `name` -> (name, None); `name(T1, T2)` -> (name, Some([T1, T2])).
/// Type strings are whitespace-normalized; commas inside nested parens (function
/// types) are not split on.
fn parse_header_signature(text: &str) -> (String, Option<Vec<String>>) {
    let Some(open) = text.find('(') else {
        return (text.trim().to_string(), None);
    };
    let name = text[..open].trim().to_string();
    let rest = &text[open + 1..];
    // Find the matching close paren, tracking nesting.
    let mut depth = 1usize;
    let mut end = rest.len();
    for (idx, ch) in rest.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    end = idx;
                    break;
                }
            }
            _ => {}
        }
    }
    let inner = &rest[..end];
    if inner.trim().is_empty() {
        return (name, Some(Vec::new()));
    }
    // Split on top-level commas.
    let mut params = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    let bytes = inner.as_bytes();
    for (idx, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                params.push(normalize_ws(&inner[start..idx]));
                start = idx + 1;
            }
            _ => {}
        }
    }
    params.push(normalize_ws(&inner[start..]));
    (name, Some(params))
}

/// Collapse internal whitespace runs to single spaces and trim.
pub fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Split a trimmed line into its first whitespace-delimited word and the rest.
fn split_first_word(text: &str) -> (&str, &str) {
    let text = text.trim_start();
    match text.find(char::is_whitespace) {
        Some(idx) => (&text[..idx], text[idx..].trim_start()),
        None => (text, ""),
    }
}

/// Strip the common leading indentation from EXAMPLE body lines and join them.
///
/// Indentation is measured and stripped in **characters**, not bytes. `trim_start`
/// is Unicode-whitespace-aware, so a byte-count minimum taken across lines indented
/// with different-width whitespace (a space on one line, U+00A0 on another) could
/// land inside a multibyte char and panic the byte slice `l[min_indent..]` with
/// "byte index N is not a char boundary" (bug-19). A char prefix is also the
/// semantically intended "common indentation".
fn dedent(lines: &[&str]) -> String {
    let leading_whitespace = |l: &str| l.chars().take_while(|c| c.is_whitespace()).count();
    let min_indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| leading_whitespace(l))
        .min()
        .unwrap_or(0);
    lines
        .iter()
        .map(|l| {
            // Only a blank line can be shorter than the common indentation (it is
            // excluded from the minimum); trim it away entirely.
            let mut chars = l.chars();
            if chars.by_ref().take(min_indent).count() == min_indent {
                chars.as_str().trim_end().to_string()
            } else {
                l.trim().to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim_matches('\n')
        .to_string()
}
