pub(crate) const TYPE_BYTE: &str = "Byte";
pub(crate) const TYPE_FIXED: &str = "Fixed";
pub(crate) const TYPE_FLOAT: &str = "Float";
pub(crate) const TYPE_INTEGER: &str = "Integer";

/// The literal type a numeric-literal string classifies to. Distinct from the
/// runtime numeric-type constants above: this is only the *literal* lattice
/// (`Integer`/`Float`/`Fixed`) that `classify_literal` decides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LiteralType {
    Integer,
    Float,
    Fixed,
}

/// Classify a *canonical* numeric-literal string (as emitted by the lexer:
/// digit separators already stripped, any radix prefix already decoded to
/// decimal, optional trailing `.`-fraction / `e`-exponent / `f`|`F` suffix)
/// into its literal type and a suffix-free, parse-ready value string.
///
/// This is the single source of truth for numeric-literal typing, replacing the
/// scattered `value.contains('.')` checks (plan-28-A §4.1). A trailing `f`
/// forces `Float` and `F` forces `Fixed` (plan-28-B); otherwise a `.` or an
/// `e`/`E` exponent is `Float`; otherwise `Integer`. The returned value string
/// has any suffix removed so every `parse::<i64>()`/`parse::<f64>()` consumer
/// keeps working.
pub(crate) fn classify_literal(text: &str) -> (String, LiteralType) {
    if let Some(stripped) = text.strip_suffix('f') {
        return (stripped.to_string(), LiteralType::Float);
    }
    if let Some(stripped) = text.strip_suffix('F') {
        return (stripped.to_string(), LiteralType::Fixed);
    }
    if text.contains('.') || text.contains('e') || text.contains('E') {
        (text.to_string(), LiteralType::Float)
    } else {
        (text.to_string(), LiteralType::Integer)
    }
}

pub(crate) fn binary_result_type(operator: &str, left: &str, right: &str) -> Option<&'static str> {
    if !is_numeric_type(left) || !is_numeric_type(right) {
        return None;
    }
    if operator == "DIV" {
        Some(TYPE_FLOAT)
    } else if left == TYPE_FIXED || right == TYPE_FIXED {
        Some(TYPE_FIXED)
    } else if left == TYPE_FLOAT || right == TYPE_FLOAT {
        Some(TYPE_FLOAT)
    } else if left == TYPE_BYTE && right == TYPE_BYTE {
        Some(TYPE_BYTE)
    } else {
        Some(TYPE_INTEGER)
    }
}

fn is_numeric_type(type_: &str) -> bool {
    matches!(type_, TYPE_BYTE | TYPE_FIXED | TYPE_FLOAT | TYPE_INTEGER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_literal_types_integers_and_floats() {
        assert_eq!(
            classify_literal("42"),
            ("42".to_string(), LiteralType::Integer)
        );
        assert_eq!(
            classify_literal("4.5"),
            ("4.5".to_string(), LiteralType::Float)
        );
        // Radix literals reach the classifier already canonicalized to decimal.
        assert_eq!(
            classify_literal("4095"),
            ("4095".to_string(), LiteralType::Integer)
        );
    }

    #[test]
    fn classify_literal_handles_exponent_and_suffixes() {
        // Exponent -> Float (plan-28-B); the value string is left parse-ready.
        assert_eq!(
            classify_literal("1e3"),
            ("1e3".to_string(), LiteralType::Float)
        );
        // f/F suffixes force Float/Fixed and are stripped from the value.
        assert_eq!(
            classify_literal("2f"),
            ("2".to_string(), LiteralType::Float)
        );
        assert_eq!(
            classify_literal("2F"),
            ("2".to_string(), LiteralType::Fixed)
        );
        assert_eq!(
            classify_literal("1.5F"),
            ("1.5".to_string(), LiteralType::Fixed)
        );
        // A suffix wins even with an exponent.
        assert_eq!(
            classify_literal("1e3F"),
            ("1e3".to_string(), LiteralType::Fixed)
        );
    }

    #[test]
    fn non_numeric_operand_has_no_result_type() {
        assert_eq!(binary_result_type("+", "String", TYPE_INTEGER), None);
        assert_eq!(binary_result_type("+", TYPE_INTEGER, "Boolean"), None);
        assert_eq!(binary_result_type("+", "String", "String"), None);
    }

    #[test]
    fn div_always_yields_float() {
        // DIV promotes to Float regardless of operand types (even Byte/Byte).
        assert_eq!(
            binary_result_type("DIV", TYPE_BYTE, TYPE_BYTE),
            Some(TYPE_FLOAT)
        );
        assert_eq!(
            binary_result_type("DIV", TYPE_INTEGER, TYPE_INTEGER),
            Some(TYPE_FLOAT)
        );
    }

    #[test]
    fn fixed_dominates_all_other_numerics() {
        assert_eq!(
            binary_result_type("+", TYPE_FIXED, TYPE_INTEGER),
            Some(TYPE_FIXED)
        );
        assert_eq!(
            binary_result_type("*", TYPE_FLOAT, TYPE_FIXED),
            Some(TYPE_FIXED)
        );
        assert_eq!(
            binary_result_type("-", TYPE_BYTE, TYPE_FIXED),
            Some(TYPE_FIXED)
        );
    }

    #[test]
    fn float_dominates_integer_and_byte() {
        assert_eq!(
            binary_result_type("+", TYPE_FLOAT, TYPE_INTEGER),
            Some(TYPE_FLOAT)
        );
        assert_eq!(
            binary_result_type("*", TYPE_BYTE, TYPE_FLOAT),
            Some(TYPE_FLOAT)
        );
    }

    #[test]
    fn byte_pair_stays_byte_but_mixed_widens_to_integer() {
        assert_eq!(
            binary_result_type("+", TYPE_BYTE, TYPE_BYTE),
            Some(TYPE_BYTE)
        );
        assert_eq!(
            binary_result_type("+", TYPE_BYTE, TYPE_INTEGER),
            Some(TYPE_INTEGER)
        );
        assert_eq!(
            binary_result_type("+", TYPE_INTEGER, TYPE_INTEGER),
            Some(TYPE_INTEGER)
        );
    }

    #[test]
    fn is_numeric_type_accepts_only_the_four_numerics() {
        for t in [TYPE_BYTE, TYPE_FIXED, TYPE_FLOAT, TYPE_INTEGER] {
            assert!(is_numeric_type(t), "{t} should be numeric");
        }
        for t in ["String", "Boolean", "Nothing", ""] {
            assert!(!is_numeric_type(t), "{t} should not be numeric");
        }
    }
}
