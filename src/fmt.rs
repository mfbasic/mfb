//! Source formatter for `mfb fmt`.
//!
//! The formatter is deliberately *lexical*, not AST-based: a formatter must
//! preserve comments and blank lines, both of which the real lexer discards. It
//! therefore re-tokenizes each physical line with a small scanner that keeps the
//! original text verbatim, then rewrites only two things:
//!
//! - **Indentation** — leading whitespace is recomputed from block nesting
//!   (`FUNC`/`SUB`/`IF`/`FOR`/`MATCH`/…) using a configurable indent width.
//! - **Capitalization** — keywords are uppercased to match the MFBASIC
//!   convention (§2 of the language spec). Identifiers, package members
//!   (`pkg::name`), and field accesses (`value.field`) are left untouched, even
//!   when they happen to spell a keyword.
//!
//! Everything else — intra-line spacing, string contents, comments, and `DOC`
//! block bodies — is preserved byte-for-byte.

use crate::lexer::{self, Keyword};

/// Format an entire source file. Pure and deterministic: same input and indent
/// width always produce the same output.
pub fn format_source(source: &str, indent_width: usize) -> String {
    if source.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = source.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut stack: Vec<Block> = Vec::new();
    let mut i = 0;
    let n = lines.len();

    while i < n {
        let raw = strip_cr(lines[i]);
        let trimmed = raw.trim();

        // Blank line: emit empty, preserving vertical spacing.
        if trimmed.is_empty() {
            out.push(String::new());
            i += 1;
            continue;
        }

        // DOC block: the body is captured verbatim (it may contain prose or
        // example source that must not be re-cased or re-indented). Only the
        // `DOC` and `END DOC` lines are re-indented to the current block level.
        if is_doc_start(trimmed) {
            let indent = indent_str(stack.len(), indent_width);
            out.push(format!("{indent}{}", doc_header(trimmed)));
            i += 1;
            let mut in_example = false;
            while i < n {
                let body = strip_cr(lines[i]);
                let words: Vec<&str> = body.split_whitespace().collect();
                let is_end = |kw: &str| {
                    words.len() == 2
                        && words[0].eq_ignore_ascii_case("END")
                        && words[1].eq_ignore_ascii_case(kw)
                };
                if !in_example && is_end("DOC") {
                    out.push(format!("{indent}END DOC"));
                    i += 1;
                    break;
                }
                if !in_example && words.len() == 1 && words[0].eq_ignore_ascii_case("EXAMPLE") {
                    in_example = true;
                } else if in_example && is_end("EXAMPLE") {
                    in_example = false;
                }
                out.push(body.trim_end().to_string());
                i += 1;
            }
            continue;
        }

        // LINK block: the native-binding DSL (`LINK … END LINK`) has its own
        // grammar — `FUNC`/`FREE` nesting and contextual words like `SYMBOL`,
        // `ABI`, `return`, `OUT` — that the block tracker does not model. Copy it
        // verbatim so a binding package is never mis-indented or mis-cased.
        if is_link_start(trimmed) {
            out.push(strip_cr(lines[i]).trim_end().to_string());
            i += 1;
            while i < n {
                let body = strip_cr(lines[i]);
                let words: Vec<&str> = body.split_whitespace().collect();
                let is_end_link = words.len() == 2
                    && words[0].eq_ignore_ascii_case("END")
                    && words[1].eq_ignore_ascii_case("LINK");
                out.push(body.trim_end().to_string());
                i += 1;
                if is_end_link {
                    break;
                }
            }
            continue;
        }

        // Gather a logical line: a leading physical line plus any continuation
        // lines (each previous physical line ending in a trailing `_`).
        let mut cased_lines: Vec<String> = Vec::new();
        let mut sig: Vec<Sig> = Vec::new();
        loop {
            let phys = strip_cr(lines[i]);
            let scanned = scan_line(phys);
            let continues = matches!(scanned.sig.last(), Some(Sig::Underscore));
            cased_lines.push(scanned.cased);
            sig.extend(scanned.sig);
            if continues && i + 1 < n {
                i += 1;
                continue;
            }
            break;
        }
        i += 1;

        let base = stack.len();
        let (first_structural, ops) = structural_ops(&sig);
        let line_indent = apply_ops(&ops, &mut stack, first_structural, base);
        let indent = indent_str(line_indent, indent_width);

        for (j, cased) in cased_lines.iter().enumerate() {
            if j == 0 {
                let body = cased.trim();
                if body.is_empty() {
                    out.push(String::new());
                } else {
                    out.push(format!("{indent}{body}"));
                }
            } else {
                // Continuation lines keep their original leading alignment so
                // hand-aligned continuations are not disturbed.
                out.push(cased.trim_end().to_string());
            }
        }
    }

    let mut result = out.join("\n");
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn indent_str(level: usize, width: usize) -> String {
    " ".repeat(level * width)
}

// --- Per-line scanning -----------------------------------------------------

/// A significant (non-whitespace, non-comment) token, classified only as far as
/// the formatter needs for indentation and continuation decisions.
#[derive(Clone, Copy, PartialEq)]
enum Sig {
    Kw(Keyword),
    /// A lone `_`, which as the last token of a line is a line continuation.
    Underscore,
    /// `::`, used to recognize a `FUNC name AS pkg::func` re-export alias.
    DoubleColon,
    /// `(`, used to tell a parameterized `FUNC` from a no-body alias.
    LParen,
    Other,
}

struct Scanned {
    cased: String,
    sig: Vec<Sig>,
}

/// Re-emit one physical line with keywords uppercased, preserving everything
/// else verbatim, and collect its significant tokens.
fn scan_line(line: &str) -> Scanned {
    let chars: Vec<char> = line.chars().collect();
    let n = chars.len();
    let mut out = String::new();
    let mut sig = Vec::new();
    let mut i = 0;
    // `suppress` is set after `.` or `::`: the following word is a field or
    // package member, never a keyword, so it must not be re-cased.
    let mut suppress = false;
    // `stmt_start` marks the start of a statement (line start or after `:`),
    // where a leading `REM` introduces a comment.
    let mut stmt_start = true;
    // The previous significant word (keyword or identifier), lowercased. Used to
    // keep `Nothing` as a type name in a type position (`AS`/`OF`/`TO Nothing`),
    // distinct from the value `NOTHING` — `OF` is not a lexer keyword, so a plain
    // keyword is not enough. Reset to `None` by any non-word token.
    let mut prev_word: Option<String> = None;

    while i < n {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' => {
                // Whitespace does not separate `AS` from a following `Nothing`.
                out.push(c);
                i += 1;
                continue;
            }
            '\'' => {
                // Comment to end of line, verbatim.
                while i < n {
                    out.push(chars[i]);
                    i += 1;
                }
            }
            '"' => {
                out.push('"');
                i += 1;
                while i < n {
                    let d = chars[i];
                    out.push(d);
                    i += 1;
                    if d == '\\' {
                        if i < n {
                            out.push(chars[i]);
                            i += 1;
                        }
                    } else if d == '"' {
                        break;
                    }
                }
                sig.push(Sig::Other);
                suppress = false;
                stmt_start = false;
            }
            '0'..='9' => {
                while i < n && chars[i].is_ascii_digit() {
                    out.push(chars[i]);
                    i += 1;
                }
                if i + 1 < n && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
                    out.push('.');
                    i += 1;
                    while i < n && chars[i].is_ascii_digit() {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
                sig.push(Sig::Other);
                suppress = false;
                stmt_start = false;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < n && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let word: String = chars[start..i].iter().collect();

                if stmt_start && word.eq_ignore_ascii_case("REM") {
                    // `REM` at statement start is a comment to end of line.
                    out.push_str(&word);
                    while i < n {
                        out.push(chars[i]);
                        i += 1;
                    }
                    continue;
                }

                let in_type_position =
                    matches!(prev_word.as_deref(), Some("as") | Some("of") | Some("to"));
                if !suppress {
                    if let Some(kw) = lexer::lookup_keyword(&word) {
                        // `Nothing` in a type position is the unit *type*, written
                        // in CapitalCamelCase; elsewhere it is the value `NOTHING`.
                        if kw == Keyword::Nothing && in_type_position {
                            out.push_str("Nothing");
                        } else {
                            out.push_str(&word.to_ascii_uppercase());
                        }
                        sig.push(Sig::Kw(kw));
                        prev_word = Some(word.to_ascii_lowercase());
                        suppress = false;
                        stmt_start = false;
                        continue;
                    }
                }

                out.push_str(&word);
                sig.push(if word == "_" {
                    Sig::Underscore
                } else {
                    Sig::Other
                });
                prev_word = Some(word.to_ascii_lowercase());
                suppress = false;
                stmt_start = false;
                continue;
            }
            ':' => {
                if chars.get(i + 1) == Some(&':') {
                    out.push_str("::");
                    i += 2;
                    sig.push(Sig::DoubleColon);
                    suppress = true;
                    stmt_start = false;
                } else if chars.get(i + 1) == Some(&'=') {
                    out.push_str(":=");
                    i += 2;
                    sig.push(Sig::Other);
                    suppress = false;
                    stmt_start = false;
                } else {
                    out.push(':');
                    i += 1;
                    sig.push(Sig::Other);
                    suppress = false;
                    stmt_start = true;
                }
            }
            '.' => {
                out.push('.');
                i += 1;
                sig.push(Sig::Other);
                suppress = true;
                stmt_start = false;
            }
            _ => {
                out.push(c);
                i += 1;
                sig.push(if c == '(' { Sig::LParen } else { Sig::Other });
                suppress = false;
                stmt_start = false;
            }
        }
        // Any non-word token (string, number, punctuation) clears the type
        // position; word arms and the whitespace arm `continue` past this.
        prev_word = None;
    }

    Scanned { cased: out, sig }
}

