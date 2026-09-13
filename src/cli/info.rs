//! `mfb info <binary>`: report what the linker recorded in an executable.
//!
//! The command reads the one file it is given. It finds the unconditional
//! `MFBasic\0` provenance note (`./mfb spec linker provenance-marker`) through the
//! file's own format — an ELF `PT_NOTE`, a Mach-O `LC_NOTE`, or the PE `.mfbnote`
//! section — and prints `Not a MFBasic binary` when there is none. It reports the
//! libraries the image loads and, for a signed build, the `mfb-signing-v1` blob in
//! its `.mfbsign` section: whether the file's contents still match the content
//! signature, and the trust chain, which is checked online against the registry
//! the blob names. No local state is read — not `~/.mfb`, not a pinned key.
//!
//! The file is untrusted input: the format readers (`crate::os::inspect`)
//! bounds-check every offset it names, and every string it carries — the
//! registry URL included — is sanitized before it reaches the terminal.

use std::collections::HashMap;
use std::path::Path;

use mfb_repository::server::IdentChainLink;
use tinyjson::JsonValue;

use crate::cli::help::INFO_HELP;
use crate::cli::CommandError;
use crate::os::inspect::{inspect, Linking};
use crate::terminal_safe::safe;

#[cfg(test)]
#[path = "info/tests.rs"]
mod tests;

/// The line printed for any file that carries no provenance note.
pub(crate) const NOT_MFBASIC: &str = "Not a MFBasic binary";

/// Printed below a signed binary's report: what the verdicts do and do not mean.
pub(crate) const SIGNED_NOTE: &str = "\
Note: \"trust chain: verified\" means the owner named above signed this build
with their own key, and the registry named above confirms that key belongs to
the owner's account. \"contents: intact\" means the file has not changed since
it was signed. Together they mean this file is exactly what the owner built.
They do not mean the program is safe or correct, or that the registry reviewed
or approved it. Anything signed with this key comes from the same account;
that includes anyone who has stolen the key. It is only as trustworthy as the
registry named above: if you do not trust that registry, this proves nothing.";

/// `mfb info <binary>`. Exactly one operand; anything else is a usage error.
///
/// Every inspection outcome exits `0` — not an MFBasic binary, unsigned, a
/// chain that does not verify, an unreachable registry, and a file that cannot
/// be read.
pub(crate) fn run_info_command(args: &[String]) -> Result<(), CommandError> {
    let [path] = args else {
        return Err(CommandError::Usage(format!(
            "mfb info accepts exactly one <binary>\n\n{INFO_HELP}"
        )));
    };
    let path = Path::new(path);
    match std::fs::read(path) {
        Ok(bytes) => print!("{}", render(path, &bytes, &OnlineRegistry)),
        Err(err) => eprintln!("error: failed to read '{}': {err}", path.display()),
    }
    Ok(())
}

/// What `mfb info` asks the registry a signed binary names. Every call takes
/// the registry URL from the binary itself; nothing is read from local state.
pub(crate) trait RegistryLookup {
    /// The registry's public key (`GET /ident`).
    fn server_key(&self, registry: &str) -> Result<Vec<u8>, String>;
    /// The owner's current ident public key, from the name binding the registry
    /// signs with `server_key` (`GET /index/<owner>#<package>`).
    fn current_ident(
        &self,
        registry: &str,
        server_key: &[u8],
        owner: &str,
        package: &str,
    ) -> Result<Vec<u8>, String>;
    /// The owner's ident rotation links, each signed by the key it retires
    /// (`GET /idents/<owner>`).
    fn ident_chain(&self, registry: &str, owner: &str) -> Result<Vec<IdentChainLink>, String>;
}

/// The registry over the network, through the repository client's
/// transport rules (https except loopback, no private-network targets, bounded
/// timeouts and response sizes).
struct OnlineRegistry;

// coverage:off — network calls; the verification they feed is unit-tested with a
// fake registry, and end to end against a loopback registry by `cli_repo_publish`.
impl RegistryLookup for OnlineRegistry {
    fn server_key(&self, registry: &str) -> Result<Vec<u8>, String> {
        mfb_repository::client::fetch_server_key(registry)
    }

    fn current_ident(
        &self,
        registry: &str,
        server_key: &[u8],
        owner: &str,
        package: &str,
    ) -> Result<Vec<u8>, String> {
        let index =
            mfb_repository::client::fetch_index_with_key(registry, server_key, owner, package)?;
        mfb_repository::package::decode_metadata_key(&index.ident_key, "identKey")
    }

    fn ident_chain(&self, registry: &str, owner: &str) -> Result<Vec<IdentChainLink>, String> {
        mfb_repository::client::fetch_ident_chain(registry, owner).map(|chain| chain.chain)
    }
}
// coverage:on

