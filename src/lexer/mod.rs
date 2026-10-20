use crate::rules;
use std::path::Path;

mod keywords;
mod literals;
pub use keywords::*;

#[derive(Clone, Debug, PartialEq)]
pub enum TokenKind {
    Identifier(String),
    Keyword(Keyword),
    String(String),
    Number(String),
    /// A backtick scalar literal `` `x` `` carrying its decoded Unicode scalar
    /// value (plan-41-A). Decoding happens at lex time so range/surrogate errors
    /// surface at the true source location and the AST stays free of raw text.
    Scalar(u32),
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
        open_brackets: Vec::new(),
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
    /// bug-821: every `(`, `[` and `{` not yet closed, innermost last. While it
    /// is non-empty a newline is NOT a statement separator (no `Newline` token
    /// is emitted), so a literal, constructor, call or grouped expression may
    /// span lines. Each entry keeps the opener's position so an unclosed bracket
    /// is reported where it was opened, not where the damage surfaced.
    open_brackets: Vec<OpenBracket>,
}

/// An opening bracket the lexer has not yet seen closed (bug-821).
struct OpenBracket {
    ch: char,
    line: usize,
    column: usize,
}

/// Keywords that can only begin a statement, block clause or declaration, never
/// an expression, a type, or a parameter. Inside an open bracket every line
/// holds part of an expression/type/parameter list, so a physical line whose
/// first word is one of these proves the bracket was left unclosed (bug-821):
/// the lexer reports it at the opener instead of swallowing the rest of the
/// file into one statement. `FUNC`/`ISOLATED` (function types), `MUT`/`RES`
/// (parameters, `RES` value types), `WITH`, `LAMBDA`, `NOT`, the literal and
/// operator keywords, `AS`, `TO` and `IN` all legitimately start a line inside
/// a bracket, so they are absent. `mfb fmt` applies the same test so its
/// logical lines agree with the lexer's statements.
pub fn begins_statement_only(keyword: Keyword) -> bool {
    matches!(
        keyword,
        Keyword::Case
            | Keyword::Continue
            | Keyword::Do
            | Keyword::Each
            | Keyword::Else
            | Keyword::ElseIf
            | Keyword::End
            | Keyword::Enum
            | Keyword::Exit
            | Keyword::Export
            | Keyword::Fail
            | Keyword::For
            | Keyword::If
            | Keyword::Import
            | Keyword::Let
            | Keyword::Loop
            | Keyword::Match
            | Keyword::Next
            | Keyword::Private
            | Keyword::Program
            | Keyword::Propagate
            | Keyword::Public
            | Keyword::Recover
            | Keyword::Return
            | Keyword::Step
            | Keyword::Testing
            | Keyword::Then
            | Keyword::Trap
            | Keyword::Type
            | Keyword::Union
            | Keyword::Until
            | Keyword::When
            | Keyword::While
    )
}

