//! Cases for the file-size budget's lexers and classifier. Each pins one rule
//! from `tools/loc-census/README.md`; the whole-tree agreement with the oracle
//! is checked by running both (plan-176-A, Phase 1 acceptance).

use std::collections::BTreeSet;

use crate::classify::{count_text, is_generated, test_mod_decls, Kind};
use crate::mfb_lex::mfb_lines;
use crate::rust_lex::{rust_code_text, rust_lines, rust_test_lines, LineKind};

/// One letter per line: `b` blank, `c` comment, `C` code, `T` test code.
fn rust_shape(text: &str) -> String {
    let tests = rust_test_lines(&rust_code_text(text));
    rust_lines(text)
        .into_iter()
        .zip(tests)
        .map(|(k, t)| match (k, t) {
            (LineKind::Blank, _) => 'b',
            (LineKind::Comment, _) => 'c',
            (LineKind::Code, false) => 'C',
            (LineKind::Code, true) => 'T',
        })
        .collect()
}

fn mfb_shape(text: &str) -> String {
    mfb_lines(text)
        .into_iter()
        .map(|(k, t)| match (k, t) {
            (LineKind::Blank, _) => 'b',
            (LineKind::Comment, _) => 'c',
            (LineKind::Code, false) => 'C',
            (LineKind::Code, true) => 'T',
        })
        .collect()
}

fn code(text: &str) -> Vec<String> {
    rust_code_text(text)
}

#[test]
fn line_comment_marker_inside_a_string_is_code() {
    assert_eq!(code("f(\"//\"); g(); // h"), vec!["f(S); g(); "]);
}

#[test]
fn quote_inside_a_raw_string_does_not_open_a_string() {
    assert_eq!(rust_shape("let s = r#\"a \" b\"#;\n// d"), "Cc");
    assert_eq!(code("let s = r#\"a \" b\"#; x")[0], "let s = S; x");
}

#[test]
fn byte_string_holding_a_comment_marker_is_code() {
    assert_eq!(code("let s = b\"//\"; x")[0], "let s = S; x");
}

#[test]
fn quote_char_literal_does_not_open_a_string() {
    assert_eq!(
        code("let q = '\"'; let s = \"a\";")[0],
        "let q = S; let s = S;"
    );
}

#[test]
fn lifetime_then_comment_marker_in_a_string() {
    assert_eq!(
        code("fn f<'a>(x: &'a str) -> &'a str { \"//\" }")[0],
        "fn f<'a>(x: &'a str) -> &'a str { S }"
    );
}

#[test]
fn nested_block_comment_spans_lines() {
    assert_eq!(rust_shape("/* a\n /* b */\n c */\nx"), "cccC");
    assert_eq!(rust_shape("/* a /* b */ c */ x"), "C");
}

#[test]
fn doc_comments_are_comments() {
    assert_eq!(
        rust_shape("/// doc\n//! inner\n/** block */\n\nfn a() {}"),
        "cccbC"
    );
}

#[test]
fn code_with_a_trailing_comment_is_code() {
    assert_eq!(rust_shape("x(); // trailing"), "C");
}

#[test]
fn multi_line_string_continuation_lines_are_code() {
    assert_eq!(rust_shape("let s = \"a\n\n// not a comment\";"), "CCC");
}

#[test]
fn cfg_test_module_block() {
    let text = "fn a() {}\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn t() {}\n}\nfn b() {}";
    assert_eq!(rust_shape(text), "CTTTTTC");
}

#[test]
fn cfg_test_module_declaration_is_one_line() {
    assert_eq!(
        rust_shape("fn a() {}\n#[cfg(test)] mod tests;\nfn b() {}"),
        "CTC"
    );
}

#[test]
fn cfg_all_test_is_a_test_item() {
    assert_eq!(
        rust_shape("#[cfg(all(test, unix))]\nfn t() {}\nfn a() {}"),
        "TTC"
    );
}

#[test]
fn cfg_testing_feature_is_not_a_test_item() {
    assert_eq!(rust_shape("#[cfg(testing)]\nfn t() {}"), "CC");
}

#[test]
fn file_level_cfg_test_makes_the_file_test_only() {
    let fc = count_text("src/x.rs", "#![cfg(test)]\n\nfn a() {}\n", &BTreeSet::new());
    assert!(fc.testfile);
    assert_eq!((fc.code, fc.test), (0, 2));
    assert_eq!(fc.kinds(), vec![(Kind::Test, 2)]);
}

#[test]
fn a_tests_directory_component_makes_the_file_test_only() {
    let fc = count_text("src/ir/tests/lower.rs", "fn a() {}\n", &BTreeSet::new());
    assert_eq!(fc.kinds(), vec![(Kind::Test, 1)]);
    let fc = count_text("tests/runtime/rt_x.rs", "fn a() {}\n", &BTreeSet::new());
    assert_eq!(fc.kinds(), vec![(Kind::Test, 1)]);
}

