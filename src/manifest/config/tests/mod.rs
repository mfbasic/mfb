use super::*;

mod edges;

fn manifest(config: &str) -> String {
    format!("{{\n  \"name\": \"demo\",\n  \"config\": {config}\n}}\n")
}

fn errors(text: &str) -> Vec<(&'static str, u32)> {
    parse_config(text)
        .expect_err("rejected")
        .into_iter()
        .map(|d| (d.rule, d.line))
        .collect()
}

fn value(config: &ProjectConfig, name: &str, key: &str) -> ConfigValue {
    config
        .configs
        .iter()
        .find(|c| c.name == name)
        .and_then(|c| c.values.iter().find(|e| e.key == key))
        .map(|e| e.value.clone())
        .expect("declared")
}

#[test]
fn absent_config_is_an_implicit_empty_release() {
    let config = parse_config("{\"name\": \"demo\"}").expect("valid");
    assert_eq!(config, implicit_release());
}

#[test]
fn valid_multi_config_builds_the_model() {
    let text = manifest(
        "{\n    \"release\": {\"apiUrl\": \"https://x\", \"retries\": 3, \"timeout\": 2.5},\n    \"dev\": {\"apiUrl\": \"http://localhost\", \"db.host\": \"h\"}\n  }",
    );
    let config = parse_config(&text).expect("valid");
    let names: Vec<_> = config.configs.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["release", "dev"]);
    assert_eq!(
        value(&config, "release", "retries"),
        ConfigValue::Integer(3)
    );
    assert_eq!(
        value(&config, "release", "timeout"),
        ConfigValue::Float(2.5, "2.5".to_string())
    );
    assert_eq!(
        value(&config, "dev", "db.host"),
        ConfigValue::String("h".to_string())
    );
    assert_eq!(config.configs[0].values[1].line, 4);
}

#[test]
fn integer_and_float_edges() {
    let ok = parse_config(&manifest(
        "{\"release\": {\"max\": 9223372036854775807, \"min\": -9223372036854775808, \"z\": -0, \"f\": 0.0, \"e\": 1e5}}",
    ))
    .expect("valid");
    assert_eq!(value(&ok, "release", "max"), ConfigValue::Integer(i64::MAX));
    assert_eq!(value(&ok, "release", "min"), ConfigValue::Integer(i64::MIN));
    assert_eq!(value(&ok, "release", "z"), ConfigValue::Integer(0));
    assert_eq!(
        value(&ok, "release", "f"),
        ConfigValue::Float(0.0, "0.0".to_string())
    );
    assert_eq!(
        value(&ok, "release", "e"),
        ConfigValue::Float(1e5, "1e5".to_string())
    );
    assert_eq!(
        errors(&manifest("{\"release\": {\"big\": 9223372036854775808}}")),
        [("PROJECT_JSON_CONFIG_VALUE", 3)]
    );
    assert_eq!(
        errors(&manifest("{\"release\": {\"inf\": 1e999}}")),
        [("PROJECT_JSON_CONFIG_VALUE", 3)]
    );
}

#[test]
fn moved_option_is_the_only_diagnostic_for_the_old_shape() {
    let text = "{\n  \"config\": {\n    \"stdinLogCap\": 4194304\n  }\n}";
    assert_eq!(errors(text), [("PROJECT_JSON_CONFIG_MOVED_OPTION", 3)]);
}

#[test]
fn shape_errors() {
    assert_eq!(errors(&manifest("[]")), [("PROJECT_JSON_CONFIG_SHAPE", 3)]);
    let text = "{\n  \"config\": {\n    \"release\": {},\n    \"dev\": 5\n  }\n}";
    assert_eq!(errors(text), [("PROJECT_JSON_CONFIG_SHAPE", 4)]);
}

#[test]
fn release_is_required() {
    let text = "{\n  \"config\": {\n    \"dev\": {}\n  }\n}";
    assert_eq!(errors(text), [("PROJECT_JSON_CONFIG_RELEASE_MISSING", 2)]);
    // Lookup is exact, so `Release` does not stand in for `release`.
    let text = "{\n  \"config\": {\n    \"Release\": {}\n  }\n}";
    assert_eq!(errors(text), [("PROJECT_JSON_CONFIG_RELEASE_MISSING", 2)]);
}

