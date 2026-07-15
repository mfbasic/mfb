use super::*;

pub(super) struct ParsedFile {
    pub(super) imports: Vec<Import>,
    pub(super) items: Vec<Item>,
}

/// Reserved error binding synthesized for a bare `TRAP` (one written without the
/// `(ident)` binding, in either the function-level or inline postfix position).
/// The `#` prefix is the internal-sentinel convention: the lexer can never emit
/// it in a user identifier, so this name cannot collide with a real one. The
/// caught `Error` stays internally bound to this name, so `PROPAGATE` and the
/// slot-keyed scope-drop cleanup work identically to a named-but-unused
/// `TRAP(e)`; the user simply has no name for it.
pub const SYNTHETIC_TRAP_BINDING: &str = "#err";

pub(super) struct FileParser<'a> {
    pub(super) path: &'a Path,
    pub(super) tokens: Vec<Token>,
    pub(super) current: usize,
    pub(super) had_error: bool,
    /// Current expression-nesting depth, bumped at each recursive re-entry of the
    /// expression grammar so pathologically nested input (e.g. `((((…))))`) is
    /// rejected with a diagnostic instead of overflowing the native stack
    /// (bug-171 finding A).
    pub(super) expr_depth: usize,
    /// Current statement-block-nesting depth, bumped at each recursive re-entry of
    /// `parse_statement_block` (every IF/FOR/WHILE/DO/MATCH body) so pathologically
    /// nested control flow is rejected with a diagnostic instead of overflowing the
    /// native stack here or in any downstream pass that re-walks the AST (audit-2
    /// FE-03 / bug-183). `expr_depth` guards only the expression grammar.
    pub(super) stmt_depth: usize,
    /// Latched once statement nesting hits the cap (bug-183). It fast-forwards the
    /// cursor to `Eof` and suppresses every subsequent diagnostic, so the deeply
    /// nested block unwinds through its ~256 enclosing `consume_end_block` calls
    /// emitting exactly one `MFB_PARSE_BLOCK_TOO_DEEP` instead of a cascade.
    pub(super) depth_exceeded: bool,
}

#[derive(Clone, Copy)]
pub(super) enum BlockTerminator {
    Case,
    Else,
    ElseIf,
    EndIf,
    EndMatch,
    Loop,
    Next,
    Wend,
}

impl<'a> FileParser<'a> {
    pub(super) fn new(path: &'a Path, tokens: Vec<Token>) -> Self {
        // `peek`/`previous` index `tokens` unchecked; the whole parser relies on
        // an `Eof`-terminated stream (the sole invariant `lexer::lex` upholds).
        // Assert it here so a hand-built, empty, or non-`Eof`-terminated token
        // vector fails loudly at construction rather than panicking mid-parse on
        // an out-of-bounds index / `current - 1` underflow (bug-171 finding D).
        assert!(
            matches!(tokens.last().map(|token| &token.kind), Some(TokenKind::Eof)),
            "FileParser requires an Eof-terminated token stream"
        );
        Self {
            path,
            tokens,
            current: 0,
            had_error: false,
            expr_depth: 0,
            stmt_depth: 0,
            depth_exceeded: false,
        }
    }

    pub(super) fn parse(&mut self) -> Result<ParsedFile, ()> {
        let mut imports = Vec::new();
        let mut items = Vec::new();
        self.skip_separators();

        while !self.is_at_end() {
            if self.match_keyword(Keyword::Import) {
                let import_token = self.previous().clone();
                let Some(module) = self.parse_qualified_name("Expected package name after IMPORT.")
                else {
                    self.synchronize();
                    self.skip_separators();
                    continue;
                };
                let alias = if self.match_keyword(Keyword::As) {
                    self.consume_identifier("Expected alias name after AS.")
                } else {
                    None
                };
                imports.push(Import {
                    module,
                    alias,
                    line: import_token.line,
                });
                self.consume_statement_end("Expected end of statement after IMPORT.");
                self.skip_separators();
                continue;
            }

            if self.check_top_level_binding_start() {
                let visibility = self.parse_visibility().unwrap_or(Visibility::Public);
                if let Some(binding) = self.parse_top_level_binding(visibility) {
                    items.push(Item::Binding(binding));
                }
                self.skip_separators();
                continue;
            }

            if self.check_top_level_func_alias() {
                if let Some(alias) = self.parse_top_level_func_alias() {
                    items.push(Item::FuncAlias(alias));
                }
                self.skip_separators();
                continue;
            }

            if self.check_top_level_item_start() {
                let visibility = self.parse_visibility().unwrap_or(Visibility::Public);
                if let Some(function) = self.parse_function() {
                    items.push(Item::Function(Function {
                        visibility,
                        ..function
                    }));
                }
                self.skip_separators();
                continue;
            }

            if self.check_top_level_resource_start() {
                if let Some(resource) = self.parse_top_level_resource() {
                    items.push(Item::Resource(resource));
                }
                self.skip_separators();
                continue;
            }

            if self.check_top_level_link_start() {
                if let Some(link) = self.parse_link_block() {
                    items.push(Item::Link(link));
                }
                self.skip_separators();
                continue;
            }

            if self.check_top_level_type_start() {
                let visibility = self.parse_visibility().unwrap_or(Visibility::Public);
                if let Some(type_decl) = self.parse_type_decl() {
                    items.push(Item::Type(TypeDecl {
                        visibility,
                        ..type_decl
                    }));
                }
                self.skip_separators();
                continue;
            }

            if self.check_keyword(Keyword::Testing) {
                if let Some(block) = self.parse_testing_block() {
                    items.push(Item::Testing(block));
                }
                self.skip_separators();
                continue;
            }

            if matches!(self.peek().kind, TokenKind::Doc(_)) {
                let token = self.advance().clone();
                if let TokenKind::Doc(raw) = token.kind {
                    if let Some(doc) = self.parse_doc_block(raw) {
                        items.push(Item::Doc(doc));
                    }
                }
                self.skip_separators();
                continue;
            }

            let token = self.peek().clone();
            self.report(
                "MFB_PARSE_UNEXPECTED_STATEMENT",
                "Expected an IMPORT, LET, MUT, SUB, FUNC, TYPE, UNION, ENUM, RESOURCE, or LINK declaration at the top level.",
                &token,
            );
            self.synchronize();
            self.skip_separators();
        }

        if self.had_error {
            Err(())
        } else {
            Ok(ParsedFile { imports, items })
        }
    }

