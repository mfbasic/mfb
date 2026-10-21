//! A built-in call mixing a named argument with positional ones: a positional
//! argument fills the next slot a named one has not already taken, and one past
//! the last parameter is an extra the arity check counts.

use crate::testutil::check_src_details;

fn found(src: &str) -> Vec<(String, String)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.detail))
        .collect()
}

#[test]
fn positional_arguments_skip_a_slot_a_named_one_filled() {
    let src = "\
IMPORT compress
FUNC unpack(bytes AS List OF Byte) AS Integer
  LET out = compress::gzipDecode(data := bytes, 100, FALSE)
  RETURN 0
END FUNC
FUNC main() AS Integer
  RETURN 0
END FUNC
";
    assert_eq!(found(src), Vec::<(String, String)>::new());
}

#[test]
fn a_positional_argument_past_the_last_parameter_is_an_extra() {
    let src = "\
IMPORT compress
FUNC unpack(bytes AS List OF Byte) AS Integer
  LET out = compress::gzipDecode(data := bytes, 100, FALSE, TRUE)
  RETURN 0
END FUNC
FUNC main() AS Integer
  RETURN 0
END FUNC
";
    let arity: Vec<String> = found(src)
        .into_iter()
        .filter(|(rule, _)| rule == "TYPE_CALL_ARITY_MISMATCH")
        .map(|(_, detail)| detail)
        .collect();
    assert_eq!(
        arity,
        ["Call to `compress.gzipDecode` has 4 argument(s), expected 1 to 3.".to_string()],
        "the extra counts toward the call's arity"
    );
}
