//! Manifest shapes at the edges: a `project.json` that cannot be read or is
//! nested too deeply, locator fields of the wrong JSON type, and the closure
//! editor over `packages` arrays it cannot edit.

use super::*;
use crate::manifest::json_edit::project_json_with_closure_fields;

#[test]
fn a_project_json_that_cannot_be_read_or_nests_too_deeply_is_refused() {
    let dir = TempDir::new().unwrap();
    let path = dir.path().join("project.json");
    fs::create_dir_all(&path).unwrap();
    assert!(validate_project_manifest(&path).is_err());
    fs::remove_dir(&path).unwrap();
    fs::write(&path, format!("{}{}", "[".repeat(4096), "]".repeat(4096))).unwrap();
    assert!(validate_project_manifest(&path).is_err());
}

#[test]
fn locator_fields_of_the_wrong_type_are_each_named() {
    let message = |locator: &str| only_finding(&format!("{{ \"foo\": [ {locator} ] }}")).message;
    for (locator, wanted) in [
        (
            r#"{ "os": 7, "type": "system", "source": "libfoo.so" }"#,
            "field `os` must be a string.",
        ),
        (
            r#"{ "os": "  ", "type": "system", "source": "libfoo.so" }"#,
            "field `os` must not be blank.",
        ),
        (
            r#"{ "os": "linux", "arch": 7, "type": "system", "source": "libfoo.so" }"#,
            "field `arch` must be a string when present.",
        ),
        (
            r#"{ "os": "linux", "libc": 7, "type": "system", "source": "libfoo.so" }"#,
            "field `libc` must be a string when present.",
        ),
        (
            r#"{ "os": "linux", "type": 7, "source": "libfoo.so" }"#,
            "field `type` must be a string when present.",
        ),
        (
            r#"{ "os": "linux", "type": "system", "source": 7 }"#,
            "field `source` must be a string.",
        ),
    ] {
        let found = message(locator);
        assert!(found.ends_with(wanted), "{locator}: {found}");
    }
}

#[test]
fn the_closure_editor_refuses_a_packages_array_it_cannot_edit() {
    assert_eq!(
        project_json_with_closure_fields("{\"name\": \"app\"}", "ada#x", true, &[]),
        Err("could not locate project.json `packages` array".to_string())
    );
    // Entries that are no objects hold no ident.
    assert_eq!(
        project_json_with_closure_fields("{\"packages\": [1, 2]}", "ada#x", true, &[]),
        Err("project.json does not declare package `ada#x`".to_string())
    );
    assert_eq!(
        project_json_with_closure_fields("{\"packages\": [{\"ident\": \"x\"]", "x", true, &[]),
        Err("malformed project.json `packages` entry".to_string())
    );
}

#[test]
fn the_closure_editor_replaces_a_required_by_of_any_shape() {
    let required = ["ada#user".to_string()];
    for (old, kept) in [
        ("\"requiredBy\": {\"bad\": 1}", "{\"bad\": 1}"),
        ("\"requiredBy\": \"bad\"", "\"bad\""),
        ("\"requiredBy\": [\"ada#old\"]", "ada#old"),
    ] {
        let contents =
            format!("{{\"packages\": [{{\"ident\": \"ada#x\", \"direct\": true, {old}}}]}}");
        let edited = project_json_with_closure_fields(&contents, "ada#x", true, &required).unwrap();
        assert!(
            edited.contains("\"requiredBy\": [\"ada#user\"]"),
            "{edited}"
        );
        assert!(!edited.contains(kept), "{edited}");
    }
}
