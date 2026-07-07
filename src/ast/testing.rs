use super::*;

impl<'a> FileParser<'a> {
    /// Parse a `TESTING … END TESTING` block (plan-18-testing.md §4). The block
    /// contains only `TGROUP` groups; `TGROUP`/`TCASE` are contextual identifiers
    /// (only `TESTING` is a reserved keyword) so they remain usable as names
    /// everywhere else.
    pub(super) fn parse_testing_block(&mut self) -> Option<TestingBlock> {
        let line = self.peek().line;
        self.consume_keyword(Keyword::Testing, "Expected TESTING.");
        self.consume_statement_end("Expected end of line after TESTING.");
        self.skip_separators();

        let mut groups = Vec::new();
        while !self.is_at_end() && !self.is_end_block(Keyword::Testing) {
            if self.check_identifier_ci("TGROUP") {
                if let Some(group) = self.parse_test_group() {
                    groups.push(group);
                }
            } else {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_TESTING_EXPECTED_TGROUP",
                    "A TESTING block may contain only TGROUP groups.",
                    &token,
                );
                self.synchronize();
            }
            self.skip_separators();
        }

        self.consume_end_block(Keyword::Testing, "Expected END TESTING to close the block.");
        Some(TestingBlock { groups, line })
    }

    fn parse_test_group(&mut self) -> Option<TestGroup> {
        let line = self.peek().line;
        self.match_identifier_ci("TGROUP");
        let description =
            self.consume_test_description("A TGROUP requires a string-literal description.")?;
        self.consume_statement_end("Expected end of line after the TGROUP description.");
        self.skip_separators();

        let mut cases = Vec::new();
        while !self.is_at_end()
            && !self.is_end_contextual("TGROUP")
            && !self.is_end_block(Keyword::Testing)
        {
            if self.check_identifier_ci("TCASE") {
                if let Some(case) = self.parse_test_case() {
                    cases.push(case);
                }
            } else {
                let token = self.peek().clone();
                self.report(
                    "MFB_PARSE_TESTING_EXPECTED_TCASE",
                    "A TGROUP may contain only TCASE cases.",
                    &token,
                );
                self.synchronize();
            }
            self.skip_separators();
        }

        self.consume_end_contextual("TGROUP", "Expected END TGROUP to close the group.");
        Some(TestGroup {
            description,
            cases,
            line,
        })
    }

    fn parse_test_case(&mut self) -> Option<TestCase> {
        let line = self.peek().line;
        self.match_identifier_ci("TCASE");
        let description =
            self.consume_test_description("A TCASE requires a string-literal description.")?;
        self.consume_statement_end("Expected end of line after the TCASE description.");
        self.skip_separators();

        let mut body = Vec::new();
        while !self.is_at_end() && !self.is_case_body_terminator() {
            if let Some(statement) = self.parse_statement() {
                body.push(statement);
            } else {
                self.synchronize();
            }
            self.skip_separators();
        }

        self.consume_end_contextual("TCASE", "Expected END TCASE to close the case.");
        Some(TestCase {
            description,
            body,
            line,
        })
    }

    /// A `TCASE` body ends at its own `END TCASE`, or — malformed — at an
    /// enclosing group/block terminator, so the missing-`END TCASE` diagnostic
    /// fires rather than the parser running away into the next construct.
    fn is_case_body_terminator(&self) -> bool {
        self.is_end_contextual("TCASE")
            || self.is_end_contextual("TGROUP")
            || self.is_end_block(Keyword::Testing)
    }

    fn consume_test_description(&mut self, detail: &str) -> Option<String> {
        if let TokenKind::String(value) = self.peek().kind.clone() {
            self.advance();
            Some(value)
        } else {
            let token = self.peek().clone();
            self.report("MFB_PARSE_TESTING_DESCRIPTION", detail, &token);
            None
        }
    }

    /// `END <name>` where `name` is a contextual identifier (`TGROUP`/`TCASE`),
    /// mirroring [`is_end_block`] for reserved-keyword blocks.
    fn is_end_contextual(&self, name: &str) -> bool {
        self.check_keyword(Keyword::End)
            && self.tokens.get(self.current + 1).is_some_and(|token| {
                matches!(&token.kind, TokenKind::Identifier(value) if value.eq_ignore_ascii_case(name))
            })
    }

    fn consume_end_contextual(&mut self, name: &str, detail: &str) -> bool {
        if !self.consume_keyword(Keyword::End, detail) {
            return false;
        }
        if !self.consume_contextual(name, "END must name the block kind it closes.") {
            return false;
        }
        self.consume_statement_end("Expected end of statement after END.")
    }
}
