//! plan-176-L: the per-field validators of `project.json` (name, description, sources,
//! kind, mode, resources). `validate_project_manifest` in `manifest/mod.rs` calls them.

use super::*;

/// Validate that `name` is a single safe path component (bug-503, audit-3 LNK-12).
///
/// The name is interpolated into every artifact path — `build/<name>.out`,
/// `build/<name>.app`, `build/<name>-<flavor>.AppDir`, `<name>.ast`, `<name>.nir`,
/// `<name>.nobj`, … — with a plain `Path::join`, and the executable is then made
/// `0755`. `..` escapes the project, a leading `/` makes the target absolute, a
/// leading `.` hides the file. Cloning an untrusted project and running
/// `mfb build` (which never runs the program) must not plant an executable at an
/// attacker-chosen path, so the manifest gate refuses such a name outright. The
/// charset is the one every `.mfp` package name already satisfies
/// (`validate_package_name`); the tree's 1,400+ manifests all conform.
///
/// Presence/type/emptiness are `validate_required_string`'s job (it runs first);
/// this only judges the shape of a non-empty string.
pub(super) fn validate_name(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(name) = manifest.get("name").and_then(|value| value.get::<String>()) else {
        return true;
    };
    if name.trim().is_empty() {
        return true;
    }
    let (line, column) = field_position(contents, "name");
    if package::validate_package_name(name).is_err() {
        rules::show_diagnostic(
            "PROJECT_JSON_NAME_INVALID",
            &format!(
                "Project name `{name}` is not a valid path component (expected \
                 [A-Za-z0-9_][A-Za-z0-9_.-]*). The name forms every build artifact's path \
                 (`build/<name>.out`, `<name>.ast`, …), so it may not contain a path \
                 separator, start with `.`, or be `..`."
            ),
            project_path,
            line,
            column,
            column + "\"name\"".len(),
        );
        return false;
    }
    true
}

pub(super) fn validate_required_string(
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

pub(super) fn validate_optional_string(
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

/// The largest `description` a `project.json` may declare, in bytes.
///
/// 4096 — twice the `url` header cap of 2048, and comfortably more than a
/// one-paragraph summary needs. Anything longer belongs in the DOC section,
/// which already exists for prose. Enforced here at manifest-parse time and
/// again when section 18 is read, so a hand-built payload cannot smuggle a
/// larger one past the compiler.
pub const MAX_DESCRIPTION_BYTES: usize = 4096;

/// Validate `description`: a capped string, **required** for `kind: "package"`
/// and optional for everything else.
///
/// plan-61-D landed this as a warning so the tree's 91 existing package
/// manifests kept building; plan-61-F migrated all of them and flipped it here.
/// The same rule code (`2-200-0016`) carries the new severity rather than a
/// second code being allocated — it is the same condition, and two codes for
/// one condition would leave the warn code permanently unreachable.
///
/// `kind: "executable"` neither requires nor rejects the field — executables
/// are never published, so it is inert there, but forbidding it would make a
/// `kind` flip needlessly lossy.
pub(super) fn validate_description(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let is_package = manifest
        .get("kind")
        .and_then(|value| value.get::<String>())
        .map(|kind| kind == "package")
        .unwrap_or(false);

    let Some(value) = manifest.get("description") else {
        if is_package {
            let (line, column) = fallback_field_position(contents);
            rules::show_diagnostic(
                "PROJECT_JSON_DESCRIPTION_MISSING",
                "A package must declare a `description`; it is shown on the registry.",
                project_path,
                line,
                column,
                column + 1,
            );
            return false;
        }
        // Absent on an executable: inert, and not a diagnostic at all.
        return true;
    };

    let (line, column) = field_position(contents, "description");
    let Some(description) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `description` must be a string when present.",
            project_path,
            line,
            column,
            column + "\"description\"".len(),
        );
        return false;
    };

    if description.len() > MAX_DESCRIPTION_BYTES {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!(
                "Field `description` exceeds the {MAX_DESCRIPTION_BYTES} byte limit ({} bytes).",
                description.len()
            ),
            project_path,
            line,
            column,
            column + "\"description\"".len(),
        );
        return false;
    }

    true
}

pub(super) fn validate_sources(
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

    let mut valid = validate_max_buffer(manifest, project_path, contents);
    valid &= validate_canvas_auto_paint_ms(manifest, project_path, contents);
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

pub(super) fn validate_kind(
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

/// Validate the optional `mode` field (plan-22-A §4.1). Absent → ok. Present but
/// not a string → `PROJECT_JSON_FIELD_TYPE` (hard). A string other than
/// `console`/`app` → soft `PROJECT_JSON_UNKNOWN_MODE` (validation continues,
/// mirroring `validate_kind`).
pub(super) fn validate_mode(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("mode") else {
        return true;
    };

    let (line, column) = field_position(contents, "mode");
    let Some(mode) = value.get::<String>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `mode` must be a string when present.",
            project_path,
            line,
            column,
            column + "\"mode\"".len(),
        );
        return false;
    };

    if !matches!(mode.as_str(), "console" | "app") {
        rules::show_diagnostic(
            "PROJECT_JSON_UNKNOWN_MODE",
            "Expected `console` or `app`; continuing validation.",
            project_path,
            line,
            column,
            column + "\"mode\"".len(),
        );
    }

    true
}

