use super::*;

mod edges;

/// bug-299 D2: `DOC` began a verbatim capture anywhere `is_statement_start`
/// held, which includes just after a `:` separator. The parser rejects a `DOC`
/// there regardless, so the capture could never yield a compilable program --
/// what it did yield was a disagreement with `fmt::is_doc_start`, which only
/// recognizes a line whose FIRST word is `DOC`. fmt therefore scanned the
/// captured prose as code, uppercasing English words that spell keywords, and
/// its `END DOC` popped the enclosing block. Capture is now line-leading, which
/// is the position the parser actually accepts.
#[test]
fn doc_captures_only_at_the_start_of_a_line() {
    // Line-leading: still a single verbatim Doc token.
    let tokens = lex(
        Path::new("main.mfb"),
        "DOC
  FUNC f
  DESC if and to
END DOC
",
    )
    .expect("lex source");
    assert!(
        tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Doc(_))),
        "a line-leading DOC must still capture: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );

    // After a `:` separator: an ordinary identifier, no capture.
    let tokens = lex(
        Path::new("main.mfb"),
        "LET x = 1 : DOC
  prose if and to
END DOC
",
    )
    .expect("lex source");
    assert!(
        !tokens
            .iter()
            .any(|token| matches!(token.kind, TokenKind::Doc(_))),
        "a DOC after `:` must not start a capture: {:?}",
        tokens.iter().map(|t| &t.kind).collect::<Vec<_>>()
    );
}

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
        "then",
        "true",
        "end",
        "enum",
        "export",
        "public",
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
    // `WEND` was removed from the language (bug-357) and is not reserved.
    assert_eq!(lookup_keyword("wend"), None);
}

#[test]
fn is_end_line_matches_only_two_word_end_keyword() {
    assert!(is_end_line(&["END", "DOC"], "DOC"));
    assert!(is_end_line(&["end", "doc"], "DOC")); // case-insensitive
    assert!(is_end_line(&["END", "EXAMPLE"], "EXAMPLE"));
    assert!(!is_end_line(&["END", "DOC"], "EXAMPLE")); // keyword must match
    assert!(!is_end_line(&["END"], "DOC")); // one word
    assert!(!is_end_line(&["END", "DOC", "X"], "DOC")); // three words
    assert!(!is_end_line(&["DOC"], "DOC")); // not an END line
}

#[test]
fn is_doc_attr_rest_accepts_empty_and_attribute_words_only() {
    assert!(is_doc_attr_rest("")); // bare `DOC`
    assert!(is_doc_attr_rest("   ")); // whitespace only
    assert!(is_doc_attr_rest("INTERNAL")); // one attribute
    assert!(is_doc_attr_rest("INTERNAL DEPRECATED")); // several
    assert!(!is_doc_attr_rest("(Int)")); // punctuation → an ordinary DOC token
    assert!(!is_doc_attr_rest("foo123")); // digits are not attribute chars
}

