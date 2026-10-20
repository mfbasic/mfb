//! Source formatter for `mfb fmt`.
//!
//! The formatter is deliberately *lexical*, not AST-based: a formatter must
//! preserve comments and blank lines, both of which the real lexer discards. It
//! therefore re-tokenizes each physical line with a small scanner that keeps the
//! original text verbatim, then rewrites only two things:
//!
//! - **Indentation** — leading whitespace is recomputed from block nesting
//!   (`FUNC`/`IF`/`FOR`/`MATCH`/…) using a configurable indent width.
//! - **Capitalization** — keywords are uppercased to match the MFBASIC
//!   convention (§2 of the language spec). Identifiers, package members
//!   (`pkg::name`), and field accesses (`value.field`) are left untouched, even
//!   when they happen to spell a keyword.
//!
//! Everything else — intra-line spacing, string contents, comments, and `DOC`
//! block bodies — is preserved byte-for-byte.

use std::fmt;

use crate::lexer::{self, Keyword};

/// Maximum block-nesting depth the formatter will re-indent. Every line costs
/// `depth × indent_width` bytes of indentation, so with no ceiling a deeply
/// nested (or deliberately hostile) source inflated quadratically on the way
/// out — 336 KB in became 512 MB out, 1.3 MB became 8.2 GB — and the result was
/// then written back over the user's file (audit-3 FE-02 / bug-502). The parser
/// rejects any program nested past `MAX_STMT_DEPTH` (256) statement blocks; the
/// formatter counts frames the parser does not (`MATCH` + `CASE`,
/// `TESTING`/`TGROUP`/`TCASE`, the LINK DSL's own `FUNC`/`CSTRUCT`/`BIND`
/// nesting), so the cap sits at four times that. Every program the parser admits
/// still formats, and a line can never carry more than
/// `MAX_NESTING_DEPTH × MAX_INDENT` bytes of indentation.
pub const MAX_NESTING_DEPTH: usize = 1024;

/// Why `format_source` refused an input. The formatter is lexical and otherwise
/// never rejects source (a malformed line is re-emitted as written), so the one
/// refusal is the nesting cap; `line` is the 1-based physical line whose block
/// opener crossed it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatError {
    NestingTooDeep { line: usize },
}

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FormatError::NestingTooDeep { line } => write!(
                f,
                "line {line} nests more than {MAX_NESTING_DEPTH} blocks deep"
            ),
        }
    }
}