impl Lexer<'_> {
    fn lex_all(&mut self) {
        while !self.is_at_end() {
            let ch = self.peek();
            match ch {
                ' ' | '\t' | '\r' => {
                    self.advance();
                }
                '\n' => self.lex_newline(),
                '\'' => self.skip_line_comment(),
                '`' => self.lex_scalar(),
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
                '[' => self.open_bracket(TokenKind::LBracket),
                ']' => self.close_bracket(TokenKind::RBracket),
                '(' => self.open_bracket(TokenKind::LParen),
                ')' => self.close_bracket(TokenKind::RParen),
                '{' => self.open_bracket(TokenKind::LBrace),
                '}' => self.close_bracket(TokenKind::RBrace),
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

        self.report_unclosed_brackets("the end of the file was reached");
        self.tokens.push(Token {
            kind: TokenKind::Eof,
            line: self.line,
            start: self.column,
            end: self.column,
        });
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
        // bug-299 D2: line-leading, not merely statement-leading.
        // `is_statement_start` also accepts a `:` separator, so `LET x = 1 : DOC`
        // began a verbatim capture -- but the PARSER rejects a `DOC` there
        // (`MFB_PARSE_UNEXPECTED_STATEMENT`), so the capture could never produce a
        // compilable program. What it did produce was a lexer/formatter
        // disagreement: `fmt::is_doc_start` only recognizes a line whose first word
        // is `DOC`, so fmt scanned the block's prose as code -- uppercasing English
        // words that spell keywords (`if`, `and`, `to`, `not`) in text the spec
        // says is preserved -- and its `END DOC` popped the enclosing block,
        // mis-indenting the rest of the file. Restricting the capture to the
        // position the parser actually accepts makes the two agree.
        if value.eq_ignore_ascii_case("DOC") && self.is_line_start() {
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
        let attrs = if is_doc_attr_rest(trimmed) {
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
            if !in_example && is_end_line(&words, "DOC") {
                terminated = true;
                break;
            }
            if !in_example && words.len() == 1 && words[0].eq_ignore_ascii_case("EXAMPLE") {
                in_example = true;
            } else if in_example && is_end_line(&words, "EXAMPLE") {
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

    /// A newline is a statement separator only outside every bracket (bug-821).
    /// Inside one it is skipped like other whitespace — unless the next line
    /// opens with a keyword that can only begin a statement, which proves the
    /// bracket was left unclosed: report it at its opener and resume normal
    /// separation so the rest of the file is not swallowed into one statement.
    fn lex_newline(&mut self) {
        if !self.open_brackets.is_empty() {
            match self.next_line_statement_keyword() {
                None => {
                    self.advance_line();
                    return;
                }
                Some(word) => {
                    let reason =
                        format!("`{word}` on line {} begins a new statement", self.line + 1);
                    self.report_unclosed_brackets(&reason);
                }
            }
        }
        self.push_simple(TokenKind::Newline, 1);
        self.advance_line();
    }

    /// The first word of the line after the newline at `self.index`, when it is
    /// a keyword that can only begin a statement ([`begins_statement_only`]).
    fn next_line_statement_keyword(&self) -> Option<String> {
        let mut at = self.index + 1;
        while matches!(self.chars.get(at), Some(' ' | '\t' | '\r')) {
            at += 1;
        }
        let start = at;
        while self
            .chars
            .get(at)
            .is_some_and(|&ch| is_identifier_continue(ch))
        {
            at += 1;
        }
        let word: String = self.chars[start..at].iter().collect();
        keyword(&word)
            .filter(|&kw| begins_statement_only(kw))
            .map(|kw| keyword_lexeme(kw).to_ascii_uppercase())
    }

    fn open_bracket(&mut self, kind: TokenKind) {
        self.open_brackets.push(OpenBracket {
            ch: self.peek(),
            line: self.line,
            column: self.column,
        });
        self.push_and_advance(kind);
    }

    /// A closer pops the innermost open bracket whatever its kind: a mismatched
    /// pair (`f(a]`) is the parser's error to report, at the closer, and popping
    /// keeps the depth — and so newline separation — in step with the source.
    /// A closer with nothing open is likewise left to the parser.
    fn close_bracket(&mut self, kind: TokenKind) {
        self.open_brackets.pop();
        self.push_and_advance(kind);
    }

    /// Report every still-open bracket at its opening position, then forget
    /// them (so lexing continues with ordinary newline separation).
    fn report_unclosed_brackets(&mut self, reason: &str) {
        for open in std::mem::take(&mut self.open_brackets) {
            self.report(
                "MFB_LEX_UNCLOSED_BRACKET",
                &format!(
                    "`{}` is never closed: {reason} before its closing `{}`.",
                    open.ch,
                    match open.ch {
                        '(' => ')',
                        '[' => ']',
                        _ => '}',
                    }
                ),
                open.line,
                open.column,
                open.column + 1,
            );
        }
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

    /// At the start of a physical line — a stricter form of
    /// [`Self::is_statement_start`] that does NOT accept a `:` separator.
    ///
    /// `DOC` uses this (bug-299 D2): a documentation block is a declaration-level
    /// construct (§21: it "may sit immediately before its declaration or stand
    /// alone"), and the parser rejects one after a `:` regardless, so capturing
    /// there only created a disagreement with the formatter.
    fn is_line_start(&self) -> bool {
        self.tokens
            .last()
            .is_none_or(|token| matches!(token.kind, TokenKind::Newline))
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

#[cfg(test)]
mod tests;
