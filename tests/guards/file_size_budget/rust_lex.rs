//! Rust line classification for the file-size budget.
//!
//! `rust_code_text` strips comments and blanks literals so the later passes see
//! only code. `rust_lines` reports each line as blank, comment-only or code, and
//! `rust_test_lines` marks the code lines that belong to a `#[cfg(test)]` item.
//! The rules are the ones `tools/loc-census/census.py` implements; the two must
//! agree on every file in scope.

/// What one source line counts as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    /// Nothing but whitespace.
    Blank,
    /// Text, but no code once comments are removed.
    Comment,
    /// At least one character of code (a literal counts as code).
    Code,
}

pub fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn starts_with(text: &[char], i: usize, pat: &str) -> bool {
    pat.chars()
        .enumerate()
        .all(|(k, p)| text.get(i + k) == Some(&p))
}

/// Length of a raw-string opener (`r"`, `r#"`, `br##"`, `cr"`) at `i`, and its
/// `#` count. The opener must start within 300 characters, as the oracle's
/// windowed match does.
fn raw_string_start(text: &[char], i: usize) -> Option<(usize, usize)> {
    let limit = (i + 300).min(text.len());
    let mut j = i;
    if j + 1 < limit && (text[j] == 'b' || text[j] == 'c') && text[j + 1] == 'r' {
        j += 2;
    } else if j < limit && text[j] == 'r' {
        j += 1;
    } else {
        return None;
    }
    let mut hashes = 0;
    while j < limit && text[j] == '#' {
        hashes += 1;
        j += 1;
    }
    if j < limit && text[j] == '"' {
        Some((j + 1 - i, hashes))
    } else {
        None
    }
}

/// Length of a char literal (`'x'`, `'\n'`, `'\x41'`, `'\u{1F600}'`) at `i`,
/// which holds a `'`. `None` for a lifetime or loop label. The literal must fit
/// in 16 characters.
fn char_literal(text: &[char], i: usize) -> Option<usize> {
    let limit = (i + 16).min(text.len());
    let at = |k: usize| if k < limit { Some(text[k]) } else { None };
    let close = |k: usize| {
        if at(k) == Some('\'') {
            Some(k + 1 - i)
        } else {
            None
        }
    };
    let c = at(i + 1)?;
    if c == '\\' {
        if at(i + 2) == Some('x')
            && at(i + 3).is_some_and(|h| h.is_ascii_hexdigit())
            && at(i + 4).is_some_and(|h| h.is_ascii_hexdigit())
        {
            if let Some(n) = close(i + 5) {
                return Some(n);
            }
        }
        if at(i + 2) == Some('u') && at(i + 3) == Some('{') {
            let mut k = i + 4;
            while at(k).is_some_and(|h| h.is_ascii_hexdigit()) {
                k += 1;
            }
            if k > i + 4 && at(k) == Some('}') {
                if let Some(n) = close(k + 1) {
                    return Some(n);
                }
            }
        }
        match at(i + 2) {
            Some(e) if e != '\n' => close(i + 3),
            _ => None,
        }
    } else if c != '\'' && c != '\n' {
        close(i + 2)
    } else {
        None
    }
}

