pub mod closure;
pub(crate) mod config;
pub mod entry;
pub mod json_edit;
pub mod libraries;
pub mod package;
pub mod url;

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use tinyjson::JsonValue;

use crate::rules;

pub(crate) fn parse_project_json(
    contents: &str,
    project_path: &Path,
) -> Result<HashMap<String, JsonValue>, String> {
    let manifest = crate::json::parse_json_bounded(contents)
        .map_err(|err| format!("failed to parse '{}': {err}", project_path.display()))?;
    manifest
        .get::<HashMap<String, JsonValue>>()
        .cloned()
        .ok_or_else(|| format!("'{}' must contain a JSON object", project_path.display()))
}

/// The default `maxBuffer`, in MiB — the ceiling on a single `OUT CBuffer`
/// allocation (plan-58-C).
///
/// A CBuffer's size is a runtime expression over the wrapper's parameters, so
/// without a ceiling it is an unbounded allocation request driven by the caller.
/// 64 MiB is 64 MB of arena under the kind-2 byte-list layout, and ~5.8 minutes
/// of stereo 48 kHz s16 audio.
pub(crate) const DEFAULT_MAX_BUFFER_MIB: u64 = 64;

/// The largest `maxBuffer` a project may ask for, in MiB. 4 GiB is far past any
/// real use and keeps `mib * 1024 * 1024` clear of `i64` trouble in the emitted
/// size gate.
pub(crate) const MAX_MAX_BUFFER_MIB: u64 = 4096;

/// Read `options.maxBuffer` (in MiB) from a validated manifest, defaulting to
/// [`DEFAULT_MAX_BUFFER_MIB`].
///
/// Returns bytes, which is what the codegen size gate compares against. The
/// manifest states MiB because that is the unit a person reasons about when
/// deciding how much memory one native read may claim.
///
/// **This is the CONSUMING project's setting, not the binding's.** LINK thunks
/// are emitted when an executable links, so the app that imports a binding
/// decides its own memory ceiling — a binding cannot raise it on the app's
/// behalf.
pub(crate) fn max_buffer_bytes(manifest: &HashMap<String, JsonValue>) -> u64 {
    manifest
        .get("options")
        .and_then(|options| options.get::<HashMap<String, JsonValue>>())
        .and_then(|options| options.get("maxBuffer"))
        .and_then(|value| value.get::<f64>())
        .map(|mib| *mib as u64)
        .unwrap_or(DEFAULT_MAX_BUFFER_MIB)
        .clamp(1, MAX_MAX_BUFFER_MIB)
        * 1024
        * 1024
}

/// Validate an optional `maxBuffer`: a positive integer number of MiB, at most
/// [`MAX_MAX_BUFFER_MIB`].
fn validate_max_buffer(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let options_value = manifest
        .get("options")
        .and_then(|options| options.get::<HashMap<String, JsonValue>>())
        .and_then(|options| options.get("maxBuffer"));
    let Some(value) = options_value else {
        return true;
    };
    let (line, column) = field_position(contents, "maxBuffer");
    let span_end = column + "\"maxBuffer\"".len();
    let Some(mib) = value.get::<f64>() else {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            "Field `maxBuffer` must be a number of MiB, for example `\"maxBuffer\": 128`.",
            project_path,
            line,
            column,
            span_end,
        );
        return false;
    };
    if *mib < 1.0 || mib.fract() != 0.0 || *mib > MAX_MAX_BUFFER_MIB as f64 {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!(
                "Field `maxBuffer` must be a whole number of MiB between 1 and {MAX_MAX_BUFFER_MIB}."
            ),
            project_path,
            line,
            column,
            span_end,
        );
        return false;
    }
    true
}

