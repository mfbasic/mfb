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
        // Anything that is not FUNC or SUB is rejected outright (bug-292). The
        // `else` used to default to `FunctionKind::Func`, so any garbage token in
        // this position was silently treated as a function header —
        // `PUBLIC ISOLATED BOGUS name(...)` compiled and linked as if it read
        // FUNC. Only `check_top_level_item_start` guarded the visibility-less
        // spelling, so the misparse was reachable exactly through a visibility or
        // ISOLATED prefix.
        let kind = match kind_token.kind {
            TokenKind::Keyword(Keyword::Sub) => FunctionKind::Sub,
            TokenKind::Keyword(Keyword::Func) => FunctionKind::Func,
            _ => {
                self.report(
                    "MFB_PARSE_UNEXPECTED_TOKEN",
                    "A function declaration must begin with FUNC or SUB.",
                    &kind_token,
                );
                self.synchronize();
                return None;
            }
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
        // The `(ident)` binding is optional: a bare `TRAP` synthesizes a
        // reserved, non-collidable name so the caught error stays internally
        // bound (PROPAGATE and slot-keyed cleanup keep working) while the user
        // has no name for it.
        let name = if self.check_kind(&TokenKind::LParen) {
            self.advance();
            let Some(name) = self.consume_identifier("TRAP must bind an error identifier.") else {
                self.synchronize();
                return None;
            };
            if !self.consume_kind(TokenKind::RParen, "TRAP error binding must close with `)`.") {
                self.synchronize();
                return None;
            }
            name
        } else {
            SYNTHETIC_TRAP_BINDING.to_string()
        };
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
}

/// Collapse internal whitespace runs to single spaces and trim.
pub fn normalize_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