/// The full `mfb info` report for `bytes`. `registry` is consulted only when the
/// binary is signed.
pub(crate) fn render(path: &Path, bytes: &[u8], registry: &dyn RegistryLookup) -> String {
    let Some(binary) = inspect(bytes) else {
        return format!("{NOT_MFBASIC}\n");
    };
    let mut out = String::new();
    let mut line = |text: String| {
        out.push_str(&text);
        out.push('\n');
    };
    line(format!("File: {}", path.display()));
    line(format!("Format: {}", binary.format));
    line(format!("Architecture: {}", binary.arch));
    match &binary.linking {
        Some(Linking::Static) => line("Linking: static".to_string()),
        Some(Linking::Dynamic(interpreter)) => line(format!(
            "Linking: dynamic (interpreter {})",
            safe(interpreter)
        )),
        None => {}
    }
    if binary.libraries.is_empty() {
        line("Libraries: none".to_string());
    } else {
        line("Libraries:".to_string());
        for library in &binary.libraries {
            line(format!("  {}", safe(library)));
        }
    }
    if !binary.search_paths.is_empty() {
        line("Search paths:".to_string());
        for path in &binary.search_paths {
            line(format!("  {}", safe(path)));
        }
    }
    line(format!("Compiler: {}", binary.compiler));
    match &binary.signing {
        None => line("Signed: no".to_string()),
        Some(blob) => {
            line("Signed: yes".to_string());
            let digest =
                crate::os::content_signature::content_digest(bytes, blob, binary.covered_end);
            for signed in signing_lines(&bytes[blob.clone()], digest, registry) {
                line(format!("  {signed}"));
            }
            line(String::new());
            line(SIGNED_NOTE.to_string());
        }
    }
    out
}

/// The `mfb-signing-v1` fields (`./mfb spec package-manager signing`).
struct SigningBlob {
    owner: String,
    author: String,
    /// Absent from a blob written before executables named their registry.
    registry: Option<String>,
    ident_key: String,
    ident_fingerprint: String,
    signing_key: String,
    signing_fingerprint: String,
    proof: String,
    proof_signature: String,
    attestation: String,
    attestation_signature: String,
    /// Absent from a blob written before executables carried one.
    content_signature: Option<String>,
}

/// The indented lines under `Signed: yes`: what the blob records, then three
/// verdicts — the file's contents, the owner's ident key, and the trust chain.
/// `content_digest` is the file's content digest
/// (`crate::os::content_signature::content_digest`), `None` when the blob does
/// not lie inside the covered range.
fn signing_lines(
    blob: &[u8],
    content_digest: Option<[u8; 32]>,
    registry: &dyn RegistryLookup,
) -> Vec<String> {
    let (blob, proof) = match parse_signing_blob(blob) {
        Ok(parsed) => parsed,
        Err(err) => {
            return vec![format!(
                "trust chain: NOT VERIFIED (malformed signing metadata: {})",
                safe(&err)
            )]
        }
    };
    let proof_text = |field: &str| {
        json_str(&proof, field)
            .map_or_else(|| "<none>".to_string(), |value| safe(value).into_owned())
    };
    let issued = match proof.get("issued") {
        Some(JsonValue::Number(seconds)) if seconds.is_finite() && seconds.fract() == 0.0 => {
            format_utc(*seconds as i64)
        }
        _ => "<none>".to_string(),
    };
    let (ident_key, trust_chain) = match verify_chain(&blob, &proof, registry) {
        Ok((repo_fingerprint, IdentState::Replaced(current))) => (
            format!("REPLACED without a rotation link (current identFingerprint {current})"),
            format!(
                "NOT VERIFIED (the owner's ident key was replaced without a rotation link; \
                 repoFingerprint {repo_fingerprint})"
            ),
        ),
        Ok((repo_fingerprint, state)) => (
            state.describe(),
            format!("verified (repoFingerprint {repo_fingerprint})"),
        ),
        Err(reason) => (
            "not checked".to_string(),
            format!("NOT VERIFIED ({})", safe(&reason)),
        ),
    };
    vec![
        format!("owner: {}", safe(&blob.owner)),
        format!("author: {}", safe(&blob.author)),
        format!(
            "registry: {}",
            blob.registry
                .as_deref()
                .map_or_else(|| "<none>".to_string(), |url| safe(url).into_owned())
        ),
        format!("ident: {}", proof_text("ident")),
        format!("version: {}", proof_text("version")),
        format!("issued: {issued}"),
        format!("ident fingerprint: {}", safe(&blob.ident_fingerprint)),
        format!("signing fingerprint: {}", safe(&blob.signing_fingerprint)),
        format!("contents: {}", contents_status(&blob, content_digest)),
        format!("ident key: {}", safe(&ident_key)),
        format!("trust chain: {trust_chain}"),
    ]
}

