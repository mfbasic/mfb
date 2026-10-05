//! Named build configurations: the `config` section of `project.json`
//! (plan-167-A §5).
//!
//! ```json
//! "config": {
//!   "release": { "apiUrl": "https://api.example.com", "retries": 3, "timeout": 2.5 },
//!   "dev":     { "apiUrl": "http://localhost:8080" }
//! }
//! ```
//!
//! Each named config is a flat map of keys to String, Integer or Float values.
//! The section is read with the lossless JSON reader, not `tinyjson`, because
//! the rules below need what `tinyjson` drops: a number's spelling decides
//! Integer vs Float, a duplicate key is an error rather than a silent overwrite,
//! and every diagnostic names the line of the key at fault.

use crate::json::lossless::{parse_lossless, LosslessMember, LosslessValue};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The config name selected when the build names none, and the one every
/// `config` section must declare.
pub(crate) const RELEASE: &str = "release";

/// The validated `config` section.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectConfig {
    /// The named configs in source order. Always holds `release`: an absent
    /// `config` section is one implicit, empty `release` (Open Decision D6).
    pub(crate) configs: Vec<NamedConfig>,
}

impl ProjectConfig {
    /// The named config `name`, as the values `os::getConfigOr` calls fold
    /// from, or `None` when no config has that name. Names match exactly.
    pub(crate) fn select(&self, name: &str, project_dir: PathBuf) -> Option<SelectedConfig> {
        let named = self.configs.iter().find(|config| config.name == name)?;
        Some(SelectedConfig {
            name: named.name.clone(),
            values: named
                .values
                .iter()
                .map(|entry| (entry.key.clone(), entry.value.clone()))
                .collect(),
            project_dir,
        })
    }
}

/// The one named config a build folds `os::getConfigOr` calls from
/// (plan-167-B). Carried on `IrProject` like `max_buffer_bytes`: it is the
/// consuming executable's setting and is never encoded into a `.mfp`.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SelectedConfig {
    pub(crate) name: String,
    pub(crate) values: BTreeMap<String, ConfigValue>,
    /// The directory the project's source paths are relative to, so a fold
    /// diagnostic can name the call's file.
    pub(crate) project_dir: PathBuf,
}

