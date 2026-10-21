//! `config` sections at the edges: text that is no JSON or no object, a
//! `false` value, a Float key typed two ways, a local file with two
//! `config` fields or a `config` that is no object, and a local file that
//! cannot be read.

use super::*;

#[test]
fn text_that_is_no_json_or_no_object() {
    assert_eq!(errors("{ \"config\": "), [("PROJECT_JSON_PARSE_FAILED", 1)]);
    // `validate_project_manifest` refuses a non-object root first; the config
    // reader alone takes it as having no config.
    assert_eq!(parse_config("[1]").expect("no config"), implicit_release());
}

#[test]
fn a_false_value_and_a_float_typed_two_ways_are_named() {
    let diagnostics = parse_config(&manifest("{\"release\": {\"on\": false}}")).unwrap_err();
    assert!(
        diagnostics[0].detail.ends_with("found `false`."),
        "{}",
        diagnostics[0].detail
    );
    let diagnostics = parse_config(&manifest(
        "{\"release\": {\"rate\": 1.5}, \"dev\": {\"rate\": \"x\"}}",
    ))
    .unwrap_err();
    assert_eq!(
        diagnostics[0].detail,
        "Key `rate` is Float in config `release` but String in config `dev`."
    );
}

#[test]
fn a_local_file_with_two_configs_or_a_config_that_is_no_object() {
    assert_eq!(
        local_errors("{\n  \"config\": {},\n  \"config\": {}\n}"),
        [("PROJECT_JSON_CONFIG_DUPLICATE", ConfigFile::Local, 3)]
    );
    assert_eq!(
        local_errors("{\n  \"config\": 5\n}"),
        [("PROJECT_JSON_CONFIG_SHAPE", ConfigFile::Local, 2)]
    );
}

#[test]
fn a_local_file_that_cannot_be_read_is_reported_against_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(PROJECT_LOCAL_JSON)).unwrap();
    let diagnostics = load_project_config(dir.path(), BASE).unwrap_err();
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (d.rule, d.file))
            .collect::<Vec<_>>(),
        [("PROJECT_JSON_READ_FAILED", ConfigFile::Local)]
    );
}