#[test]
fn string_escapes_are_decoded_including_unknown_escapes() {
    // Source: "a\"b\\c\nd\te\zf" — decodes \" \\ \n \t and passes an unknown
    // escape (\z) through as the bare character.
    let tokens = lex(Path::new("main.mfb"), "\"a\\\"b\\\\c\\nd\\te\\zf\"\n").expect("lex source");
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
fn scalar_literals_decode_to_codepoints() {
    // Raw scalar, string-shared escapes, backtick and apostrophe escapes,
    // and an astral `\u{...}` (plan-41-A).
    for (source, expected) in [
        ("`A`\n", 'A' as u32),
        ("`\\n`\n", '\n' as u32),
        ("`\\t`\n", '\t' as u32),
        ("`\\r`\n", '\r' as u32),
        ("`\\0`\n", 0u32),
        ("`\\\\`\n", '\\' as u32),
        ("`\\``\n", '`' as u32),
        ("`'`\n", '\'' as u32),
        ("`\\u{1F600}`\n", 0x1F600),
        ("`中`\n", '中' as u32),
    ] {
        let tokens = lex(Path::new("main.mfb"), source).expect("lex source");
        assert_eq!(
            tokens[0].kind,
            TokenKind::Scalar(expected),
            "source {source:?}"
        );
    }
}

#[test]
fn apostrophe_comment_swallows_backticks_inertly() {
    // A `'` line comment containing backticks must not start a scalar literal;
    // the whole line is a comment and only the Newline/Eof remain.
    let tokens = lex(Path::new("main.mfb"), "' a `A` and `\\u{1F600}` here\n").expect("lex source");
    assert_eq!(
        tokens.iter().map(|token| &token.kind).collect::<Vec<_>>(),
        vec![&TokenKind::Newline, &TokenKind::Eof]
    );
}

#[test]
fn malformed_scalar_literals_are_errors() {
    // Empty, multi-scalar, unterminated, bad escape, surrogate, out-of-range.
    for source in [
        "``\n",
        "`ab`\n",
        "`A\n",
        "`\\q`\n",
        "`\\u{D800}`\n",
        "`\\u{110000}`\n",
        "`\n",
    ] {
        assert!(
            lex(Path::new("main.mfb"), source).is_err(),
            "expected lex error for {source:?}"
        );
    }
}

#[test]
fn radix_literals_decode_to_canonical_decimal() {
    for (source, expected) in [
        ("0xFFF", "4095"),
        ("0o777", "511"),
        ("0b1010", "10"),
        ("0XaB", "171"),
        ("0xFF_FF", "65535"),
        ("0b1111_1111", "255"),
    ] {
        let tokens = lex(Path::new("main.mfb"), &format!("{source}\n")).expect("lex source");
        assert_eq!(
            tokens[0].kind,
            TokenKind::Number(expected.to_string()),
            "for {source}"
        );
    }
}

#[test]
fn digit_separators_are_stripped() {
    for (source, expected) in [
        ("1_000", "1000"),
        ("1_2_3", "123"),
        ("1_000_000", "1000000"),
    ] {
        let tokens = lex(Path::new("main.mfb"), &format!("{source}\n")).expect("lex source");
        assert_eq!(tokens[0].kind, TokenKind::Number(expected.to_string()));
    }
}

#[test]
fn malformed_numbers_are_errors() {
    for source in [
        "0x\n",
        "0b2\n",
        "0o8\n",
        "0xG\n",
        "1__2\n",
        "0x_1\n",
        "0xFFFFFFFFFFFFFFFFF\n",
    ] {
        assert!(
            lex(Path::new("main.mfb"), source).is_err(),
            "expected lex error for {source:?}"
        );
    }
}

#[test]
fn exponent_literals_keep_their_text() {
    for (source, expected) in [
        ("1e3", "1e3"),
        ("1E3", "1E3"),
        ("1e+3", "1e+3"),
        ("1e-3", "1e-3"),
        ("2.5e2", "2.5e2"),
        ("1_0e1_0", "10e10"),
    ] {
        let tokens = lex(Path::new("main.mfb"), &format!("{source}\n")).expect("lex source");
        assert_eq!(
            tokens[0].kind,
            TokenKind::Number(expected.to_string()),
            "for {source}"
        );
    }
}

#[test]
fn malformed_exponent_lexes_as_number_then_identifier() {
    // `1e` is `1` then identifier `e`; `1e+` is `1` then `e` then `+`.
    let tokens = lex(Path::new("main.mfb"), "1e\n").expect("lex source");
    assert_eq!(tokens[0].kind, TokenKind::Number("1".to_string()));
    assert_eq!(tokens[1].kind, TokenKind::Identifier("e".to_string()));
}

#[test]
fn type_suffixes_are_kept_in_the_token() {
    for (source, expected) in [
        ("2f", "2f"),
        ("2F", "2F"),
        ("1.5f", "1.5f"),
        ("1e3F", "1e3F"),
        ("2.5e2f", "2.5e2f"),
        // `m`/`M` (Money, plan-29-A §4.4) are kept just like `f`/`F`.
        ("2m", "2m"),
        ("2M", "2M"),
        ("1.25m", "1.25m"),
        ("1.25M", "1.25M"),
    ] {
        let tokens = lex(Path::new("main.mfb"), &format!("{source}\n")).expect("lex source");
        assert_eq!(
            tokens[0].kind,
            TokenKind::Number(expected.to_string()),
            "for {source}"
        );
    }
    // `1foo` is `1` then identifier `foo`, not a suffixed literal.
    let tokens = lex(Path::new("main.mfb"), "1foo\n").expect("lex source");
    assert_eq!(tokens[0].kind, TokenKind::Number("1".to_string()));
    assert_eq!(tokens[1].kind, TokenKind::Identifier("foo".to_string()));
    // `1motorcar` is `1` then identifier — the `m` is only a suffix when no
    // identifier-continue char follows.
    let tokens = lex(Path::new("main.mfb"), "1motorcar\n").expect("lex source");
    assert_eq!(tokens[0].kind, TokenKind::Number("1".to_string()));
    assert_eq!(
        tokens[1].kind,
        TokenKind::Identifier("motorcar".to_string())
    );
}

#[test]
fn leading_underscore_is_an_identifier_not_a_number() {
    let tokens = lex(Path::new("main.mfb"), "_1\n").expect("lex source");
    assert_eq!(tokens[0].kind, TokenKind::Identifier("_1".to_string()));
}

#[test]
fn trailing_underscore_after_digit_still_continues_line() {
    // `1_` followed by a newline is the line-continuation token, not a
    // separator: no Newline is emitted between the `1` and the next line.
    let tokens = lex(Path::new("main.mfb"), "1_\n2\n").expect("lex source");
    assert_eq!(tokens[0].kind, TokenKind::Number("1".to_string()));
    assert_eq!(tokens[1].kind, TokenKind::Number("2".to_string()));
}

#[test]
fn unterminated_string_on_line_is_an_error() {
    // Newline before the closing quote.
    assert!(lex(Path::new("main.mfb"), "\"abc\ndef\n").is_err());
}

#[test]
fn backslash_before_newline_is_an_unterminated_string() {
    // bug-12: the catch-all escape arm used to embed the newline in the value
    // and consume it with `advance` (which never bumps `self.line`), so every
    // later token reported a line number one short. There is no in-string line
    // continuation: this is an unterminated string, like a bare newline.
    assert!(lex(Path::new("main.mfb"), "LET a = \"abc\\\ndef\"\nLET b = 1\n").is_err());
    // The newline is left unconsumed (so the outer scanner still counts it),
    // and no String token is produced.
    let mut lexer = Lexer {
        path: Path::new("main.mfb"),
        chars: "\"abc\\\ndef\"\n".chars().collect(),
        index: 0,
        line: 1,
        column: 1,
        tokens: Vec::new(),
        had_error: false,
        internal: false,
        open_brackets: Vec::new(),
    };
    lexer.lex_string();
    assert!(lexer.had_error);
    assert!(lexer.tokens.is_empty());
    assert_eq!(lexer.peek(), '\n');
    assert_eq!(lexer.line, 1);
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

fn kinds(source: &str) -> Vec<TokenKind> {
    lex(Path::new("main.mfb"), source)
        .expect("lex source")
        .into_iter()
        .map(|token| token.kind)
        .collect()
}

fn newline_count(source: &str) -> usize {
    kinds(source)
        .iter()
        .filter(|kind| **kind == TokenKind::Newline)
        .count()
}

fn lex_diagnostics(source: &str) -> (bool, Vec<(String, String)>) {
    let (result, collected) =
        crate::rules::collecting_diagnostics(|| lex(Path::new("main.mfb"), source).is_ok());
    (result, collected)
}

/// bug-821: a newline inside an open `[`, `{` or `(` is not a separator.
#[test]
fn newline_inside_open_brackets_is_not_a_separator() {
    // One separator each: the one after the closing bracket.
    assert_eq!(newline_count("LET x = [\n  1,\n  2\n]\n"), 1);
    assert_eq!(newline_count("LET m = Map OF K TO V {\n  1 := 2\n}\n"), 1);
    assert_eq!(newline_count("f(\n  a,\n  b)\n"), 1);
    assert_eq!(newline_count("LET y = (1 +\n  2)\n"), 1);
    // Nested: depth returns to zero only after the outer closer.
    assert_eq!(newline_count("LET n = [[1,\n2],\n[3]]\nLET z = 1\n"), 2);
    // A blank line and a comment-only line inside the bracket add nothing.
    assert_eq!(newline_count("LET x = [\n\n  1, ' one [\n  ' (\n  2]\n"), 1);
    // Outside brackets every newline still separates.
    assert_eq!(newline_count("LET a = 1\nLET b = 2\n"), 2);
    // `_` continuation keeps working inside and outside a bracket.
    assert_eq!(newline_count("LET s = \"a\" & _\n  \"b\"\n"), 1);
    assert_eq!(newline_count("LET x = [1, _\n  2]\n"), 1);
}

/// bug-821: brackets inside a string, a scalar or a comment do not count.
#[test]
fn brackets_in_strings_scalars_and_comments_do_not_change_depth() {
    assert_eq!(newline_count("LET s = \"[(\"\nLET t = 1\n"), 2);
    assert_eq!(newline_count("LET c = `[`\nLET t = 1\n"), 2);
    assert_eq!(newline_count("LET a = 1 ' f(\nLET t = 1\n"), 2);
    assert_eq!(newline_count("REM f(\nLET t = 1\n"), 2);
    assert_eq!(newline_count("LET x = [\"]\", \")\"\n]\n"), 1);
}

/// bug-821: a closer pops the innermost opener whatever its kind, and a
/// closer with nothing open is left to the parser, so the depth tracks the
/// source and neither case is a lexer error.
#[test]
fn mismatched_and_stray_closers_are_left_to_the_parser() {
    assert_eq!(newline_count("f(a]\nLET t = 1\n"), 2);
    assert_eq!(newline_count("]\n)\n}\nLET t = 1\n"), 4);
}

/// bug-821: a bracket still open at end of file is reported at its opener.
#[test]
fn unclosed_bracket_at_eof_is_reported() {
    let (ok, diagnostics) = lex_diagnostics("LET x = [1,\n  2,\n");
    assert!(!ok);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].0, "MFB_LEX_UNCLOSED_BRACKET");
    assert!(
        diagnostics[0].1.contains("`[` is never closed")
            && diagnostics[0].1.contains("end of the file"),
        "{diagnostics:?}"
    );
    // Every still-open bracket is reported, each at its own opener.
    let (ok, diagnostics) = lex_diagnostics("f([1,\n");
    assert!(!ok);
    let names: Vec<_> = diagnostics.iter().map(|d| d.1.clone()).collect();
    assert_eq!(diagnostics.len(), 2, "{names:?}");
    assert!(names[0].starts_with("`(`") && names[1].starts_with("`[`"));
}

