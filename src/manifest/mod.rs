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

mod arena_memory;
mod fields;
mod locator_check;
pub(crate) use arena_memory::{
    arena_memory_limits, DEFAULT_ARENA_MEMORY_CHECK, DEFAULT_ARENA_MEMORY_MIB,
    DEFAULT_ARENA_MEMORY_PERCENT,
};
pub(crate) use fields::*;
pub(crate) use locator_check::*;

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

    if !arena_memory::validate_arena_memory(manifest, project_path, &contents) {
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