/// Validate the optional `resources` array (plan-55-A §4.1). Each entry is an
/// object with a string `src` (a project-relative glob, `**` allowed) and a
/// string `dst` (a destination directory under the build output). Absent is
/// valid — resources are opt-in. Mirrors `validate_sources`' diagnostics, with
/// one extra rule: a `dst` must not escape the build output, so an absolute
/// `dst` or one containing a `..` component is rejected (§1 non-goal).
/// Reject a manifest path that is not confined to the project tree (bug-298).
///
/// A `resources` entry's `src` and `dst` are both documented as project-relative,
/// but only `dst` was checked and only against Unix spellings. An absolute `src`
/// makes `Path::join` discard the project root entirely and a `../…` `src` walks
/// above it, so `mfb build` on an untrusted or third-party project copied
/// arbitrary readable files into the distributable — with exit 0 and no
/// diagnostic.
///
/// The rules mirror [`libraries::source_is_bare`]'s escape half. That function is
/// deliberately *stricter* — a library `source` must be a bare filename with no
/// separator at all — so the two cannot share one predicate; what is shared is the
/// set of escapes, including the `\` and drive-prefix cases it already rejects
/// "so plan-47 does not inherit a hole". A resource path legitimately contains
/// `/`, so only the escaping constructs are refused here.
pub(super) fn path_stays_in_project(value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err("is empty".to_string());
    }
    if value.contains('\0') {
        return Err("contains a NUL byte".to_string());
    }
    if value.starts_with('/') || value.starts_with('\\') {
        return Err(format!(
            "is absolute (`{value}`) — it must be relative to the project root"
        ));
    }
    // A Windows drive prefix is absolute on a plan-47 host even without a leading
    // separator, and `Path::join` would discard the base there just as it does for
    // a leading `/` here.
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[1] == b':' && bytes[0].is_ascii_alphabetic() {
        return Err(format!(
            "carries a drive prefix (`{}:`) — it must be relative to the project root",
            bytes[0] as char
        ));
    }
    // Split on both separators: the previous `dst` guard split only on `/`, so
    // `..\..\etc` walked straight through it.
    if value.split(['/', '\\']).any(|component| component == "..") {
        return Err(format!(
            "contains a `..` component (`{value}`) — it may not escape the project root"
        ));
    }
    Ok(())
}

pub(super) fn validate_resources(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let Some(value) = manifest.get("resources") else {
        return true;
    };

    let (line, column) = field_position(contents, "resources");
    let Some(resources) = value.get::<Vec<JsonValue>>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `resources` must be an array.",
            project_path,
            line,
            column,
            column + "\"resources\"".len(),
        );
        return false;
    };

    let mut valid = true;
    for (index, resource) in resources.iter().enumerate() {
        let Some(resource) = resource.get::<HashMap<String, JsonValue>>() else {
            rules::show_diagnostic(
                "PROJECT_JSON_FIELD_TYPE",
                &format!("Resource entry #{index} must be an object."),
                project_path,
                line,
                column,
                column + "\"resources\"".len(),
            );
            valid = false;
            continue;
        };

        // `src`: required, a non-empty string glob.
        match resource.get("src").and_then(|value| value.get::<String>()) {
            // bug-298: `src` was checked only for non-emptiness, so the READ side
            // of the copy was unbounded while the write side was contained.
            Some(src) => {
                if let Err(reason) = path_stays_in_project(src) {
                    rules::show_diagnostic(
                        "PROJECT_JSON_FIELD_TYPE",
                        &format!(
                            "Resource entry #{index} field `src` {reason}. A resource source is \
                             a glob relative to the project root."
                        ),
                        project_path,
                        line,
                        column,
                        column + "\"resources\"".len(),
                    );
                    valid = false;
                }
            }
            None => {
                rules::show_diagnostic(
                    "PROJECT_JSON_FIELD_TYPE",
                    &format!(
                        "Resource entry #{index} field `src` must be a non-empty string glob."
                    ),
                    project_path,
                    line,
                    column,
                    column + "\"resources\"".len(),
                );
                valid = false;
            }
        }

        // `dst`: required, a string that must not escape the output tree.
        match resource.get("dst").and_then(|value| value.get::<String>()) {
            Some(dst) => {
                // bug-298: this split only on `/` and treated only a leading `/`
                // as absolute, so `..\..\etc` and `C:\foo` walked through it.
                if let Err(reason) = path_stays_in_project(dst) {
                    rules::show_diagnostic(
                        "PROJECT_JSON_FIELD_TYPE",
                        &format!(
                            "Resource entry #{index} field `dst` {reason}. A resource \
                             destination is a relative path within the build output."
                        ),
                        project_path,
                        line,
                        column,
                        column + "\"resources\"".len(),
                    );
                    valid = false;
                }
            }
            None => {
                rules::show_diagnostic(
                    "PROJECT_JSON_FIELD_TYPE",
                    &format!("Resource entry #{index} field `dst` must be a string."),
                    project_path,
                    line,
                    column,
                    column + "\"resources\"".len(),
                );
                valid = false;
            }
        }
    }

    valid
}
