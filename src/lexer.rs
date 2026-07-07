use crate::rules;
use std::path::Path;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Keyword(Keyword),
    String(String),
    Number(String),
    Dot,
    Comma,
    Colon,
    DoubleColon,
    LBracket,
    RBracket,
    LParen,
    RParen,
    LBrace,
    RBrace,
    Equal,
    NotEqual,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    ColonEqual,
    Plus,
    Minus,
    Star,
    Slash,
    Ampersand,
    Caret,
    PipeGreater,
    Arrow,
    /// A whole `DOC ... END DOC` block, captured verbatim. The free-form text in
    /// a documentation block (descriptions, error notes, example source) must not
    /// be tokenized like code, so the lexer slurps the entire block into one token
    /// and the parser turns its raw lines into a `DocBlock` AST node.
    Doc(DocRaw),
    Newline,
    Eof,
}

/// Raw, untokenized contents of a `DOC ... END DOC` block.
#[derive(Clone, Debug, PartialEq)]
pub struct DocRaw {
    /// Source line of the `DOC` keyword.
    pub line: usize,
    /// Whitespace-separated words after `DOC` on the keyword line (e.g. `INTERNAL`).
    pub attrs: Vec<String>,
    /// Body lines between the `DOC` line and the closing `END DOC`, verbatim.
    pub lines: Vec<DocRawLine>,
}

