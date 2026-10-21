use super::*;

#[test]
fn without_packages_missing_array_errors() {
    let err = project_json_without_packages("{}", &["x"]).unwrap_err();
    assert!(
        err.contains("could not locate project.json `packages` array"),
        "{err}"
    );
}

#[test]
fn updated_version_missing_array_errors() {
    let err = project_json_with_updated_version("{}", "x", "2.0.0", None).unwrap_err();
    assert!(
        err.contains("could not locate project.json `packages` array"),
        "{err}"
    );
}

#[test]
fn without_packages_malformed_entry_errors() {
    // The array `[ … ]` terminates, but the inner `{` object is never closed,
    // so `matching_json_delimiter` for the object returns None.
    let contents = "{\"packages\":[ { \"ident\": \"a\" ]";
    let err = project_json_without_packages(contents, &["a"]).unwrap_err();
    assert_eq!(err, "malformed project.json `packages` entry");
}

#[test]
fn updated_version_malformed_entry_errors() {
    let contents = "{\"packages\":[ { \"ident\": \"a\" ]";
    let err = project_json_with_updated_version(contents, "a", "2.0.0", None).unwrap_err();
    assert_eq!(err, "malformed project.json `packages` entry");
}

#[test]
fn package_name_is_the_fallback_ident_when_removing() {
    let contents = r#"{"packages":[{"name":"owner#pkg","version":"1.0.0"}]}"#;
    let updated = project_json_without_packages(contents, &["owner#pkg"]).unwrap();
    assert_eq!(updated, r#"{"packages":[]}"#);
}

#[test]
fn updating_a_dependency_without_a_version_reports_the_ident() {
    let contents = r#"{"packages":[{"ident":"owner#pkg"}]}"#;
    let error =
        project_json_with_updated_version(contents, "owner#pkg", "2.0.0", None).unwrap_err();
    assert!(error.contains("owner#pkg"));
    assert!(error.contains("no `version` field"));
}

#[test]
fn malformed_pin_fields_report_the_broken_component() {
    assert_eq!(
        rewrite_pin_field("{\"pin\":   ", false).unwrap_err(),
        "malformed pin value"
    );
    assert_eq!(
        rewrite_pin_field("{\"pin\":true", false).unwrap_err(),
        "malformed pin value"
    );
}

#[test]
fn find_json_punct_scans_over_quoted_escapes() {
    // A quoted string carrying an escaped quote precedes the target `:`, so
    // the in-string escape/quote branches all execute before the punct match.
    let contents = "\"a\\\"b\" :";
    let colon = find_json_punct(contents, 0, b':').unwrap();
    assert_eq!(&contents[colon..colon + 1], ":");
    // No target after the string -> None (whitespace-only tail).
    assert_eq!(find_json_punct("\"x\" ", 0, b':'), None);
}

#[test]
fn matching_delimiter_scans_over_escaped_string() {
    // The braced object contains a string with a backslash escape, exercising
    // the escaped-byte reset inside `matching_json_delimiter`.
    let contents = "{\"a\\\"\"}";
    let end = matching_json_delimiter(contents, 0, b'{', b'}').unwrap();
    assert_eq!(end, contents.len() - 1);
}
