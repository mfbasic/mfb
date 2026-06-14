#![allow(dead_code)]

use unicode_casefold::UnicodeCaseFold;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Scalar {
    pub(crate) value: char,
    pub(crate) byte_width: usize,
}

pub(crate) fn validate_utf8(bytes: &[u8]) -> Result<(), String> {
    std::str::from_utf8(bytes)
        .map(|_| ())
        .map_err(|_| "invalid UTF-8".to_string())
}

pub(crate) fn decode_next(bytes: &[u8]) -> Result<Option<Scalar>, String> {
    let Some(first) = bytes.first() else {
        return Ok(None);
    };
    let width = utf8_scalar_width(*first)?;
    if bytes.len() < width {
        return Err("truncated UTF-8 scalar".to_string());
    }
    let text = std::str::from_utf8(&bytes[..width]).map_err(|_| "invalid UTF-8".to_string())?;
    let value = text
        .chars()
        .next()
        .ok_or_else(|| "invalid UTF-8 scalar".to_string())?;
    Ok(Some(Scalar {
        value,
        byte_width: width,
    }))
}

pub(crate) fn encode_scalar(value: char, output: &mut Vec<u8>) {
    let mut buffer = [0; 4];
    output.extend_from_slice(value.encode_utf8(&mut buffer).as_bytes());
}

pub(crate) fn scalar_count(value: &str) -> usize {
    value.chars().count()
}

pub(crate) fn byte_offset_for_scalar_index(
    value: &str,
    scalar_index: usize,
) -> Result<usize, String> {
    if scalar_index == scalar_count(value) {
        return Ok(value.len());
    }
    value
        .char_indices()
        .nth(scalar_index)
        .map(|(offset, _)| offset)
        .ok_or_else(|| "scalar index out of range".to_string())
}

pub(crate) fn scalar_index_for_byte_offset(
    value: &str,
    byte_offset: usize,
) -> Result<usize, String> {
    if byte_offset == value.len() {
        return Ok(value.chars().count());
    }
    if byte_offset > value.len() || !value.is_char_boundary(byte_offset) {
        return Err("byte offset is not a scalar boundary".to_string());
    }
    Ok(value[..byte_offset].chars().count())
}

pub(crate) fn is_scalar_boundary(value: &str, byte_offset: usize) -> bool {
    value.is_char_boundary(byte_offset)
}

pub(crate) fn mid(value: &str, start: usize, length: usize) -> Result<String, String> {
    let start_offset = byte_offset_for_scalar_index(value, start)?;
    let end_scalar = start
        .checked_add(length)
        .ok_or_else(|| "scalar index overflow".to_string())?;
    let end_offset = byte_offset_for_scalar_index(value, end_scalar)?;
    Ok(value[start_offset..end_offset].to_string())
}

pub(crate) fn find(value: &str, needle: &str, start: usize) -> Result<usize, String> {
    let start_offset = byte_offset_for_scalar_index(value, start)?;
    if needle.is_empty() {
        return Ok(start);
    }
    let Some(relative) = value[start_offset..].find(needle) else {
        return Err("not found".to_string());
    };
    let byte_offset = start_offset + relative;
    scalar_index_for_byte_offset(value, byte_offset)
}

pub(crate) fn is_whitespace(value: char) -> bool {
    value.is_whitespace()
}

pub(crate) fn trim(value: &str) -> String {
    value.trim_matches(is_whitespace).to_string()
}

pub(crate) fn trim_start(value: &str) -> String {
    value.trim_start_matches(is_whitespace).to_string()
}

pub(crate) fn trim_end(value: &str) -> String {
    value.trim_end_matches(is_whitespace).to_string()
}

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

pub(crate) fn split(value: &str, delimiter: &str) -> Result<Vec<String>, String> {
    if delimiter.is_empty() {
        return Err("delimiter must not be empty".to_string());
    }
    Ok(value.split(delimiter).map(str::to_string).collect())
}

pub(crate) fn join(parts: &[String], delimiter: &str) -> String {
    parts.join(delimiter)
}