// --- Block structure -------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Block {
    Func,
    Sub,
    Type,
    Union,
    Enum,
    If,
    For,
    While,
    Do,
    Match,
    Trap,
    Case,
}

#[derive(Clone, Copy)]
enum Op {
    /// Opens a block; increases indentation for following lines.
    Open(Block),
    /// `ELSE`/`ELSEIF`: prints dedented one level, leaving the `IF` frame open.
    Else,
    /// `CASE`: closes any previous case arm and opens a new one.
    Case,
    /// `END X`: closes a block. The keyword identifies it (for `END MATCH`).
    End(Option<Keyword>),
    /// `NEXT`/`WEND`/`LOOP`: closes the top loop block.
    Pop,
}

/// Extract the block-structure operations for one logical line, and whether the
/// line's first significant token is itself a structural one (which determines
/// whether the line is printed dedented).
fn structural_ops(sig: &[Sig]) -> (bool, Vec<Op>) {
    let first_kw = sig.iter().find_map(|s| match s {
        Sig::Kw(k) => Some(*k),
        _ => None,
    });
    // A multi-line `IF` ends the line with `THEN`; a single-line
    // `IF cond THEN stmt [ELSE stmt]` has tokens after `THEN` and opens nothing.
    let multiline_if =
        first_kw == Some(Keyword::If) && matches!(sig.last(), Some(Sig::Kw(Keyword::Then)));

    // A `[vis] FUNC|SUB name AS pkg::func` re-export alias has a `::` target and
    // no parameter list; it declares no body and opens no block. (The grammar
    // uses exactly this shape to disambiguate, so a real signature returning a
    // `::`-qualified type cannot occur.)
    let has_dcolon = sig.iter().any(|s| matches!(s, Sig::DoubleColon));
    let has_lparen = sig.iter().any(|s| matches!(s, Sig::LParen));
    let func_alias = has_dcolon && !has_lparen;

    let mut ops = Vec::new();
    let mut first_structural = false;
    let mut prev_kw: Option<Keyword> = None;

    for (idx, s) in sig.iter().enumerate() {
        match s {
            Sig::Kw(k) => {
                if let Some(op) =
                    classify(*k, prev_kw, idx == 0, multiline_if, func_alias, sig, idx)
                {
                    if idx == 0 {
                        first_structural = true;
                    }
                    ops.push(op);
                }
                prev_kw = Some(*k);
            }
            _ => prev_kw = None,
        }
    }

    (first_structural, ops)
}