/// Format an entire source file. Pure and deterministic: same input and indent
/// width always produce the same output. The only refusal is
/// [`FormatError::NestingTooDeep`], past [`MAX_NESTING_DEPTH`] open blocks.
pub fn format_source(source: &str, indent_width: usize) -> Result<String, FormatError> {
    if source.is_empty() {
        return Ok(String::new());
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

        // DOC block: a `DOC … END DOC` block is re-indented to the surrounding
        // block level, with the body one level deeper and `EXAMPLE` source one
        // level deeper still. Body text and casing are preserved verbatim — the
        // body is free-form prose (where a word like `if` is not a keyword) and
        // example source whose intent must not be altered.
        if is_doc_start(trimmed) {
            format_doc_block(&lines, &mut i, n, stack.len(), indent_width, &mut out);
            continue;
        }

        // LINK block: the native-binding DSL (`LINK … END LINK`) has its own
        // grammar with `FUNC`/`FREE` nesting and contextual words (`SYMBOL`,
        // `ABI`, `return`, `OUT`) that the keyword tracker does not model. Re-
        // indent from that nesting, preserving text and casing so a contextual
        // word such as `return` in an `ABI` line is never recased.
        if is_link_start(trimmed) {
            format_link_block(&lines, &mut i, n, stack.len(), indent_width, &mut out)?;
            continue;
        }

        // Gather a logical line: a leading physical line plus any continuation
        // lines — each previous physical line ending in a trailing `_`, or
        // ending inside an open `(`, `[` or `{` (bug-821, the lexer's rule).
        let first_physical_line = i + 1;
        let mut cased_lines: Vec<String> = Vec::new();
        // Per gathered line, where its code ends (see `Scanned::code_end`).
        let mut code_ends: Vec<Option<(usize, bool)>> = Vec::new();
        let mut sig: Vec<Sig> = Vec::new();
        let mut depth = 0usize;
        loop {
            let phys = strip_cr(lines[i]);
            let mut scanned = scan_line(phys, !cased_lines.is_empty());
            if depth > 0 {
                normalize_leading_comma(&mut scanned, &mut cased_lines, &mut code_ends);
            }
            for &open in &scanned.brackets {
                depth = if open {
                    depth + 1
                } else {
                    depth.saturating_sub(1)
                };
            }
            let continues = matches!(scanned.sig.last(), Some(Sig::Underscore))
                || (depth > 0 && i + 1 < n && !begins_statement(lines[i + 1]));
            // A line left holding nothing but the comma it gave up is dropped.
            if !(scanned.leading_comma.is_some() && scanned.cased.trim().is_empty()) {
                cased_lines.push(scanned.cased);
                code_ends.push(scanned.code_end);
            }
            sig.extend(scanned.sig);
            if continues && i + 1 < n {
                i += 1;
                continue;
            }
            break;
        }
        i += 1;

        let level = stack.len();
        let (mut first_structural, mut ops) =
            structural_ops(&sig, is_func_alias(&cased_lines.join(" ")));
        // bug-348: a line-leading `TGROUP`/`TCASE` opens a block the keyword
        // stream cannot see, since neither word is a keyword.
        if ops.is_empty() {
            if let Some(block) = contextual_block_opener(trimmed) {
                ops.push(Op::Open(block));
                first_structural = true;
            }
        }
        let line_indent = apply_ops(&ops, &mut stack, first_structural, level);
        if stack.len() > MAX_NESTING_DEPTH {
            return Err(FormatError::NestingTooDeep {
                line: first_physical_line,
            });
        }
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
    Ok(result)
}

/// bug-821: rewrite a leading-comma line inside an open bracket (`, "B"`) to
/// the trailing-comma form: the comma moves to just after the code on the
/// nearest earlier line of the same logical line (before any `'` comment or
/// `_`), and becomes a space where it stood so the element keeps its column.
/// Nothing moves when there is no earlier code, or that code already ends in an
/// opening bracket or a comma (`[⏎, a` and `a,⏎, b` are malformed; the parser
/// reports them, and the formatter re-emits malformed source as written).
fn normalize_leading_comma(
    scanned: &mut Scanned,
    cased_lines: &mut [String],
    code_ends: &mut [Option<(usize, bool)>],
) {
    let Some(at) = scanned.leading_comma else {
        return;
    };
    let Some(prev) = code_ends.iter().rposition(Option::is_some) else {
        scanned.leading_comma = None;
        return;
    };
    let Some((end, false)) = code_ends[prev] else {
        scanned.leading_comma = None;
        return;
    };
    cased_lines[prev].insert(end, ',');
    code_ends[prev] = Some((end + 1, true));
    scanned.cased.replace_range(at..at + 1, " ");
    // The comma was this line's only code: nothing is left to end it.
    if scanned.code_end == Some((at + 1, true)) {
        scanned.code_end = None;
    }
}

/// Whether a physical line opens with a keyword that can only begin a
/// statement (`lexer::begins_statement_only`). Inside an open bracket such a
/// line proves the bracket was left unclosed, so — as the lexer does — the
/// formatter stops continuing the logical line there rather than swallowing
/// the rest of the file into it.
fn begins_statement(line: &str) -> bool {
    let trimmed = line.trim_start();
    let end = trimmed
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(trimmed.len());
    lexer::lookup_keyword(&trimmed[..end]).is_some_and(lexer::begins_statement_only)
}

fn strip_cr(line: &str) -> &str {
    line.strip_suffix('\r').unwrap_or(line)
}

fn indent_str(level: usize, indent_width: usize) -> String {
    " ".repeat(level * indent_width)
}

// --- Per-line scanning -----------------------------------------------------

/// A significant (non-whitespace, non-comment) token, classified only as far as
/// the formatter needs for indentation and continuation decisions.
#[derive(Clone, Copy)]
enum Sig {
    Kw(Keyword),
    /// A lone `_`, which as the last token of a line is a line continuation.
    Underscore,
    /// `(`, used to tell a function type `FUNC(…)` from a declaration.
    LParen,
    Other,
}

struct Scanned {
    cased: String,
    sig: Vec<Sig>,
    /// bug-821: the line's brackets in source order, `true` for an opener
    /// (`(`, `[`, `{`) and `false` for a closer. Brackets inside strings,
    /// scalars and comments are not code and do not appear.
    brackets: Vec<bool>,
    /// Byte offset in `cased` of a `,` that is the line's first significant
    /// token (a leading-comma layout inside an open bracket).
    leading_comma: Option<usize>,
    /// Byte offset in `cased` just past the line's last significant token,
    /// ignoring a trailing `_` continuation marker, and whether that token was
    /// an opening bracket or a comma (after which no comma may be added).
    code_end: Option<(usize, bool)>,
}

/// Re-emit one physical line with keywords uppercased, preserving everything
/// else verbatim, and collect its significant tokens. `continuation` marks a
/// line that continues the previous one (after a trailing `_`, or inside an
/// open bracket): it does not start a statement, so a leading `REM` there is an
/// identifier, exactly as the lexer reads it.
fn scan_line(line: &str, continuation: bool) -> Scanned {
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
    let mut stmt_start = !continuation;
    let mut brackets = Vec::new();
    let mut leading_comma = None;
    let mut code_end: Option<(usize, bool)> = None;
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
                code_end = Some((out.len(), false));
                suppress = false;
                stmt_start = false;
            }
            // A backtick Scalar literal is opaque, exactly like a string (bug-293).
            // Without this arm it fell to the catch-all, so the scanner read the
            // scalar's *contents* as code: `` `"` `` opened string mode, which then
            // closed at the next real string's opening quote and left that string's
            // body scanned as code — any word spelling a keyword got uppercased,
            // silently rewriting a literal the fmt spec guarantees byte-for-byte.
            // `` `'` `` desynchronized the same way into comment mode.
            //
            // Mirrors the string arm including backslash escapes, since `` `\`` ``
            // is a legal scalar and its escaped backtick must not close the token.
            '`' => {
                out.push('`');
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
                    } else if d == '`' {
                        break;
                    }
                }
                sig.push(Sig::Other);
                code_end = Some((out.len(), false));
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
                code_end = Some((out.len(), false));
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
                        code_end = Some((out.len(), false));
                        prev_word = Some(word.to_ascii_lowercase());
                        suppress = false;
                        stmt_start = false;
                        continue;
                    }
                }

                out.push_str(&word);
                if word == "_" {
                    sig.push(Sig::Underscore);
                } else {
                    sig.push(Sig::Other);
                    code_end = Some((out.len(), false));
                }
                prev_word = Some(word.to_ascii_lowercase());
                suppress = false;
                stmt_start = false;
                continue;
            }
            ':' => {
                if chars.get(i + 1) == Some(&':') {
                    out.push_str("::");
                    i += 2;
                    sig.push(Sig::Other);
                    code_end = Some((out.len(), false));
                    suppress = true;
                    stmt_start = false;
                } else if chars.get(i + 1) == Some(&'=') {
                    out.push_str(":=");
                    i += 2;
                    sig.push(Sig::Other);
                    code_end = Some((out.len(), false));
                    suppress = false;
                    stmt_start = false;
                } else {
                    out.push(':');
                    i += 1;
                    sig.push(Sig::Other);
                    code_end = Some((out.len(), false));
                    suppress = false;
                    stmt_start = true;
                }
            }
            '.' => {
                out.push('.');
                i += 1;
                sig.push(Sig::Other);
                code_end = Some((out.len(), false));
                suppress = true;
                stmt_start = false;
            }
            _ => {
                if c == ',' && sig.is_empty() {
                    leading_comma = Some(out.len());
                }
                match c {
                    '(' | '[' | '{' => brackets.push(true),
                    ')' | ']' | '}' => brackets.push(false),
                    _ => {}
                }
                out.push(c);
                i += 1;
                sig.push(if c == '(' { Sig::LParen } else { Sig::Other });
                code_end = Some((out.len(), matches!(c, '(' | '[' | '{' | ',')));
                suppress = false;
                stmt_start = false;
            }
        }
        // Any non-word token (string, number, punctuation) clears the type
        // position; word arms and the whitespace arm `continue` past this.
        prev_word = None;
    }

    Scanned {
        cased: out,
        sig,
        brackets,
        leading_comma,
        code_end,
    }
}