    pub(super) fn is_end_link(&self) -> bool {
        self.check_keyword(Keyword::End)
            && self.peek_next().is_some_and(|token| {
                matches!(&token.kind, TokenKind::Identifier(value) if value.eq_ignore_ascii_case("LINK"))
            })
    }

    pub(super) fn match_identifier_ci(&mut self, expected: &str) -> bool {
        if self.check_identifier_ci(expected) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn consume_contextual(&mut self, expected: &str, detail: &str) -> bool {
        if self.match_identifier_ci(expected) {
            true
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
            false
        }
    }

    pub(super) fn check_visibility(&self) -> bool {
        self.check_keyword(Keyword::Private)
            || self.check_keyword(Keyword::Public)
            || self.check_keyword(Keyword::Export)
    }

    pub(super) fn skip_separators(&mut self) {
        while self.match_any(&[TokenKind::Newline, TokenKind::Colon]) {}
    }

    pub(super) fn synchronize(&mut self) {
        while !self.is_at_end() && !self.is_statement_end() {
            self.advance();
        }
    }

    pub(super) fn is_statement_end(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Newline | TokenKind::Colon | TokenKind::Eof
        )
    }

    pub(super) fn match_keyword(&mut self, keyword: Keyword) -> bool {
        if self.check_keyword(keyword) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn check_keyword(&self, keyword: Keyword) -> bool {
        matches!(self.peek().kind, TokenKind::Keyword(current) if current == keyword)
    }

    pub(super) fn check_identifier_ci(&self, expected: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(value) if value.eq_ignore_ascii_case(expected))
    }

    pub(super) fn match_kind(&mut self, kind: TokenKind) -> bool {
        if self.check_kind(&kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn match_any(&mut self, kinds: &[TokenKind]) -> bool {
        if kinds.iter().any(|kind| self.check_kind(kind)) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn match_any_keywords(&mut self, keywords: &[Keyword]) -> bool {
        if keywords.iter().any(|keyword| self.check_keyword(*keyword)) {
            self.advance();
            true
        } else {
            false
        }
    }

    pub(super) fn check_kind(&self, kind: &TokenKind) -> bool {
        std::mem::discriminant(&self.peek().kind) == std::mem::discriminant(kind)
    }

    pub(super) fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    pub(super) fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    pub(super) fn peek_next(&self) -> Option<&Token> {
        self.tokens.get(self.current + 1)
    }

    pub(super) fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }

    pub(super) fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    pub(super) fn report(&mut self, rule: &str, detail: &str, token: &Token) {
        self.had_error = true;
        // After the statement-depth cap latches, the deeply nested block unwinds
        // through ~256 `consume_end_block` calls that each see `Eof`; swallow their
        // diagnostics so only the single `MFB_PARSE_BLOCK_TOO_DEEP` is shown.
        if self.depth_exceeded {
            return;
        }
        rules::show_diagnostic(rule, detail, self.path, token.line, token.start, token.end);
    }

    pub(super) fn report_at(&mut self, rule: &str, detail: &str, line: usize) {
        self.had_error = true;
        if self.depth_exceeded {
            return;
        }
        rules::show_diagnostic(rule, detail, self.path, line, 1, 1);
    }

    /// Jump the cursor to the terminating `Eof` token so every enclosing parse loop
    /// unwinds immediately without consuming more input (used by the statement-depth
    /// guard, bug-183).
    pub(super) fn seek_to_end(&mut self) {
        self.current = self.tokens.len().saturating_sub(1);
    }
}