fn classify(
    k: Keyword,
    prev_kw: Option<Keyword>,
    is_first: bool,
    multiline_if: bool,
    func_alias: bool,
    sig: &[Sig],
    idx: usize,
) -> Option<Op> {
    use Keyword as K;
    // A block keyword following `END` (e.g. the `FUNC` in `END FUNC`) names the
    // block being closed, not a new opener — the `END` already produced the
    // close op.
    if prev_kw == Some(K::End) {
        return None;
    }
    // A loop/routine keyword following EXIT/CONTINUE (e.g. `EXIT FOR`) names a
    // target, not a block opener.
    let after_exit = matches!(prev_kw, Some(K::Exit) | Some(K::Continue));
    // `WHILE` following `DO`/`LOOP` (`DO WHILE c`, `LOOP WHILE c`) is a loop
    // condition, not a `WHILE … WEND` block opener.
    let while_is_condition = matches!(prev_kw, Some(K::Do) | Some(K::Loop));
    match k {
        K::If => (is_first && multiline_if).then_some(Op::Open(Block::If)),
        K::ElseIf | K::Else => is_first.then_some(Op::Else),
        K::Case => Some(Op::Case),
        K::End => Some(Op::End(next_keyword(sig, idx))),
        K::Next | K::Wend | K::Loop => Some(Op::Pop),
        // FUNC/SUB may be preceded by visibility/ISOLATED modifiers, so they are
        // not required to lead the line; EXIT FUNC/SUB is excluded by `after_exit`.
        // A function *type* `FUNC(…) AS T` / `SUB(…)` (the keyword immediately
        // followed by `(`) is an annotation, not a declaration, and opens nothing.
        K::Func => {
            (!after_exit && !func_alias && !func_type(sig, idx)).then_some(Op::Open(Block::Func))
        }
        K::Sub => {
            (!after_exit && !func_alias && !func_type(sig, idx)).then_some(Op::Open(Block::Sub))
        }
        // FOR/WHILE/DO open a loop block. They do *not* when the keyword is an
        // exit target (`EXIT FOR`) or, for WHILE, a loop condition (`DO WHILE c`).
        // A single-line loop (`FOR i … : … : NEXT`) opens and closes on one line,
        // netting to zero, so mid-line position is fine.
        K::For => (!after_exit).then_some(Op::Open(Block::For)),
        K::While => (!after_exit && !while_is_condition).then_some(Op::Open(Block::While)),
        K::Do => (!after_exit).then_some(Op::Open(Block::Do)),
        K::Type => Some(Op::Open(Block::Type)),
        K::Union => Some(Op::Open(Block::Union)),
        K::Enum => Some(Op::Open(Block::Enum)),
        K::Match => Some(Op::Open(Block::Match)),
        K::Trap => Some(Op::Open(Block::Trap)),
        _ => None,
    }
}

