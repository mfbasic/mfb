pub(crate) const TYPE_BYTE: &str = "Byte";
pub(crate) const TYPE_FIXED: &str = "Fixed";
pub(crate) const TYPE_FLOAT: &str = "Float";
pub(crate) const TYPE_INTEGER: &str = "Integer";

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