/// One verbatim body line of a `DOC` block, with its source line number.
#[derive(Clone, Debug, PartialEq)]
pub struct DocRawLine {
    pub line: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Keyword {
    As,
    Case,
    Continue,
    Do,
    Else,
    ElseIf,
    False,
    Fail,
    Exit,
    For,
    Each,
    Func,
    If,
    In,
    Import,
    Isolated,
    Let,
    Lambda,
    Loop,
    Div,
    Mod,
    Match,
    Mut,
    Nothing,
    And,
    Or,
    Not,
    Next,
    Xor,
    Return,
    Sub,
    Testing,
    Then,
    True,
    End,
    Enum,
    Export,
    Public,
    Program,
    Private,
    Propagate,
    Recover,
    Res,
    Step,
    To,
    Type,
    Trap,
    Until,
    Union,
    When,
    While,
    Wend,
    With,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

pub fn lex(path: &Path, source: &str) -> Result<Vec<Token>, ()> {
    lex_with(path, source, false)
}

/// Lex `source`, optionally in *internal* mode. In internal mode the lexer
/// rewrites a leading `__` on each identifier to the untypeable internal sigil
/// (`__json_parse` -> `#json_parse`), making compiler-internal names unforgeable
/// by user code. Only the built-in injected packages are lexed this way.
pub fn lex_with(path: &Path, source: &str, internal: bool) -> Result<Vec<Token>, ()> {
    let mut lexer = Lexer {
        path,
        chars: source.chars().collect(),
        index: 0,
        line: 1,
        column: 1,
        tokens: Vec::new(),
        had_error: false,
        internal,
    };
    lexer.lex_all();

    if lexer.had_error {
        Err(())
    } else {
        Ok(lexer.tokens)
    }
}

struct Lexer<'a> {
    path: &'a Path,
    chars: Vec<char>,
    index: usize,
    line: usize,
    column: usize,
    tokens: Vec<Token>,
    had_error: bool,
    /// When set, identifiers beginning `__` are rewritten to their internal
    /// sigil form (see [`lex_with`]). Public names (no `__` prefix) are untouched.
    internal: bool,
}

impl Lexer<'_> {
    fn lex_all(&mut self) {
        while !self.is_at_end() {
            let ch = self.peek();
            match ch {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => {
                    self.push_simple(TokenKind::Newline, 1);
                    self.advance_line();
                }
                '\'' => self.skip_line_comment(),
                '"' => self.lex_string(),
                '0'..='9' => self.lex_number(),
                'A'..='Z' | 'a'..='z' => self.lex_identifier_or_keyword(),
                '_' => {
                    if !self.lex_line_continuation() {
                        self.lex_identifier_or_keyword();
                    }
                }
                '.' => self.push_and_advance(TokenKind::Dot),
                ',' => self.push_and_advance(TokenKind::Comma),
                ':' => {
                    if self.peek_next() == Some(':') {
                        self.push_simple(TokenKind::DoubleColon, 2);
                        self.advance();
                        self.advance();
                    } else if self.peek_next() == Some('=') {
                        self.push_simple(TokenKind::ColonEqual, 2);
                        self.advance();
                        self.advance();
                    } else {
                        self.push_and_advance(TokenKind::Colon);
                    }
                }
                '[' => self.push_and_advance(TokenKind::LBracket),
                ']' => self.push_and_advance(TokenKind::RBracket),
                '(' => self.push_and_advance(TokenKind::LParen),
                ')' => self.push_and_advance(TokenKind::RParen),
                '{' => self.push_and_advance(TokenKind::LBrace),
                '}' => self.push_and_advance(TokenKind::RBrace),
                '=' => self.push_and_advance(TokenKind::Equal),
                '<' => {
                    if self.peek_next() == Some('=') {
                        self.push_simple(TokenKind::LessEqual, 2);
                        self.advance();
                        self.advance();
                    } else if self.peek_next() == Some('>') {
                        self.push_simple(TokenKind::NotEqual, 2);
                        self.advance();
                        self.advance();
                    } else {
                        self.push_and_advance(TokenKind::Less);
                    }
                }
                '>' => {
                    if self.peek_next() == Some('=') {
                        self.push_simple(TokenKind::GreaterEqual, 2);
                        self.advance();
                        self.advance();
                    } else {
                        self.push_and_advance(TokenKind::Greater);
                    }
                }
                '+' => self.push_and_advance(TokenKind::Plus),
                '-' => {
                    if self.peek_next() == Some('>') {
                        self.push_simple(TokenKind::Arrow, 2);
                        self.advance();
                        self.advance();
                    } else {
                        self.push_and_advance(TokenKind::Minus);
                    }
                }
                '*' => self.push_and_advance(TokenKind::Star),
                '/' => self.push_and_advance(TokenKind::Slash),
                '&' => self.push_and_advance(TokenKind::Ampersand),
                '^' => self.push_and_advance(TokenKind::Caret),
                '|' => {
                    if self.peek_next() == Some('>') {
                        self.push_simple(TokenKind::PipeGreater, 2);
                        self.advance();
                        self.advance();
                    } else {
                        self.report(
                            "MFB_LEX_UNEXPECTED_CHARACTER",
                            "Unexpected character `|`.",
                            self.line,
                            self.column,
                            self.column + 1,
                        );
                        self.advance();
                    }
                }
                _ => {
                    self.report(
                        "MFB_LEX_UNEXPECTED_CHARACTER",
                        &format!("Unexpected character `{}`.", ch.escape_debug()),
                        self.line,
                        self.column,
                        self.column + 1,
                    );
                    self.advance();
                }
            }
        }

        self.tokens.push(Token {
            kind: TokenKind::Eof,
            line: self.line,
            start: self.column,
            end: self.column,
        });
    }