/// `options.canvasAutoPaintMS` when the manifest does not say: how long a
/// `canvas::AutoPaint` holds its `enterPaint` and its `exitPaint`, in milliseconds.
///
/// 100, by the owner's eye (plan-172-K): the design's 50 was too short to see in a real
/// window.
pub(crate) const DEFAULT_CANVAS_AUTO_PAINT_MS: u64 = 100;
/// The longest `options.canvasAutoPaintMS` accepted: ten seconds. A flash longer than
/// that is a state, and `hoverPaint` is the state.
pub(crate) const MAX_CANVAS_AUTO_PAINT_MS: u64 = 10_000;

/// Read `options.canvasAutoPaintMS` from a validated manifest, defaulting to
/// [`DEFAULT_CANVAS_AUTO_PAINT_MS`].
///
/// **This is the CONSUMING project's setting**, like `options.maxBuffer`: the value is
/// baked into the executable's canvas state when it links, so a package that draws an
/// `AutoPaint` gets the timing of the program that imports it.
pub(crate) fn canvas_auto_paint_ms(manifest: &HashMap<String, JsonValue>) -> u64 {
    manifest
        .get("options")
        .and_then(|options| options.get::<HashMap<String, JsonValue>>())
        .and_then(|options| options.get("canvasAutoPaintMS"))
        .and_then(|value| value.get::<f64>())
        .map(|ms| *ms as u64)
        .unwrap_or(DEFAULT_CANVAS_AUTO_PAINT_MS)
        .clamp(1, MAX_CANVAS_AUTO_PAINT_MS)
}

/// Validate an optional `canvasAutoPaintMS`: a whole number of milliseconds from 1
/// through [`MAX_CANVAS_AUTO_PAINT_MS`].
fn validate_canvas_auto_paint_ms(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let options_value = manifest
        .get("options")
        .and_then(|options| options.get::<HashMap<String, JsonValue>>())
        .and_then(|options| options.get("canvasAutoPaintMS"));
    let Some(value) = options_value else {
        return true;
    };
    let (line, column) = field_position(contents, "canvasAutoPaintMS");
    let span_end = column + "\"canvasAutoPaintMS\"".len();
    let whole = value
        .get::<f64>()
        .filter(|ms| **ms >= 1.0 && ms.fract() == 0.0 && **ms <= MAX_CANVAS_AUTO_PAINT_MS as f64);
    if whole.is_none() {
        rules::show_diagnostic(
            "PROJECT_JSON_FIELD_TYPE",
            &format!(
                "Field `canvasAutoPaintMS` must be a whole number of milliseconds between 1 and {MAX_CANVAS_AUTO_PAINT_MS}, for example `\"canvasAutoPaintMS\": 100`."
            ),
            project_path,
            line,
            column,
            span_end,
        );
        return false;
    }
    true
}

#[cfg(test)]
mod canvas_auto_paint_tests {
    use super::*;

    fn manifest(options: &str) -> HashMap<String, JsonValue> {
        let parsed: JsonValue = format!("{{\"options\": {options}}}")
            .parse()
            .expect("valid JSON");
        parsed
            .get::<HashMap<String, JsonValue>>()
            .expect("object")
            .clone()
    }

    /// plan-172-K: `options.canvasAutoPaintMS` is read as written, defaults to 100,
    /// and is kept inside the range the validator accepts even if a manifest reaches
    /// the reader unvalidated.
    #[test]
    fn canvas_auto_paint_ms_reads_defaults_and_clamps() {
        assert_eq!(DEFAULT_CANVAS_AUTO_PAINT_MS, 100);
        assert_eq!(canvas_auto_paint_ms(&manifest("{}")), 100);
        assert_eq!(canvas_auto_paint_ms(&HashMap::new()), 100);
        assert_eq!(
            canvas_auto_paint_ms(&manifest("{\"canvasAutoPaintMS\": 250}")),
            250
        );
        assert_eq!(
            canvas_auto_paint_ms(&manifest("{\"canvasAutoPaintMS\": 0}")),
            1
        );
        assert_eq!(
            canvas_auto_paint_ms(&manifest("{\"canvasAutoPaintMS\": 999999}")),
            MAX_CANVAS_AUTO_PAINT_MS
        );
        assert_eq!(
            canvas_auto_paint_ms(&manifest("{\"canvasAutoPaintMS\": \"fast\"}")),
            100
        );
    }

