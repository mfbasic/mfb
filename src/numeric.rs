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