#[test]
fn a_mixed_file_has_code_and_inline_kinds() {
    let text = "fn a() {}\nfn b() {}\n#[cfg(test)]\nmod tests {\n}\n";
    let fc = count_text("src/x.rs", text, &BTreeSet::new());
    assert!(!fc.testfile);
    assert_eq!(fc.kinds(), vec![(Kind::Code, 2), (Kind::Inline, 3)]);
}

#[test]
fn a_declared_test_module_file_is_test_only() {
    let roots: BTreeSet<String> = ["src/ir/verify/cases.rs".to_string()].into();
    let fc = count_text("src/ir/verify/cases.rs", "fn a() {}\n", &roots);
    assert!(fc.testfile);
    let roots: BTreeSet<String> = ["src/ir/verify/cases".to_string()].into();
    assert!(count_text("src/ir/verify/cases/more.rs", "fn a() {}\n", &roots).testfile);
    assert!(!count_text("src/ir/verify/casesx.rs", "fn a() {}\n", &roots).testfile);
}

#[test]
fn test_module_declarations_are_found() {
    let text = "#[cfg(test)]\nmod tests;\n#[cfg(test)]\n#[path = \"verify_cases.rs\"]\n#[allow(dead_code)]\npub(crate) mod cases;\n#[cfg(test)]\nmod inline {\n}\n";
    assert_eq!(
        test_mod_decls(text),
        vec![
            ("tests".to_string(), None),
            ("cases".to_string(), Some("verify_cases.rs".to_string())),
        ]
    );
}

#[test]
fn mfb_comment_lines() {
    assert_eq!(
        mfb_shape("' a comment\nREM x\nREM\nREMAINDER = 1\n\nPRINT 1"),
        "cccCbC"
    );
}

#[test]
fn mfb_doc_block_is_comment() {
    assert_eq!(
        mfb_shape("DOC INTERNAL\nany text\nEND DOC\nPRINT 1\nDOCUMENT = 1"),
        "cccCC"
    );
}

#[test]
fn mfb_testing_block_is_test() {
    assert_eq!(
        mfb_shape("PRINT 1\nTESTING\n' c\nPRINT 2\nEND TESTING\nPRINT 3"),
        "CTcTTC"
    );
}

#[test]
fn mfb_test_file_is_test_only() {
    let fc = count_text(
        "packages/zip/src/test_x.mfb",
        "PRINT 1\n' c\n",
        &BTreeSet::new(),
    );
    assert_eq!(fc.kinds(), vec![(Kind::Test, 1)]);
}

#[test]
fn generated_banner_in_the_first_three_lines() {
    assert!(is_generated(
        "fn a() {}\n\n// GENERATED by tools/x — do not edit\n"
    ));
    assert!(!is_generated("fn a() {}\n\n\n// GENERATED by tools/x\n"));
    assert!(is_generated("' GENERATED file\n"));
    assert!(!is_generated(
        "//! Helpers for the generated documentation\n"
    ));
    assert!(!is_generated("let GENERATED = 1;\n"));
    let fc = count_text("src/x.rs", "// GENERATED\n", &BTreeSet::new());
    assert!(fc.generated);
    assert!(fc.kinds().is_empty());
}

#[test]
fn extent_const_table_opened_with_a_slice() {
    let text =
        "#[cfg(test)]\nconst T: &[(u8, u8)] = &[\n    (1, 2),\n    (3, 4),\n];\nfn after() {}";
    assert_eq!(rust_shape(text), "TTTTTC");
}

#[test]
fn extent_use_item() {
    assert_eq!(rust_shape("#[cfg(test)]\nuse std::fmt;\nfn a() {}"), "TTC");
}

#[test]
fn extent_enum_variant() {
    let text = "enum E {\n    A,\n    #[cfg(test)]\n    B(u8),\n    C,\n}";
    assert_eq!(rust_shape(text), "CCTTCC");
}

#[test]
fn extent_last_struct_field_without_a_comma() {
    let text = "struct S {\n    a: u8,\n    #[cfg(test)]\n    b: u8\n}";
    assert_eq!(rust_shape(text), "CCTTC");
}

#[test]
fn extent_match_arm_with_a_block() {
    let text = "match x {\n    #[cfg(test)]\n    1 => {\n        a();\n    }\n    _ => {}\n}";
    assert_eq!(rust_shape(text), "CTTTTCC");
}

#[test]
fn extent_match_arm_with_an_expression() {
    let text = "match x {\n    #[cfg(test)]\n    1 => a(),\n    _ => b(),\n}";
    assert_eq!(rust_shape(text), "CTTCC");
}

#[test]
fn extent_bare_block() {
    let text = "fn f() {\n    #[cfg(test)]\n    {\n        a();\n    }\n    b();\n}";
    assert_eq!(rust_shape(text), "CTTTTCC");
}

#[test]
fn extent_stacked_attributes() {
    let text = "#[cfg(test)]\n#[allow(dead_code)]\nfn helper() {\n}\nfn a() {}";
    assert_eq!(rust_shape(text), "TTTTC");
}