fn utf8_scalar_width(first: u8) -> Result<usize, String> {
    match first {
        0x00..=0x7f => Ok(1),
        0xc2..=0xdf => Ok(2),
        0xe0..=0xef => Ok(3),
        0xf0..=0xf4 => Ok(4),
        _ => Err("invalid UTF-8 leading byte".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_utf8() {
        assert!(validate_utf8("aé日😀".as_bytes()).is_ok());
        assert!(validate_utf8(&[0xf0, 0x28, 0x8c, 0x28]).is_err());
    }

    #[test]
    fn decodes_next_scalar() {
        let scalar = decode_next("😀x".as_bytes()).unwrap().unwrap();
        assert_eq!(scalar.value, '😀');
        assert_eq!(scalar.byte_width, 4);
        assert_eq!(decode_next(&[]).unwrap(), None);
        assert!(decode_next(&[0xe2, 0x82]).is_err());
    }

    #[test]
    fn encodes_scalar() {
        let mut output = Vec::new();
        encode_scalar('é', &mut output);
        encode_scalar('日', &mut output);
        assert_eq!(String::from_utf8(output).unwrap(), "é日");
    }

    #[test]
    fn maps_scalar_indexes_and_byte_offsets() {
        let value = "aé日😀";
        assert_eq!(scalar_count(value), 4);
        assert_eq!(byte_offset_for_scalar_index(value, 0).unwrap(), 0);
        assert_eq!(byte_offset_for_scalar_index(value, 1).unwrap(), 1);
        assert_eq!(byte_offset_for_scalar_index(value, 2).unwrap(), 3);
        assert_eq!(byte_offset_for_scalar_index(value, 3).unwrap(), 6);
        assert_eq!(byte_offset_for_scalar_index(value, 4).unwrap(), 10);
        assert_eq!(scalar_index_for_byte_offset(value, 6).unwrap(), 3);
        assert!(scalar_index_for_byte_offset(value, 2).is_err());
        assert!(is_scalar_boundary(value, 6));
        assert!(!is_scalar_boundary(value, 2));
    }

    #[test]
    fn slices_mid_by_scalar_indexes() {
        assert_eq!(mid("aé日😀z", 1, 3).unwrap(), "é日😀");
        assert_eq!(mid("aé日", 3, 0).unwrap(), "");
        assert!(mid("aé日", 4, 1).is_err());
    }

    #[test]
    fn finds_by_scalar_index() {
        assert_eq!(find("aé日é", "日", 0).unwrap(), 2);
        assert_eq!(find("aé日é", "é", 2).unwrap(), 3);
        assert_eq!(find("aé日é", "", 2).unwrap(), 2);
        assert!(find("aé日é", "x", 0).is_err());
    }

    #[test]
    fn recognizes_unicode_whitespace_and_trims() {
        assert!(is_whitespace('\u{00a0}'));
        assert!(is_whitespace('\u{3000}'));
        assert_eq!(trim("\u{00a0}é\u{3000}"), "é");
        assert_eq!(trim_start("\u{3000}wide\u{3000}"), "wide\u{3000}");
        assert_eq!(trim_end("\u{3000}wide\u{3000}"), "\u{3000}wide");
    }

    #[test]
    fn maps_upper_and_lower() {
        assert_eq!(upper("straße"), "STRASSE");
        assert_eq!(lower("İ"), "i\u{307}");
        assert_eq!(upper("é日😀"), "É日😀");
    }

    #[test]
    fn folds_case() {
        assert_eq!(case_fold("Straße"), "strasse");
        assert_eq!(case_fold("K"), "k");
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

    #[test]
    fn splits_and_joins() {
        assert_eq!(split("a😀b😀c", "😀").unwrap(), vec!["a", "b", "c"]);
        assert!(split("abc", "").is_err());
        assert_eq!(
            join(&["😀".to_string(), "é".to_string(), "日".to_string()], "|"),
            "😀|é|日"
        );
    }
}