// --- Block structure -------------------------------------------------------

#[derive(Clone, Copy, PartialEq)]
enum Block {
    Func,
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
    /// bug-348: `TESTING`, and its contextual `TGROUP`/`TCASE` children. All
    /// three open real, nestable blocks in the language, but none was modelled
    /// here -- so nothing was pushed while the matching `END` lines *were*
    /// classified as `Op::End` and popped, printing every body line at depth 0.
    Testing,
    Tgroup,
    Tcase,
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
    /// `NEXT`/`LOOP`: an *implicit* block closer — it closes the top loop block
    /// without an `END <kind>` keyword, unlike [`Op::End`]. (`END WHILE` routes
    /// through `End` like every other `END <kind>` closer.)
    EndImplicit,
}

/// Whether a logical line is a `[vis] FUNC name AS pkg::member` re-export
/// alias, which declares no body and so opens no block.
///
/// This is the parser's own test (`check_top_level_func_alias`, bug-480): the
/// alias shape is ambiguous with a parameterless function whose return type is
/// qualified, and it is an alias only when `pkg::member` names neither a
/// built-in type nor a built-in resource. `FUNC face AS canvas::Font` is
/// therefore a function with a body, while `FUNC close AS lib::close` is an
/// alias. A `::` anywhere later - `AS List OF canvas::DrawItem` - is never
/// alias-shaped, and neither is any line with a parameter list.
fn is_func_alias(text: &str) -> bool {
    let code = text.split('\'').next().unwrap_or("");
    let mut words = code.split_whitespace().peekable();
    if words.peek().is_some_and(|w| {
        ["PRIVATE", "PUBLIC", "EXPORT"]
            .iter()
            .any(|v| w.eq_ignore_ascii_case(v))
    }) {
        words.next();
    }
    let Some(keyword) = words.next() else {
        return false;
    };
    if !keyword.eq_ignore_ascii_case("FUNC") {
        return false;
    }
    let Some(name) = words.next() else {
        return false;
    };
    if name.contains('(') || !words.next().is_some_and(|w| w.eq_ignore_ascii_case("AS")) {
        return false;
    }
    // The target is `package::member`. The parser reads it as tokens, so spaces
    // around the `::` are allowed and are closed up here.
    let rest = words.collect::<Vec<_>>().join(" ");
    let rest = rest.replace(" ::", "::").replace(":: ", "::");
    let Some(target) = rest.split_whitespace().next() else {
        return false;
    };
    let Some((package, member)) = target.split_once("::") else {
        return false;
    };
    let is_word = |w: &str| !w.is_empty() && w.chars().all(|c| c.is_alphanumeric() || c == '_');
    if !is_word(package) {
        return false;
    }
    // Like the parser, a malformed target after `pkg::` is still read as an
    // alias, and left for the parser to report.
    if !is_word(member) {
        return true;
    }
    let qualified = format!("{package}.{member}");
    !crate::codegen::builtins::is_qualified_builtin_resource(&qualified)
        && crate::codegen::builtins::qualified_builtin_type(&qualified).is_none()
}

