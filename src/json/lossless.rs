//! A strict RFC 8259 JSON reader that keeps what `tinyjson` throws away
//! (plan-167-A §4).
//!
//! `tinyjson` decodes every number to `f64` (so `3` and `3.0` are the same value
//! and integers past 2^53 lose precision), lets a duplicate object key silently
//! overwrite the earlier one, and stores objects in a `HashMap` (so member order
//! and positions are gone). The `config` section of `project.json` needs all
//! three: Integer vs Float is decided by the number's spelling, a duplicate key
//! is an error, and every diagnostic points at the offending key's line.
//!
//! This reader keeps each number's exact lexeme, keeps object members in source
//! order with duplicates, and records the 1-based line of every member's key. It
//! is used only for the config sections; every other manifest field stays on
//! `tinyjson`.

use super::MAX_JSON_NESTING_DEPTH;

/// A parsed JSON value that loses nothing the config validator needs.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum LosslessValue {
    Null,
    Bool(bool),
    /// The number exactly as written, e.g. `-12`, `3.0`, `1e5`.
    Number(String),
    /// The string with its escapes decoded.
    String(String),
    Array(Vec<LosslessValue>),
    /// Members in source order; a repeated key appears once per occurrence.
    Object(Vec<LosslessMember>),
}

/// One `"key": value` member of an object.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LosslessMember {
    pub(crate) key: String,
    /// The 1-based line of the member's key token.
    pub(crate) line: u32,
    pub(crate) value: LosslessValue,
}

/// A syntax error, with the 1-based line it was found on.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LosslessError {
    pub(crate) line: u32,
    pub(crate) message: String,
}

/// Parse `text` as one JSON value, rejecting anything RFC 8259 rejects (trailing
/// commas, comments, leading zeros, `+1`, `.5`, lone surrogates, control
/// characters inside strings, trailing content) and any nesting deeper than
/// [`MAX_JSON_NESTING_DEPTH`].
pub(crate) fn parse_lossless(text: &str) -> Result<LosslessValue, LosslessError> {
    let mut parser = Parser {
        bytes: text.as_bytes(),
        pos: 0,
        line: 1,
        depth: 0,
    };
    parser.skip_whitespace();
    let value = parser.value()?;
    parser.skip_whitespace();
    if parser.pos < parser.bytes.len() {
        return Err(parser.error("unexpected content after the JSON value"));
    }
    Ok(value)
}

struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// The current 1-based line. A raw newline can only occur in whitespace,
    /// because a string rejects every control character, so counting newlines
    /// in `skip_whitespace` keeps this exact.
    line: u32,
    /// Open arrays and objects around the current position.
    depth: usize,
}

