//! One sanitizer for externally-sourced text bound for the operator's terminal.
//!
//! Names, versions, paths, and `.mfp` header fields come from untrusted manifests
//! and packages. Written raw, they let a malicious package forge the very report
//! the operator uses to decide whether to trust it (bug-24, bug-210):
//!
//! * **C0/C1 controls** — an embedded ESC/CSI recolors or erases output; `\r`
//!   returns to column 0 and overwrites the line just printed; `\n` forges whole
//!   rows.
//! * **Bidi/format overrides** — U+202E (RLO) and friends visually reorder a run
//!   without changing its bytes, so `legit\u{202e}drowssap.mfp` renders as
//!   `legitpassword.mfp`. The isolate/embedding controls (U+2066–U+2069,
//!   U+202A–U+202D), the implicit marks (U+200E/U+200F, U+061C), and the
//!   zero-width/joiner set (U+200B–U+200D, U+2060–U+2064, U+FEFF) are all
//!   invisible yet semantically active, so they are escaped too.
//!
//! Everything escaped is rendered as `\u{XXXX}`, which is unambiguous and inert.
//! A well-formed value contains none of these and passes through unchanged (and
//! unallocated).

use std::borrow::Cow;
use std::fmt::Write;

/// Whether `ch` must not reach the terminal verbatim: a C0/C1 control, or a
/// Unicode bidi/format code point that is invisible but semantically active.
fn is_terminal_unsafe(ch: char) -> bool {
    ch.is_control()
        || matches!(ch,
            // Arabic letter mark.
            '\u{061C}'
            // ZWSP, ZWNJ, ZWJ, LRM, RLM.
            | '\u{200B}'..='\u{200F}'
            // LRE, RLE, PDF, LRO, RLO.
            | '\u{202A}'..='\u{202E}'
            // Word joiner and the invisible math operators.
            | '\u{2060}'..='\u{2064}'
            // LRI, RLI, FSI, PDI.
            | '\u{2066}'..='\u{2069}'
            // Zero-width no-break space / BOM.
            | '\u{FEFF}')
}

/// Escape every terminal-unsafe code point in `value` as `\u{XXXX}`.
pub(crate) fn safe(value: &str) -> Cow<'_, str> {
    if !value.chars().any(is_terminal_unsafe) {
        return Cow::Borrowed(value);
    }
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        if is_terminal_unsafe(ch) {
            let _ = write!(escaped, "\\u{{{:04x}}}", ch as u32);
        } else {
            escaped.push(ch);
        }
    }
    Cow::Owned(escaped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_formed_passes_through_unallocated() {
        assert!(matches!(safe("shape-2.1.0"), Cow::Borrowed("shape-2.1.0")));
    }

    #[test]
    fn escapes_c0_c1_controls() {
        assert_eq!(safe("a\u{1b}[31mb"), "a\\u{001b}[31mb");
        assert_eq!(safe("row\nforged"), "row\\u{000a}forged");
        assert_eq!(safe("line\rover"), "line\\u{000d}over");
        assert_eq!(safe("c1\u{0085}x"), "c1\\u{0085}x");
    }

    #[test]
    fn escapes_bidi_and_format_overrides() {
        // bug-210: RLO visually reverses the run that follows it.
        assert_eq!(
            safe("legit\u{202e}drowssap.mfp"),
            "legit\\u{202e}drowssap.mfp"
        );
        assert_eq!(safe("a\u{200e}b"), "a\\u{200e}b");
        assert_eq!(safe("a\u{2066}b\u{2069}"), "a\\u{2066}b\\u{2069}");
        assert_eq!(safe("a\u{061c}b"), "a\\u{061c}b");
        assert_eq!(safe("a\u{200b}b"), "a\\u{200b}b");
        assert_eq!(safe("a\u{feff}b"), "a\\u{feff}b");
    }

    #[test]
    fn ordinary_non_ascii_is_preserved() {
        // Only the invisible/active set is escaped — real text is untouched.
        assert!(matches!(safe("naïve-café-日本"), Cow::Borrowed(_)));
    }
}
