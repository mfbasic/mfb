pub mod entry;
pub mod package;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tinyjson::JsonValue;

use crate::rules;

pub(crate) fn parse_project_json(
    contents: &str,
    project_path: &Path,
) -> Result<HashMap<String, JsonValue>, String> {
    let manifest: JsonValue = contents.parse().map_err(|err: tinyjson::JsonParseError| {
        format!("failed to parse '{}': {err}", project_path.display())
    })?;
    manifest
        .get::<HashMap<String, JsonValue>>()
        .cloned()
        .ok_or_else(|| format!("'{}' must contain a JSON object", project_path.display()))
}

pub(crate) fn validate_project_manifest(
    project_path: &Path,
) -> Result<HashMap<String, JsonValue>, ()> {
    if !project_path.exists() {
        rules::show_diagnostic(
            "PROJECT_JSON_MISSING",
            "Run `mfb init <location>` first or build from a directory that contains project.json.",
            project_path,
            1,
            1,
            1,
        );
        return Err(());
    }

    let contents = fs::read_to_string(project_path).map_err(|err| {
        rules::show_diagnostic(
            "PROJECT_JSON_READ_FAILED",
            &err.to_string(),
            project_path,
            1,
            1,
            1,
        );
    })?;

    let manifest: JsonValue = contents.parse().map_err(|err: tinyjson::JsonParseError| {
        let column = err.column().max(1);
        rules::show_diagnostic(
            "PROJECT_JSON_PARSE_FAILED",
            &err.to_string(),
            project_path,
            err.line(),
            column,
            column + 1,
        );
    })?;

    let Some(manifest) = manifest.get::<HashMap<String, JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_ROOT_TYPE",
            "The top-level JSON value must be an object with project fields.",
            project_path,
            1,
            1,
            1,
        );
        return Err(());
    };

    let mut valid = true;

    for field in ["name", "version", "mfb"] {
        if !validate_required_string(manifest, project_path, &contents, field) {
            valid = false;
        }
    }

    if !validate_sources(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_optional_string(manifest, project_path, &contents, "entry") {
        valid = false;
    }

    if !validate_optional_string(manifest, project_path, &contents, "author") {
        valid = false;
    }

    if !validate_optional_string(manifest, project_path, &contents, "url") {
        valid = false;
    }

    if !validate_kind(manifest, project_path, &contents) {
        valid = false;
    }

    if valid {
        Ok(manifest.clone())
    } else {
        Err(())
    }
}

fn validate_required_string(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    field: &str,
) -> bool {
    let Some(value) = manifest.get(field) else {
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            &format!("Required field `{field}` is missing."),
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
    };

    let (line, column) = field_position(contents, field);
    let Some(value) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!("Field `{field}` must be a string."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    };

    if value.trim().is_empty() {
        rules::show_diagnostic(
            "PROJECT_JSON_EMPTY_FIELD",
            &format!("Field `{field}` must contain a non-empty string."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    }

    true
}

fn validate_optional_string(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    field: &str,
) -> bool {
    let Some(value) = manifest.get(field) else {
        return true;
    };

    if value.get::<String>().is_some() {
        return true;
    }

    let (line, column) = field_position(contents, field);
    rules::show_diagnostic(
        "PROJECT_JSON_FIELD_TYPE",
        &format!("Field `{field}` must be a string when present."),
        project_path,
        line,
        column,
        column + field.len() + 2,
    );
    false
}

fn validate_sources(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("sources") else {
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            "Required field `sources` is missing.",
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
    };

    let (line, column) = field_position(contents, "sources");
    let Some(sources) = value.get::<Vec<JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `sources` must be an array.",
            project_path,
            line,
            column,
            column + "\"sources\"".len(),
        );
        return false;
    };

    if sources.is_empty() {
        rules::show_diagnostic(
            "PROJECT_JSON_EMPTY_SOURCES",
            "Add at least one source entry, for example `{ \"root\": \"src\" }`.",
            project_path,
            line,
            column,
            column + "\"sources\"".len(),
        );
        return false;
    }

    let mut valid = true;
    for (index, source) in sources.iter().enumerate() {
        let Some(source) = source.get::<HashMap<String, JsonValue>>() else {
            rules::show_diagnostic(
                "PROJECT_JSON_FIELD_TYPE",
                &format!("Source entry #{index} must be an object."),
                project_path,
                line,
                column,
                column + "\"sources\"".len(),
            );
            valid = false;
            continue;
        };

        if !validate_required_string(source, project_path, contents, "root") {
            valid = false;
        }
        if !validate_source_pattern_field(source, project_path, contents, index, "include") {
            valid = false;
        }
        if !validate_source_pattern_field(source, project_path, contents, index, "exclude") {
            valid = false;
        }
    }

    valid
}

fn validate_source_pattern_field(
    source: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
    index: usize,
    field: &str,
) -> bool {
    let Some(value) = source.get(field) else {
        return true;
    };
    let (line, column) = field_position(contents, field);
    let Some(patterns) = value.get::<Vec<JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!("Source entry #{index} field `{field}` must be an array of strings."),
            project_path,
            line,
            column,
            column + field.len() + 2,
        );
        return false;
    };
    if patterns
        .iter()
        .all(|pattern| pattern.get::<String>().is_some())
    {
        return true;
    }
    rules::show_diagnostic(
        "PROJECT_JSON_FIELD_TYPE",
        &format!("Source entry #{index} field `{field}` must be an array of strings."),
        project_path,
        line,
        column,
        column + field.len() + 2,
    );
    false
}