/// Decode the blob and the proof JSON it embeds.
fn parse_signing_blob(blob: &[u8]) -> Result<(SigningBlob, JsonObject), String> {
    let text = std::str::from_utf8(blob).map_err(|_| "not UTF-8".to_string())?;
    let value = json_object(
        text.trim_end_matches(['\0', '\n', '\r']),
        "signing metadata",
    )?;
    let field = |name: &str| {
        json_str(&value, name)
            .map(str::to_string)
            .ok_or_else(|| format!("missing `{name}`"))
    };
    let format = field("format")?;
    if format != "mfb-signing-v1" {
        return Err(format!("unknown format `{format}`"));
    }
    let signature_type = field("signatureType")?;
    if signature_type != "Ed25519" {
        return Err(format!("unknown signatureType `{signature_type}`"));
    }
    let blob = SigningBlob {
        owner: field("owner")?,
        author: field("author")?,
        registry: json_str(&value, "registry").map(str::to_string),
        ident_key: field("identKey")?,
        ident_fingerprint: field("identFingerprint")?,
        signing_key: field("signingKey")?,
        signing_fingerprint: field("signingFingerprint")?,
        proof: field("proof")?,
        proof_signature: field("proofSignature")?,
        attestation: field("attestation")?,
        attestation_signature: field("attestationSignature")?,
        content_signature: json_str(&value, "contentSignature").map(str::to_string),
    };
    let proof = json_object(&blob.proof, "proof")?;
    Ok((blob, proof))
}

/// Whether the file's bytes are the ones the one-off signing key sealed.
fn contents_status(blob: &SigningBlob, content_digest: Option<[u8; 32]>) -> String {
    use mfb_repository::crypto;

    let Some(content_signature) = blob.content_signature.as_deref() else {
        return "not signed (no contentSignature)".to_string();
    };
    let signing_public =
        match mfb_repository::package::decode_metadata_key(&blob.signing_key, "signingKey") {
            Ok(key) if crypto::fingerprint(&key) == blob.signing_fingerprint => key,
            Ok(_) => {
                return "unverified (signingFingerprint does not match signingKey)".to_string()
            }
            Err(err) => return format!("unverified ({})", safe(&err)),
        };
    let Some(content_digest) = content_digest else {
        return "unverified (the signing section lies outside the signed range)".to_string();
    };
    match crypto::decode_bytes(content_signature, "contentSignature").and_then(|signature| {
        crypto::verify(
            &signing_public,
            &crypto::executable_signing_input(&content_digest),
            &signature,
        )
    }) {
        Ok(()) => "intact".to_string(),
        Err(_) => "MODIFIED (content signature does not match the file)".to_string(),
    }
}

/// Where the owner's ident key stands at the registry today, against the key
/// that signed the proof.
enum IdentState {
    /// The signing ident is still the owner's current key.
    Current,
    /// The owner rotated away from the signing ident through signed links; the
    /// payload is the current key's fingerprint.
    Rotated(String),
    /// The owner's current key is not reachable from the signing ident through
    /// signed links — a re-anchor; the payload is the current key's fingerprint.
    Replaced(String),
    /// The registry could not answer.
    Unknown(String),
}

impl IdentState {
    fn describe(&self) -> String {
        match self {
            IdentState::Current => "current".to_string(),
            IdentState::Rotated(current) => {
                format!("rotated since signing (current identFingerprint {current})")
            }
            IdentState::Replaced(current) => {
                format!("REPLACED without a rotation link (current identFingerprint {current})")
            }
            IdentState::Unknown(reason) => format!("unknown ({reason})"),
        }
    }
}