#[test]
fn name_pattern_applies_to_configs_and_keys() {
    let text = "{\n  \"config\": {\n    \"release\": {\"1st\": 1, \"ok_key.x-y\": 2},\n    \"bad name\": {}\n  }\n}";
    assert_eq!(
        errors(text),
        [
            ("PROJECT_JSON_CONFIG_NAME", 3),
            ("PROJECT_JSON_CONFIG_NAME", 4)
        ]
    );
    assert_eq!(
        errors(&manifest("{\"release\": {\"\": 1}}")),
        [("PROJECT_JSON_CONFIG_NAME", 3)]
    );
}

#[test]
fn duplicates_are_case_insensitive() {
    let text = "{\n  \"config\": {\n    \"release\": {\n      \"port\": 1,\n      \"Port\": 2\n    },\n    \"RELEASE\": {}\n  }\n}";
    let diagnostics = parse_config(text).expect_err("rejected");
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (d.rule, d.line))
            .collect::<Vec<_>>(),
        [
            ("PROJECT_JSON_CONFIG_DUPLICATE", 5),
            ("PROJECT_JSON_CONFIG_DUPLICATE", 7)
        ]
    );
    assert!(diagnostics[0].detail.contains("already declared on line 4"));
    // A repeated top-level `config` is ambiguous too.
    let text = "{\n  \"config\": {\"release\": {}},\n  \"config\": {\"release\": {}}\n}";
    assert_eq!(errors(text), [("PROJECT_JSON_CONFIG_DUPLICATE", 3)]);
}

#[test]
fn values_must_be_string_integer_or_float() {
    let text = "{\n  \"config\": {\n    \"release\": {\n      \"a\": true,\n      \"b\": null,\n      \"c\": [1],\n      \"d\": {},\n      \"e\": \"x\\u0000y\"\n    }\n  }\n}";
    let diagnostics = parse_config(text).expect_err("rejected");
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (d.rule, d.line))
            .collect::<Vec<_>>(),
        (4..=8)
            .map(|line| ("PROJECT_JSON_CONFIG_VALUE", line))
            .collect::<Vec<_>>()
    );
    assert!(diagnostics[0].detail.ends_with("found `true`."));
    assert!(diagnostics[4].detail.contains("U+0000"));
}

#[test]
fn a_key_has_one_type_across_configs() {
    let text = "{\n  \"config\": {\n    \"release\": {\"port\": 80, \"rate\": 1.5},\n    \"dev\": {\"port\": \"80\", \"rate\": 2.0}\n  }\n}";
    let diagnostics = parse_config(text).expect_err("rejected");
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (d.rule, d.line))
            .collect::<Vec<_>>(),
        [("PROJECT_JSON_CONFIG_TYPE_CONFLICT", 4)]
    );
    assert_eq!(
        diagnostics[0].detail,
        "Key `port` is Integer in config `release` but String in config `dev`."
    );
}

#[test]
fn config_keys_named_like_top_level_fields_get_their_own_line() {
    // `field_position` would put `name` on line 2; the lossless reader knows better.
    let text = "{\n  \"name\": \"demo\",\n  \"config\": {\n    \"release\": {\n      \"name\": true\n    }\n  }\n}";
    assert_eq!(errors(text), [("PROJECT_JSON_CONFIG_VALUE", 5)]);
}

const BASE: &str = "{\n  \"config\": {\n    \"release\": {\"apiUrl\": \"https://prod\", \"retries\": 3},\n    \"dev\": {\"apiUrl\": \"http://localhost\"}\n  }\n}";

/// Merge `local` over [`BASE`], as `load_project_config` does.
fn merged(local: &str) -> Result<ProjectConfig, Vec<ConfigDiagnostic>> {
    merge_local(
        parse_config(BASE).expect("base"),
        parse_local_config(local)?,
    )
}

fn local_errors(local: &str) -> Vec<(&'static str, ConfigFile, u32)> {
    merged(local)
        .expect_err("rejected")
        .into_iter()
        .map(|d| (d.rule, d.file, d.line))
        .collect()
}

#[test]
fn a_local_override_that_keeps_the_type_replaces_the_value() {
    let config = merged("{\"config\": {\"dev\": {\"apiUrl\": \"http://mine\"}}}").expect("ok");
    assert_eq!(
        value(&config, "dev", "apiUrl"),
        ConfigValue::String("http://mine".to_string())
    );
    assert_eq!(
        value(&config, "release", "apiUrl"),
        ConfigValue::String("https://prod".to_string())
    );
}