fn validate_kind(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("kind") else {
        let (line, column) = fallback_field_position(contents);
        rules::show_diagnostic(
            "PROJECT_JSON_REQUIRED_FIELD",
            "Required field `kind` is missing.",
            project_path,
            line,
            column,
            column + 1,
        );
        return false;
    };

    let (line, column) = field_position(contents, "kind");
    let Some(kind) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `kind` must be a string when present.",
            project_path,
            line,
            column,
            column + "\"kind\"".len(),
        );
        return false;
    };

    if !matches!(kind.as_str(), "executable" | "package") {
        rules::show_diagnostic(
            "PROJECT_JSON_UNKNOWN_KIND",
            "Expected `executable` or `package`; continuing validation.",
            project_path,
            line,
            column,
            column + "\"kind\"".len(),
        );
    }

    true
}

pub(crate) fn project_kind(manifest: &HashMap<String, JsonValue>) -> &str {
    manifest
        .get("kind")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .expect("validated project manifests must include a string `kind` field")
}

pub(crate) fn entry_point(manifest: &HashMap<String, JsonValue>) -> &str {
    manifest
        .get("entry")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        .unwrap_or("main")
}

pub(crate) fn validate_packages_array(manifest: &HashMap<String, JsonValue>) -> Result<(), String> {
    if manifest
        .get("packages")
        .is_some_and(|value| value.get::<Vec<JsonValue>>().is_none())
    {
        return Err("project.json field `packages` must be an array when present".to_string());
    }
    Ok(())
}

pub(crate) fn field_position(contents: &str, field: &str) -> (usize, usize) {
    let needle = format!("\"{field}\"");
    for (index, line) in contents.lines().enumerate() {
        if let Some(column) = line.find(&needle) {
            return (index + 1, column + 1);
        }
    }

    fallback_field_position(contents)
}

pub(crate) fn fallback_field_position(contents: &str) -> (usize, usize) {
    if contents.is_empty() {
        (1, 1)
    } else {
        (contents.lines().count().max(1), 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_manifest(contents: &str) -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("project.json");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        (dir, path)
    }

    const VALID: &str = "{\n  \"name\": \"demo\",\n  \"version\": \"1.0.0\",\n  \"mfb\": \"1.0\",\n  \"kind\": \"executable\",\n  \"entry\": \"main\",\n  \"author\": \"me\",\n  \"url\": \"https://x\",\n  \"sources\": [ { \"root\": \"src\", \"include\": [\"*.mfb\"], \"exclude\": [\"skip.mfb\"] } ]\n}\n";

    #[test]
    fn valid_manifest_parses() {
        let (_dir, path) = write_manifest(VALID);
        let manifest = validate_project_manifest(&path).expect("valid");
        assert_eq!(project_kind(&manifest), "executable");
        assert_eq!(entry_point(&manifest), "main");
        validate_packages_array(&manifest).expect("no packages ok");
    }

    #[test]
    fn missing_file_is_error() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nope.json");
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn unparseable_json_is_error() {
        let (_dir, path) = write_manifest("{ not json");
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn non_object_root_is_error() {
        let (_dir, path) = write_manifest("[1, 2, 3]");
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn missing_required_field_is_error() {
        let (_dir, path) =
            write_manifest("{\n  \"version\": \"1.0\",\n  \"mfb\": \"1.0\",\n  \"kind\": \"package\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}");
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn wrong_type_field_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": 5,\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn empty_string_field_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"   \",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn optional_field_wrong_type_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"entry\": 3,\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn sources_missing_is_error() {
        let (_dir, path) =
            write_manifest("{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\"\n}");
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn sources_wrong_type_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"sources\": \"src\"\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn sources_empty_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"sources\": []\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn source_entry_not_object_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"sources\": [ \"oops\" ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn source_pattern_wrong_type_is_error() {
        // include is not an array.
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"sources\": [ { \"root\": \"src\", \"include\": \"x\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn source_pattern_non_string_element_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"package\",\n  \"sources\": [ { \"root\": \"src\", \"exclude\": [1] } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn kind_missing_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn kind_wrong_type_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": 7,\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn unknown_kind_still_validates_ok() {
        // An unknown kind only warns; validation succeeds.
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"library\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_ok());
    }

    #[test]
    fn entry_point_defaults_to_main() {
        let manifest =
            parse_project_json("{ \"kind\": \"executable\" }", Path::new("project.json")).unwrap();
        assert_eq!(entry_point(&manifest), "main");
    }

    #[test]
    fn packages_array_wrong_type_is_error() {
        let manifest =
            parse_project_json("{ \"packages\": \"x\" }", Path::new("project.json")).unwrap();
        assert!(validate_packages_array(&manifest).is_err());
    }

    #[test]
    fn parse_project_json_rejects_non_object() {
        assert!(parse_project_json("[]", Path::new("project.json")).is_err());
        assert!(parse_project_json("{ broken", Path::new("project.json")).is_err());
    }

    #[test]
    fn field_position_finds_and_falls_back() {
        let contents = "{\n  \"name\": \"x\"\n}";
        assert_eq!(field_position(contents, "name"), (2, 3));
        assert_eq!(field_position(contents, "absent"), (3, 1));
        assert_eq!(fallback_field_position(""), (1, 1));
    }
}
