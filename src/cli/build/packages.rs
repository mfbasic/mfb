use super::*;

/// Result of verifying one installed dependency (audit-1 PKG-01, plan-23 §3.5).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum PackageVerification {
    /// `signature_type == 1` and the full §3.5 chain verifies against the
    /// pinned trust anchors.
    Verified,
    /// `signature_type == 0` — no signature present.
    Unsigned,
    /// A signed package that fails any link of the chain, or a malformed
    /// container. Always fatal.
    Tampered,
}

impl PackageVerification {
    pub(crate) fn label(self) -> &'static str {
        match self {
            PackageVerification::Verified => "Verified",
            PackageVerification::Unsigned => "Unsigned",
            PackageVerification::Tampered => "Tampered",
        }
    }
}

/// A classified dependency plus, when Tampered, the §3.5 refusal: the 6-605
/// rule name and a human detail line naming the broken chain link.
pub(crate) struct PackageClassification {
    pub(crate) state: PackageVerification,
    pub(crate) refusal: Option<(&'static str, String)>,
}

impl PackageClassification {
    fn ok(state: PackageVerification) -> Self {
        Self {
            state,
            refusal: None,
        }
    }

    fn tampered(rule: &'static str, detail: String) -> Self {
        Self {
            state: PackageVerification::Tampered,
            refusal: Some((rule, detail)),
        }
    }
}

/// Verify every declared dependency and print `uses <name> - [<state>]` for each
/// (audit-1 PKG-01). Verification is a hard build gate: all packages are checked
/// and reported first, then the build aborts with a non-zero exit if any package
/// is Tampered, or if an Unsigned package is not permitted by policy.
///
/// The trust anchor is the `identKey` pinned in the importing project's
/// `project.json` dependency entry — never the key embedded in the untrusted
/// file. Unsigned dependencies from a local source (`file:`/`local:`, or no
/// source) are permitted; unsigned dependencies from a remote source require the
/// `--unsigned` opt-in.
pub(crate) fn verify_and_report_packages(
    project_dir: &Path,
    manifest: &HashMap<String, JsonValue>,
    allow_unsigned: bool,
) -> Result<(), ()> {
    let Some(packages) = manifest
        .get("packages")
        .and_then(|value| value.get::<Vec<JsonValue>>())
    else {
        return Ok(());
    };

    let mut refusals: Vec<(&'static str, String)> = Vec::new();
    for entry in packages {
        let Some(object) = entry.get::<HashMap<String, JsonValue>>() else {
            continue;
        };
        let Some(name) = object.get("name").and_then(|value| value.get::<String>()) else {
            continue;
        };
        let source = object
            .get("source")
            .and_then(|value| value.get::<String>())
            .map(String::as_str)
            .unwrap_or_default();
        let trust_anchor = object
            .get("identKey")
            .or_else(|| object.get("ident_key"))
            .and_then(|value| value.get::<String>())
            .map(String::as_str);

        let package_file = project_dir.join("packages").join(format!("{name}.mfp"));
        if !package_file.is_file() {
            // A missing dependency is reported by the later install check with a
            // more actionable message; do not emit a verification line for it.
            continue;
        }

        let classification = classify_installed_package(&package_file, trust_anchor);
        println!("uses {name} - [{}]", classification.state.label());
        match classification.state {
            PackageVerification::Verified => {}
            PackageVerification::Unsigned => {
                if !source_is_local(source) && !allow_unsigned {
                    refusals.push((
                        "PACKAGE_UNSIGNED_REMOTE",
                        format!(
                            "package `{name}` is unsigned but its source is not local; pass --unsigned to allow it"
                        ),
                    ));
                }
            }
            PackageVerification::Tampered => {
                let (rule, detail) = classification
                    .refusal
                    .unwrap_or(("PACKAGE_SIGNATURE_INVALID", String::new()));
                refusals.push((
                    rule,
                    format!("package `{name}` failed verification ({detail}); refusing to build"),
                ));
            }
        }
    }

    if refusals.is_empty() {
        Ok(())
    } else {
        for (rule, detail) in &refusals {
            rules::show_general_diagnostic(rule, detail);
        }
        Err(())
    }
}

/// A dependency `source` that resolves to a file on disk the project controls,
/// rather than a remote/registry fetch. Unsigned local dependencies are the
/// common local-development case and are permitted without `--unsigned`.
pub(super) fn source_is_local(source: &str) -> bool {
    source.is_empty() || source.starts_with("file:") || source.starts_with("local:")
}

/// Classify an installed `.mfp` (audit-1 PKG-01) by the plan-23 §3.5 chain.
/// Any parse error is treated as Tampered — a malformed container on the
/// trusted import path is never benign.
///
/// Anchors: the `identKey` pinned in the importing project's `project.json`
/// (never the file-embedded key) and the registry key pinned as `server.pub`.
/// The chain walks pinned server key → attestation → pinned ident → proof →
/// one-off signing key → bytes; any swapped byte or key breaks a link, and
/// each broken link maps to its own 6-605 diagnostic.
pub(crate) fn classify_installed_package(
    path: &Path,
    trust_anchor: Option<&str>,
) -> PackageClassification {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) => {
            return PackageClassification::tampered(
                "PACKAGE_INVALID",
                format!("failed to read '{}': {err}", path.display()),
            );
        }
    };
    let package = match mfb_repository::package::parse_mfp_package(&bytes) {
        Ok(package) => package,
        Err(err) => return PackageClassification::tampered("PACKAGE_INVALID", err),
    };
    if package.signature_type == 0 {
        return PackageClassification::ok(PackageVerification::Unsigned);
    }
    // §3.5 step 1 — the header identKey must be the pinned ident key. A
    // signed package with no pinned anchor cannot be trusted (the
    // file-embedded key is attacker-controlled).
    let Some(trust_anchor) = trust_anchor else {
        return PackageClassification::tampered(
            "PACKAGE_IDENT_KEY_UNTRUSTED",
            "the importing project pins no identKey for this signed package".to_string(),
        );
    };
    let pinned_ident = match decode_trust_anchor(trust_anchor) {
        Ok(pinned_ident) => pinned_ident,
        Err(err) => {
            return PackageClassification::tampered(
                "PACKAGE_IDENT_KEY_UNTRUSTED",
                format!("the pinned identKey is malformed: {err}"),
            );
        }
    };
    let header_ident =
        match mfb_repository::package::decode_metadata_key(&package.ident_key, "identKey") {
            Ok(header_ident) => header_ident,
            Err(err) => {
                return PackageClassification::tampered(
                    "PACKAGE_IDENT_KEY_UNTRUSTED",
                    format!("the package identKey is malformed: {err}"),
                );
            }
        };
    if header_ident != pinned_ident {
        return PackageClassification::tampered(
            "PACKAGE_IDENT_KEY_UNTRUSTED",
            "the package identKey does not match the identKey pinned in project.json".to_string(),
        );
    }
    // §3.5 step 2 — the attestation verifies under the pinned registry key
    // and pins this exact package.
    let repo_url = mfb_repository::client::repo_url_from_env();
    let paths = match super::super::local_paths_for_repo(&repo_url) {
        Ok(paths) => paths,
        Err(err) => return PackageClassification::tampered("PACKAGE_ATTESTATION_INVALID", err),
    };
    let server_key = match mfb_repository::local::read_pinned_server_key(&paths) {
        Ok(server_key) => server_key,
        Err(_) => {
            return PackageClassification::tampered(
                "PACKAGE_ATTESTATION_INVALID",
                "no pinned registry key; run `mfb repo auth <owner>` against the registry to pin server.pub".to_string(),
            );
        }
    };
    let repo_fingerprint = mfb_repository::crypto::fingerprint(&server_key);
    if let Err(err) =
        mfb_repository::package::verify_attestation(&package, &server_key, &repo_fingerprint)
    {
        return PackageClassification::tampered("PACKAGE_ATTESTATION_INVALID", err);
    }
    // §3.5 step 3 — the proof verifies under the (pinned) ident key.
    if let Err(err) = mfb_repository::package::verify_proof(&package, &pinned_ident) {
        return PackageClassification::tampered("PACKAGE_PROOF_INVALID", err);
    }
    // §3.5 steps 4–5 — the package signature verifies under the one-off
    // signing key over the signed prefix, and the payload hash weld holds.
    if let Err(err) = mfb_repository::package::verify_package_signature(&package) {
        return PackageClassification::tampered("PACKAGE_SIGNATURE_INVALID", err);
    }
    if let Err(err) = mfb_repository::package::verify_payload_hash(&package) {
        return PackageClassification::tampered("PACKAGE_PAYLOAD_HASH_MISMATCH", err);
    }
    PackageClassification::ok(PackageVerification::Verified)
}

/// Decode a pinned trust-anchor public key. Accepts the header key format
/// (`ed25519:<base64url>`) as well as a bare base64url key.
pub(super) fn decode_trust_anchor(value: &str) -> Result<Vec<u8>, String> {
    mfb_repository::package::decode_metadata_key(value, "identKey")
}
