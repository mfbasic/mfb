use super::*;

impl<'a> FileParser<'a> {
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
        let mut cstructs = Vec::new();
        while !self.is_at_end() {
            if self.is_end_link() {
                self.advance(); // END
                self.advance(); // LINK
                self.consume_statement_end("Expected end of statement after END LINK.");
                return Some(LinkBlock {
                    library,
                    alias,
                    functions,
                    cstructs,
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
            if self.match_identifier_ci("CSTRUCT") {
                if let Some(cstruct) = self.parse_cstruct() {
                    cstructs.push(cstruct);
                } else {
                    self.synchronize();
                }
                self.skip_separators();
                continue;
            }
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "A LINK block may only contain native FUNC and CSTRUCT declarations.",
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

    /// `CSTRUCT <CName> AS <MfbType>` / `<field> <ctype>`… / `END CSTRUCT`
    /// (plan-50-B). The `CSTRUCT` identifier has already been consumed.
    ///
    /// Fields are read in **C declaration order** — the order is load-bearing,
    /// since it drives the computed offsets. There is deliberately no offset,
    /// size, or padding syntax: layout is computed, never declared.
    pub(super) fn parse_cstruct(&mut self) -> Option<CStructDecl> {
        let line = self.previous().line;
        let name = self.consume_identifier("CSTRUCT requires a C struct name.")?;
        if !self.consume_keyword(
            Keyword::As,
            "CSTRUCT requires `AS <Type>` naming the MFBASIC record it maps to.",
        ) {
            self.synchronize();
            return None;
        }
        let maps_to =
            self.consume_identifier("CSTRUCT `AS` requires the MFBASIC record type name.")?;
        self.consume_statement_end("Expected end of statement after the CSTRUCT header.");
        self.skip_separators();

        let mut fields = Vec::new();
        while !self.is_at_end() {
            if self.check_keyword(Keyword::End) {
                self.advance(); // END
                if !self.match_identifier_ci("CSTRUCT") {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        "END must name the block kind it closes (END CSTRUCT).",
                        &token,
                    );
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END CSTRUCT.");
                return Some(CStructDecl {
                    name,
                    maps_to,
                    fields,
                    line,
                });
            }
            let field_line = self.peek().line;
            let Some(field_name) = self.consume_identifier("Expected a CSTRUCT field name.") else {
                self.synchronize();
                self.skip_separators();
                continue;
            };
            let Some(ctype) = self.parse_c_type_name() else {
                self.synchronize();
                self.skip_separators();
                continue;
            };
            self.consume_statement_end("Expected end of statement after a CSTRUCT field.");
            self.skip_separators();
            fields.push(CStructField {
                name: field_name,
                ctype,
                line: field_line,
            });
        }
        let token = self.peek().clone();
        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "CSTRUCT block reached end-of-file before its END CSTRUCT statement.",
            &token,
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
        let (return_type, return_resource, return_state_type) = if self.match_keyword(Keyword::As) {
            let return_resource = self.match_keyword(Keyword::Res);
            let return_type = self.parse_type_name();
            // A `RES` return may carry a `STATE T` clause (plan-53-A): the native
            // func produces a resource record holding a `T` payload. Only after
            // `RES`, mirroring the ordinary-func and param rules — a bare native
            // return has no STATE (§15.5's escape rule holds at the native boundary
            // too: only a `RES` producer can carry state out).
            let return_state_type = if return_resource {
                self.parse_optional_state()
            } else {
                None
            };
            (return_type, return_resource, return_state_type)
        } else {
            (None, false, None)
        };
        self.consume_statement_end("Expected end of native function header.");
        self.skip_separators();

        let mut symbol: Option<String> = None;
        let mut abi: Option<AbiSpec> = None;
        let mut consts = Vec::new();
        let mut success_on: Option<Expression> = None;
        let mut result: Option<Expression> = None;
        let mut bind_in: Vec<BindIn> = Vec::new();
        let mut bind_state: Option<BindState> = None;
        let mut buffers: Vec<BufferSpec> = Vec::new();
        let mut result_length: Option<Expression> = None;
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
            // plan-50-H: `RETURN <expr>` is the ONE result clause. `RETURN db` is
            // the degenerate case where the expression is a bare slot reference;
            // `RETURN status = 100` is the computed case that used to spell
            // RESULT. No grammar ambiguity: a native FUNC body holds clauses, not
            // statements, so `Keyword::Return` here can only be this clause.
            if self.match_keyword(Keyword::Return) {
                result = self.parse_expression();
                // plan-58-B: `RETURN <expr> LENGTH <expr>`. `LENGTH` is an
                // ordinary identifier, not an operator, so `parse_expression`
                // stops cleanly before it and this is unambiguous.
                if self.match_identifier_ci("LENGTH") {
                    result_length = self.parse_expression();
                }
                self.consume_statement_end("Expected end of statement after RETURN.");
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("BIND") {
                // `BIND STATE <res> = <slot>` (plan-53-B) vs `BIND IN <slot> …`
                // (plan-50-E). STATE is a single-line clause; IN opens a block.
                if self.check_identifier_ci("STATE") {
                    self.advance(); // STATE
                    if let Some(bs) = self.parse_bind_state() {
                        if bind_state.is_some() {
                            self.report(
                                "MFB_PARSE_UNEXPECTED_STATEMENT",
                                "A native FUNC may declare at most one BIND STATE.",
                                &func_token,
                            );
                        } else {
                            bind_state = Some(bs);
                        }
                    }
                } else if let Some(bind) = self.parse_bind_in() {
                    bind_in.push(bind);
                }
                self.skip_separators();
                continue;
            }
            if self.match_identifier_ci("FREE") {
                free = self.parse_free_block();
                self.skip_separators();
                continue;
            }
            // plan-58-A: `BUFFER <slot> SIZE <expr>` gives an OUT CBuffer slot its
            // runtime byte capacity. A duplicate is NOT reported here — unlike
            // BIND STATE, whose "at most one" is a parse-level cardinality — it is
            // collected and rejected by `check_buffer_slots` rule 2, so the
            // package path gets the identical diagnostic.
            if self.match_identifier_ci("BUFFER") {
                if let Some(spec) = self.parse_buffer_spec() {
                    buffers.push(spec);
                }
                self.skip_separators();
                continue;
            }
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "A native FUNC body may only contain SYMBOL, ABI, CONST, SUCCESS_ON, ERROR_ON, RETURN, BIND IN, BUFFER, or FREE clauses.",
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
            return_state_type,
            symbol,
            abi,
            consts,
            success_on,
            result,
            bind_in,
            bind_state,
            buffers,
            result_length,
            free,
            line: func_token.line,
        })
    }

    /// `BUFFER <slot> SIZE <expr>` — a single-line clause (plan-58-A §4.2). The
    /// `BUFFER` keyword has already been consumed.
    ///
    /// Nothing semantic is checked here: whether `<slot>` exists, is a `CBuffer`,
    /// is `OUT`, or is returned are all `check_buffer_slots` rules, because the
    /// parser cannot protect the `.mfp` package path and a second rule list there
    /// is drift bait (the same argument plan-50-A made).
    pub(super) fn parse_buffer_spec(&mut self) -> Option<BufferSpec> {
        let Some(slot) = self.consume_identifier("BUFFER requires an ABI slot name.") else {
            self.synchronize();
            return None;
        };
        if !self.match_identifier_ci("SIZE") {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "BUFFER requires `SIZE <expr>` giving the slot's capacity in bytes.",
                &token,
            );
            self.synchronize();
            return None;
        }
        let size = self.parse_expression()?;
        self.consume_statement_end("Expected end of statement after BUFFER.");
        Some(BufferSpec { slot, size })
    }

    /// `BIND STATE <resource-slot> = <out-struct-slot>` — a single-line clause
    /// (unlike `BIND IN … END BIND`). `STATE` and `BIND` are already consumed.
    pub(super) fn parse_bind_state(&mut self) -> Option<BindState> {
        let resource_slot =
            self.consume_identifier("BIND STATE requires the resource slot name.")?;
        if !self.consume_kind(
            TokenKind::Equal,
            "BIND STATE requires `= <out-struct-slot>`.",
        ) {
            self.synchronize();
            return None;
        }
        let struct_slot =
            self.consume_identifier("BIND STATE requires an OUT struct slot name.")?;
        self.consume_statement_end("Expected end of statement after BIND STATE.");
        Some(BindState {
            resource_slot,
            struct_slot,
        })
    }

    /// `BIND IN <slot>` / `<field> = <expr>`… / `END BIND` (plan-50-E).
    ///
    /// Writes named struct fields before the call. Every field the block does not
    /// name is zero, so the caller supplies only the real inputs — no dummy record
    /// stuffed with values the C library immediately overwrites. The `BIND`
    /// identifier has already been consumed.
    pub(super) fn parse_bind_in(&mut self) -> Option<BindIn> {
        let line = self.previous().line;
        // `IN` is a keyword (FOR EACH x IN xs), not an identifier.
        if !self.match_keyword(Keyword::In) {
            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_TOKEN",
                "BIND requires a direction: `BIND IN <slot>`.",
                &token,
            );
            self.synchronize();
            return None;
        }
        let slot = self.consume_identifier("BIND IN requires an ABI slot name.")?;
        self.consume_statement_end("Expected end of statement after BIND IN <slot>.");
        self.skip_separators();

        let mut fields = Vec::new();
        while !self.is_at_end() {
            if self.check_keyword(Keyword::End) {
                self.advance(); // END
                if !self.match_identifier_ci("BIND") {
                    let token = self.peek().clone();
                    self.report(
                        "MFB_PARSE_UNEXPECTED_TOKEN",
                        "END must name the block kind it closes (END BIND).",
                        &token,
                    );
                    self.synchronize();
                    return None;
                }
                self.consume_statement_end("Expected end of statement after END BIND.");
                return Some(BindIn { slot, fields, line });
            }
            let field_line = self.peek().line;
            let Some(field) = self.consume_identifier("Expected a struct field name.") else {
                self.synchronize();
                self.skip_separators();
                continue;
            };
            if !self.consume_kind(TokenKind::Equal, "BIND IN field requires `= <value>`.") {
                self.synchronize();
                self.skip_separators();
                continue;
            }
            let Some(value) = self.parse_expression() else {
                self.synchronize();
                self.skip_separators();
                continue;
            };
            self.consume_statement_end("Expected end of statement after a BIND IN field.");
            self.skip_separators();
            fields.push(BindInField {
                name: field,
                value,
                line: field_line,
            });
        }
        let token = self.peek().clone();
        self.report(
            "MFB_PARSE_UNTERMINATED_BLOCK",
            "BIND block reached end-of-file before its END BIND statement.",
            &token,
        );
        None
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

        // A FREE block that reaches `END FREE` without a SYMBOL and an ABI
        // clause must not be silently dropped (that would leave `free: None`,
        // indistinguishable from "no FREE declared", so the deallocator vanishes
        // and every call leaks its native return buffer). Diagnose it as a
        // malformed FREE block, matching the missing-SYMBOL/ABI diagnostics the
        // sibling link-FUNC path emits.
        if symbol.is_none() || param.is_none() || return_ctype.is_none() {
            let token = self.previous().clone();
            self.report(
                "NATIVE_FREE_INVALID",
                "A FREE block must declare both a SYMBOL and an ABI clause.",
                &token,
            );
            return None;
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
                // plan-50-E: INOUT and an explicit IN join OUT. INOUT is matched
                // first: it shares a prefix with IN, so the order is load-bearing.
                // IN is optional — an unmarked slot is an input — but a struct slot
                // reads better spelled out next to its INOUT sibling.
                let direction = if self.match_identifier_ci("INOUT") {
                    crate::ir::AbiDirection::InOut
                } else if self.match_identifier_ci("OUT") {
                    crate::ir::AbiDirection::Out
                } else {
                    // `IN` is a keyword (FOR EACH x IN xs), not an identifier.
                    self.match_keyword(Keyword::In);
                    crate::ir::AbiDirection::In
                };
                let ctype = self.parse_c_type_name()?;
                slots.push(AbiSlot {
                    name,
                    ctype,
                    direction,
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
    /// An ABI slot name is an ordinary identifier.
    ///
    /// plan-50-H deleted the `return` special case: the result is named by the
    /// `RETURN <expr>` clause, so `return` carries no meaning here and is simply
    /// a keyword the parser will not accept as a name.
    pub(super) fn parse_abi_slot_name(&mut self) -> Option<String> {
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
        // plan-50-E/G: `SIZEOF <CStruct>` pins a struct's computed size. The C
        // library often validates it (libsndfile fails the call when `datasize`
        // is not sizeof(SF_FORMAT_INFO)), and the compiler already knows the
        // number — hardcoding it in every binding is the fragility
        // `compute_c_layout` exists to remove.
        if self.match_identifier_ci("SIZEOF") {
            let (line2, col) = (self.previous().line, self.previous().start);
            let name = self.consume_identifier("SIZEOF requires a CSTRUCT name.")?;
            return Some(ConstPin {
                slot,
                value: Expression::Unary {
                    operator: "SIZEOF".to_string(),
                    operand: Box::new(Expression::Identifier(name)),
                    line: line2,
                    column: col,
                },
                line,
            });
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
}