    fn lex_string(&mut self) {
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

    fn lex_number(&mut self) {
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

        self.tokens.push(Token {
            kind: TokenKind::Number(value),
            line,
            start,
            end: self.column,
        });
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

    fn lex_identifier_or_keyword(&mut self) {
        let line = self.line;
        let start = self.column;
        let mut value = String::new();

        while !self.is_at_end() && is_identifier_continue(self.peek()) {
            value.push(self.peek());
            self.advance();
        }

        if value.eq_ignore_ascii_case("REM") && self.is_statement_start() {
            self.skip_line_comment();
            return;
        }

        // `DOC` at the start of a statement begins a documentation block whose
        // body is captured verbatim (see [`TokenKind::Doc`]). The keyword line may
        // only carry attribute words (e.g. `INTERNAL`); anything else (`DOC = 1`,
        // `DOC(x)`) is an ordinary identifier and falls through below.
        if value.eq_ignore_ascii_case("DOC") && self.is_statement_start() {
            if let Some(doc) = self.try_capture_doc_block(line) {
                self.tokens.push(Token {
                    kind: TokenKind::Doc(doc),
                    line,
                    start,
                    end: self.column,
                });
                // The block's trailing newline was consumed during capture; emit a
                // synthetic separator so the next line still counts as a statement
                // start (for a following DOC/REM) and the parser sees a terminator.
                self.push_simple(TokenKind::Newline, 1);
                return;
            }
        }

        // In an internal file, rewrite a leading `__` to the untypeable internal
        // sigil so the resulting name cannot collide with any user identifier
        // (keywords never carry a `__` prefix, so this only ever affects names).
        if self.internal && value.starts_with("__") {
            value = crate::internal_name::internalize(&value);
        }

        let kind = keyword(&value)
            .map(TokenKind::Keyword)
            .unwrap_or(TokenKind::Identifier(value));
        self.tokens.push(Token {
            kind,
            line,
            start,
            end: self.column,
        });
    }

    /// Attempt to slurp a `DOC ... END DOC` block beginning just after the `DOC`
    /// keyword. Returns `None` (leaving the cursor untouched) when the keyword
    /// line carries non-attribute text, so the caller can treat `DOC` as a plain
    /// identifier instead.
    fn try_capture_doc_block(&mut self, doc_line: usize) -> Option<DocRaw> {
        let saved_index = self.index;
        let saved_line = self.line;
        let saved_column = self.column;

        // Read the remainder of the keyword line: only attribute words allowed.
        let mut rest = String::new();
        while !self.is_at_end() && self.peek() != '\n' {
            rest.push(self.peek());
            self.advance();
        }
        let trimmed = rest.trim();
        let attrs = if trimmed.is_empty() {
            Vec::new()
        } else if trimmed
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || ch == ' ' || ch == '\t')
        {
            trimmed.split_whitespace().map(str::to_string).collect()
        } else {
            // Not a doc-block keyword line; roll back and let `DOC` lex normally.
            self.index = saved_index;
            self.line = saved_line;
            self.column = saved_column;
            return None;
        };

        // Consume the newline ending the keyword line.
        if !self.is_at_end() {
            self.advance_line();
        }

        let mut lines = Vec::new();
        let mut in_example = false;
        let mut terminated = false;
        while !self.is_at_end() {
            let line_no = self.line;
            let mut text = String::new();
            while !self.is_at_end() && self.peek() != '\n' {
                text.push(self.peek());
                self.advance();
            }
            if text.ends_with('\r') {
                text.pop();
            }
            if !self.is_at_end() {
                self.advance_line();
            }

            let words: Vec<&str> = text.split_whitespace().collect();
            let is_end = |kw: &str| {
                words.len() == 2
                    && words[0].eq_ignore_ascii_case("END")
                    && words[1].eq_ignore_ascii_case(kw)
            };
            if !in_example && is_end("DOC") {
                terminated = true;
                break;
            }
            if !in_example && words.len() == 1 && words[0].eq_ignore_ascii_case("EXAMPLE") {
                in_example = true;
            } else if in_example && is_end("EXAMPLE") {
                in_example = false;
            }
            lines.push(DocRawLine {
                line: line_no,
                text,
            });
        }

        if !terminated {
            self.report(
                "DOC_UNTERMINATED",
                "DOC block reached end of file before its `END DOC` line.",
                doc_line,
                1,
                1,
            );
        }

        Some(DocRaw {
            line: doc_line,
            attrs,
            lines,
        })
    }

    fn lex_line_continuation(&mut self) -> bool {
        if self.peek() != '_' {
            return false;
        }

        let mut lookahead = self.index + 1;
        while let Some(ch) = self.chars.get(lookahead).copied() {
            match ch {
                ' ' | '\t' | '\r' => lookahead += 1,
                '\n' => {
                    self.advance();
                    while self.index < lookahead {
                        self.advance();
                    }
                    self.advance_line();
                    return true;
                }
                _ => return false,
            }
        }

        false
    }

    fn skip_line_comment(&mut self) {
        while !self.is_at_end() && self.peek() != '\n' {
            self.advance();
        }
    }

    fn is_statement_start(&self) -> bool {
        self.tokens
            .last()
            .is_none_or(|token| matches!(token.kind, TokenKind::Newline | TokenKind::Colon))
    }

    fn push_and_advance(&mut self, kind: TokenKind) {
        self.push_simple(kind, 1);
        self.advance();
    }

    fn push_simple(&mut self, kind: TokenKind, width: usize) {
        self.tokens.push(Token {
            kind,
            line: self.line,
            start: self.column,
            end: self.column + width,
        });
    }

    fn advance(&mut self) {
        self.index += 1;
        self.column += 1;
    }

    fn advance_line(&mut self) {
        self.index += 1;
        self.line += 1;
        self.column = 1;
    }

    fn peek(&self) -> char {
        self.chars[self.index]
    }

    fn peek_next(&self) -> Option<char> {
        self.chars.get(self.index + 1).copied()
    }

    fn is_at_end(&self) -> bool {
        self.index >= self.chars.len()
    }

    fn report(&mut self, rule: &str, detail: &str, line: usize, start: usize, end: usize) {
        self.had_error = true;
        rules::show_diagnostic(rule, detail, self.path, line, start, end);
    }
}

/// Look up a keyword by its lexeme, case-insensitively. Exposed for source tools
/// (such as `mfb fmt`) that re-tokenize raw text without building a full lexer.
pub fn lookup_keyword(value: &str) -> Option<Keyword> {
    keyword(value)
}

fn keyword(value: &str) -> Option<Keyword> {
    if value.eq_ignore_ascii_case("AS") {
        Some(Keyword::As)
    } else if value.eq_ignore_ascii_case("CASE") {
        Some(Keyword::Case)
    } else if value.eq_ignore_ascii_case("CONTINUE") {
        Some(Keyword::Continue)
    } else if value.eq_ignore_ascii_case("DO") {
        Some(Keyword::Do)
    } else if value.eq_ignore_ascii_case("ELSE") {
        Some(Keyword::Else)
    } else if value.eq_ignore_ascii_case("ELSEIF") {
        Some(Keyword::ElseIf)
    } else if value.eq_ignore_ascii_case("FALSE") {
        Some(Keyword::False)
    } else if value.eq_ignore_ascii_case("FAIL") {
        Some(Keyword::Fail)
    } else if value.eq_ignore_ascii_case("EXIT") {
        Some(Keyword::Exit)
    } else if value.eq_ignore_ascii_case("FOR") {
        Some(Keyword::For)
    } else if value.eq_ignore_ascii_case("EACH") {
        Some(Keyword::Each)
    } else if value.eq_ignore_ascii_case("FUNC") {
        Some(Keyword::Func)
    } else if value.eq_ignore_ascii_case("IF") {
        Some(Keyword::If)
    } else if value.eq_ignore_ascii_case("IN") {
        Some(Keyword::In)
    } else if value.eq_ignore_ascii_case("IMPORT") {
        Some(Keyword::Import)
    } else if value.eq_ignore_ascii_case("ISOLATED") {
        Some(Keyword::Isolated)
    } else if value.eq_ignore_ascii_case("LET") {
        Some(Keyword::Let)
    } else if value.eq_ignore_ascii_case("LAMBDA") {
        Some(Keyword::Lambda)
    } else if value.eq_ignore_ascii_case("LOOP") {
        Some(Keyword::Loop)
    } else if value.eq_ignore_ascii_case("DIV") {
        Some(Keyword::Div)
    } else if value.eq_ignore_ascii_case("MOD") {
        Some(Keyword::Mod)
    } else if value.eq_ignore_ascii_case("MATCH") {
        Some(Keyword::Match)
    } else if value.eq_ignore_ascii_case("MUT") {
        Some(Keyword::Mut)
    } else if value.eq_ignore_ascii_case("RES") {
        Some(Keyword::Res)
    } else if value.eq_ignore_ascii_case("NOTHING") {
        Some(Keyword::Nothing)
    } else if value.eq_ignore_ascii_case("AND") {
        Some(Keyword::And)
    } else if value.eq_ignore_ascii_case("OR") {
        Some(Keyword::Or)
    } else if value.eq_ignore_ascii_case("NOT") {
        Some(Keyword::Not)
    } else if value.eq_ignore_ascii_case("NEXT") {
        Some(Keyword::Next)
    } else if value.eq_ignore_ascii_case("XOR") {
        Some(Keyword::Xor)
    } else if value.eq_ignore_ascii_case("RETURN") {
        Some(Keyword::Return)
    } else if value.eq_ignore_ascii_case("SUB") {
        Some(Keyword::Sub)
    } else if value.eq_ignore_ascii_case("THEN") {
        Some(Keyword::Then)
    } else if value.eq_ignore_ascii_case("TRUE") {
        Some(Keyword::True)
    } else if value.eq_ignore_ascii_case("END") {
        Some(Keyword::End)
    } else if value.eq_ignore_ascii_case("ENUM") {
        Some(Keyword::Enum)
    } else if value.eq_ignore_ascii_case("EXPORT") {
        Some(Keyword::Export)
    } else if value.eq_ignore_ascii_case("PUBLIC") {
        Some(Keyword::Public)
    } else if value.eq_ignore_ascii_case("PROGRAM") {
        Some(Keyword::Program)
    } else if value.eq_ignore_ascii_case("PRIVATE") {
        Some(Keyword::Private)
    } else if value.eq_ignore_ascii_case("PROPAGATE") {
        Some(Keyword::Propagate)
    } else if value.eq_ignore_ascii_case("RECOVER") {
        Some(Keyword::Recover)
    } else if value.eq_ignore_ascii_case("STEP") {
        Some(Keyword::Step)
    } else if value.eq_ignore_ascii_case("TO") {
        Some(Keyword::To)
    } else if value.eq_ignore_ascii_case("TYPE") {
        Some(Keyword::Type)
    } else if value.eq_ignore_ascii_case("TRAP") {
        Some(Keyword::Trap)
    } else if value.eq_ignore_ascii_case("UNTIL") {
        Some(Keyword::Until)
    } else if value.eq_ignore_ascii_case("UNION") {
        Some(Keyword::Union)
    } else if value.eq_ignore_ascii_case("WHEN") {
        Some(Keyword::When)
    } else if value.eq_ignore_ascii_case("TESTING") {
        Some(Keyword::Testing)
    } else if value.eq_ignore_ascii_case("WHILE") {
        Some(Keyword::While)
    } else if value.eq_ignore_ascii_case("WEND") {
        Some(Keyword::Wend)
    } else if value.eq_ignore_ascii_case("WITH") {
        Some(Keyword::With)
    } else {
        None
    }
}

/// The canonical lexeme for a keyword, used when a keyword token is accepted in
/// a name position (e.g. a native `LINK` function named `step`, which collides
/// with the `STEP` keyword). Definition and call sites both canonicalize through
/// here, so they match consistently.
pub fn keyword_lexeme(keyword: Keyword) -> &'static str {
    match keyword {
        Keyword::As => "as",
        Keyword::Case => "case",
        Keyword::Continue => "continue",
        Keyword::Do => "do",
        Keyword::Else => "else",
        Keyword::ElseIf => "elseif",
        Keyword::False => "false",
        Keyword::Fail => "fail",
        Keyword::Exit => "exit",
        Keyword::For => "for",
        Keyword::Each => "each",
        Keyword::Func => "func",
        Keyword::If => "if",
        Keyword::In => "in",
        Keyword::Import => "import",
        Keyword::Isolated => "isolated",
        Keyword::Let => "let",
        Keyword::Lambda => "lambda",
        Keyword::Loop => "loop",
        Keyword::Div => "div",
        Keyword::Mod => "mod",
        Keyword::Match => "match",
        Keyword::Mut => "mut",
        Keyword::Nothing => "nothing",
        Keyword::And => "and",
        Keyword::Or => "or",
        Keyword::Not => "not",
        Keyword::Next => "next",
        Keyword::Xor => "xor",
        Keyword::Return => "return",
        Keyword::Sub => "sub",
        Keyword::Testing => "testing",
        Keyword::Then => "then",
        Keyword::True => "true",
        Keyword::End => "end",
        Keyword::Enum => "enum",
        Keyword::Export => "export",
        Keyword::Public => "public",
        Keyword::Program => "program",
        Keyword::Private => "private",
        Keyword::Propagate => "propagate",
        Keyword::Recover => "recover",
        Keyword::Res => "res",
        Keyword::Step => "step",
        Keyword::To => "to",
        Keyword::Type => "type",
        Keyword::Trap => "trap",
        Keyword::Until => "until",
        Keyword::Union => "union",
        Keyword::When => "when",
        Keyword::While => "while",
        Keyword::Wend => "wend",
        Keyword::With => "with",
    }
}

