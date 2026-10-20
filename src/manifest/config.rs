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
//!
//! An optional, untracked `project-local.json` beside `project.json` overrides
//! and extends the `config` section for one machine (plan-167-C,
//! [`load_project_config`]).

use crate::json::lossless::{parse_lossless, LosslessMember, LosslessValue};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The config name selected when the build names none, and the one every
/// `config` section must declare.
pub(crate) const RELEASE: &str = "release";

/// The per-machine file that overrides and extends `config` (plan-167-C).
pub(crate) const PROJECT_LOCAL_JSON: &str = "project-local.json";

/// Which file a config entry or diagnostic comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConfigFile {
    Project,
    Local,
}

/// The validated `config` section.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProjectConfig {
    /// The line of the `config` member, or `None` when the section is absent.
    pub(crate) line: Option<u32>,
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

    /// `PROJECT_JSON_CONFIG_UNKNOWN` for a selected `name` no config declares,
    /// at the `config` line (line 1 when the section is absent), listing the
    /// declared names.
    pub(crate) fn unknown_name(&self, name: &str) -> ConfigDiagnostic {
        let mut declared: Vec<&str> = self.configs.iter().map(|c| c.name.as_str()).collect();
        declared.sort_unstable();
        ConfigDiagnostic::new(
            "PROJECT_JSON_CONFIG_UNKNOWN",
            format!(
                "No config named `{name}`. Declared: {}.",
                declared.join(", ")
            ),
            self.line.unwrap_or(1),
            if self.line.is_some() { "config" } else { "" },
        )
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
    /// The 1-based line of the config's name (1 for the implicit `release`).
    pub(crate) line: u32,
    pub(crate) values: Vec<ConfigEntry>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ConfigEntry {
    pub(crate) key: String,
    /// The file the entry was declared in, and the 1-based line of its key.
    pub(crate) file: ConfigFile,
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
    /// The file at fault, and the 1-based line of the key at fault in it.
    pub(crate) file: ConfigFile,
    pub(crate) line: u32,
    /// The key at fault, which the renderer underlines on `line`.
    pub(crate) key: String,
}

impl ConfigDiagnostic {
    /// A diagnostic against `project.json`; [`parse_local_config`] re-tags its own.
    fn new(rule: &'static str, detail: String, line: u32, key: &str) -> Self {
        Self {
            rule,
            detail,
            file: ConfigFile::Project,
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

    let mut config = check_named_configs(named, true, &mut diagnostics);
    config.line = Some(config_member.line);
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
        line: None,
        configs: vec![NamedConfig {
            name: RELEASE.to_string(),
            line: 1,
            values: Vec::new(),
        }],
    }
}

/// The members of `config`: names, shapes, keys and values (§5.1–§5.3). A
/// config or key that fails a check is left out of the model.
/// `legacy_moved_option` turns a `stdinLogCap` member into the actionable
/// `PROJECT_JSON_CONFIG_MOVED_OPTION`; only `project.json` ever held one.
fn check_named_configs(
    named: &[LosslessMember],
    legacy_moved_option: bool,
    diagnostics: &mut Vec<ConfigDiagnostic>,
) -> ProjectConfig {
    let mut configs = Vec::new();
    let mut seen: Vec<&LosslessMember> = Vec::new();
    for member in named {
        if legacy_moved_option && member.key == "stdinLogCap" {
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
            line: member.line,
            values: check_entries(&member.key, entries, diagnostics),
        });
    }
    ProjectConfig {
        line: None,
        configs,
    }
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
                file: ConfigFile::Project,
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
                    let mut diagnostic = ConfigDiagnostic::new(
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
                    );
                    diagnostic.file = entry.file;
                    diagnostics.push(diagnostic);
                }
                Some(_) => {}
                None => first_seen.push((&entry.key, &named.name, found)),
            }
        }
    }
}

/// Validate a `project-local.json` text: a root object whose only member is an
/// optional `config`, checked with the `project.json` rules except that
/// `release` is not required (the file is partial). Returns its named configs,
/// every entry and diagnostic tagged [`ConfigFile::Local`].
fn parse_local_config(text: &str) -> Result<Vec<NamedConfig>, Vec<ConfigDiagnostic>> {
    let local = |mut diagnostics: Vec<ConfigDiagnostic>| {
        for diagnostic in &mut diagnostics {
            diagnostic.file = ConfigFile::Local;
        }
        diagnostics
    };
    let root = parse_lossless(text).map_err(|err| {
        local(vec![ConfigDiagnostic::new(
            "PROJECT_JSON_PARSE_FAILED",
            err.message,
            err.line,
            "",
        )])
    })?;
    let LosslessValue::Object(members) = root else {
        return Err(local(vec![ConfigDiagnostic::new(
            "PROJECT_LOCAL_JSON_FIELD",
            format!(
                "The top level of {PROJECT_LOCAL_JSON} must be an object holding only `config`."
            ),
            1,
            "",
        )]));
    };
    let mut diagnostics = Vec::new();
    let mut config_member: Option<&LosslessMember> = None;
    for member in &members {
        if member.key != "config" {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_LOCAL_JSON_FIELD",
                format!(
                    "Field `{}` is not allowed in {PROJECT_LOCAL_JSON}; it may only override `config`.",
                    member.key
                ),
                member.line,
                &member.key,
            ));
        } else if let Some(first) = config_member {
            diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_DUPLICATE",
                format!("Field `config` is already declared on line {}.", first.line),
                member.line,
                "config",
            ));
        } else {
            config_member = Some(member);
        }
    }
    let mut configs = Vec::new();
    if let Some(member) = config_member {
        match &member.value {
            LosslessValue::Object(named) => {
                configs = check_named_configs(named, false, &mut diagnostics).configs;
            }
            _ => diagnostics.push(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_SHAPE",
                "Field `config` must be an object of named configs.".to_string(),
                member.line,
                "config",
            )),
        }
    }
    if !diagnostics.is_empty() {
        return Err(local(diagnostics));
    }
    for entry in configs
        .iter_mut()
        .flat_map(|config| config.values.iter_mut())
    {
        entry.file = ConfigFile::Local;
    }
    Ok(configs)
}

