use super::*;

impl<'a> FileParser<'a> {
    pub(super) fn is_end_block(&self, keyword: Keyword) -> bool {
        self.check_keyword(Keyword::End)
            && self.tokens.get(self.current + 1).is_some_and(
                |token| matches!(token.kind, TokenKind::Keyword(current) if current == keyword),
            )
    }

    pub(super) fn consume_end_block(&mut self, keyword: Keyword, detail: &str) -> bool {
        if !self.consume_keyword(Keyword::End, detail) {
            return false;
        }
        if !self.consume_keyword(keyword, "END must name the block kind it closes.") {
            return false;
        }
        self.consume_statement_end("Expected end of statement after END.")
    }

    pub(super) fn consume_identifier(&mut self, detail: &str) -> Option<String> {
        if let TokenKind::Identifier(value) = self.peek().kind.clone() {
            self.advance();
            Some(value)
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_INVALID_IDENTIFIER", detail, &token);
            None
        }
    }

    pub(super) fn consume_qualified_identifier_part(&mut self) -> Option<String> {
        if let Some(part) = self.consume_numeric_identifier_part() {
            return Some(part);
        }
        // A qualified member may be named after a keyword (e.g. `sqliteLink::step`).
        self.consume_name_or_keyword("Expected identifier after `::`.")
    }

    /// Consume an identifier, or a keyword token used in a name position
    /// (canonicalized through `lexer::keyword_lexeme` so definitions and call
    /// sites agree).
    pub(super) fn consume_name_or_keyword(&mut self, detail: &str) -> Option<String> {
        match self.peek().kind.clone() {
            TokenKind::Identifier(value) => {
                self.advance();
                Some(value)
            }
            TokenKind::Keyword(keyword) => {
                self.advance();
                Some(lexer::keyword_lexeme(keyword).to_string())
            }
            _ => {
                let token = self.peek().clone();
                self.report("MFB_PARSE_INVALID_IDENTIFIER", detail, &token);
                None
            }
        }
    }

    pub(super) fn consume_numeric_identifier_part(&mut self) -> Option<String> {
        let TokenKind::Number(number) = self.peek().kind.clone() else {
            return None;
        };
        let Some(next) = self.tokens.get(self.current + 1) else {
            return None;
        };
        let TokenKind::Identifier(identifier) = next.kind.clone() else {
            return None;
        };
        let current = self.peek().clone();
        if current.line != next.line || current.end != next.start {
            return None;
        }
        self.advance();
        self.advance();
        Some(format!("{number}{identifier}"))
    }

    pub(super) fn consume_keyword(&mut self, keyword: Keyword, detail: &str) -> bool {
        if self.match_keyword(keyword) {
            true
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
            false
        }
    }

    pub(super) fn consume_kind(&mut self, kind: TokenKind, detail: &str) -> bool {
        if self.match_kind(kind) {
            true
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
            false
        }
    }

    pub(super) fn consume_statement_end(&mut self, detail: &str) -> bool {
        if self.is_statement_end() {
            self.skip_separators();
            true
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
            false
        }
    }

    pub(super) fn consume_simple_statement_end(&mut self, detail: &str, allow_else_terminator: bool) -> bool {
        if self.is_statement_end() {
            self.skip_separators();
            return true;
        }
        if allow_else_terminator && self.check_keyword(Keyword::Else) {
            return true;
        }
        let token = self.peek().clone();
        self.report("MFB_PARSE_UNEXPECTED_TOKEN", detail, &token);
        false
    }
}