impl Default for SelectedConfig {
    /// An empty `release`: every call yields its default. The selection for an
    /// IR that no build configured — decoded package IR, which never folds, and
    /// unit-test projects.
    fn default() -> Self {
        Self {
            name: RELEASE.to_string(),
            values: BTreeMap::new(),
            project_dir: PathBuf::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct NamedConfig {
    pub(crate) name: String,
    pub(crate) values: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConfigEntry {
    pub(crate) key: String,
    /// The 1-based line of the key in its file.
    pub(crate) line: u32,
    pub(crate) value: ConfigValue,
}

/// A config value, typed by its JSON spelling (§5.2).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ConfigValue {
    String(String),
    Integer(i64),
    /// The value, and the lexeme it was written as. The lexeme is what a
    /// folded constant carries, so the program sees exactly the written number.
    Float(f64, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigType {
    String,
    Integer,
    Float,
}

impl ConfigType {
    pub(crate) fn name(self) -> &'static str {
        match self {
            ConfigType::String => "String",
            ConfigType::Integer => "Integer",
            ConfigType::Float => "Float",
        }
    }
}

impl ConfigValue {
    pub(crate) fn config_type(&self) -> ConfigType {
        match self {
            ConfigValue::String(_) => ConfigType::String,
            ConfigValue::Integer(_) => ConfigType::Integer,
            ConfigValue::Float(..) => ConfigType::Float,
        }
    }
}

/// One rejected part of a `config` section.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConfigDiagnostic {
    /// The `src/rules/table.rs` rule name.
    pub(crate) rule: &'static str,
    pub(crate) detail: String,
    /// The 1-based line of the key at fault.
    pub(crate) line: u32,
    /// The key at fault, which the renderer underlines on `line`.
    pub(crate) key: String,
}

impl ConfigDiagnostic {
    fn new(rule: &'static str, detail: String, line: u32, key: &str) -> Self {
        Self {
            rule,
            detail,
            line,
            key: key.to_string(),
        }
    }
}

/// `^[A-Za-z][A-Za-z0-9_.-]*$` (Open Decision D9). A dotted name is one name,
/// not a path.
fn is_valid_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    bytes.next().is_some_and(|b| b.is_ascii_alphabetic())
        && bytes.all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'))
}

const NAME_PATTERN: &str = "[A-Za-z][A-Za-z0-9_.-]*";

/// What a rejected value is, for the 2-200-0023 detail.
fn describe(value: &LosslessValue) -> &'static str {
    match value {
        LosslessValue::Null => "`null`",
        LosslessValue::Bool(true) => "`true`",
        LosslessValue::Bool(false) => "`false`",
        LosslessValue::Array(_) => "an array",
        LosslessValue::Object(_) => "an object",
        // Only reached for numbers and strings that fail §5.2's checks; the
        // caller supplies those descriptions.
        LosslessValue::Number(_) | LosslessValue::String(_) => "a value",
    }
}

/// Type one value per §5.2, or say why it has no config type.
fn config_value(value: &LosslessValue) -> Result<ConfigValue, &'static str> {
    match value {
        LosslessValue::String(text) => {
            if text.contains('\0') {
                Err("a String containing U+0000")
            } else {
                Ok(ConfigValue::String(text.clone()))
            }
        }
        LosslessValue::Number(lexeme) => {
            if lexeme.contains(['.', 'e', 'E']) {
                match lexeme.parse::<f64>() {
                    Ok(value) if value.is_finite() => Ok(ConfigValue::Float(value, lexeme.clone())),
                    _ => Err("a Float too large to represent"),
                }
            } else {
                lexeme
                    .parse::<i64>()
                    .map(ConfigValue::Integer)
                    .map_err(|_| "an Integer outside the 64-bit range")
            }
        }
        other => Err(describe(other)),
    }
}

/// Validate the `config` section of a `project.json` text and build its model
/// (§5.1–§5.4). Every problem is reported, not just the first.
pub(crate) fn parse_config(text: &str) -> Result<ProjectConfig, Vec<ConfigDiagnostic>> {
    let root = match parse_lossless(text) {
        Ok(root) => root,
        Err(err) => {
            return Err(vec![ConfigDiagnostic::new(
                "PROJECT_JSON_PARSE_FAILED",
                err.message,
                err.line,
                "",
            )])
        }
    };
    let LosslessValue::Object(members) = root else {
        // `validate_project_manifest` rejects a non-object root before this runs.
        return Ok(implicit_release());
    };

    let mut diagnostics = Vec::new();
    let mut config_member: Option<&LosslessMember> = None;
    for member in members.iter().filter(|m| m.key == "config") {
        match config_member {
            Some(first) => diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_DUPLICATE",
                format!("Field `config` is already declared on line {}.", first.line),
                member.line,
                "config",
            )),
            None => config_member = Some(member),
        }
    }
    let Some(config_member) = config_member else {
        return finish(implicit_release(), diagnostics);
    };
    let LosslessValue::Object(named) = &config_member.value else {
        diagnostics.push(ConfigDiagnostic::new(
            "PROJECT_JSON_CONFIG_SHAPE",
            "Field `config` must be an object of named configs.".to_string(),
            config_member.line,
            "config",
        ));
        return Err(diagnostics);
    };

    let config = check_named_configs(named, &mut diagnostics);
    let moved_option = named.iter().any(|m| m.key == "stdinLogCap");
    if !moved_option && !named.iter().any(|m| m.key == RELEASE) {
        diagnostics.push(ConfigDiagnostic::new(
            "PROJECT_JSON_CONFIG_RELEASE_MISSING",
            "Field `config` must contain a `release` config.".to_string(),
            config_member.line,
            "config",
        ));
    }
    check_type_consistency(&config, &mut diagnostics);
    finish(config, diagnostics)
}

fn finish(
    config: ProjectConfig,
    diagnostics: Vec<ConfigDiagnostic>,
) -> Result<ProjectConfig, Vec<ConfigDiagnostic>> {
    if diagnostics.is_empty() {
        Ok(config)
    } else {
        Err(diagnostics)
    }
}

