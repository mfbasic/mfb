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
    LParen,
    RParen,
    Equal,
    Plus,
    Minus,
    Star,
    Slash,
    Ampersand,
    Caret,
    Newline,
    Eof,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Keyword {
    As,
    False,
    Func,
    Import,
    Let,
    Mut,
    Return,
    Sub,
    True,
    End,
    Enum,
    Type,
    Union,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

pub fn lex(path: &Path, source: &str) -> Result<Vec<Token>, ()> {
    let mut lexer = Lexer {
        path,
        chars: source.chars().collect(),
        index: 0,
        line: 1,
        column: 1,
        tokens: Vec::new(),
        had_error: false,
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
                'A'..='Z' | 'a'..='z' | '_' => self.lex_identifier_or_keyword(),
                '.' => self.push_and_advance(TokenKind::Dot),
                ',' => self.push_and_advance(TokenKind::Comma),
                ':' => self.push_and_advance(TokenKind::Colon),
                '(' => self.push_and_advance(TokenKind::LParen),
                ')' => self.push_and_advance(TokenKind::RParen),
                '=' => self.push_and_advance(TokenKind::Equal),
                '+' => self.push_and_advance(TokenKind::Plus),
                '-' => self.push_and_advance(TokenKind::Minus),
                '*' => self.push_and_advance(TokenKind::Star),
                '/' => self.push_and_advance(TokenKind::Slash),
                '&' => self.push_and_advance(TokenKind::Ampersand),
                '^' => self.push_and_advance(TokenKind::Caret),
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

    fn lex_number(&mut self) {
        let line = self.line;
        let start = self.column;
        let mut value = String::new();

        while !self.is_at_end() && self.peek().is_ascii_digit() {
            value.push(self.peek());
            self.advance();
        }

        if !self.is_at_end()
            && self.peek() == '.'
            && self.peek_next().is_some_and(|ch| ch.is_ascii_digit())
        {
            value.push(self.peek());
            self.advance();
            while !self.is_at_end() && self.peek().is_ascii_digit() {
                value.push(self.peek());
                self.advance();
            }
        }

        self.tokens.push(Token {
            kind: TokenKind::Number(value),
            line,
            start,
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

        if value.eq_ignore_ascii_case("REM")
            && self
                .tokens
                .last()
                .is_none_or(|token| matches!(token.kind, TokenKind::Newline))
        {
            self.skip_line_comment();
            return;
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

    fn skip_line_comment(&mut self) {
        while !self.is_at_end() && self.peek() != '\n' {
            self.advance();
        }
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

fn keyword(value: &str) -> Option<Keyword> {
    if value.eq_ignore_ascii_case("AS") {
        Some(Keyword::As)
    } else if value.eq_ignore_ascii_case("FALSE") {
        Some(Keyword::False)
    } else if value.eq_ignore_ascii_case("FUNC") {
        Some(Keyword::Func)
    } else if value.eq_ignore_ascii_case("IMPORT") {
        Some(Keyword::Import)
    } else if value.eq_ignore_ascii_case("LET") {
        Some(Keyword::Let)
    } else if value.eq_ignore_ascii_case("MUT") {
        Some(Keyword::Mut)
    } else if value.eq_ignore_ascii_case("RETURN") {
        Some(Keyword::Return)
    } else if value.eq_ignore_ascii_case("SUB") {
        Some(Keyword::Sub)
    } else if value.eq_ignore_ascii_case("TRUE") {
        Some(Keyword::True)
    } else if value.eq_ignore_ascii_case("END") {
        Some(Keyword::End)
    } else if value.eq_ignore_ascii_case("ENUM") {
        Some(Keyword::Enum)
    } else if value.eq_ignore_ascii_case("TYPE") {
        Some(Keyword::Type)
    } else if value.eq_ignore_ascii_case("UNION") {
        Some(Keyword::Union)
    } else {
        None
    }
}

fn is_identifier_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