/// Whether the keyword at `idx` is a function *type* (`FUNC(`/`SUB(`), i.e. the
/// next token is `(` — an annotation, not a declaration.
fn func_type(sig: &[Sig], idx: usize) -> bool {
    matches!(sig.get(idx + 1), Some(Sig::LParen))
}

fn next_keyword(sig: &[Sig], idx: usize) -> Option<Keyword> {
    sig[idx + 1..].iter().find_map(|s| match s {
        Sig::Kw(k) => Some(*k),
        _ => None,
    })
}

/// Apply a logical line's operations to the block stack and return the
/// indentation level at which the line should be printed.
fn apply_ops(ops: &[Op], stack: &mut Vec<Block>, first_structural: bool, base: usize) -> usize {
    let mut line_indent = base;
    for (i, op) in ops.iter().enumerate() {
        let ind = match op {
            Op::Open(block) => {
                let ind = stack.len();
                stack.push(*block);
                ind
            }
            Op::Else => stack.len().saturating_sub(1),
            Op::Case => {
                if stack.last() == Some(&Block::Case) {
                    stack.pop();
                }
                let ind = stack.len();
                stack.push(Block::Case);
                ind
            }
            Op::End(kw) => {
                if *kw == Some(Keyword::Match) && stack.last() == Some(&Block::Case) {
                    stack.pop();
                }
                stack.pop();
                stack.len()
            }
            Op::Pop => {
                stack.pop();
                stack.len()
            }
        };
        if i == 0 && first_structural {
            line_indent = ind;
        }
    }
    line_indent
}

