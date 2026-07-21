pub mod entry;
pub mod libraries;
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

/// Read `maxBuffer` (in MiB) from a validated manifest, defaulting to
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
        .get("maxBuffer")
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
    let Some(value) = manifest.get("maxBuffer") else {
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

    if !validate_optional_string(manifest, project_path, &contents, "icon") {
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

    let mut valid = validate_max_buffer(manifest, project_path, contents);
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
/// `"config"` section's `stdinLogCap` (plan-15 D3), baked into the executable.
/// Returns `None` (the runtime bakes `STDIN_LOG_CAP_DEFAULT` = 4 MiB) when the
/// key is absent or not a positive integer of at least one read chunk (8 KiB) —
/// a smaller cap could not hold a single chunk and would stall the reader.
pub(crate) fn stdin_log_cap(manifest: &HashMap<String, JsonValue>) -> Option<u64> {
    manifest
        .get("config")
        .and_then(|config| config.get::<HashMap<String, JsonValue>>())
        .and_then(|config| config.get("stdinLogCap"))
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
mod tests {
    /// bug-298: `mfb build` validated a `resources` entry's `dst` against escape
    /// but checked `src` only for non-emptiness, so the READ side of the copy was
    /// unbounded while the write side was contained. An absolute `src` makes
    /// `Path::join` discard the project root and a `../…` `src` walks above it, so
    /// building an untrusted or third-party project copied arbitrary readable
    /// files into the distributable with exit 0 and no diagnostic. The `dst` guard
    /// was separately Unix-only.
    #[test]
    fn resource_paths_may_not_escape_the_project() {
        // Escapes, in every spelling. The last two are the ones the old `dst`
        // guard missed entirely: it split only on `/` and treated only a leading
        // `/` as absolute.
        for escaping in [
            "/etc/passwd",
            "/tmp/secret/*.conf",
            "../outside/*.conf",
            "a/../../outside/*.conf",
            "\\\\server\\share",
            "..\\..\\outside\\*.conf",
            "C:\\secret\\*.conf",
            "d:/secret/*.conf",
            "",
            "   ",
        ] {
            assert!(
                super::path_stays_in_project(escaping).is_err(),
                "{escaping:?} must be rejected"
            );
        }

        // Ordinary project-relative resource paths keep working -- including
        // globs, nested directories, a trailing separator, and a leading `./`.
        for contained in [
            "assets/*.png",
            "data/loops/*.ogg",
            "cfg/",
            "./data/x.txt",
            "a.b.c",
            "dir/..file",
            "dotted../x",
        ] {
            assert!(
                super::path_stays_in_project(contained).is_ok(),
                "{contained:?} must be accepted"
            );
        }
    }

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
    fn mode_app_validates_and_is_detected() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"executable\",\n  \"mode\": \"app\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        let manifest = validate_project_manifest(&path).expect("valid");
        assert!(build_mode_is_app(&manifest));
    }

    #[test]
    fn mode_console_is_not_app() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"executable\",\n  \"mode\": \"console\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        let manifest = validate_project_manifest(&path).expect("valid");
        assert!(!build_mode_is_app(&manifest));
    }

    #[test]
    fn absent_mode_is_not_app() {
        let (_dir, path) = write_manifest(VALID);
        let manifest = validate_project_manifest(&path).expect("valid");
        assert!(!build_mode_is_app(&manifest));
    }

    #[test]
    fn unknown_mode_still_validates_ok() {
        // An unknown mode only warns (like kind); validation succeeds.
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"executable\",\n  \"mode\": \"kiosk\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        let manifest = validate_project_manifest(&path).expect("valid");
        // An unrecognized mode is not treated as app.
        assert!(!build_mode_is_app(&manifest));
    }

    #[test]
    fn non_string_mode_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"executable\",\n  \"mode\": 3,\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn non_string_icon_is_error() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"executable\",\n  \"icon\": 7,\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn icon_path_accessor_reads_field() {
        let (_dir, path) = write_manifest(
            "{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \"kind\": \"executable\",\n  \"icon\": \"art/icon.png\",\n  \"sources\": [ { \"root\": \"src\" } ]\n}",
        );
        let manifest = validate_project_manifest(&path).expect("valid");
        assert_eq!(icon_path(&manifest), Some("art/icon.png"));
        assert_eq!(
            icon_path(&validate_project_manifest(&write_manifest(VALID).1).unwrap()),
            None
        );
    }

    // ---- `resources` section (plan-55-A §4.1) ----

    /// Build a manifest string with the given `resources` JSON fragment spliced in.
    fn manifest_with_resources(resources: &str) -> String {
        format!(
            "{{\n  \"name\": \"n\",\n  \"version\": \"1\",\n  \"mfb\": \"1\",\n  \
             \"kind\": \"executable\",\n  \"resources\": {resources},\n  \
             \"sources\": [ {{ \"root\": \"src\" }} ]\n}}"
        )
    }

    #[test]
    fn resources_valid_entry_is_accepted() {
        let (_dir, path) = write_manifest(&manifest_with_resources(
            "[ { \"src\": \"data/**/*.ogg\", \"dst\": \"music/\" } ]",
        ));
        let manifest = validate_project_manifest(&path).expect("valid resources");
        let entries = resource_entries(&manifest);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].src, "data/**/*.ogg");
        assert_eq!(entries[0].dst, "music/");
    }

    #[test]
    fn resources_absent_is_valid_and_empty() {
        let manifest = validate_project_manifest(&write_manifest(VALID).1).expect("valid");
        assert!(resource_entries(&manifest).is_empty());
    }

    #[test]
    fn resources_non_array_is_rejected() {
        let (_dir, path) = write_manifest(&manifest_with_resources("\"nope\""));
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn resources_non_object_entry_is_rejected() {
        let (_dir, path) = write_manifest(&manifest_with_resources("[ \"nope\" ]"));
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn resources_missing_or_empty_src_is_rejected() {
        let (_dir, path) = write_manifest(&manifest_with_resources("[ { \"dst\": \"music/\" } ]"));
        assert!(validate_project_manifest(&path).is_err());
        let (_dir, path) = write_manifest(&manifest_with_resources(
            "[ { \"src\": \"   \", \"dst\": \"music/\" } ]",
        ));
        assert!(validate_project_manifest(&path).is_err());
        let (_dir, path) = write_manifest(&manifest_with_resources(
            "[ { \"src\": 3, \"dst\": \"music/\" } ]",
        ));
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn resources_missing_or_nonstring_dst_is_rejected() {
        let (_dir, path) = write_manifest(&manifest_with_resources("[ { \"src\": \"a/*.ogg\" } ]"));
        assert!(validate_project_manifest(&path).is_err());
        let (_dir, path) = write_manifest(&manifest_with_resources(
            "[ { \"src\": \"a/*.ogg\", \"dst\": 3 } ]",
        ));
        assert!(validate_project_manifest(&path).is_err());
    }

    #[test]
    fn resources_escaping_dst_is_rejected() {
        let (_dir, path) = write_manifest(&manifest_with_resources(
            "[ { \"src\": \"a/*.ogg\", \"dst\": \"/etc\" } ]",
        ));
        assert!(validate_project_manifest(&path).is_err());
        let (_dir, path) = write_manifest(&manifest_with_resources(
            "[ { \"src\": \"a/*.ogg\", \"dst\": \"../out\" } ]",
        ));
        assert!(validate_project_manifest(&path).is_err());
        let (_dir, path) = write_manifest(&manifest_with_resources(
            "[ { \"src\": \"a/*.ogg\", \"dst\": \"music/../..\" } ]",
        ));
        assert!(validate_project_manifest(&path).is_err());
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

    // ---- `libraries` section (plan-46-A §4.4) ----
    //
    // Asserted by *message*, not just rule code: `PROJECT_JSON_LIBRARY_INVALID`
    // covers a dozen distinct causes, so the message is the actionable part and a
    // code-only assertion would let every message regress unnoticed.

    /// Build a manifest object carrying just a `libraries` section.
    fn libraries_manifest(section: &str) -> HashMap<String, JsonValue> {
        let json = format!("{{ \"libraries\": {section} }}");
        let value: JsonValue = json.parse().expect("test manifest parses");
        value
            .get::<HashMap<String, JsonValue>>()
            .cloned()
            .expect("test manifest is an object")
    }

    /// The single finding for a section expected to have exactly one violation.
    fn only_finding(section: &str) -> LibraryFinding {
        let mut findings = check_libraries(&libraries_manifest(section));
        assert_eq!(
            findings.len(),
            1,
            "expected exactly one finding, got: {findings:#?}"
        );
        findings.remove(0)
    }

    fn assert_valid(section: &str) {
        let findings = check_libraries(&libraries_manifest(section));
        assert!(
            findings.is_empty(),
            "expected a valid section, got: {findings:#?}"
        );
    }

    #[test]
    fn absent_libraries_section_is_valid() {
        let manifest = libraries_manifest("{}");
        assert!(check_libraries(&manifest).is_empty());
        // And a manifest with no `libraries` key at all validates as it did before
        // the section existed.
        let (_dir, path) = write_manifest(VALID);
        assert!(validate_project_manifest(&path).is_ok());
    }

    #[test]
    fn non_object_libraries_is_field_type_error() {
        let finding = only_finding("[1, 2]");
        assert_eq!(finding.rule, "PROJECT_JSON_FIELD_TYPE");
        assert!(
            finding.message.contains("must be an object"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn non_array_library_value_is_field_type_error() {
        let finding = only_finding(r#"{ "sqlite3": "libsqlite3.so.0" }"#);
        assert_eq!(finding.rule, "PROJECT_JSON_FIELD_TYPE");
        assert!(
            finding.message.contains("must map to an array"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn empty_locator_array_is_empty_field_error() {
        let finding = only_finding(r#"{ "sqlite3": [] }"#);
        assert_eq!(finding.rule, "PROJECT_JSON_EMPTY_FIELD");
        assert!(
            finding.message.contains("has no locators"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn non_object_locator_is_field_type_error() {
        let finding = only_finding(r#"{ "sqlite3": ["libsqlite3.so.0"] }"#);
        assert_eq!(finding.rule, "PROJECT_JSON_FIELD_TYPE");
        assert!(
            finding.message.contains("locator #0 must be an object"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn missing_os_names_the_field_and_accepted_set() {
        let finding = only_finding(r#"{ "sqlite3": [ { "source": "libsqlite3.so.0" } ] }"#);
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("missing the required `os` field")
                && finding.message.contains("`linux`")
                && finding.message.contains("`macos`"),
            "message must name the field and the accepted set: {}",
            finding.message
        );
    }

    #[test]
    fn unknown_os_token_names_the_token_and_accepted_set() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "solaris", "type": "system", "source": "libsqlite3.so" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("unknown `os` \"solaris\"")
                && finding.message.contains("`linux`"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn unknown_arch_token_names_the_token_and_the_wildcard_option() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "arch": "sparc", "type": "system", "source": "libsqlite3.so" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("unknown `arch` \"sparc\"")
                && finding.message.contains("Omit `arch`"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn unknown_libc_token_names_the_token_and_the_wildcard_option() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "libc": "uclibc", "type": "system", "source": "libsqlite3.so" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("unknown `libc` \"uclibc\"")
                && finding.message.contains("`glibc`")
                && finding.message.contains("`musl`"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn unknown_type_token_explains_the_vendor_default() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "type": "shared", "source": "libsqlite3.so" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("unknown `type` \"shared\"")
                && finding.message.contains("`system`")
                && finding.message.contains("defaults to `vendor`"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn macos_locator_carrying_libc_is_rejected_with_the_reason() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "macos", "libc": "glibc", "type": "system", "source": "libsqlite3.dylib" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("macOS has no libc axis"),
            "message must say why, not just that it is invalid: {}",
            finding.message
        );
    }

    #[test]
    fn blank_source_is_rejected() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": "  " } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("must not be blank"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn missing_source_is_rejected() {
        let finding = only_finding(r#"{ "sqlite3": [ { "os": "linux", "type": "system" } ] }"#);
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding
                .message
                .contains("missing the required `source` field"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn source_with_a_path_separator_names_the_offending_character() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": "usr/lib/libsqlite3.so" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("path separator (`/`)"),
            "message must name the offending character: {}",
            finding.message
        );

        // Backslashes too — plan-47 must not inherit a hole.
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": "lib\\sqlite3.so" } ] }"#,
        );
        assert!(
            finding.message.contains("path separator (`\\`)"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn source_of_dot_dot_is_rejected() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": ".." } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("directory reference"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn source_with_an_interior_nul_is_rejected() {
        // `source` is emitted verbatim as a C string, so an interior NUL would
        // silently truncate the dlopen argument.
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": "libsqlite3.so\u0000evil" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("NUL byte"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn source_with_a_windows_drive_prefix_is_rejected() {
        let finding = only_finding(
            r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": "C:sqlite3.dll" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("drive prefix"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn linux_vendor_locator_missing_arch_is_rejected_with_the_reason() {
        let finding = only_finding(
            r#"{ "foo": [ { "os": "linux", "libc": "musl", "type": "vendor", "source": "libfoo.so" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("omits `arch`") && finding.message.contains("no fat ELF"),
            "message must say why a wildcard cannot be true: {}",
            finding.message
        );
    }

    #[test]
    fn linux_vendor_locator_missing_libc_is_rejected_with_the_reason() {
        let finding = only_finding(
            r#"{ "foo": [ { "os": "linux", "arch": "x86_64", "type": "vendor", "source": "libfoo.so" } ] }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_INVALID");
        assert!(
            finding.message.contains("omits `libc`")
                && finding.message.contains("will not load on musl"),
            "message: {}",
            finding.message
        );
    }

    #[test]
    fn linux_vendor_locator_missing_both_axes_reports_both() {
        let findings = check_libraries(&libraries_manifest(
            r#"{ "foo": [ { "os": "linux", "type": "vendor", "source": "libfoo.so" } ] }"#,
        ));
        assert_eq!(findings.len(), 2, "both axes are missing: {findings:#?}");
        assert!(findings.iter().any(|f| f.message.contains("omits `arch`")));
        assert!(findings.iter().any(|f| f.message.contains("omits `libc`")));
    }

    #[test]
    fn two_vendor_locators_sharing_a_source_conflict() {
        // The real bug this catches: an author copied an entry for a new platform
        // and forgot to rename the blob. Across *all* logical names, since
        // `vendor/` is flat.
        let finding = only_finding(
            r#"{
                "foo": [
                    { "os": "linux", "arch": "x86_64", "libc": "glibc", "type": "vendor", "source": "libshared.so" }
                ],
                "bar": [
                    { "os": "linux", "arch": "aarch64", "libc": "glibc", "type": "vendor", "source": "libshared.so" }
                ]
            }"#,
        );
        assert_eq!(finding.rule, "PROJECT_JSON_LIBRARY_SOURCE_CONFLICT");
        assert!(
            finding.message.contains("libshared.so")
                && finding.message.contains("one filename means one file"),
            "message: {}",
            finding.message
        );
    }

    // ---- positive cases that must pass (§4.3, §3.2) ----

    #[test]
    fn two_system_locators_may_share_a_soname() {
        // The most common manifest anyone will write: the same soname on two
        // arches. A blanket uniqueness check would false-positive here.
        assert_valid(
            r#"{
                "sqlite3": [
                    { "os": "linux", "arch": "x86_64", "type": "system", "source": "libsqlite3.so.0" },
                    { "os": "linux", "arch": "aarch64", "type": "system", "source": "libsqlite3.so.0" }
                ]
            }"#,
        );
    }

    #[test]
    fn linux_system_locator_may_omit_both_axes() {
        // One line covering all six Linux slots — `arch` and `libc` are symmetric
        // wildcards for a `system` locator.
        assert_valid(
            r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": "libsqlite3.so.0" } ] }"#,
        );
    }

    #[test]
    fn macos_vendor_locator_may_omit_arch() {
        // Mach-O fat binaries are real, so a universal `.dylib` is a legitimate
        // vendor locator with no `arch`.
        assert_valid(
            r#"{ "imaging": [ { "os": "macos", "type": "vendor", "source": "libimaging.dylib" } ] }"#,
        );
    }

    #[test]
    fn the_representative_manifest_validates() {
        // The plan-46-A §1 worked example.
        assert_valid(
            r#"{
                "sqlite3": [
                    { "os": "macos", "type": "system", "source": "libsqlite3.dylib" },
                    { "os": "linux", "type": "system", "source": "libsqlite3.so.0" },
                    { "os": "linux", "arch": "riscv64", "libc": "musl", "source": "libsqlite3-riscv64-musl.so" }
                ]
            }"#,
        );
    }

    #[test]
    fn a_well_formed_section_passes_full_manifest_validation() {
        let contents = VALID.trim_end().trim_end_matches('}').to_string()
            + ",\n  \"libraries\": { \"sqlite3\": [ { \"os\": \"linux\", \"type\": \"system\", \"source\": \"libsqlite3.so.0\" } ] }\n}\n";
        let (_dir, path) = write_manifest(&contents);
        assert!(
            validate_project_manifest(&path).is_ok(),
            "a well-formed libraries section must validate"
        );
    }

    #[test]
    fn a_malformed_section_fails_full_manifest_validation() {
        let contents = VALID.trim_end().trim_end_matches('}').to_string()
            + ",\n  \"libraries\": { \"sqlite3\": [ { \"os\": \"solaris\", \"source\": \"x.so\" } ] }\n}\n";
        let (_dir, path) = write_manifest(&contents);
        assert!(
            validate_project_manifest(&path).is_err(),
            "an unknown os token must fail the build"
        );
    }
}