/// bug-821: a line opening with a statement-only keyword ends an unclosed
/// bracket's run: it is reported, and lexing resumes with ordinary newline
/// separation instead of swallowing the rest of the file.
#[test]
fn statement_keyword_after_unclosed_bracket_is_reported() {
    let source = "FUNC main() AS Nothing\n  LET xs = [1, 2\n  IF TRUE THEN\n    LET y = f(1\n  end if\nEND FUNC\n";
    let (ok, diagnostics) = lex_diagnostics(source);
    assert!(!ok);
    assert_eq!(diagnostics.len(), 2, "{diagnostics:?}");
    assert!(diagnostics[0]
        .1
        .contains("`[` is never closed: `IF` on line 3"));
    assert!(diagnostics[1]
        .1
        .contains("`(` is never closed: `END` on line 5"));
    // Keywords that legitimately begin a line inside a bracket do not end it.
    for line in [
        "LAMBDA(n AS Integer) -> n",
        "NOT done",
        "WITH r { a := 1 }",
        "TRUE",
        "NOTHING",
        "AND b",
        "MUT a AS Integer",
        "RES f AS File",
        "FUNC(Integer) AS Integer",
        "ISOLATED FUNC() AS Integer",
        "AS Integer",
        "TO Integer",
        "IN CInt",
    ] {
        let source = format!("f(\n  {line}\n)\n");
        assert_eq!(newline_count(&source), 1, "{line}");
    }
}