fn is_identifier_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trailing_underscore_continues_line_without_newline_token() {
        let tokens = lex(
            Path::new("main.mfb"),
            "LET msg = \"hello \" & _\n          \"world\"\n",
        )
        .expect("lex source");

        assert_eq!(
            tokens.iter().map(|token| &token.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Keyword(Keyword::Let),
                &TokenKind::Identifier("msg".to_string()),
                &TokenKind::Equal,
                &TokenKind::String("hello ".to_string()),
                &TokenKind::Ampersand,
                &TokenKind::String("world".to_string()),
                &TokenKind::Newline,
                &TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn underscore_identifier_remains_available_when_not_trailing() {
        let tokens = lex(Path::new("main.mfb"), "sum(_)\n").expect("lex source");

        assert_eq!(
            tokens.iter().map(|token| &token.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Identifier("sum".to_string()),
                &TokenKind::LParen,
                &TokenKind::Identifier("_".to_string()),
                &TokenKind::RParen,
                &TokenKind::Newline,
                &TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn apostrophe_comments_skip_to_newline() {
        let tokens = lex(Path::new("main.mfb"), "' ignored\nLET value = 1\n").expect("lex source");

        assert_eq!(
            tokens.iter().map(|token| &token.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Newline,
                &TokenKind::Keyword(Keyword::Let),
                &TokenKind::Identifier("value".to_string()),
                &TokenKind::Equal,
                &TokenKind::Number("1".to_string()),
                &TokenKind::Newline,
                &TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn rem_comments_skip_to_newline_at_statement_start() {
        let tokens = lex(
            Path::new("main.mfb"),
            "rEm ignored\nLET value = 1 : REM also ignored\nLET other = 2\n",
        )
        .expect("lex source");

        assert_eq!(
            tokens.iter().map(|token| &token.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Newline,
                &TokenKind::Keyword(Keyword::Let),
                &TokenKind::Identifier("value".to_string()),
                &TokenKind::Equal,
                &TokenKind::Number("1".to_string()),
                &TokenKind::Colon,
                &TokenKind::Newline,
                &TokenKind::Keyword(Keyword::Let),
                &TokenKind::Identifier("other".to_string()),
                &TokenKind::Equal,
                &TokenKind::Number("2".to_string()),
                &TokenKind::Newline,
                &TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn identifiers_containing_rem_remain_identifiers() {
        let tokens =
            lex(Path::new("main.mfb"), "LET premium = remember + REMvalue\n").expect("lex source");

        assert_eq!(
            tokens.iter().map(|token| &token.kind).collect::<Vec<_>>(),
            vec![
                &TokenKind::Keyword(Keyword::Let),
                &TokenKind::Identifier("premium".to_string()),
                &TokenKind::Equal,
                &TokenKind::Identifier("remember".to_string()),
                &TokenKind::Plus,
                &TokenKind::Identifier("REMvalue".to_string()),
                &TokenKind::Newline,
                &TokenKind::Eof,
            ]
        );
    }

    // Every keyword lexeme round-trips: lookup_keyword recognizes it (case-
    // insensitively) and keyword_lexeme maps the variant back to that lexeme.
    // Drives all arms of both big match statements.
    #[test]
    fn keyword_lookup_and_lexeme_round_trip_for_all_keywords() {
        const LEXEMES: &[&str] = &[
            "as",
            "case",
            "continue",
            "do",
            "else",
            "elseif",
            "false",
            "fail",
            "exit",
            "for",
            "each",
            "func",
            "if",
            "in",
            "import",
            "isolated",
            "let",
            "lambda",
            "loop",
            "div",
            "mod",
            "match",
            "mut",
            "nothing",
            "and",
            "or",
            "not",
            "next",
            "xor",
            "return",
            "sub",
            "then",
            "true",
            "end",
            "enum",
            "export",
            "public",
            "program",
            "private",
            "propagate",
            "recover",
            "res",
            "step",
            "to",
            "type",
            "trap",
            "until",
            "union",
            "when",
            "while",
            "wend",
            "with",
        ];
        for lexeme in LEXEMES {
            let keyword =
                lookup_keyword(lexeme).unwrap_or_else(|| panic!("`{lexeme}` should be a keyword"));
            assert_eq!(keyword_lexeme(keyword), *lexeme, "round-trip for {lexeme}");
            // Case-insensitive recognition.
            assert_eq!(lookup_keyword(&lexeme.to_uppercase()), Some(keyword));
        }
        assert_eq!(lookup_keyword("notakeyword"), None);
    }

    #[test]
    fn string_escapes_are_decoded_including_unknown_escapes() {
        // Source: "a\"b\\c\nd\te\zf" — decodes \" \\ \n \t and passes an unknown
        // escape (\z) through as the bare character.
        let tokens =
            lex(Path::new("main.mfb"), "\"a\\\"b\\\\c\\nd\\te\\zf\"\n").expect("lex source");
        assert_eq!(
            tokens[0].kind,
            TokenKind::String("a\"b\\c\nd\tezf".to_string())
        );
    }

    #[test]
    fn carriage_return_and_nul_escapes_decode() {
        // "\r" -> U+000D, "\0" -> U+0000 (plan-27 Phase 1).
        let tokens = lex(Path::new("main.mfb"), "\"a\\rb\\0c\"\n").expect("lex source");
        assert_eq!(tokens[0].kind, TokenKind::String("a\rb\0c".to_string()));
    }

    #[test]
    fn unicode_scalar_escapes_decode() {
        // "\u{41}" -> 'A', "\u{1F600}" -> 😀 (4-byte), case-insensitive hex,
        // 1-digit and 6-digit bounds (plan-27 Phase 2).
        let tokens = lex(
            Path::new("main.mfb"),
            "\"\\u{41}-\\u{1F600}-\\u{6a}-\\u{0}-\\u{10FFFF}\"\n",
        )
        .expect("lex source");
        assert_eq!(
            tokens[0].kind,
            TokenKind::String("A-\u{1F600}-j-\u{0}-\u{10FFFF}".to_string())
        );
    }

    #[test]
    fn malformed_unicode_escapes_are_errors() {
        // Missing brace, empty, out of range, surrogate, unterminated, non-hex.
        for source in [
            "\"\\u41\"\n",
            "\"\\u{}\"\n",
            "\"\\u{110000}\"\n",
            "\"\\u{D800}\"\n",
            "\"\\u{1F600\"\n",
            "\"\\u{7fffffff}\"\n",
            "\"\\u{GG}\"\n",
        ] {
            assert!(
                lex(Path::new("main.mfb"), source).is_err(),
                "expected lex error for {source:?}"
            );
        }
    }

    #[test]
    fn unterminated_string_on_line_is_an_error() {
        // Newline before the closing quote.
        assert!(lex(Path::new("main.mfb"), "\"abc\ndef\n").is_err());
    }

    #[test]
    fn unterminated_string_at_eof_is_an_error() {
        // End-of-file before the closing quote (no trailing newline).
        assert!(lex(Path::new("main.mfb"), "\"abc").is_err());
        // A trailing backslash at EOF (escape with nothing after it) also fails.
        assert!(lex(Path::new("main.mfb"), "\"abc\\").is_err());
    }

    #[test]
    fn unexpected_character_is_reported_as_an_error() {
        assert!(lex(Path::new("main.mfb"), "LET x = @\n").is_err());
        // A lone `|` (not `|>`) is unexpected.
        assert!(lex(Path::new("main.mfb"), "LET x = 1 | 2\n").is_err());
    }

    #[test]
    fn pipe_greater_is_a_single_operator_token() {
        let tokens = lex(Path::new("main.mfb"), "x |> f\n").expect("lex source");
        assert!(tokens.iter().any(|t| t.kind == TokenKind::PipeGreater));
    }

    #[test]
    fn decimal_numbers_lex_as_a_single_number_token() {
        let tokens = lex(Path::new("main.mfb"), "LET pi = 3.14\n").expect("lex source");
        assert!(tokens
            .iter()
            .any(|t| t.kind == TokenKind::Number("3.14".to_string())));
    }
}
