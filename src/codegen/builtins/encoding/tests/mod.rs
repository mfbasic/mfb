use super::*;
use crate::codegen::registry::{self, registry};

fn project(src: &str) -> crate::ast::AstProject {
    let file = crate::ast::parse_source(std::path::Path::new("main.mfb"), "main.mfb", src)
        .expect("parse source");
    crate::ast::AstProject {
        name: "test".to_string(),
        files: vec![file],
    }
}

#[test]
fn encoding_registered_on_the_clean_room_registry() {
    let pkg = registry()
        .resolve_package("encoding")
        .expect("encoding package");
    // 30 public members (2 overloaded + 28 non-overloaded), the last two being
    // `codepageDecode` (plan-123-A) and `codepageEncode` (plan-123-B).
    assert_eq!(pkg.functions().len(), 30);
}

#[test]
fn generic_dispatch_reaches_encoding() {
    assert!(registry().is_member("encoding.hexEncode"));
    assert!(!registry().is_member("encoding.nope"));
    // Fixed-return non-overloaded members have a static nominal return.
    assert_eq!(
        registry::call_return_type_typed("encoding.hexEncode")
            .map(|t| t.name().into_owned())
            .as_deref(),
        Some("String")
    );
}

#[test]
fn non_overloaded_members_rewrite_to_their_internal_symbol() {
    assert_eq!(
        registry::rewrite_target("encoding.hexEncode", &[]),
        Some("__encoding_hexEncode")
    );
    assert_eq!(
        registry::rewrite_target("encoding.varintDecode", &[]),
        Some("__encoding_varintDecode")
    );
    // The overloaded names carry no registry rewrite target: IR lowering leaves
    // the canonical name in place for the monomorphizer.
    assert_eq!(registry::rewrite_target("encoding.utf8Encode", &[]), None);
    assert_eq!(registry::rewrite_target("encoding.utf8Decode", &[]), None);
}

#[test]
fn resolve_call_types_the_members() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        registry::resolve_call("encoding.hexEncode", &s(&["List OF Byte"]), false),
        Some("String".to_string())
    );
    assert_eq!(
        registry::resolve_call("encoding.hexDecode", &s(&["String"]), false),
        Some("List OF Byte".to_string())
    );
    // utf8Encode default (pre-monomorph) return is List OF Byte — and it stays
    // that way now that BOTH return-type forms are registered (bug-530): the two
    // rows share a parameter list, and overload selection takes the first row
    // that unifies, so registering the `List OF Integer` form for the man page
    // did not move what the registry answers. The call's expected type still
    // picks the form, in the monomorphizer.
    assert_eq!(
        registry()
            .resolve_func("encoding.utf8Encode")
            .unwrap()
            .function
            .implementations()
            .len(),
        2
    );
    assert_eq!(
        registry::resolve_call("encoding.utf8Encode", &s(&["String"]), false),
        Some("List OF Byte".to_string())
    );
    // utf8Decode is a parameter overload: both element types resolve.
    assert_eq!(
        registry::resolve_call("encoding.utf8Decode", &s(&["List OF Byte"]), false),
        Some("String".to_string())
    );
    assert_eq!(
        registry::resolve_call("encoding.utf8Decode", &s(&["List OF Integer"]), false),
        Some("String".to_string())
    );
    // A wrong argument type resolves to nothing.
    assert_eq!(
        registry::resolve_call("encoding.utf8Encode", &s(&["Integer"]), false),
        None
    );
}

#[test]
fn arity_is_unary_across_members() {
    assert_eq!(registry().arity("encoding.hexEncode"), Some((1, 1)));
    assert_eq!(registry().arity("encoding.utf8Decode"), Some((1, 1)));
}

#[test]
fn diagnostic_metadata_via_registry() {
    // `utf8Decode`'s overloaded phrasing is a descriptor hint; every other
    // member's diagnostic equals the generic per-position render.
    assert_eq!(
        registry::expected_arguments("encoding.utf8Decode"),
        Some("List OF Byte or List OF Integer")
    );
    assert_eq!(
        registry::expected_arguments("encoding.hexEncode"),
        Some("List OF Byte")
    );
    // The overloaded member has no single positional signature; the per-position
    // render of a unary member is its one argument type.
    assert_eq!(
        crate::codegen::builtins::argument_types_typed("encoding.utf8Decode"),
        None
    );
    assert_eq!(
        crate::codegen::builtins::argument_types_typed("encoding.hexEncode"),
        Some(vec![crate::types::ParameterType::list_of(
            crate::types::ParameterType::Byte
        )])
    );
    assert_eq!(
        registry::call_param_names("encoding.utf8Encode"),
        Some(vec![vec!["value", "text"]])
    );
    assert_eq!(
        registry::call_param_names("encoding.punycodeDecode"),
        Some(vec![vec!["asciiDomain"]])
    );
    assert!(registry::expected_arguments("encoding.nope").is_none());
}