/// An EXAMPLE with no `END EXAMPLE` swallows the `END DOC` after it (inside an
/// example, `END DOC` is example text). The block is still DOC_UNTERMINATED,
/// but the report names the open EXAMPLE instead of claiming there is no
/// `END DOC` at all; a block simply missing its `END DOC` keeps the plain text.
#[test]
fn an_unterminated_doc_names_the_example_that_swallowed_its_end() {
    let (ok, diagnostics) = lex_diagnostics(
        "DOC\nFUNC helper()\nEXAMPLE\n  a\nEND DOC\nFUNC helper() AS Integer\n  RETURN 7\nEND FUNC\n",
    );
    assert!(!ok);
    assert_eq!(
        diagnostics,
        [(
            "DOC_UNTERMINATED".to_string(),
            "DOC block reached end of file before its `END DOC` line: the EXAMPLE at line 3 \
             has no `END EXAMPLE`, so every line after it, an `END DOC` included, was read \
             as example text."
                .to_string()
        )]
    );
    let (ok, diagnostics) = lex_diagnostics("DOC\nFUNC helper()\nDESC never closed\n");
    assert!(!ok);
    assert_eq!(
        diagnostics,
        [(
            "DOC_UNTERMINATED".to_string(),
            "DOC block reached end of file before its `END DOC` line.".to_string()
        )]
    );
}