/// Each line's code text: comments removed, every string, raw-string and char
/// literal replaced by `S` (once on its first line and once on each line it
/// continues onto). The result has one entry per `\n`-separated line.
pub fn rust_code_text(text: &str) -> Vec<String> {
    let t: Vec<char> = text.chars().collect();
    let n = t.len();
    let mut out = vec![String::new(); text.matches('\n').count() + 1];
    let (mut i, mut line) = (0usize, 0usize);
    let prev_ident = |i: usize| i > 0 && ident_char(t[i - 1]);
    while i < n {
        let c = t[i];
        if c == '\n' {
            line += 1;
            i += 1;
            continue;
        }
        if starts_with(&t, i, "//") {
            while i < n && t[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if starts_with(&t, i, "/*") {
            let mut d = 1;
            i += 2;
            while i < n && d > 0 {
                if starts_with(&t, i, "/*") {
                    d += 1;
                    i += 2;
                } else if starts_with(&t, i, "*/") {
                    d -= 1;
                    i += 2;
                } else {
                    if t[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }
        if let Some((len, hashes)) = raw_string_start(&t, i).filter(|_| !prev_ident(i)) {
            out[line].push('S');
            i += len;
            let end: String = std::iter::once('"')
                .chain(std::iter::repeat_n('#', hashes))
                .collect();
            while i < n && !starts_with(&t, i, &end) {
                if t[i] == '\n' {
                    line += 1;
                    out[line].push('S');
                }
                i += 1;
            }
            i += end.chars().count();
            continue;
        }
        if c == '"' || ((c == 'b' || c == 'c') && t.get(i + 1) == Some(&'"') && !prev_ident(i)) {
            if c != '"' {
                i += 1;
            }
            out[line].push('S');
            i += 1;
            while i < n && t[i] != '"' {
                if t[i] == '\\' {
                    i += 1;
                }
                if i < n && t[i] == '\n' {
                    line += 1;
                    out[line].push('S');
                }
                i += 1;
            }
            i += 1;
            continue;
        }
        if c == '\'' {
            if let Some(len) = char_literal(&t, i) {
                out[line].push('S');
                i += len;
                continue;
            }
        }
        out[line].push(c);
        i += 1;
    }
    out
}

/// Blank / comment / code for each line of `text`.
pub fn rust_lines(text: &str) -> Vec<LineKind> {
    rust_code_text(text)
        .iter()
        .zip(text.split('\n'))
        .map(|(code, raw)| {
            if !code.trim().is_empty() {
                LineKind::Code
            } else if raw.trim().is_empty() {
                LineKind::Blank
            } else {
                LineKind::Comment
            }
        })
        .collect()
}

/// Byte offset of the first `#[cfg(test` / `#[cfg(all(test` (followed by a
/// non-identifier character) in a line of code text.
fn cfg_test_at(s: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(off) = s[from..].find("#[cfg(") {
        let start = from + off;
        let mut rest = &s[start + "#[cfg(".len()..];
        if let Some(r) = rest.strip_prefix("all(") {
            rest = r;
        }
        if let Some(after) = rest.strip_prefix("test") {
            if !after.chars().next().is_some_and(ident_char) {
                return Some(start);
            }
        }
        from = start + 1;
    }
    None
}

/// For each line of code text (from `rust_code_text`), whether it is a code line
/// inside a `#[cfg(test)]` / `#[cfg(all(test, …))]` item. The item runs from its
/// attribute to the first `;` or `,` at the attribute's bracket depth, the `}`
/// that closes a body opened at that depth, or a closer that drops below it (a
/// line starting with that closer belongs to the enclosing construct).
pub fn rust_test_lines(code: &[String]) -> Vec<bool> {
    let mut test = Vec::with_capacity(code.len());
    // (bracket depth at the attribute, whether the item's `{` body opened)
    let mut state: Option<(i64, bool)> = None;
    let mut depth: i64 = 0;
    for line in code {
        let s = line.trim();
        if s.is_empty() {
            test.push(false);
            continue;
        }
        let mut line_is_test = state.is_some();
        let mut at_start = true;
        let mut scan = s;
        if state.is_none() {
            if let Some(start) = cfg_test_at(s) {
                line_is_test = true;
                if start > 0 {
                    at_start = false;
                    for ch in s[..start].chars() {
                        match ch {
                            '(' | '[' | '{' => depth += 1,
                            ')' | ']' | '}' => depth -= 1,
                            _ => {}
                        }
                    }
                    scan = &s[start..];
                }
                state = Some((depth, false));
            }
        }
        for (i, ch) in scan.chars().enumerate() {
            match ch {
                '(' | '[' | '{' => depth += 1,
                ')' | ']' | '}' => {
                    depth -= 1;
                    if ch == '}' && matches!(state, Some((d, true)) if d == depth) {
                        state = None;
                    }
                }
                ';' | ',' if matches!(state, Some((d, _)) if d == depth) => state = None,
                _ => {}
            }
            if let Some((d, _)) = state {
                if depth < d {
                    state = None;
                    if i == 0 && at_start {
                        line_is_test = false;
                    }
                }
            }
            if let Some((d, body)) = state.as_mut() {
                if ch == '{' && depth == *d + 1 {
                    *body = true;
                }
            }
        }
        test.push(line_is_test);
    }
    test
}
