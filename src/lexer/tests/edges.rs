//! Lexing at the edges: recovery from a bad escape at the end of a line, a
//! scalar literal cut by a newline, digit separators in fractions and
//! exponents, an exponent letter at the end of the file, line continuations
//! with trailing blanks, and `DOC` lines that are no doc block or run off the
//! end of the file.

use super::*;

/// The tokens `src` lexes to (when it lexes) and every diagnostic detail.
fn lexed(src: &str) -> (Vec<TokenKind>, Vec<String>) {
    let (tokens, diagnostics) =
        crate::rules::collecting_diagnostics(|| lex(Path::new("main.mfb"), src));
    (
        tokens
            .map(|tokens| tokens.into_iter().map(|token| token.kind).collect())
            .unwrap_or_default(),
        diagnostics.into_iter().map(|(_, detail)| detail).collect(),
    )
}

fn has_identifier(tokens: &[TokenKind], name: &str) -> bool {
    tokens
        .iter()
        .any(|kind| matches!(kind, TokenKind::Identifier(id) if id == name))
}

const BAD_SEPARATOR: &str = "A `_` digit separator must sit between two digits.";

#[test]
fn a_bad_escape_abandons_only_the_rest_of_its_line() {
    // One diagnostic each: the rest of the literal up to the newline is
    // skipped, not re-lexed into a second error, and the next line lexes.
    for src in [
        "LET s = \"\\u{zz} abc\nLET t = 1\n",
        "LET c = `\\u{zz}\nLET d = 1\n",
    ] {
        let (_, details) = lexed(src);
        assert_eq!(details.len(), 1, "{src:?}: {details:?}");
    }
}

#[test]
fn a_scalar_escape_cut_by_the_end_of_the_line_is_refused() {
    let (_, details) = lexed("LET c = `\\\nLET d = 1\n");
    assert_eq!(
        details[0],
        "A scalar literal reached the end of the line before a closing backtick."
    );
}

#[test]
fn separators_in_a_fraction_or_exponent_sit_between_digits() {
    for number in ["1.5__0", "1e5__0", "1_"] {
        let (_, details) = lexed(&format!("LET v = {number}"));
        assert_eq!(details, [BAD_SEPARATOR], "{number}");
    }
}

#[test]
fn an_exponent_letter_at_the_end_of_the_file_is_an_identifier() {
    let (tokens, details) = lexed("LET v = 1e");
    assert!(details.is_empty(), "{details:?}");
    assert!(tokens.contains(&TokenKind::Number("1".to_string())));
    assert!(has_identifier(&tokens, "e"));
}

#[test]
fn a_continuation_may_be_followed_by_blanks() {
    let (tokens, details) = lexed("LET v = 1_ \t\n  + 2\n");
    assert!(details.is_empty(), "{details:?}");
    assert!(tokens.contains(&TokenKind::Number("2".to_string())));
    // An underscore followed only by blanks to the end of the file continues
    // nothing: it is the `_` placeholder, the last token before the end.
    let (tokens, details) = lexed("LET v = a _  ");
    assert!(details.is_empty(), "{details:?}");
    let continued = lexed("LET v = a _  \n  b\n").0;
    assert!(tokens.len() < continued.len(), "{tokens:?}");
}

#[test]
fn a_doc_line_that_is_no_doc_block_or_never_ends() {
    // `DOC = 1` is an assignment to an identifier named DOC.
    let (tokens, details) = lexed("DOC = 1\n");
    assert!(details.is_empty(), "{details:?}");
    assert!(has_identifier(&tokens, "DOC"));
    // A DOC keyword line, or a block whose last line has no newline, at the
    // end of the file.
    for src in ["DOC", "DOC\nFUNC f"] {
        let (_, details) = lexed(src);
        assert_eq!(
            details,
            ["DOC block reached end of file before its `END DOC` line."],
            "{src:?}"
        );
    }
}