fn implicit_release() -> ProjectConfig {
    ProjectConfig {
        configs: vec![NamedConfig {
            name: RELEASE.to_string(),
            values: Vec::new(),
        }],
    }
}

/// The members of `config`: names, shapes, keys and values (§5.1–§5.3). A
/// config or key that fails a check is left out of the model.
fn check_named_configs(
    named: &[LosslessMember],
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> ProjectConfig {
    let mut configs = Vec::new();
    let mut seen: Vec<&LosslessMember> = Vec::new();
    for member in named {
        if member.key == "stdinLogCap" {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_MOVED_OPTION",
                "Rename this `config` object to `options`; `config` now holds named build configurations."
                    .to_string(),
                member.line,
                &member.key,
            ));
            continue;
        }
        if !is_valid_name(&member.key) {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_NAME",
                format!("Config name `{}` must match `{NAME_PATTERN}`.", member.key),
                member.line,
                &member.key,
            ));
            continue;
        }
        if let Some(first) = seen
            .iter()
            .find(|s| s.key.eq_ignore_ascii_case(&member.key))
        {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_DUPLICATE",
                format!(
                    "Config `{}` is already declared on line {}.",
                    member.key, first.line
                ),
                member.line,
                &member.key,
            ));
            continue;
        }
        seen.push(member);
        let LosslessValue::Object(entries) = &member.value else {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_SHAPE",
                format!(
                    "Config `{}` must be an object of key/value pairs.",
                    member.key
                ),
                member.line,
                &member.key,
            ));
            continue;
        };
        configs.push(NamedConfig {
            name: member.key.clone(),
            values: check_entries(&member.key, entries, diagnostics),
        });
    }
    ProjectConfig { configs }
}

fn check_entries(
    config: &str,
    entries: &[LosslessMember],
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> Vec<ConfigEntry> {
    let mut values: Vec<ConfigEntry> = Vec::new();
    let mut seen: Vec<&LosslessMember> = Vec::new();
    for entry in entries {
        if !is_valid_name(&entry.key) {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_NAME",
                format!(
                    "Config `{config}` key `{}` must match `{NAME_PATTERN}`.",
                    entry.key
                ),
                entry.line,
                &entry.key,
            ));
            continue;
        }
        if let Some(first) = seen.iter().find(|s| s.key.eq_ignore_ascii_case(&entry.key)) {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_DUPLICATE",
                format!(
                    "Key `{}` in config `{config}` is already declared on line {}.",
                    entry.key, first.line
                ),
                entry.line,
                &entry.key,
            ));
            continue;
        }
        seen.push(entry);
        match config_value(&entry.value) {
            Ok(value) => values.push(ConfigEntry {
                key: entry.key.clone(),
                line: entry.line,
                value,
            }),
            Err(what) => diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_VALUE",
                format!(
                    "Config `{config}` key `{}` must be a String, Integer or Float; found {what}.",
                    entry.key
                ),
                entry.line,
                &entry.key,
            )),
        }
    }
    values
}

/// A key declared in several configs has one type in all of them (§5.4).
/// Each conflict is reported at the later declaration, against the first
/// config that declared the key.
pub(crate) fn check_type_consistency(
    config: &ProjectConfig,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) {
    let mut first_seen: Vec<(&str, &str, ConfigType)> = Vec::new();
    for named in &config.configs {
        for entry in &named.values {
            let found = entry.value.config_type();
            match first_seen.iter().find(|(key, ..)| *key == entry.key) {
                Some((_, first_config, first_type)) if *first_type != found => {
                    diagnostics.push(ConfigDiagnostic::new(
                        "PROJECT_JSON_CONFIG_TYPE_CONFLICT",
                        format!(
                            "Key `{}` is {} in config `{first_config}` but {} in config `{}`.",
                            entry.key,
                            first_type.name(),
                            found.name(),
                            named.name
                        ),
                        entry.line,
                        &entry.key,
                    ));
                }
                Some(_) => {}
                None => first_seen.push((&entry.key, &named.name, found)),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