/// Extract the block-structure operations for one logical line, and whether the
/// line's first significant token is itself a structural one (which determines
/// whether the line is printed dedented).
fn structural_ops(sig: &[Sig], func_alias: bool) -> (bool, Vec<Op>) {
    let first_kw = sig.iter().find_map(|s| match s {
        Sig::Kw(k) => Some(*k),
        _ => None,
    });
    // A multi-line `IF` ends the line with `THEN`; a single-line
    // `IF cond THEN stmt [ELSE stmt]` has tokens after `THEN` and opens nothing.
    let multiline_if =
        first_kw == Some(Keyword::If) && matches!(sig.last(), Some(Sig::Kw(Keyword::Then)));

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
    // condition, not a `WHILE … END WHILE` block opener.
    let while_is_condition = matches!(prev_kw, Some(K::Do) | Some(K::Loop));
    match k {
        K::If => (is_first && multiline_if).then_some(Op::Open(Block::If)),
        K::ElseIf | K::Else => is_first.then_some(Op::Else),
        K::Case => Some(Op::Case),
        K::End => Some(Op::End(next_keyword(sig, idx))),
        K::Next | K::Loop => Some(Op::EndImplicit),
        // FUNC may be preceded by visibility/ISOLATED modifiers, so it is not
        // required to lead the line; EXIT FUNC is excluded by `after_exit`. A
        // function *type* `FUNC(…) AS T` (the keyword immediately followed by
        // `(`) is an annotation, not a declaration, and opens nothing.
        K::Func => {
            (!after_exit && !func_alias && !func_type(sig, idx)).then_some(Op::Open(Block::Func))
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
        // bug-348: `TESTING` is a keyword and reaches here; `TGROUP`/`TCASE` are
        // contextual identifiers that never scan as `Sig::Kw`, so they are opened
        // word-wise by `contextual_block_opener` instead.
        K::Testing => Some(Op::Open(Block::Testing)),
        _ => None,
    }
}

/// The block a line-leading contextual word opens, if any (bug-348).
///
/// `TGROUP` and `TCASE` are contextual identifiers, not keywords, so they never
/// scan as `Sig::Kw` and cannot be classified from the keyword stream at all.
/// Their `END TGROUP` / `END TCASE` closers *do* reach `classify` (via `END`) and
/// pop, so without this the stack was popped for frames that were never pushed.
fn contextual_block_opener(trimmed: &str) -> Option<Block> {
    let first = trimmed.split_whitespace().next().unwrap_or("");
    if first.eq_ignore_ascii_case("TGROUP") {
        Some(Block::Tgroup)
    } else if first.eq_ignore_ascii_case("TCASE") {
        Some(Block::Tcase)
    } else {
        None
    }
}

/// Whether the keyword at `idx` is a function *type* (`FUNC(`), i.e. the
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
fn apply_ops(ops: &[Op], stack: &mut Vec<Block>, first_structural: bool, level: usize) -> usize {
    let mut line_indent = level;
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
            Op::EndImplicit => {
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

/// Whether a trimmed line begins a `DOC ... END DOC` block: `DOC` followed only
/// by attribute words such as `INTERNAL`. Shares the lexer's attribute-line
/// recognizer so the two can't drift.
fn is_doc_start(trimmed: &str) -> bool {
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("");
    first.eq_ignore_ascii_case("DOC") && lexer::is_doc_attr_rest(parts.next().unwrap_or(""))
}

/// Re-indent a whole `DOC … END DOC` block. `i` enters pointing at the `DOC`
/// line and leaves pointing past the block's last line. `level` is the block
/// level of the surrounding code.
fn format_doc_block(
    lines: &[&str],
    i: &mut usize,
    n: usize,
    level: usize,
    indent_width: usize,
    out: &mut Vec<String>,
) {
    out.push(format!(
        "{}{}",
        indent_str(level, indent_width),
        doc_header(strip_cr(lines[*i]).trim())
    ));
    *i += 1;
    let body = level + 1;
    let mut in_example = false;
    let mut example: Vec<&str> = Vec::new();
    while *i < n {
        let raw = strip_cr(lines[*i]);
        let words: Vec<&str> = raw.split_whitespace().collect();
        if in_example {
            if lexer::is_end_line(&words, "EXAMPLE") {
                flush_example(&example, body + 1, indent_width, out);
                example.clear();
                out.push(format!("{}END EXAMPLE", indent_str(body, indent_width)));
                in_example = false;
            } else {
                example.push(raw);
            }
        } else if lexer::is_end_line(&words, "DOC") {
            out.push(format!("{}END DOC", indent_str(level, indent_width)));
            *i += 1;
            return;
        } else if words.len() == 1 && words[0].eq_ignore_ascii_case("EXAMPLE") {
            out.push(format!("{}EXAMPLE", indent_str(body, indent_width)));
            in_example = true;
        } else if raw.trim().is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("{}{}", indent_str(body, indent_width), raw.trim()));
        }
        *i += 1;
    }
    // Unterminated block: flush any pending example source so nothing is lost.
    if in_example {
        flush_example(&example, body + 1, indent_width, out);
    }
}

/// Emit `EXAMPLE` source lines anchored at `level`, preserving each line's
/// indentation relative to the least-indented line so the code's own nesting
/// survives while the block as a whole is re-anchored.
fn flush_example(lines: &[&str], level: usize, indent_width: usize, out: &mut Vec<String>) {
    let min = lines
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let prefix = level * indent_width;
    for line in lines {
        if line.trim().is_empty() {
            out.push(String::new());
        } else {
            let lead = line.len() - line.trim_start().len();
            let rel = lead.saturating_sub(min);
            out.push(format!("{}{}", " ".repeat(prefix + rel), line.trim()));
        }
    }
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

// --- LINK blocks -----------------------------------------------------------

/// Re-indent a whole `LINK … END LINK` block from its `FUNC`/`FREE` nesting. `i`
/// enters pointing at the `LINK` line and leaves pointing past `END LINK`. Text
/// and casing are preserved; only leading indentation changes. The DSL's own
/// nesting is charged against the same [`MAX_NESTING_DEPTH`] as the block stack
/// outside it (bug-502).
fn format_link_block(
    lines: &[&str],
    i: &mut usize,
    n: usize,
    level: usize,
    indent_width: usize,
    out: &mut Vec<String>,
) -> Result<(), FormatError> {
    out.push(format!(
        "{}{}",
        indent_str(level, indent_width),
        strip_cr(lines[*i]).trim()
    ));
    *i += 1;
    let mut depth = level + 1;
    while *i < n {
        let text = strip_cr(lines[*i]).trim();
        let words: Vec<&str> = text.split_whitespace().collect();
        let first = words.first().copied().unwrap_or("");
        let second = words.get(1).copied().unwrap_or("");
        // bug-356: `CSTRUCT … END CSTRUCT` and `BIND IN … END BIND` are block
        // constructs inside a LINK block too. Without them their bodies were
        // emitted at the enclosing depth, silently de-indenting the nesting a
        // reader relies on by one level. `BIND STATE` is a single line with no
        // body and correctly opens nothing.
        let is_bind_block = first.eq_ignore_ascii_case("BIND") && second.eq_ignore_ascii_case("IN");
        let is_opener = first.eq_ignore_ascii_case("FUNC")
            || first.eq_ignore_ascii_case("FREE")
            || first.eq_ignore_ascii_case("CSTRUCT")
            || is_bind_block;
        let is_closer = first.eq_ignore_ascii_case("END")
            && (second.eq_ignore_ascii_case("FUNC")
                || second.eq_ignore_ascii_case("FREE")
                || second.eq_ignore_ascii_case("CSTRUCT")
                || second.eq_ignore_ascii_case("BIND"));
        if first.eq_ignore_ascii_case("END") && second.eq_ignore_ascii_case("LINK") {
            out.push(format!("{}END LINK", indent_str(level, indent_width)));
            *i += 1;
            return Ok(());
        } else if text.is_empty() {
            out.push(String::new());
        } else if is_closer {
            depth = depth.saturating_sub(1);
            out.push(format!("{}{}", indent_str(depth, indent_width), text));
        } else if is_opener {
            out.push(format!("{}{}", indent_str(depth, indent_width), text));
            depth += 1;
            if depth > MAX_NESTING_DEPTH {
                return Err(FormatError::NestingTooDeep { line: *i + 1 });
            }
        } else {
            out.push(format!("{}{}", indent_str(depth, indent_width), text));
        }
        *i += 1;
    }
    Ok(())
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

#[cfg(test)]
mod tests;
