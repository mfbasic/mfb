//! Compile-time (constant-fold) Unicode oracles. Only the five table-consuming
//! builtins that are folded on static strings live here — `find`/`mid` and the
//! scalar-index helpers were deleted (audit-unicode #5): they had no callers,
//! read like a live fold path, and misled the strings/Unicode audit. `find` and
//! `mid` are never constant-folded; both always lower to the runtime path so an
//! out-of-range argument raises the catchable runtime error, never a build
//! error. If folding them is ever added, fold the error condition back to the
//! runtime path — do not turn the catchable 77050001 into a build error.

use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) fn upper(value: &str) -> String {
    value.chars().flat_map(char::to_uppercase).collect()
}

pub(crate) fn lower(value: &str) -> String {
    value.chars().flat_map(char::to_lowercase).collect()
}

pub(crate) fn case_fold(value: &str) -> String {
    value.case_fold().collect()
}

pub(crate) fn normalize_nfc(value: &str) -> String {
    value.nfc().collect()
}

pub(crate) fn graphemes(value: &str) -> Vec<String> {
    UnicodeSegmentation::graphemes(value, true)
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_upper_and_lower() {
        assert_eq!(upper("straße"), "STRASSE");
        assert_eq!(lower("İ"), "i\u{307}");
        assert_eq!(upper("é日😀"), "É日😀");
    }

    #[test]
    fn folds_case() {
        assert_eq!(case_fold("Straße"), "strasse");
        assert_eq!(case_fold("K"), "k");
    }

    #[test]
    fn normalizes_nfc() {
        assert_eq!(normalize_nfc("Cafe\u{301}"), "Café");
        assert_eq!(normalize_nfc("A\u{30a}"), "Å");
        assert_eq!(normalize_nfc("\u{1100}\u{1161}"), "가");
    }

    #[test]
    fn segments_graphemes() {
        assert_eq!(graphemes("a\u{301}"), vec!["a\u{301}"]);
        assert_eq!(graphemes("👨‍👩‍👧‍👦"), vec!["👨‍👩‍👧‍👦"]);
        assert_eq!(graphemes("🇺🇸x"), vec!["🇺🇸", "x"]);
    }
}