    /// The validator takes a whole number from 1 through the maximum and nothing else.
    #[test]
    fn canvas_auto_paint_ms_validates_its_range_and_type() {
        let path = Path::new("project.json");
        for (options, ok) in [
            ("{}", true),
            ("{\"canvasAutoPaintMS\": 1}", true),
            ("{\"canvasAutoPaintMS\": 100}", true),
            ("{\"canvasAutoPaintMS\": 10000}", true),
            ("{\"canvasAutoPaintMS\": 0}", false),
            ("{\"canvasAutoPaintMS\": 10001}", false),
            ("{\"canvasAutoPaintMS\": 12.5}", false),
            ("{\"canvasAutoPaintMS\": -3}", false),
            ("{\"canvasAutoPaintMS\": \"100\"}", false),
        ] {
            let contents = format!("{{\"options\": {options}}}");
            assert_eq!(
                validate_canvas_auto_paint_ms(&manifest(options), path, &contents),
                ok,
                "{options}"
            );
        }
    }
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

    // bug-398: bound nesting depth before tinyjson recurses, so an untrusted
    // deeply nested project.json yields a clean diagnostic instead of a
    // stack-overflow abort. Ordinary malformed input keeps tinyjson's positional
    // error below.
    if let Err(message) = crate::json::check_json_depth(&contents) {
        rules::show_diagnostic("PROJECT_JSON_PARSE_FAILED", &message, project_path, 1, 1, 1);
        return Err(());
    }

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
    if !validate_name(manifest, project_path, &contents) {
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

    if !validate_optional_string(manifest, project_path, &contents, "icon") {
        valid = false;
    }

    if !validate_description(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_kind(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_mode(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_resources(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_libraries(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_packages(manifest, project_path, &contents) {
        valid = false;
    }

    if !validate_config(project_path, &contents) {
        valid = false;
    }

    if valid {
        Ok(manifest.clone())
    } else {
        Err(())
    }
}

/// plan-167-A: validate the named build configurations in `config` (see
/// [`config::parse_config`]) and render every problem at its key's line. A
/// manifest still carrying the pre-plan-167 `"config": {"stdinLogCap": N}`
/// gets `PROJECT_JSON_CONFIG_MOVED_OPTION`, naming `options.stdinLogCap`.
fn validate_config(project_path: &Path, contents: &str) -> bool {
    let Err(diagnostics) = config::parse_config(contents) else {
        return true;
    };
    for diagnostic in diagnostics {
        render_config_diagnostic(&diagnostic, project_path, contents);
    }
    false
}

/// Render one config diagnostic, underlining its key on its line.
pub(crate) fn render_config_diagnostic(
    diagnostic: &config::ConfigDiagnostic,
    path: &Path,
    contents: &str,
) {
    let line = diagnostic.line.max(1) as usize;
    let needle = format!("\"{}\"", diagnostic.key);
    let (column, end) = contents
        .lines()
        .nth(line - 1)
        .filter(|_| !diagnostic.key.is_empty())
        .and_then(|text| text.find(&needle))
        .map(|index| (index + 1, index + 1 + needle.len()))
        .unwrap_or((1, 1));
    rules::show_diagnostic(diagnostic.rule, &diagnostic.detail, path, line, column, end);
}

/// The `packages` key, when present, must be an array (per-entry shape is checked
/// downstream). Before bug-353 this was validated only on the `pkg` subcommands
/// via [`validate_packages_array`]; `validate_project_manifest` — the sole gate
/// for `build`/`test`/`fmt`/`doc`/`audit` — skipped it, so a non-array `packages`
/// was silently ignored and every dependency vanished with no diagnostic (and a
/// later `IMPORT_PACKAGE_NOT_DECLARED` that contradicted the file). Wiring the
/// check in here fixes all five commands at once.
fn validate_packages(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    if validate_packages_array(manifest).is_ok() {
        return true;
    }
    let (line, column) = field_position(contents, "packages");
    rules::show_diagnostic(
        "PROJECT_JSON_FIELD_TYPE",
        "The `packages` field must be an array when present.",
        project_path,
        line,
        column,
        column + "\"packages\"".len(),
    );
    false
}

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
fn validate_name(
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
fn validate_description(
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

/// Validate the optional `mode` field (plan-22-A §4.1). Absent → ok. Present but
/// not a string → `PROJECT_JSON_FIELD_TYPE` (hard). A string other than
/// `console`/`app` → soft `PROJECT_JSON_UNKNOWN_MODE` (validation continues,
/// mirroring `validate_kind`).
fn validate_mode(
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
fn path_stays_in_project(value: &str) -> Result<(), String> {
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

fn validate_resources(
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
/// One `libraries` schema violation: the rule to raise and the message naming
/// the specific cause.
///
/// `validate_libraries` renders these; [`check_libraries`] produces them. The
/// split exists so the rules can be unit-tested by **message** — one rule code
/// (`PROJECT_JSON_LIBRARY_INVALID`) covers a dozen distinct mistakes, so the
/// message is what makes a diagnostic actionable, and asserting only on the code
/// would let every message regress unnoticed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LibraryFinding {
    pub rule: &'static str,
    pub message: String,
}

impl LibraryFinding {
    fn invalid(message: String) -> Self {
        Self {
            rule: "PROJECT_JSON_LIBRARY_INVALID",
            message,
        }
    }

    fn wrong_type(message: String) -> Self {
        Self {
            rule: "PROJECT_JSON_FIELD_TYPE",
            message,
        }
    }
}

/// Validate the optional `libraries` section (plan-46-A §4.4) and emit a
/// diagnostic per finding.
///
/// The section maps each `LINK` logical library name to per-platform locators.
/// Absent → valid (the section is optional). Present → a strict schema walk;
/// [`libraries::project_libraries`] then parses leniently on the assumption this
/// ran.
///
/// Diagnostic positions anchor on the `libraries` field itself; entry-level
/// precision is a nice-to-have, and each message names the offending library and
/// locator index instead.
fn validate_libraries(
    manifest: &HashMap<String, JsonValue>,
    project_path: &Path,
    contents: &str,
) -> bool {
    let findings = check_libraries(manifest);
    if findings.is_empty() {
        return true;
    }

    let (line, column) = field_position(contents, "libraries");
    let span_end = column + "\"libraries\"".len();
    let mut valid = true;
    for finding in &findings {
        rules::show_diagnostic(
            finding.rule,
            &finding.message,
            project_path,
            line,
            column,
            span_end,
        );
        if rules::is_error(finding.rule) {
            valid = false;
        }
    }
    valid
}

/// The pure `libraries` schema walk (plan-46-A §4.4). Returns one finding per
/// violation, in deterministic order; an empty vector means the section is valid
/// (or absent — it is optional).
pub(crate) fn check_libraries(manifest: &HashMap<String, JsonValue>) -> Vec<LibraryFinding> {
    let mut findings = Vec::new();

    let Some(value) = manifest.get("libraries") else {
        return findings;
    };

    let Some(section) = value.get::<HashMap<String, JsonValue>>() else {
        findings.push(LibraryFinding::wrong_type(
            "Field `libraries` must be an object mapping each logical library name to an array \
             of locators."
                .to_string(),
        ));
        return findings;
    };

    // The canonical target axes, from the backend registry — plan-46-B's coverage
    // check and plan-46-C's resolver must match against this same vocabulary.
    let known_oses = crate::target::registered_target_oses();
    let known_arches = crate::target::registered_target_arches();

    // Vendor `source` filenames are unique project-wide (§4.3): `vendor/` is flat,
    // so one filename means one file. Scoped to vendor locators only — system
    // sonames legitimately repeat across arches, and a blanket check would
    // false-positive on the most common manifest anyone will write.
    let mut vendor_sources: HashMap<String, String> = HashMap::new();

    // Deterministic finding order regardless of HashMap iteration order.
    let mut logical_names: Vec<&String> = section.keys().collect();
    logical_names.sort();

    for logical in logical_names {
        let Some(entries) = section[logical].get::<Vec<JsonValue>>() else {
            findings.push(LibraryFinding::wrong_type(format!(
                "Library `{logical}` must map to an array of locator objects."
            )));
            continue;
        };

        if entries.is_empty() {
            findings.push(LibraryFinding {
                rule: "PROJECT_JSON_EMPTY_FIELD",
                message: format!(
                    "Library `{logical}` has no locators. Add at least one, for example \
                     `{{ \"os\": \"linux\", \"type\": \"system\", \"source\": \"lib{logical}.so.0\" }}`."
                ),
            });
            continue;
        }

        for (index, entry) in entries.iter().enumerate() {
            let Some(entry) = entry.get::<HashMap<String, JsonValue>>() else {
                findings.push(LibraryFinding::wrong_type(format!(
                    "Library `{logical}` locator #{index} must be an object."
                )));
                continue;
            };
            check_locator(
                logical,
                index,
                entry,
                &known_oses,
                &known_arches,
                &mut vendor_sources,
                &mut findings,
            );
        }
    }

    findings
}

/// Validate one locator object, appending a finding per violation.
fn check_locator(
    logical: &str,
    index: usize,
    entry: &HashMap<String, JsonValue>,
    known_oses: &[String],
    known_arches: &[String],
    vendor_sources: &mut HashMap<String, String>,
    findings: &mut Vec<LibraryFinding>,
) {
    let at = format!("Library `{logical}` locator #{index}");

    // `os`: required, non-blank, and a canonical target token. An unknown token is
    // a hard error rather than a warning like `kind`/`mode`, because it yields a
    // dead entry — whereas an unknown `kind` still leaves a runnable default.
    let os = match entry.get("os") {
        None => {
            findings.push(LibraryFinding::invalid(format!(
                "{at} is missing the required `os` field. Expected one of: {}.",
                token_list(known_oses)
            )));
            return;
        }
        Some(value) => match value.get::<String>() {
            None => {
                findings.push(LibraryFinding::invalid(format!(
                    "{at} field `os` must be a string."
                )));
                return;
            }
            Some(os) if os.trim().is_empty() => {
                findings.push(LibraryFinding::invalid(format!(
                    "{at} field `os` must not be blank."
                )));
                return;
            }
            Some(os) => os.trim().to_string(),
        },
    };

    if !known_oses.iter().any(|known| known == &os) {
        findings.push(LibraryFinding::invalid(format!(
            "{at} has unknown `os` \"{os}\". Expected one of: {}.",
            token_list(known_oses)
        )));
        return;
    }

    // `arch`: optional; None = any arch (symmetric with `libc`).
    let mut axes_valid = true;
    let arch = match entry.get("arch") {
        None => None,
        Some(value) => match value.get::<String>() {
            None => {
                findings.push(LibraryFinding::invalid(format!(
                    "{at} field `arch` must be a string when present."
                )));
                axes_valid = false;
                None
            }
            Some(arch) => {
                let arch = arch.trim().to_string();
                if known_arches.iter().any(|known| known == &arch) {
                    Some(arch)
                } else {
                    findings.push(LibraryFinding::invalid(format!(
                        "{at} has unknown `arch` \"{arch}\". Expected one of: {}. Omit `arch` to \
                         match any architecture.",
                        token_list(known_arches)
                    )));
                    axes_valid = false;
                    None
                }
            }
        },
    };

    // `libc`: optional; None = any libc. Meaningless on macOS, which has no libc
    // axis at all — rejecting it there is consistent with every other
    // unknown-token case.
    let libc = match entry.get("libc") {
        None => None,
        Some(value) => match value.get::<String>() {
            None => {
                findings.push(LibraryFinding::invalid(format!(
                    "{at} field `libc` must be a string when present."
                )));
                axes_valid = false;
                None
            }
            Some(libc) => {
                let libc = libc.trim().to_string();
                if os == "macos" {
                    findings.push(LibraryFinding::invalid(format!(
                        "{at} sets `libc` on `os: \"macos\"` — macOS has no libc axis, so the \
                         field is meaningless there; remove it."
                    )));
                    axes_valid = false;
                    None
                } else if libraries::Libc::from_token(&libc).is_some() {
                    Some(libc)
                } else {
                    findings.push(LibraryFinding::invalid(format!(
                        "{at} has unknown `libc` \"{libc}\". Expected `glibc` or `musl`. Omit \
                         `libc` to match either flavor."
                    )));
                    axes_valid = false;
                    None
                }
            }
        },
    };

    // `type`: optional, defaulting to `vendor` so a mistake fails closed (§3.1).
    let lib_type = match entry.get("type") {
        None => libraries::LibType::default(),
        Some(value) => match value.get::<String>() {
            None => {
                findings.push(LibraryFinding::invalid(format!(
                    "{at} field `type` must be a string when present."
                )));
                return;
            }
            Some(token) => match libraries::LibType::from_token(token.trim()) {
                Some(lib_type) => lib_type,
                None => {
                    // Hard error: an unknown token would otherwise silently take the
                    // `vendor` default and surface as a confusing missing-file error
                    // two build phases later.
                    findings.push(LibraryFinding::invalid(format!(
                        "{at} has unknown `type` \"{}\". Expected `system` (found by the dynamic \
                         loader) or `vendor` (a file shipped in `vendor/`). Omitting `type` \
                         defaults to `vendor`.",
                        token.trim()
                    )));
                    return;
                }
            },
        },
    };

    // A Linux `vendor` locator must name its exact target (§3.2): the file is one
    // concrete build and there is no fat ELF, so a wildcard there is a claim that
    // cannot be true. macOS is exempt from the `arch` half — fat Mach-O binaries
    // are real, so a universal `.dylib` with `arch` omitted is legitimate.
    //
    // Suppressed when an axis was itself malformed: the omission finding would be
    // a confusing second complaint about the same field.
    if lib_type == libraries::LibType::Vendor && os == "linux" && axes_valid {
        if arch.is_none() {
            findings.push(LibraryFinding::invalid(format!(
                "{at} is a `vendor` locator on `os: \"linux\"` but omits `arch`. A vendored file \
                 is one concrete build — there is no fat ELF — so it must name the exact `arch` \
                 it was compiled for."
            )));
        }
        if libc.is_none() {
            findings.push(LibraryFinding::invalid(format!(
                "{at} is a `vendor` locator on `os: \"linux\"` but omits `libc`. A shared object \
                 built against glibc will not load on musl, so it must name the exact `libc` it \
                 was compiled against."
            )));
        }
    }

    // `source`: required, and a bare filename (§4.2).
    let source = match entry.get("source") {
        None => {
            findings.push(LibraryFinding::invalid(format!(
                "{at} is missing the required `source` field."
            )));
            return;
        }
        Some(value) => match value.get::<String>() {
            None => {
                findings.push(LibraryFinding::invalid(format!(
                    "{at} field `source` must be a string."
                )));
                return;
            }
            Some(source) => source.trim().to_string(),
        },
    };

    if let Err(reason) = libraries::source_is_bare(&source) {
        findings.push(LibraryFinding::invalid(format!(
            "{at} field `source` {reason}"
        )));
        return;
    }

    if lib_type == libraries::LibType::Vendor {
        if let Some(previous) = vendor_sources.insert(source.clone(), logical.to_string()) {
            findings.push(LibraryFinding {
                rule: "PROJECT_JSON_LIBRARY_SOURCE_CONFLICT",
                message: format!(
                    "{at} declares `source` \"{source}\", already declared by a `vendor` locator \
                     for library `{previous}`. `vendor/` is flat, so one filename means one file \
                     — give each vendored build its own filename."
                ),
            });
        }
    }
}

/// Render a token set for a diagnostic message: ``​`macos`, `linux`​``.
fn token_list(tokens: &[String]) -> String {
    tokens
        .iter()
        .map(|token| format!("`{token}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether the manifest requests app mode via `"mode": "app"` (plan-22-A §4.1).
/// Composed with the `-app` CLI flag in `build_project`.
pub(crate) fn build_mode_is_app(manifest: &HashMap<String, JsonValue>) -> bool {
    manifest
        .get("mode")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
        == Some("app")
}

/// The project-relative `icon` path (plan-22-A §4.3), if present. Resolution and
/// existence checking happen in `build_project` when app mode is active.
pub(crate) fn icon_path(manifest: &HashMap<String, JsonValue>) -> Option<&str> {
    manifest
        .get("icon")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
}

/// A declared `resources` entry (plan-55-A §4.1): a `src` glob (project-relative,
/// `**` allowed) paired with a `dst` directory under the build output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResourceEntry {
    pub(crate) src: String,
    pub(crate) dst: String,
}

/// The declared `resources` entries (plan-55-A §4.1), `src`/`dst` pairs. Empty
/// when the manifest declares none. Assumes the manifest already passed
/// `validate_resources`, so a present entry is a well-formed object with string
/// `src`/`dst`; a malformed entry that somehow reaches here is skipped rather
/// than panicking.
pub(crate) fn resource_entries(manifest: &HashMap<String, JsonValue>) -> Vec<ResourceEntry> {
    let Some(resources) = manifest
        .get("resources")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Vec::new();
    };

    resources
        .iter()
        .filter_map(|entry| {
            let entry = entry.get::<HashMap<String, JsonValue>>()?;
            let src = entry.get("src").and_then(|value| value.get::<String>())?;
            let dst = entry.get("dst").and_then(|value| value.get::<String>())?;
            Some(ResourceEntry {
                src: src.clone(),
                dst: dst.clone(),
            })
        })
        .collect()
}

/// The `version` string, which the macOS app-mode bundle publishes as
/// `CFBundleShortVersionString`/`CFBundleVersion` (bug-248). Always present for a
/// manifest that passed `validate_project_manifest` — `version` is required and
/// validated non-empty there.
pub(crate) fn project_version(manifest: &HashMap<String, JsonValue>) -> Option<&str> {
    manifest
        .get("version")
        .and_then(|value| value.get::<String>())
        .map(String::as_str)
}

/// The stdin broadcast-log backpressure cap in bytes from the `project.json`
/// `"options"` section's `stdinLogCap` (moved from `"config"` by
/// plan-167-A), baked into the executable.
/// Returns `None` (the runtime bakes `STDIN_LOG_CAP_DEFAULT` = 4 MiB) when the
/// key is absent or not a positive integer of at least one read chunk (8 KiB) —
/// a smaller cap could not hold a single chunk and would stall the reader.
pub(crate) fn stdin_log_cap(manifest: &HashMap<String, JsonValue>) -> Option<u64> {
    manifest
        .get("options")
        .and_then(|options| options.get::<HashMap<String, JsonValue>>())
        .and_then(|options| options.get("stdinLogCap"))
        .and_then(|value| value.get::<f64>().copied())
        .filter(|bytes| bytes.is_finite() && *bytes >= 8192.0)
        .map(|bytes| bytes as u64)
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
mod tests;
