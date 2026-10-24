//! The growing `String` self-updates (`collection/assign/string_self_update/grow.rs`)
//! handed a module the front end would have rejected (`nir_retype.rs`).
//!
//! `s = strings::padRightToWidth(s, columns, padChar)` measures the result before it
//! grows `s`: the column count must be an `Integer`, and any other type is refused by
//! name rather than read as a column count.

use super::nir_retype::{refusal, retyped};
use crate::types::ParameterType;

const SRC: &str = r#"IMPORT io
IMPORT strings

FUNC pad(s0 AS String, n AS Integer) AS Integer
  MUT s AS String = s0
  s = strings::padRightToWidth(s, n, "*")
  RETURN len(s)
END FUNC

FUNC main() AS Integer
  io::print(toString(pad("a", 3)))
  RETURN 0
END FUNC
"#;

#[test]
fn a_pad_to_width_column_count_must_be_an_integer() {
    for columns in ["String", "Float"] {
        let err = refusal(
            retyped(SRC, "pad", &[("n", ParameterType::parse(columns))]),
            &format!("padRightToWidth with {columns} columns"),
        );
        assert!(
            err.contains(&format!(
                "strings.padToWidth columns must be Integer, got {columns}"
            )),
            "{err}"
        );
    }
}
