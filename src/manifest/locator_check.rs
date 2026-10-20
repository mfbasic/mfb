//! plan-176-L: validation of the `libraries` section — every native-library locator's
//! shape and target coverage (`check_libraries`, `LibraryFinding`).

use super::*;

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
pub(super) fn validate_libraries(
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