#[test]
fn a_local_override_that_changes_the_type_is_rejected_at_the_local_line() {
    let local = "{\n  \"config\": {\n    \"release\": {\n      \"retries\": \"3\"\n    }\n  }\n}";
    let diagnostics = merged(local).expect_err("rejected");
    assert_eq!(
        diagnostics
            .iter()
            .map(|d| (d.rule, d.file, d.line))
            .collect::<Vec<_>>(),
        [("PROJECT_LOCAL_JSON_TYPE_OVERRIDE", ConfigFile::Local, 4)]
    );
    assert!(diagnostics[0]
        .detail
        .contains("Integer in project.json (line 3)"));
}

#[test]
fn a_local_file_adds_keys_and_configs() {
    let config = merged(
        "{\"config\": {\"dev\": {\"apiKey\": \"secret\"}, \"mine\": {\"apiUrl\": \"http://me\"}}}",
    )
    .expect("ok");
    assert_eq!(
        value(&config, "dev", "apiKey"),
        ConfigValue::String("secret".to_string())
    );
    let mine = config
        .select("mine", PathBuf::from("p"))
        .expect("a local config is selectable");
    assert_eq!(
        mine.values.get("apiUrl"),
        Some(&ConfigValue::String("http://me".to_string()))
    );
}

#[test]
fn the_local_root_may_only_hold_config() {
    assert_eq!(
        local_errors("{\n  \"name\": \"x\",\n  \"config\": {}\n}"),
        [("PROJECT_LOCAL_JSON_FIELD", ConfigFile::Local, 2)]
    );
    assert_eq!(
        local_errors("[1]"),
        [("PROJECT_LOCAL_JSON_FIELD", ConfigFile::Local, 1)]
    );
}

#[test]
fn a_local_file_needs_no_release() {
    assert!(merged("{\"config\": {\"dev\": {\"retries\": 1}}}").is_ok());
    assert!(merged("{}").is_ok());
}

#[test]
fn local_duplicates_and_type_conflicts_are_reported_in_the_local_file() {
    assert_eq!(
        local_errors("{\n  \"config\": {\n    \"dev\": {\"a\": 1,\n      \"A\": 2}\n  }\n}"),
        [("PROJECT_JSON_CONFIG_DUPLICATE", ConfigFile::Local, 4)]
    );
    // `retries` is an Integer in `release`; a local `dev.retries` String
    // conflicts with it across configs, not as an override.
    assert_eq!(
        local_errors(
            "{\n  \"config\": {\n    \"dev\": {\n      \"retries\": \"many\"\n    }\n  }\n}"
        ),
        [("PROJECT_JSON_CONFIG_TYPE_CONFLICT", ConfigFile::Local, 4)]
    );
    // A spelling that differs from project.json's only in case would be a
    // second key, not an override.
    assert_eq!(
        local_errors("{\n  \"config\": {\n    \"dev\": {\n      \"ApiUrl\": \"x\"\n    }\n  }\n}"),
        [("PROJECT_JSON_CONFIG_DUPLICATE", ConfigFile::Local, 4)]
    );
    assert_eq!(
        local_errors("{\n  \"config\": {\n    \"Dev\": {}\n  }\n}"),
        [("PROJECT_JSON_CONFIG_DUPLICATE", ConfigFile::Local, 3)]
    );
}

#[test]
fn malformed_local_json_is_a_parse_error_in_the_local_file() {
    assert_eq!(
        local_errors("{\n  \"config\": {,}\n}"),
        [("PROJECT_JSON_PARSE_FAILED", ConfigFile::Local, 2)]
    );
}

#[test]
fn load_project_config_reads_the_local_file_only_when_present() {
    let dir = std::env::temp_dir().join(format!("mfb_cfg_load_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let _ = std::fs::remove_file(dir.join(PROJECT_LOCAL_JSON));
    let without = load_project_config(&dir, BASE).expect("no local file");
    assert_eq!(without, parse_config(BASE).expect("base"));
    std::fs::write(
        dir.join(PROJECT_LOCAL_JSON),
        "{\"config\": {\"dev\": {\"apiKey\": \"k\"}}}",
    )
    .expect("write");
    let with = load_project_config(&dir, BASE).expect("local file");
    assert_eq!(
        value(&with, "dev", "apiKey"),
        ConfigValue::String("k".to_string())
    );
    let _ = std::fs::remove_dir_all(&dir);
}