/// Walk the plan-23 chain the blob embeds, against the registry it names: the
/// keys match their fingerprints; the proof verifies under the ident key and
/// names this owner and these keys; the registry at the blob's URL holds the key
/// the attestation's `repoFingerprint` names; the attestation verifies under it
/// and pins the same owner, ident, version and keys. Then asks that registry
/// where the owner's ident key stands. Answers the registry fingerprint and the
/// ident state.
fn verify_chain(
    blob: &SigningBlob,
    proof: &JsonObject,
    registry: &dyn RegistryLookup,
) -> Result<(String, IdentState), String> {
    use mfb_repository::crypto;
    use mfb_repository::package::decode_metadata_key;

    let ident_public = decode_metadata_key(&blob.ident_key, "identKey")?;
    if crypto::fingerprint(&ident_public) != blob.ident_fingerprint {
        return Err("identFingerprint does not match identKey".to_string());
    }
    let signing_public = decode_metadata_key(&blob.signing_key, "signingKey")?;
    if crypto::fingerprint(&signing_public) != blob.signing_fingerprint {
        return Err("signingFingerprint does not match signingKey".to_string());
    }
    if blob.author != blob.owner {
        return Err("author does not match the signing owner".to_string());
    }

    let proof_signature = crypto::decode_bytes(&blob.proof_signature, "proofSignature")?;
    crypto::verify(
        &ident_public,
        &crypto::proof_signing_input(blob.proof.as_bytes()),
        &proof_signature,
    )
    .map_err(|_| "invalid proof signature".to_string())?;
    expect(proof, "proof", "owner", &blob.owner)?;
    expect(proof, "proof", "identFingerprint", &blob.ident_fingerprint)?;
    expect(
        proof,
        "proof",
        "signingFingerprint",
        &blob.signing_fingerprint,
    )?;
    let signed =
        |field: &str| json_str(proof, field).ok_or_else(|| format!("proof has no `{field}`"));
    let ident = signed("ident")?;
    let version = signed("version")?;
    let package = match ident.split_once('#') {
        Some((ident_owner, package)) if ident_owner.eq_ignore_ascii_case(&blob.owner) => package,
        _ => return Err("proof ident does not belong to the signing owner".to_string()),
    };

    let url = blob
        .registry
        .as_deref()
        .ok_or("no registry in the signing metadata")?;
    let server_public = registry
        .server_key(url)
        .map_err(|err| format!("registry {url} is unreachable: {err}"))?;
    let repo_fingerprint = crypto::fingerprint(&server_public);
    let attestation = json_object(&blob.attestation, "attestation")?;
    if json_str(&attestation, "repoFingerprint") != Some(repo_fingerprint.as_str()) {
        return Err(format!(
            "registry {url} holds a key other than the one that signed the attestation \
             (repoFingerprint {repo_fingerprint})"
        ));
    }
    let attestation_signature =
        crypto::decode_bytes(&blob.attestation_signature, "attestationSignature")?;
    crypto::verify(
        &server_public,
        &crypto::attestation_signing_input(blob.attestation.as_bytes()),
        &attestation_signature,
    )
    .map_err(|_| "invalid attestation signature".to_string())?;
    for (field, value) in [
        ("owner", blob.owner.as_str()),
        ("ident", ident),
        ("version", version),
        ("identFingerprint", blob.ident_fingerprint.as_str()),
        ("signingFingerprint", blob.signing_fingerprint.as_str()),
    ] {
        expect(&attestation, "attestation", field, value)?;
    }

    let state = match registry.current_ident(url, &server_public, &blob.owner, package) {
        Err(err) => IdentState::Unknown(err),
        Ok(current) if current == ident_public => IdentState::Current,
        Ok(current) => {
            let current_fingerprint = crypto::fingerprint(&current);
            match registry.ident_chain(url, &blob.owner).and_then(|chain| {
                mfb_repository::client::follow_ident_chain(&blob.owner, &ident_public, &chain)
            }) {
                Err(err) => IdentState::Unknown(err),
                Ok(Some(successor)) if successor == current => {
                    IdentState::Rotated(current_fingerprint)
                }
                Ok(_) => IdentState::Replaced(current_fingerprint),
            }
        }
    };
    Ok((repo_fingerprint, state))
}

type JsonObject = HashMap<String, JsonValue>;

fn expect(object: &JsonObject, what: &str, field: &str, value: &str) -> Result<(), String> {
    if json_str(object, field) == Some(value) {
        Ok(())
    } else {
        Err(format!(
            "{what} {field} does not match the signing metadata"
        ))
    }
}

fn json_str<'a>(object: &'a JsonObject, field: &str) -> Option<&'a str> {
    match object.get(field) {
        Some(JsonValue::String(value)) => Some(value),
        _ => None,
    }
}

/// Parse untrusted JSON through the depth-bounded parser (bug-398), requiring an
/// object at the top.
fn json_object(text: &str, what: &str) -> Result<JsonObject, String> {
    match crate::json::parse_json_bounded(text) {
        Ok(JsonValue::Object(object)) => Ok(object),
        _ => Err(format!("{what} is not a JSON object")),
    }
}

/// Unix seconds as `YYYY-MM-DD HH:MM:SS UTC` (proleptic Gregorian).
fn format_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    // Days since 1970-01-01 to a civil date (Howard Hinnant's `civil_from_days`).
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        time / 3_600,
        time % 3_600 / 60,
        time % 60
    )
}
