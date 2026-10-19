//! MFBASIC line classification for the file-size budget.
//!
//! A line is not code when it is blank, starts with `'`, starts with the word
//! `REM`, or lies in a `DOC … END DOC` block (the `DOC` line may carry uppercase
//! attribute words). Code lines from `TESTING` through `END TESTING` are test
//! lines. Same rules as `tools/loc-census/census.py`.

use crate::rust_lex::{ident_char, LineKind};

/// `s` starts with the word `word` (the next character is not a word character).
fn starts_with_word(s: &str, word: &str) -> bool {
    s.strip_prefix(word)
        .is_some_and(|rest| !rest.chars().next().is_some_and(ident_char))
}

/// A `DOC` opener: `DOC` alone or followed by whitespace-separated uppercase words.
fn is_doc_open(s: &str) -> bool {
    let Some(rest) = s.strip_prefix("DOC") else {
        return false;
    };
    rest.is_empty()
        || (rest.starts_with(char::is_whitespace)
            && rest
                .split_whitespace()
                .all(|w| w.chars().all(|c| c.is_ascii_uppercase())))
}

fn is_rem(s: &str) -> bool {
    s.strip_prefix("REM")
        .is_some_and(|rest| rest.is_empty() || rest.starts_with(char::is_whitespace))
}

/// Per line: its kind, and whether a code line is inside a `TESTING` block.
pub fn mfb_lines(text: &str) -> Vec<(LineKind, bool)> {
    let mut out = Vec::new();
    let (mut in_doc, mut in_test) = (false, false);
    for raw in text.split('\n') {
        let s = raw.trim();
        if in_doc {
            if starts_with_word(s, "END DOC") {
                in_doc = false;
            }
            out.push((
                if s.is_empty() {
                    LineKind::Blank
                } else {
                    LineKind::Comment
                },
                false,
            ));
            continue;
        }
        if is_doc_open(s) {
            in_doc = true;
            out.push((LineKind::Comment, false));
            continue;
        }
        if s.is_empty() {
            out.push((LineKind::Blank, false));
            continue;
        }
        if s.starts_with('\'') || is_rem(s) {
            out.push((LineKind::Comment, false));
            continue;
        }
        if s == "TESTING" {
            in_test = true;
        }
        out.push((LineKind::Code, in_test));
        if starts_with_word(s, "END TESTING") {
            in_test = false;
        }
    }
    out
}