// --- DOC blocks ------------------------------------------------------------

/// Whether a trimmed line begins a `DOC ... END DOC` block (mirrors the lexer:
/// `DOC` followed only by attribute words such as `INTERNAL`).
fn is_doc_start(trimmed: &str) -> bool {
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("");
    if !first.eq_ignore_ascii_case("DOC") {
        return false;
    }
    let rest = parts.next().unwrap_or("").trim();
    rest.is_empty()
        || rest
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == ' ' || c == '\t')
}

/// Whether a trimmed line begins a native `LINK "lib" AS alias` block. The
/// trailing string literal distinguishes it from any ordinary use of the word.
fn is_link_start(trimmed: &str) -> bool {
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("");
    if !first.eq_ignore_ascii_case("LINK") {
        return false;
    }
    parts.next().unwrap_or("").trim_start().starts_with('"')
}

/// The re-cased `DOC` header line, uppercasing only the `DOC` keyword and
/// leaving any attribute words as written.
fn doc_header(trimmed: &str) -> String {
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let _ = parts.next();
    let rest = parts.next().unwrap_or("").trim();
    if rest.is_empty() {
        "DOC".to_string()
    } else {
        format!("DOC {rest}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(source: &str) -> String {
        format_source(source, 2)
    }

    #[test]
    fn reindents_nested_blocks_and_uppercases_keywords() {
        let input = "func main as Integer\nlet x = 1\nif x > 0 then\nio::print(\"pos\")\nend if\nreturn 0\nend func\n";
        let expected = "FUNC main AS Integer\n  LET x = 1\n  IF x > 0 THEN\n    io::print(\"pos\")\n  END IF\n  RETURN 0\nEND FUNC\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn match_case_indentation_matches_spec() {
        let input = "FUNC area(s AS Shape) AS Float\nMATCH s\nCASE Circle(c) : RETURN c.radius\nCASE Rect(r) : RETURN r.w\nEND MATCH\nEND FUNC\n";
        let expected = "FUNC area(s AS Shape) AS Float\n  MATCH s\n    CASE Circle(c) : RETURN c.radius\n    CASE Rect(r) : RETURN r.w\n  END MATCH\nEND FUNC\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn elseif_chain_dedents() {
        let input = "IF a THEN\nx\nELSEIF b THEN\ny\nELSE\nz\nEND IF\n";
        let expected = "IF a THEN\n  x\nELSEIF b THEN\n  y\nELSE\n  z\nEND IF\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn field_and_package_names_keep_case() {
        // `next`, `to`, `step` are keywords but here name a field, a package
        // member, and (after `::`) a function; none should be uppercased.
        let input = "LET a = node.next\nLET b = pkg::step\n";
        assert_eq!(fmt(input), input);
    }

    #[test]
    fn single_line_if_does_not_indent_following_line() {
        let input = "IF x > 0 THEN io::print(\"p\") ELSE io::print(\"n\")\nlet y = 1\n";
        let expected = "IF x > 0 THEN io::print(\"p\") ELSE io::print(\"n\")\nLET y = 1\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn one_line_block_with_separators_is_net_zero() {
        let input = "FOR i = 1 TO 3 : io::print(toString(i)) : NEXT\nlet done = 1\n";
        let expected = "FOR i = 1 TO 3 : io::print(toString(i)) : NEXT\nLET done = 1\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn exit_and_continue_targets_do_not_open_blocks() {
        let input = "DO\nEXIT DO\nCONTINUE DO\nLOOP UNTIL done\nlet after = 1\n";
        let expected = "DO\n  EXIT DO\n  CONTINUE DO\nLOOP UNTIL done\nLET after = 1\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn doc_block_body_is_verbatim_but_header_reindents() {
        let input = "TYPE T\ndoc\n  free  form   text\n  END DOC\nx AS Integer\nEND TYPE\n";
        let expected = "TYPE T\n  DOC\n  free  form   text\n  END DOC\n  x AS Integer\nEND TYPE\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn comments_blank_lines_and_strings_are_preserved() {
        let input = "' a comment\n\nLET s = \"IF nested THEN keyword\"   ' trailing\n";
        assert_eq!(fmt(input), input);
    }

    #[test]
    fn line_continuation_keeps_following_lines() {
        let input = "let msg = \"hello \" & _\n          \"world\"\n";
        let expected = "LET msg = \"hello \" & _\n          \"world\"\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn inline_trap_block_indents_handler() {
        let input = "FUNC f() AS Integer\nRES x = open(p) TRAP(e)\nio::print(e.message)\nRECOVER 0\nEND TRAP\nRETURN 0\nEND FUNC\n";
        let expected = "FUNC f() AS Integer\n  RES x = open(p) TRAP(e)\n    io::print(e.message)\n    RECOVER 0\n  END TRAP\n  RETURN 0\nEND FUNC\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn do_while_opens_one_block_not_two() {
        // The `WHILE` in `DO WHILE` is a condition, not a second block opener.
        let input = "DO WHILE running\nwork()\nLOOP\nlet after = 1\n";
        let expected = "DO WHILE running\n  work()\nLOOP\nLET after = 1\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn exported_declarations_open_a_block() {
        let input = "EXPORT FUNC f() AS Integer\nRETURN 0\nEND FUNC\n";
        let expected = "EXPORT FUNC f() AS Integer\n  RETURN 0\nEND FUNC\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn custom_indent_width() {
        let input = "FUNC f()\nx\nEND FUNC\n";
        let expected = "FUNC f()\n    x\nEND FUNC\n";
        assert_eq!(format_source(input, 4), expected);
    }

    #[test]
    fn nothing_is_a_type_after_as_but_a_value_elsewhere() {
        let input = "type t\nvalue AS Nothing\nend type\nLET x = nothing\n";
        let expected = "TYPE t\n  value AS Nothing\nEND TYPE\nLET x = NOTHING\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn function_type_annotation_does_not_open_a_block() {
        // The `FUNC(Integer) AS Integer` return type must not open a second block.
        let input = "FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer\nRETURN base\nEND FUNC\nlet x = 1\n";
        let expected = "FUNC makeAdder(base AS Integer) AS FUNC(Integer) AS Integer\n  RETURN base\nEND FUNC\nLET x = 1\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn nothing_is_a_type_after_of_and_to() {
        let input = "let t AS Thread OF Nothing TO Integer = start()\n";
        assert_eq!(
            fmt(input),
            "LET t AS Thread OF Nothing TO Integer = start()\n"
        );
    }

    #[test]
    fn func_alias_does_not_open_a_block() {
        let input = "EXPORT FUNC close AS pkg::close\nlet after = 1\n";
        let expected = "EXPORT FUNC close AS pkg::close\nLET after = 1\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn single_line_while_inside_if_is_balanced() {
        let input = "FUNC f() AS Integer\nIF TRUE THEN RETURN 3 ELSE WHILE FALSE : WEND\nRETURN 0\nEND FUNC\n";
        let expected = "FUNC f() AS Integer\n  IF TRUE THEN RETURN 3 ELSE WHILE FALSE : WEND\n  RETURN 0\nEND FUNC\n";
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn link_block_is_copied_verbatim() {
        let input = concat!(
            "LINK \"sqlite3\" AS link\n",
            "  FUNC open(path AS String) AS RES Db\n",
            "    SYMBOL \"sqlite3_open\"\n",
            "    ABI (path CString, return OUT CPtr) AS status CInt32\n",
            "  END FUNC\n",
            "END LINK\n",
            "let after = 1\n",
        );
        let expected = concat!(
            "LINK \"sqlite3\" AS link\n",
            "  FUNC open(path AS String) AS RES Db\n",
            "    SYMBOL \"sqlite3_open\"\n",
            "    ABI (path CString, return OUT CPtr) AS status CInt32\n",
            "  END FUNC\n",
            "END LINK\n",
            "LET after = 1\n",
        );
        assert_eq!(fmt(input), expected);
    }

    #[test]
    fn empty_source_stays_empty() {
        assert_eq!(format_source("", 2), "");
    }
}