#[test]
fn reassembled_source_parses() {
    let source = registry().resolve_package("encoding").unwrap().get_mfb();
    assert!(crate::ast::parse_source_internal(
        std::path::Path::new(SOURCE_LABEL),
        SOURCE_DOC,
        &source,
    )
    .is_ok());
}

#[test]
fn augmented_project_injects_when_imported() {
    let ast = project("IMPORT encoding\nFUNC main AS Nothing\nEND FUNC\n");
    assert!(registry()
        .resolve_package("encoding")
        .unwrap()
        .is_imported_by(&crate::codegen::registry::ProjectView::of_ast(&ast)));
    assert_eq!(
        augmented_project(&ast).expect("augment").files.len(),
        ast.files.len() + 1
    );
}

#[test]
fn augmented_project_noop_without_import() {
    let ast = project("FUNC main AS Nothing\nEND FUNC\n");
    assert!(!registry()
        .resolve_package("encoding")
        .unwrap()
        .is_imported_by(&crate::codegen::registry::ProjectView::of_ast(&ast)));
    assert_eq!(
        augmented_project(&ast).expect("augment").files.len(),
        ast.files.len()
    );
}

/// bug-606: every member's declared errors, per overload, are exactly the codes
/// its source body can raise — a member is fallible when its body, or a helper
/// it calls, reaches `FAIL error(77050003, …)`. The Errors table on
/// `mfb man encoding <member>` is rendered from these lists, and the
/// fallibility verdict (`mfb spec language error-model` §8.6 rule 11) is read
/// off them, so an empty list on a raising member is a page claiming the call
/// never fails. Each raising member was probed at runtime and trapped
/// `77050003`.
#[test]
fn members_declare_the_errors_they_raise() {
    const INVALID_FORMAT: &str = "ErrInvalidFormat";
    let cases: &[(&str, &[&[&str]])] = &[
        (
            "encoding.percentDecode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.formUrlDecode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.base32Decode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.base64Decode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.base64UrlDecode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.hexDecode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.htmlUnescape",
            &[&["ErrInvalidFormat", "ErrEncoding", "ErrOutOfMemory"]],
        ),
        ("encoding.sleb128Decode", &[&[INVALID_FORMAT]]),
        ("encoding.uleb128Decode", &[&[INVALID_FORMAT]]),
        (
            "encoding.uleb128Encode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.utf16Decode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.utf32Decode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.utf8Decode",
            &[&[INVALID_FORMAT], &[INVALID_FORMAT]],
        ),
        ("encoding.varintDecode", &[&[INVALID_FORMAT]]),
        (
            "encoding.codepageDecode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.codepageEncode",
            &[&["ErrInvalidFormat", "ErrOutOfMemory"]],
        ),
        (
            "encoding.punycodeDecode",
            &[&["ErrInvalidFormat", "ErrEncoding", "ErrOutOfMemory"]],
        ),
        // No path in the body or its helpers reaches a `FAIL`; the ones that
        // build their result can run out of memory (bug-698).
        ("encoding.base32Encode", &[&[]]),
        ("encoding.base64Encode", &[&[]]),
        ("encoding.base64UrlEncode", &[&[]]),
        ("encoding.formUrlEncode", &[&["ErrOutOfMemory"]]),
        ("encoding.hexEncode", &[&[]]),
        ("encoding.htmlEscape", &[&[]]),
        ("encoding.percentEncode", &[&["ErrOutOfMemory"]]),
        ("encoding.punycodeEncode", &[&["ErrOutOfMemory"]]),
        ("encoding.sleb128Encode", &[&["ErrOutOfMemory"]]),
        ("encoding.utf16Encode", &[&["ErrOutOfMemory"]]),
        ("encoding.utf32Encode", &[&["ErrOutOfMemory"]]),
        ("encoding.utf8Encode", &[&[], &[]]),
        ("encoding.varintEncode", &[&["ErrOutOfMemory"]]),
    ];
    assert_eq!(
        cases.len(),
        registry()
            .resolve_package("encoding")
            .expect("encoding")
            .functions()
            .len(),
        "every public member is listed"
    );
    for (member, expected) in cases {
        let function = registry().resolve_func(member).expect(member).function;
        let declared: Vec<Vec<&str>> = function
            .implementations
            .iter()
            .map(|implementation| implementation.errors.clone())
            .collect();
        let expected: Vec<Vec<&str>> = expected.iter().map(|errors| errors.to_vec()).collect();
        assert_eq!(declared, expected, "{member} declared errors per overload");
    }
}
