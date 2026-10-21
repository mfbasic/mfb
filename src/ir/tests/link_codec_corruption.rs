//! The `.mfp` LINK-table codec (`ir::binary::link_codec`) on a stateful,
//! `BIND IN`-carrying link function, and on bytes a crafted package corrupts.
//!
//! A stateful native function's `STATE` record and `BIND STATE` marshalling
//! ride an append-only trailer, and a `BIND IN` field may carry an integer
//! literal; both must round-trip exactly. The decoder parses the integers the
//! wire carries as text — a `CONST` pin, a `BIND IN` literal, a link
//! expression's integer — and a non-integer there is a decode error naming
//! what was malformed, never a silent zero handed to the C callee.

use crate::ir::{decode_binary_repr, encode_binary_repr, IrProject};

const CONST_VALUE: i64 = 987_654_321;
const BIND_LITERAL: i64 = 123_456_789;
const EXPR_INT: i64 = 555_444_333;

/// The variant corpus with its link function made stateful and given a
/// `BIND IN` literal, a distinctive `CONST` and a distinctive expression int.
fn project() -> IrProject {
    let mut project = crate::ir::variant_corpus_tests::variant_corpus();
    let link = project
        .link_functions
        .first_mut()
        .expect("the corpus carries a link function");
    link.consts = vec![("flags".to_string(), CONST_VALUE)];
    link.bind_in = vec![crate::ir::IrBindIn {
        slot: "db".to_string(),
        fields: vec![
            crate::ir::IrBindInField {
                name: "mode".to_string(),
                param: None,
                literal: Some(BIND_LITERAL),
            },
            crate::ir::IrBindInField {
                name: "path".to_string(),
                param: Some("path".to_string()),
                literal: None,
            },
        ],
    }];
    link.return_state_type = Some(crate::types::ParameterType::named("Cursor"));
    link.bind_state = Some("db".to_string());
    link.result_length = Some(crate::ir::IrLinkExpr::Int(EXPR_INT));
    project
}

#[test]
fn a_stateful_bind_in_link_function_round_trips() {
    let project = project();
    let bytes = encode_binary_repr(&project);
    let decoded = decode_binary_repr(&bytes).expect("decode");
    assert_eq!(project.to_json(), decoded.to_json());
    let link = &decoded.link_functions[0];
    assert_eq!(
        link.return_state_type
            .as_ref()
            .map(|t| t.name().into_owned()),
        Some("Cursor".to_string())
    );
    assert_eq!(link.bind_state.as_deref(), Some("db"));
    assert_eq!(link.bind_in[0].fields[0].literal, Some(BIND_LITERAL));
}

/// Replace the one occurrence of `value`'s decimal text in the encoding with
/// the same text whose last digit is a letter, keeping every length prefix
/// valid.
fn corrupted(value: i64) -> Vec<u8> {
    let mut bytes = encode_binary_repr(&project());
    let needle = value.to_string().into_bytes();
    let at = bytes
        .windows(needle.len())
        .position(|window| window == needle.as_slice())
        .expect("the value is encoded as text");
    assert!(
        bytes[at + 1..]
            .windows(needle.len())
            .all(|window| window != needle.as_slice()),
        "the value must occur once for the corruption to be targeted"
    );
    bytes[at + needle.len() - 1] = b'x';
    bytes
}

#[test]
fn a_non_integer_const_pin_is_a_decode_error() {
    let error = decode_binary_repr(&corrupted(CONST_VALUE))
        .err()
        .expect("a corrupted CONST must not decode");
    assert!(error.contains("invalid LINK const value"), "{error}");
}

#[test]
fn a_non_integer_bind_in_literal_is_a_decode_error() {
    let error = decode_binary_repr(&corrupted(BIND_LITERAL))
        .err()
        .expect("a corrupted BIND IN literal must not decode");
    assert!(error.contains("invalid BIND IN literal"), "{error}");
}

#[test]
fn a_non_integer_link_expression_int_is_a_decode_error() {
    let error = decode_binary_repr(&corrupted(EXPR_INT))
        .err()
        .expect("a corrupted link expression must not decode");
    assert!(error.contains("invalid LINK expr int"), "{error}");
}
