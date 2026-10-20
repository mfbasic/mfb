//! plan-176-M: the keyword table — spelling to `Keyword` and back, plus the line-shape
//! predicates built on it. The token loop lives in `lexer/mod.rs`.

use super::*;

/// Look up a keyword by its lexeme, case-insensitively. Exposed for source tools
/// (such as `mfb fmt`) that re-tokenize raw text without building a full lexer.
pub fn lookup_keyword(value: &str) -> Option<Keyword> {
    keyword(value)
}

/// Whether `words` (a line split on whitespace) is exactly `END <keyword>`,
/// case-insensitively — the DOC/EXAMPLE block terminator both the lexer's DOC
/// capture and `mfb fmt` recognize. Exposed for source tools (such as `mfb fmt`)
/// that re-tokenize raw text without building a full lexer.
pub fn is_end_line(words: &[&str], keyword: &str) -> bool {
    words.len() == 2
        && words[0].eq_ignore_ascii_case("END")
        && words[1].eq_ignore_ascii_case(keyword)
}

/// Whether `rest` — the text following the `DOC` keyword on its line — contains
/// only attribute words (e.g. `INTERNAL`), the check that distinguishes a
/// `DOC … END DOC` block header from an ordinary `DOC` token. Exposed for source
/// tools (such as `mfb fmt`) that re-tokenize raw text without building a full
/// lexer.
pub fn is_doc_attr_rest(rest: &str) -> bool {
    let rest = rest.trim();
    rest.is_empty()
        || rest
            .chars()
            .all(|ch| ch.is_ascii_alphabetic() || ch == ' ' || ch == '\t')
}

pub(super) fn keyword(value: &str) -> Option<Keyword> {
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
        Keyword::With => "with",
    }
}

pub(super) fn is_identifier_continue(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}