impl Parser<'_> {
    fn error(&self, message: &str) -> LosslessError {
        LosslessError {
            line: self.line,
            message: message.to_string(),
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn skip_whitespace(&mut self) {
        while let Some(byte) = self.peek() {
            match byte {
                b'\n' => self.line += 1,
                b' ' | b'\t' | b'\r' => {}
                _ => return,
            }
            self.pos += 1;
        }
    }

    fn expect_literal(
        &mut self,
        literal: &[u8],
        value: LosslessValue,
    ) -> Result<LosslessValue, LosslessError> {
        if self.bytes[self.pos..].starts_with(literal) {
            self.pos += literal.len();
            Ok(value)
        } else {
            Err(self.error("invalid literal; expected `true`, `false` or `null`"))
        }
    }

    fn value(&mut self) -> Result<LosslessValue, LosslessError> {
        match self.peek() {
            None => Err(self.error("unexpected end of input; expected a value")),
            Some(b'{') => self.object(),
            Some(b'[') => self.array(),
            Some(b'"') => Ok(LosslessValue::String(self.string()?)),
            Some(b't') => self.expect_literal(b"true", LosslessValue::Bool(true)),
            Some(b'f') => self.expect_literal(b"false", LosslessValue::Bool(false)),
            Some(b'n') => self.expect_literal(b"null", LosslessValue::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.error("unexpected character; expected a value")),
        }
    }

    fn enter(&mut self) -> Result<(), LosslessError> {
        self.depth += 1;
        if self.depth > MAX_JSON_NESTING_DEPTH {
            return Err(self.error(&format!(
                "JSON nested too deeply (maximum {MAX_JSON_NESTING_DEPTH} levels)"
            )));
        }
        Ok(())
    }

    fn object(&mut self) -> Result<LosslessValue, LosslessError> {
        self.enter()?;
        self.pos += 1; // `{`
        let mut members = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b'}') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(LosslessValue::Object(members));
        }
        loop {
            self.skip_whitespace();
            if self.peek() != Some(b'"') {
                return Err(self.error("expected a string key in an object"));
            }
            let line = self.line;
            let key = self.string()?;
            self.skip_whitespace();
            if self.peek() != Some(b':') {
                return Err(self.error("expected `:` after an object key"));
            }
            self.pos += 1;
            self.skip_whitespace();
            let value = self.value()?;
            members.push(LosslessMember { key, line, value });
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b'}') => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Ok(LosslessValue::Object(members));
                }
                _ => return Err(self.error("expected `,` or `}` after an object member")),
            }
        }
    }

    fn array(&mut self) -> Result<LosslessValue, LosslessError> {
        self.enter()?;
        self.pos += 1; // `[`
        let mut items = Vec::new();
        self.skip_whitespace();
        if self.peek() == Some(b']') {
            self.pos += 1;
            self.depth -= 1;
            return Ok(LosslessValue::Array(items));
        }
        loop {
            self.skip_whitespace();
            items.push(self.value()?);
            self.skip_whitespace();
            match self.peek() {
                Some(b',') => self.pos += 1,
                Some(b']') => {
                    self.pos += 1;
                    self.depth -= 1;
                    return Ok(LosslessValue::Array(items));
                }
                _ => return Err(self.error("expected `,` or `]` after an array element")),
            }
        }
    }

    /// `-?(0|[1-9][0-9]*)(\.[0-9]+)?([eE][+-]?[0-9]+)?`, kept as written.
    fn number(&mut self) -> Result<LosslessValue, LosslessError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => {
                self.pos += 1;
                if self.peek().is_some_and(|b| b.is_ascii_digit()) {
                    return Err(self.error("a number may not have a leading zero"));
                }
            }
            Some(b'1'..=b'9') => self.digits(),
            _ => return Err(self.error("expected a digit in a number")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                return Err(self.error("expected a digit after `.` in a number"));
            }
            self.digits();
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if !self.peek().is_some_and(|b| b.is_ascii_digit()) {
                return Err(self.error("expected a digit in a number's exponent"));
            }
            self.digits();
        }
        // The grammar above admits ASCII only, so this slice is valid UTF-8.
        let lexeme = std::str::from_utf8(&self.bytes[start..self.pos])
            .expect("a number lexeme is ASCII")
            .to_string();
        Ok(LosslessValue::Number(lexeme))
    }

    fn digits(&mut self) {
        while self.peek().is_some_and(|b| b.is_ascii_digit()) {
            self.pos += 1;
        }
    }

    fn hex4(&mut self) -> Result<u32, LosslessError> {
        let Some(hex) = self.bytes.get(self.pos..self.pos + 4) else {
            return Err(self.error("expected four hex digits after `\\u`"));
        };
        let mut code = 0u32;
        for &byte in hex {
            let digit = (byte as char)
                .to_digit(16)
                .ok_or_else(|| self.error("expected four hex digits after `\\u`"))?;
            code = code * 16 + digit;
        }
        self.pos += 4;
        Ok(code)
    }

    /// A string token starting at `"`, with its escapes decoded.
    fn string(&mut self) -> Result<String, LosslessError> {
        self.pos += 1; // opening `"`
        let mut out = String::new();
        loop {
            // Copy the run of plain bytes up to the next `"`, `\` or control
            // character in one step. The input is a `&str`, and the run ends at
            // an ASCII byte, so the run is whole UTF-8.
            let run_start = self.pos;
            while let Some(byte) = self.peek() {
                if byte == b'"' || byte == b'\\' || byte < 0x20 {
                    break;
                }
                self.pos += 1;
            }
            out.push_str(
                std::str::from_utf8(&self.bytes[run_start..self.pos])
                    .expect("a run ending at an ASCII byte of a &str is valid UTF-8"),
            );
            match self.peek() {
                None => return Err(self.error("unterminated string")),
                Some(b'"') => {
                    self.pos += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.pos += 1;
                    let Some(escape) = self.peek() else {
                        return Err(self.error("unterminated string"));
                    };
                    self.pos += 1;
                    match escape {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error("invalid escape in a string")),
                    }
                }
                Some(_) => {
                    return Err(self.error("a control character must be escaped in a string"))
                }
            }
        }
    }

    /// The scalar named by a `\uXXXX` escape (the `\u` already consumed),
    /// joining a surrogate pair and rejecting a lone surrogate.
    fn unicode_escape(&mut self) -> Result<char, LosslessError> {
        let first = self.hex4()?;
        let code = match first {
            0xD800..=0xDBFF => {
                if !self.bytes[self.pos..].starts_with(b"\\u") {
                    return Err(self.error("a high surrogate must be followed by a low surrogate"));
                }
                self.pos += 2;
                let second = self.hex4()?;
                if !(0xDC00..=0xDFFF).contains(&second) {
                    return Err(self.error("a high surrogate must be followed by a low surrogate"));
                }
                0x10000 + ((first - 0xD800) << 10) + (second - 0xDC00)
            }
            0xDC00..=0xDFFF => {
                return Err(self.error("a low surrogate must follow a high surrogate"))
            }
            _ => first,
        };
        char::from_u32(code).ok_or_else(|| self.error("invalid unicode escape"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tinyjson::JsonValue;

    fn object(text: &str) -> Vec<LosslessMember> {
        match parse_lossless(text).expect("parses") {
            LosslessValue::Object(members) => members,
            other => panic!("expected an object, got {other:?}"),
        }
    }

    #[test]
    fn number_lexemes_are_kept_exactly() {
        for lexeme in [
            "3",
            "3.0",
            "-0",
            "1e5",
            "2E-3",
            "-12",
            "1.25e+10",
            "9223372036854775808",
        ] {
            assert_eq!(
                parse_lossless(lexeme),
                Ok(LosslessValue::Number(lexeme.to_string())),
                "{lexeme}"
            );
        }
    }

    #[test]
    fn duplicate_keys_are_kept_in_order() {
        let members = object(r#"{"a": 1, "b": 2, "a": 3}"#);
        let keys: Vec<_> = members.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["a", "b", "a"]);
        assert_eq!(members[2].value, LosslessValue::Number("3".to_string()));
    }

    #[test]
    fn member_lines_are_the_key_lines() {
        let text = "{\n  \"a\": 1,\n\n  \"b\": {\n    \"c\": \"x\"\n  },\n  \"d\":\n    2\n}";
        let members = object(text);
        assert_eq!(
            members.iter().map(|m| m.line).collect::<Vec<_>>(),
            [2, 4, 7]
        );
        let LosslessValue::Object(inner) = &members[1].value else {
            panic!("b is an object");
        };
        assert_eq!(inner[0].line, 5);
    }

    #[test]
    fn escapes_and_surrogate_pairs_are_decoded() {
        assert_eq!(
            parse_lossless(r#""q\" b\\ s\/ \b\f\n\r\t \u00e9 \ud83d\ude00""#),
            Ok(LosslessValue::String(
                "q\" b\\ s/ \u{8}\u{c}\n\r\t é 😀".to_string()
            ))
        );
        // Non-ASCII text passes through unescaped.
        assert_eq!(
            parse_lossless("\"héllo ✓\""),
            Ok(LosslessValue::String("héllo ✓".to_string()))
        );
    }

    #[test]
    fn lone_surrogates_are_rejected() {
        for text in [
            r#""\ud83d""#,
            r#""\ud83dx""#,
            r#""\ude00""#,
            r#""\ud83d\u0041""#,
        ] {
            assert!(parse_lossless(text).is_err(), "{text}");
        }
    }

    #[test]
    fn non_rfc_8259_input_is_rejected() {
        for text in [
            r#"{"a": 1,}"#,
            "[1, 2,]",
            "{\"a\": 1 // c\n}",
            "/* c */ {}",
            "01",
            "+1",
            ".5",
            "1.",
            "1e",
            "-",
            "{'a': 1}",
            "\"tab\there\"",
            "\"a\nb\"",
            "\"\\x\"",
            "{} {}",
            "tru",
            "",
            "{\"a\" 1}",
            "[1 2]",
        ] {
            assert!(parse_lossless(text).is_err(), "should reject {text:?}");
        }
    }

    #[test]
    fn errors_carry_their_line() {
        let err = parse_lossless("{\n  \"a\": 1,\n  \"b\": 01\n}").expect_err("leading zero");
        assert_eq!(err.line, 3);
    }

    #[test]
    fn nesting_is_capped_like_check_json_depth() {
        let at_cap = "[".repeat(MAX_JSON_NESTING_DEPTH) + &"]".repeat(MAX_JSON_NESTING_DEPTH);
        assert!(parse_lossless(&at_cap).is_ok());
        let past = "[".repeat(MAX_JSON_NESTING_DEPTH + 1) + &"]".repeat(MAX_JSON_NESTING_DEPTH + 1);
        let err = parse_lossless(&past).expect_err("past the cap");
        assert!(err.message.contains("nested too deeply"), "{}", err.message);
        // A huge depth fails at the cap, never by overflowing the stack.
        let huge = "[".repeat(120_000);
        assert!(parse_lossless(&huge).is_err());
    }

    /// The test-only bridge to `tinyjson`'s model: numbers become `f64` and a
    /// repeated key keeps its last value, which is what `tinyjson` does.
    fn to_tinyjson(value: &LosslessValue) -> JsonValue {
        match value {
            LosslessValue::Null => JsonValue::Null,
            LosslessValue::Bool(b) => JsonValue::Boolean(*b),
            LosslessValue::Number(lexeme) => {
                JsonValue::Number(lexeme.parse::<f64>().expect("a JSON number lexeme"))
            }
            LosslessValue::String(s) => JsonValue::String(s.clone()),
            LosslessValue::Array(items) => {
                JsonValue::Array(items.iter().map(to_tinyjson).collect())
            }
            LosslessValue::Object(members) => {
                let mut map = HashMap::new();
                for member in members {
                    map.insert(member.key.clone(), to_tinyjson(&member.value));
                }
                JsonValue::Object(map)
            }
        }
    }

    fn project_json_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                project_json_files(&path, out);
            } else if path.file_name().is_some_and(|name| name == "project.json") {
                out.push(path);
            }
        }
    }

    #[test]
    fn agrees_with_tinyjson_on_every_project_json_under_tests() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
        let mut files = Vec::new();
        project_json_files(&root, &mut files);
        assert!(files.len() > 1000, "found only {} manifests", files.len());
        let mut parsed = 0;
        for path in &files {
            let Ok(text) = std::fs::read_to_string(path) else {
                continue;
            };
            let tiny = text.parse::<JsonValue>();
            let lossless = parse_lossless(&text);
            match (tiny, lossless) {
                (Ok(tiny), Ok(lossless)) => {
                    assert_eq!(to_tinyjson(&lossless), tiny, "{}", path.display());
                    parsed += 1;
                }
                (Err(_), Err(_)) => {}
                (tiny, lossless) => panic!(
                    "{}: tinyjson ok={} lossless ok={}",
                    path.display(),
                    tiny.is_ok(),
                    lossless.is_ok()
                ),
            }
        }
        assert!(parsed > 1000, "parsed only {parsed}");
    }
}
