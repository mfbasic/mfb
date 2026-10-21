//! plan-180-A: `os::exit` with a constant code outside the host range
//! `0..=255` is refused when the program is checked — a negated literal
//! (`-1`) counts as a constant.

use crate::testutil::check_src_details;

fn found(src: &str) -> Vec<(String, String)> {
    check_src_details(src)
        .into_iter()
        .map(|d| (d.rule, d.detail))
        .collect()
}

#[test]
fn a_negated_exit_code_constant_is_out_of_range() {
    let src = "IMPORT os\nFUNC main() AS Integer\n  os::exit(-1)\n  RETURN 0\nEND FUNC\n";
    assert_eq!(
        found(src),
        [(
            "OS_EXIT_CODE_OUT_OF_RANGE".to_string(),
            "os::exit constant exit code must be in the host range 0..255.".to_string()
        )]
    );
}
