//! plan-176-M: the lexer's literal scanners — strings, scalars, Unicode escapes and numbers
//! (decimal, exponent and radix forms), with their error recovery. The token loop lives in
//! `lexer/mod.rs`.

use super::*;

impl Lexer<'_> {
    pub(super) fn lex_string(&mut self) {
        let line = self.line;
        let start = self.column;
        self.advance();

        let mut value = String::new();
        while !self.is_at_end() {
            let ch = self.peek();
            if ch == '"' {
                self.advance();
                self.tokens.push(Token {
                    kind: TokenKind::String(value),
                    line,
                    start,
                    end: self.column,
                });
                return;
            }

            if ch == '\n' {
                self.report(
                    "MFB_LEX_UNTERMINATED_STRING",
                    "String literal reached the end of the line before a closing quote.",
                    line,
                    start,
                    self.column,
                );
                return;
            }

            if ch == '\\' {
                self.advance();
                if self.is_at_end() {
                    break;
                }
                let escaped = self.peek();
                // A backslash immediately before a newline is not a line
                // continuation — no such feature exists. Without this guard the
                // fall-through arm below would embed a literal newline in the
                // value and consume it with `advance`, which never bumps
                // `self.line`, desyncing every later token's line number
                // (bug-12). Leave the newline unconsumed, exactly as the
                // unescaped-newline guard above does.
                if escaped == '\n' {
                    self.report(
                        "MFB_LEX_UNTERMINATED_STRING",
                        "String literal reached the end of the line before a closing quote.",
                        line,
                        start,
                        self.column,
                    );
                    return;
                }
                match escaped {
                    '"' => value.push('"'),
                    '\\' => value.push('\\'),
                    'n' => value.push('\n'),
                    't' => value.push('\t'),
                    'r' => value.push('\r'),
                    '0' => value.push('\0'),
                    'u' => {
                        // `\u{HEX}` Unicode scalar escape. Consume past `u`, then
                        // scan `{`, 1-6 hex digits, `}`, and validate via
                        // `char::from_u32` (the single validity oracle: rejects
                        // surrogates and > U+10FFFF). On any malformed form report
                        // MFB_LEX_INVALID_UNICODE_ESCAPE and stop lexing the token.
                        self.advance();
                        match self.lex_unicode_escape(line, start) {
                            Some(ch) => {
                                value.push(ch);
                                continue;
                            }
                            None => {
                                // The escape was already reported. Consume the rest
                                // of the string literal so the remainder is not
                                // re-lexed as a second (spurious) token.
                                self.recover_string_literal();
                                return;
                            }
                        }
                    }
                    _ => value.push(escaped),
                }
                self.advance();
            } else {
                value.push(ch);
                self.advance();
            }
        }

        self.report(
            "MFB_LEX_UNTERMINATED_STRING",
            "String literal reached end-of-file before a closing quote.",
            line,
            start,
            self.column,
        );
    }

    /// After a fatal in-string error (an invalid `\u{...}` escape), skip the rest
    /// of the current string literal up to and including the closing `"`, or up to
    /// a newline / EOF, so the abandoned remainder is not re-lexed as a spurious
    /// second token. Does not emit diagnostics — the caller already reported.
    fn recover_string_literal(&mut self) {
        while !self.is_at_end() {
            let ch = self.peek();
            if ch == '\n' {
                return;
            }
            self.advance();
            if ch == '"' {
                return;
            }
        }
    }

    /// Lex a backtick scalar literal `` `x` `` (plan-41-A). The cursor is on the
    /// opening backtick. Consumes exactly one Unicode scalar — either a raw source
    /// scalar or an escape reusing the string-escape machinery (`` `\n` ``,
    /// `` `\\` ``, `` `\`` ``, `` `\u{1F600}` ``) — then requires a closing
    /// backtick, emitting `TokenKind::Scalar(codepoint)`. Rejects empty (`` `` ``),
    /// multi-scalar (`` `ab` ``), unterminated, and invalid-escape literals with
    /// the `TYPE_SCALAR_LITERAL_*` diagnostics (a malformed `\u{...}` reuses the
    /// shared `MFB_LEX_INVALID_UNICODE_ESCAPE` path so string and scalar agree).
    pub(super) fn lex_scalar(&mut self) {
        let line = self.line;
        let start = self.column;
        self.advance(); // consume opening backtick

        if self.is_at_end() || self.peek() == '\n' {
            self.report(
                "TYPE_SCALAR_LITERAL_INVALID",
                "A scalar literal reached the end of the line before a closing backtick.",
                line,
                start,
                self.column,
            );
            return;
        }

        let ch = self.peek();
        if ch == '`' {
            // Empty literal `` `` ``.
            self.advance();
            self.report(
                "TYPE_SCALAR_LITERAL_EMPTY",
                "A scalar literal must contain exactly one Unicode scalar; `` is empty.",
                line,
                start,
                self.column,
            );
            return;
        }

        let code_point: u32 = if ch == '\\' {
            self.advance();
            if self.is_at_end() || self.peek() == '\n' {
                self.report(
                    "TYPE_SCALAR_LITERAL_INVALID",
                    "A scalar literal reached the end of the line before a closing backtick.",
                    line,
                    start,
                    self.column,
                );
                return;
            }
            let escaped = self.peek();
            match escaped {
                '`' => {
                    self.advance();
                    '`' as u32
                }
                '\\' => {
                    self.advance();
                    '\\' as u32
                }
                'n' => {
                    self.advance();
                    '\n' as u32
                }
                't' => {
                    self.advance();
                    '\t' as u32
                }
                'r' => {
                    self.advance();
                    '\r' as u32
                }
                '0' => {
                    self.advance();
                    0
                }
                'u' => {
                    // `\u{HEX}` — reuse the string decoder (the single validity
                    // oracle: rejects surrogates and > U+10FFFF).
                    self.advance();
                    match self.lex_unicode_escape(line, start) {
                        Some(decoded) => decoded as u32,
                        None => {
                            self.recover_scalar_literal();
                            return;
                        }
                    }
                }
                other => {
                    self.report(
                        "TYPE_SCALAR_LITERAL_INVALID",
                        &format!(
                            "`\\{other}` is not a valid scalar-literal escape (use `\\n`, `\\t`, `\\r`, `\\0`, `\\\\`, `` \\` ``, or `\\u{{...}}`)."
                        ),
                        line,
                        start,
                        self.column,
                    );
                    self.recover_scalar_literal();
                    return;
                }
            }
        } else {
            self.advance();
            ch as u32
        };

        if self.is_at_end() || self.peek() == '\n' {
            self.report(
                "TYPE_SCALAR_LITERAL_INVALID",
                "A scalar literal reached the end of the line before a closing backtick.",
                line,
                start,
                self.column,
            );
            return;
        }
        if self.peek() != '`' {
            // More than one scalar before the closing backtick.
            self.report(
                "TYPE_SCALAR_LITERAL_TOO_MANY",
                "A scalar literal must contain exactly one Unicode scalar.",
                line,
                start,
                self.column,
            );
            self.recover_scalar_literal();
            return;
        }
        self.advance(); // consume closing backtick
        self.tokens.push(Token {
            kind: TokenKind::Scalar(code_point),
            line,
            start,
            end: self.column,
        });
    }

    /// After a fatal scalar-literal error, skip to and including the closing
    /// backtick, or to a newline / EOF, so the remainder is not re-lexed as a
    /// spurious token. Does not emit diagnostics — the caller already reported.
    fn recover_scalar_literal(&mut self) {
        while !self.is_at_end() {
            let ch = self.peek();
            if ch == '\n' {
                return;
            }
            self.advance();
            if ch == '`' {
                return;
            }
        }
    }

    /// Scan the body of a `\u{HEX}` escape: the cursor is positioned just past
    /// the `u`, at the expected `{`. Returns the decoded scalar on success, or
    /// `None` after reporting `MFB_LEX_INVALID_UNICODE_ESCAPE` for any malformed
    /// form (missing `{`, no digits, more than 6 digits, out-of-range magnitude,
    /// unterminated brace, or an invalid scalar such as a surrogate). All cursor
    /// moves go through `advance`/`peek`/`is_at_end` so column/line tracking is
    /// preserved. `line`/`start` are the enclosing string literal's origin, used
    /// for the diagnostic span.
    fn lex_unicode_escape(&mut self, line: usize, start: usize) -> Option<char> {
        let column = self.column;
        if self.is_at_end() || self.peek() != '{' {
            self.report(
                "MFB_LEX_INVALID_UNICODE_ESCAPE",
                "A `\\u` escape must be followed by `{`, as in `\\u{1F600}`.",
                line,
                start,
                column,
            );
            return None;
        }
        self.advance(); // consume `{`

        let mut digits = String::new();
        while !self.is_at_end() {
            let ch = self.peek();
            if ch.is_ascii_hexdigit() {
                digits.push(ch);
                self.advance();
                if digits.len() > 6 {
                    self.report(
                        "MFB_LEX_INVALID_UNICODE_ESCAPE",
                        "A `\\u{...}` escape accepts at most 6 hex digits.",
                        line,
                        start,
                        self.column,
                    );
                    return None;
                }
            } else {
                break;
            }
        }

        if self.is_at_end() || self.peek() == '\n' || self.peek() == '"' {
            self.report(
                "MFB_LEX_INVALID_UNICODE_ESCAPE",
                "A `\\u{...}` escape is missing its closing `}`.",
                line,
                start,
                self.column,
            );
            return None;
        }
        if self.peek() != '}' {
            self.report(
                "MFB_LEX_INVALID_UNICODE_ESCAPE",
                "A `\\u{...}` escape may contain only hex digits between the braces.",
                line,
                start,
                self.column,
            );
            return None;
        }
        if digits.is_empty() {
            self.report(
                "MFB_LEX_INVALID_UNICODE_ESCAPE",
                "A `\\u{...}` escape needs at least one hex digit.",
                line,
                start,
                self.column,
            );
            return None;
        }
        self.advance(); // consume `}`

        let code_point = u32::from_str_radix(&digits, 16).ok();
        match code_point.and_then(char::from_u32) {
            Some(ch) => Some(ch),
            None => {
                self.report(
                    "MFB_LEX_INVALID_UNICODE_ESCAPE",
                    "A `\\u{...}` escape must name a Unicode scalar value (U+0000..U+D7FF or U+E000..U+10FFFF).",
                    line,
                    start,
                    self.column,
                );
                None
            }
        }
    }

    pub(super) fn lex_number(&mut self) {
        let line = self.line;
        let start = self.column;

        // Radix prefix: `0x`/`0o`/`0b` (case-insensitive prefix letter) followed
        // by base-appropriate digits, decoded to canonical decimal (plan-28-A §4.3).
        if self.peek() == '0' {
            if let Some(radix) = self.peek_next().and_then(|ch| match ch {
                'x' | 'X' => Some(16u32),
                'o' | 'O' => Some(8),
                'b' | 'B' => Some(2),
                _ => None,
            }) {
                self.lex_radix_number(line, start, radix);
                return;
            }
        }

        // Decimal integer part with `_` digit separators between digits.
        let mut value = String::new();
        if !self.scan_base_digits(&mut value, 10) {
            return;
        }

        if !self.is_at_end()
            && self.peek() == '.'
            && self.peek_next().is_some_and(|ch| ch.is_ascii_digit())
        {
            value.push('.');
            self.advance();
            if !self.scan_base_digits(&mut value, 10) {
                return;
            }
        }

        // Scientific-notation exponent (plan-28-B §4.1): `e`/`E`, an optional
        // `+`/`-`, then one or more digits (with `_` separators). Consumed only
        // when a well-formed exponent follows; otherwise `e` is not part of the
        // number (`1e` lexes as `1` then identifier `e`).
        if !self.is_at_end()
            && (self.peek() == 'e' || self.peek() == 'E')
            && self.exponent_follows()
        {
            value.push(self.peek());
            self.advance();
            if self.peek() == '+' || self.peek() == '-' {
                value.push(self.peek());
                self.advance();
            }
            if !self.scan_base_digits(&mut value, 10) {
                return;
            }
        }

        // Type suffix (plan-28-B §4.2, plan-29-A §4.4): a single trailing `f`
        // (Float), `F` (Fixed), or `m`/`M` (Money), only when not followed by an
        // identifier-continue char (so `1foo` is `1` then `foo`, but `1f` is a
        // suffixed literal). The suffix stays in the token value; `classify_literal`
        // reads and strips it.
        if !self.is_at_end()
            && matches!(self.peek(), 'f' | 'F' | 'm' | 'M')
            && self
                .peek_next()
                .is_none_or(|ch| !is_identifier_continue(ch))
        {
            value.push(self.peek());
            self.advance();
        }

        self.tokens.push(Token {
            kind: TokenKind::Number(value),
            line,
            start,
            end: self.column,
        });
    }

    /// Non-consuming check: does a well-formed exponent follow the `e`/`E` at the
    /// current cursor? An optional `+`/`-` then at least one ASCII digit.
    fn exponent_follows(&self) -> bool {
        let mut lookahead = self.index + 1;
        if let Some(sign) = self.chars.get(lookahead).copied() {
            if sign == '+' || sign == '-' {
                lookahead += 1;
            }
        }
        self.chars
            .get(lookahead)
            .is_some_and(|ch| ch.is_ascii_digit())
    }

    /// Whether `ch` is a valid digit in the given radix (2, 8, 10, or 16).
    fn is_base_digit(ch: char, radix: u32) -> bool {
        match radix {
            16 => ch.is_ascii_hexdigit(),
            10 => ch.is_ascii_digit(),
            8 => ('0'..='7').contains(&ch),
            2 => ch == '0' || ch == '1',
            _ => false,
        }
    }

    /// Non-consuming check: is the `_` at the current cursor a line continuation
    /// (followed only by horizontal whitespace and then a newline)? Mirrors the
    /// lookahead in `lex_line_continuation`.
    fn underscore_is_line_continuation(&self) -> bool {
        let mut lookahead = self.index + 1;
        while let Some(ch) = self.chars.get(lookahead).copied() {
            match ch {
                ' ' | '\t' | '\r' => lookahead += 1,
                '\n' => return true,
                _ => return false,
            }
        }
        false
    }

    /// Scan a run of base-`radix` digits into `value`, accepting a single `_`
    /// separator **only** between two base digits and stripping it (plan-28-A
    /// §4.2). Returns `true` on success; on a misplaced separator (leading,
    /// trailing, doubled) that is not a line continuation, reports
    /// `MFB_LEX_MALFORMED_NUMBER` and returns `false`. A trailing `_` that forms a
    /// line continuation ends the run cleanly (the main loop handles it).
    fn scan_base_digits(&mut self, value: &mut String, radix: u32) -> bool {
        let mut last_was_digit = false;
        while !self.is_at_end() {
            let ch = self.peek();
            if Self::is_base_digit(ch, radix) {
                value.push(ch);
                self.advance();
                last_was_digit = true;
            } else if ch == '_' {
                let next_is_digit = self
                    .peek_next()
                    .is_some_and(|next| Self::is_base_digit(next, radix));
                if last_was_digit && next_is_digit {
                    self.advance(); // consume the separator, do not push it
                    last_was_digit = false;
                } else if last_was_digit && self.underscore_is_line_continuation() {
                    break; // trailing `_` + newline: leave the continuation intact
                } else {
                    self.report(
                        "MFB_LEX_MALFORMED_NUMBER",
                        "A `_` digit separator must sit between two digits.",
                        self.line,
                        self.column,
                        self.column + 1,
                    );
                    return false;
                }
            } else {
                break;
            }
        }
        true
    }

    /// Scan a radix literal (`0x`/`0o`/`0b`) whose prefix is at the cursor. The
    /// base digits (with separators) are decoded to a canonical **decimal**
    /// string so every downstream Integer-literal consumer is unchanged
    /// (plan-28-A §4.3). Reports `MFB_LEX_MALFORMED_NUMBER` on empty/invalid
    /// digits and `MFB_LEX_NUMBER_OUT_OF_RANGE` on a magnitude above `u64::MAX`.
    fn lex_radix_number(&mut self, line: usize, start: usize, radix: u32) {
        let base_name = match radix {
            16 => "hexadecimal",
            8 => "octal",
            2 => "binary",
            _ => "",
        };
        self.advance(); // `0`
        self.advance(); // prefix letter

        let mut digits = String::new();
        if !self.scan_base_digits(&mut digits, radix) {
            return;
        }

        // A base-appropriate-looking but invalid digit (e.g. `0o8`, `0b2`, `0xG`)
        // or no digits at all (`0x`) is a malformed literal. An ordinary
        // non-alphanumeric terminator (`0b10 + 1`, `0xFF.foo`) just ends it.
        if !self.is_at_end() && self.peek().is_ascii_alphanumeric() {
            let ch = self.peek();
            self.report(
                "MFB_LEX_MALFORMED_NUMBER",
                &format!("`{ch}` is not a valid digit in a {base_name} literal."),
                self.line,
                self.column,
                self.column + 1,
            );
            return;
        }
        if digits.is_empty() {
            self.report(
                "MFB_LEX_MALFORMED_NUMBER",
                &format!("A {base_name} literal needs at least one digit after the prefix."),
                line,
                start,
                self.column,
            );
            return;
        }

        match u128::from_str_radix(&digits, radix) {
            Ok(magnitude) if magnitude <= u64::MAX as u128 => {
                self.tokens.push(Token {
                    kind: TokenKind::Number(magnitude.to_string()),
                    line,
                    start,
                    end: self.column,
                });
            }
            _ => {
                self.report(
                    "MFB_LEX_NUMBER_OUT_OF_RANGE",
                    "This numeric literal is too large to represent.",
                    line,
                    start,
                    self.column,
                );
            }
        }
    }
}