/// Merge `project-local.json`'s configs over `project.json`'s (plan-167-C §4).
///
/// A local key replaces the same key of the same config and must keep its
/// type (`PROJECT_LOCAL_JSON_TYPE_OVERRIDE`); a new key or a new config is
/// added. A local key or config name that matches a `project.json` one only
/// when case is ignored is `PROJECT_JSON_CONFIG_DUPLICATE`: overriding is by
/// exact name, so it would otherwise declare a second spelling. The cross-config
/// type rule then runs over the merged result.
fn merge_local(
    mut config: ProjectConfig,
    local: Vec<NamedConfig>,
) -> Result<ProjectConfig, Vec<ConfigDiagnostic>> {
    let mut diagnostics = Vec::new();
    let mut tagged = |mut diagnostic: ConfigDiagnostic| {
        diagnostic.file = ConfigFile::Local;
        diagnostics.push(diagnostic);
    };
    for local_config in local {
        if let Some(near) = config.configs.iter().find(|c| {
            c.name != local_config.name && c.name.eq_ignore_ascii_case(&local_config.name)
        }) {
            tagged(ConfigDiagnostic::new(
                "PROJECT_JSON_CONFIG_DUPLICATE",
                format!(
                    "Config `{}` differs only in case from config `{}` in project.json.",
                    local_config.name, near.name
                ),
                local_config.line,
                &local_config.name,
            ));
            continue;
        }
        let Some(base) = config
            .configs
            .iter_mut()
            .find(|c| c.name == local_config.name)
        else {
            config.configs.push(local_config);
            continue;
        };
        for entry in local_config.values {
            let existing = base
                .values
                .iter()
                .position(|e| e.key.eq_ignore_ascii_case(&entry.key));
            match existing {
                None => base.values.push(entry),
                Some(index) if base.values[index].key != entry.key => {
                    let near = &base.values[index];
                    tagged(ConfigDiagnostic::new(
                        "PROJECT_JSON_CONFIG_DUPLICATE",
                        format!(
                            "Key `{}` in config `{}` differs only in case from `{}`, declared in project.json on line {}.",
                            entry.key, base.name, near.key, near.line
                        ),
                        entry.line,
                        &entry.key,
                    ));
                }
                Some(index) => {
                    let declared = &base.values[index];
                    let (was, now) = (declared.value.config_type(), entry.value.config_type());
                    if was == now {
                        base.values[index] = entry;
                    } else {
                        tagged(ConfigDiagnostic::new(
                            "PROJECT_LOCAL_JSON_TYPE_OVERRIDE",
                            format!(
                                "Key `{}` in config `{}` is {} in project.json (line {}); {PROJECT_LOCAL_JSON} cannot override it with {} {}.",
                                entry.key,
                                base.name,
                                was.name(),
                                declared.line,
                                if now == ConfigType::Integer { "an" } else { "a" },
                                now.name()
                            ),
                            entry.line,
                            &entry.key,
                        ));
                    }
                }
            }
        }
    }
    check_type_consistency(&config, &mut diagnostics);
    finish(config, diagnostics)
}

/// The project's configs for a build: `project.json`'s `config` section, with
/// `project-local.json` merged over it when that file exists in `project_dir`
/// (plan-167-C §3.2). Each diagnostic names the file it belongs to.
///
/// Only the invoked project's build calls this. `validate_project_manifest`
/// checks `project.json` alone, so `mfb fmt`/`mfb doc` never read the local
/// file, and a source dependency's local file is never read.
pub(crate) fn load_project_config(
    project_dir: &Path,
    manifest_text: &str,
) -> Result<ProjectConfig, Vec<ConfigDiagnostic>> {
    let config = parse_config(manifest_text)?;
    let local_path = project_dir.join(PROJECT_LOCAL_JSON);
    if !local_path.exists() {
        return Ok(config);
    }
    let local_text = std::fs::read_to_string(&local_path).map_err(|err| {
        let mut diagnostic =
            ConfigDiagnostic::new("PROJECT_JSON_READ_FAILED", err.to_string(), 1, "");
        diagnostic.file = ConfigFile::Local;
        vec![diagnostic]
    })?;
    merge_local(config, parse_local_config(&local_text)?)
}

#[cfg(test)]
mod tests;
