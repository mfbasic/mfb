use super::*;

mod edits_and_imports;
use crate::manifest::json_edit::*;
use crate::manifest::url::*;
use tinyjson::JsonValue;

// --- .mfp header byte builder -----------------------------------------

/// Append a length-prefixed (u32 LE) byte field to a buffer.
fn push_field(buf: &mut Vec<u8>, value: &[u8]) {
    buf.extend_from_slice(&(value.len() as u32).to_le_bytes());
    buf.extend_from_slice(value);
}

/// Build a minimal, structurally-valid unsigned v1.0 `.mfp` header with an
/// empty binary-representation body. Overridable pieces let each test tweak
/// one aspect and assert the corresponding error.
fn build_mfp(name: &str, version: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&MFP_MAGIC);
    buf.extend_from_slice(&1u16.to_le_bytes()); // container major
    buf.extend_from_slice(&0u16.to_le_bytes()); // container minor
    buf.extend_from_slice(&1u16.to_le_bytes()); // binary_repr major
    buf.extend_from_slice(&0u16.to_le_bytes()); // binary_repr minor
    buf.extend_from_slice(&0u32.to_le_bytes()); // flags
    push_field(&mut buf, name.as_bytes()); // name (required)
    push_field(&mut buf, b""); // ident
    push_field(&mut buf, version.as_bytes()); // version (required)
    push_field(&mut buf, b""); // author
    push_field(&mut buf, b""); // url
    push_field(&mut buf, b""); // identKey
    push_field(&mut buf, b""); // signingKey
    push_field(&mut buf, b""); // proof
    push_field(&mut buf, &[0u8; 64]); // proofSig
    push_field(&mut buf, b""); // attestation
    push_field(&mut buf, &[0u8; 64]); // attestationSig
    buf.extend_from_slice(&[0u8; 32]); // packageBinaryHash
    buf.extend_from_slice(&0u64.to_le_bytes()); // binary_repr length = 0
    buf.extend_from_slice(&0u16.to_le_bytes()); // signature type = unsigned
    buf.extend_from_slice(&0u32.to_le_bytes()); // signature length = 0
    buf
}

/// Read a header expecting failure, returning the error string. `MfpHeader`
/// is not `Debug`, so `unwrap_err` is unavailable.
fn header_err(path: &Path) -> String {
    match read_mfp_header(path) {
        Ok(_) => panic!("expected read_mfp_header to fail for {}", path.display()),
        Err(err) => err,
    }
}

fn write_temp(bytes: &[u8]) -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("pkg.mfp");
    fs::write(&path, bytes).expect("write mfp");
    (dir, path)
}

#[test]
fn package_metadata_readers_handle_a_malformed_package_by_contract() {
    let (_dir, path) = write_temp(b"not an mfp package");

    assert!(resource_closers_from_files(std::slice::from_ref(&path)).is_empty());
    assert!(verify_foreign_type_abi_consistency(&[path]).is_err());
}

/// A structurally-valid unsigned v1.0 `.mfp` that **both** decoders will
/// consider well-formed apart from the one field under test.
///
/// This cannot reuse [`build_mfp`]: that helper writes `proofSig` and
/// `attestationSig` as 64 zero bytes, and the registry additionally enforces
/// that an unsigned package carries *none* of the trust chain ("unsigned
/// .mfp package must not carry proofSig"). A fixture rejected for that
/// reason would make the divergence tests below pass for the wrong reason.
/// Here every chain field is genuinely empty.
fn build_mfp_for_both_decoders(name: &str, ident: &str) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&MFP_MAGIC);
    buf.extend_from_slice(&1u16.to_le_bytes()); // container major
    buf.extend_from_slice(&0u16.to_le_bytes()); // container minor
    buf.extend_from_slice(&1u16.to_le_bytes()); // binary_repr major
    buf.extend_from_slice(&0u16.to_le_bytes()); // binary_repr minor
    buf.extend_from_slice(&0u32.to_le_bytes()); // flags
    push_field(&mut buf, name.as_bytes());
    push_field(&mut buf, ident.as_bytes());
    push_field(&mut buf, b"1.2.3");
    push_field(&mut buf, b""); // author
    push_field(&mut buf, b""); // url
    push_field(&mut buf, b""); // identKey
    push_field(&mut buf, b""); // signingKey
    push_field(&mut buf, b""); // proof
    push_field(&mut buf, b""); // proofSig — empty, per the unsigned rule
    push_field(&mut buf, b""); // attestation
    push_field(&mut buf, b""); // attestationSig — empty, likewise
    buf.extend_from_slice(&[0u8; 32]); // packageBinaryHash
    buf.extend_from_slice(&0u64.to_le_bytes()); // binary_repr length = 0
    buf.extend_from_slice(&0u16.to_le_bytes()); // signature type = unsigned
    buf.extend_from_slice(&0u32.to_le_bytes()); // signature length = 0
    buf
}

/// **plan-126-B Phase 3 — the test that proves bug-340 B8 was honored
/// rather than silently undone.**
///
/// The `.mfp` fixed-prefix field readers are now shared
/// (`mfb_wire::mfp::read_mfp_string`), so the danger is that sharing the
/// *primitives* quietly merged the *policies* too. It did not, and this is
/// the one place both policies are reachable from: the compiler crate can
/// call `mfb_repository::package::parse_mfp_package`, the registry crate
/// cannot call `read_mfp_header`.
///
/// The fixture carries an **empty `ident`**, which is the exact field the
/// two decoders disagree about. Hand the *same bytes* to both:
///
/// * `read_mfp_header` accepts it — a locally-built package has no ident.
/// * `parse_mfp_package` rejects it — an unidentified package cannot be
///   published.
///
/// If someone later "simplifies" the two decoders into one, one of these
/// two assertions fails no matter which policy they pick.
#[test]
fn the_two_mfp_decoders_still_disagree_about_an_empty_ident_by_design() {
    let bytes = build_mfp_for_both_decoders("mypkg", "");
    let (_dir, path) = write_temp(&bytes);

    // The manifest reader's policy: `ident` is optional.
    let header = read_mfp_header(&path).expect("the manifest reader accepts an absent ident");
    assert_eq!(header.ident, "");
    assert_eq!(header.name, "mypkg");

    // The registry's policy, on the very same bytes: `ident` is required.
    let err = mfb_repository::package::parse_mfp_package(&bytes)
        .expect_err("the registry must refuse a package with no ident");
    assert_eq!(err, ".mfp ident must not be empty");

    // And the guard is specifically about `ident`, not the shared reader
    // refusing this fixture generally: fill the ident in and the registry
    // accepts what it just refused, while the manifest reader still does.
    let with_ident = build_mfp_for_both_decoders("mypkg", "alice#mypkg");
    let parsed = mfb_repository::package::parse_mfp_package(&with_ident)
        .expect("the same fixture with an ident present parses cleanly");
    assert_eq!(parsed.ident, "alice#mypkg");
    let (_dir2, path2) = write_temp(&with_ident);
    assert_eq!(
        read_mfp_header(&path2).expect("still valid here too").ident,
        "alice#mypkg",
    );
}

/// The name charset guard is the manifest reader's *other* policy, and the
/// registry deliberately does not apply it: a header name becomes
/// `packages/<name>.mfp` on the client, so `..` there escapes the project,
/// while the registry never turns a header name into a local path.
#[test]
fn only_the_manifest_reader_applies_the_name_charset_guard() {
    // A path-traversing name, with an ident present so the registry's own
    // required-ident guard cannot be what rejects it.
    let bytes = build_mfp_for_both_decoders("../../escape", "alice#mypkg");

    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(
        err.contains("not a valid path component"),
        "the manifest reader must refuse a traversing name: {err}"
    );

    // The registry accepts it: the name reaches no filesystem there.
    let parsed = mfb_repository::package::parse_mfp_package(&bytes)
        .expect("the registry applies no name charset guard");
    assert_eq!(parsed.name, "../../escape");
}

#[test]
fn read_mfp_header_parses_valid_unsigned_package() {
    let (_dir, path) = write_temp(&build_mfp("mypkg", "1.2.3"));
    let header = read_mfp_header(&path).expect("valid header");
    assert_eq!(header.name, "mypkg");
    assert_eq!(header.version, "1.2.3");
    assert_eq!(header.container_major, 1);
    assert_eq!(header.container_minor, 0);
    assert_eq!(header.signature_type, 0);
    assert_eq!(header.signature_length, 0);
    assert_eq!(header.binary_repr_length, 0);
}

#[test]
fn read_mfp_header_rejects_a_name_that_is_not_a_path_component() {
    // A consumer installs this as `packages/<name>.mfp`; a traversing name
    // would escape the project directory.
    for name in [
        "../../../../home/victim/.config/autostart/x",
        "..",
        ".",
        "a/b",
        "a\\b",
        ".hidden",
        "-rf",
        "na me",
    ] {
        let (_dir, path) = write_temp(&build_mfp(name, "1.0.0"));
        let err = header_err(&path);
        assert!(
            err.contains("not a valid path component"),
            "name `{name}` gave `{err}`"
        );
    }
    // Legitimate names still parse.
    for name in ["mypkg", "my_pkg", "my-pkg", "pkg.v2", "_x", "9lives"] {
        let (_dir, path) = write_temp(&build_mfp(name, "1.0.0"));
        assert!(read_mfp_header(&path).is_ok(), "name `{name}` must parse");
    }
}

#[test]
fn package_dependencies_drops_blank_and_traversing_names() {
    // bug-340 B9 / bug-195: `package_dependencies` feeds `.mfp` package
    // metadata and interpolates each dependency `name` into
    // `packages/<name>.mfp`, so it must apply the same guards as the sibling
    // `project_package_dependency` — a blank name or a name that is not a
    // single path component (`../evil`, `a/b`) must be dropped, not carried
    // into the wire format. Before the fix this copy had no guard.
    let entry = |name: &str| {
        let mut obj = HashMap::new();
        obj.insert("name".to_string(), JsonValue::String(name.to_string()));
        JsonValue::Object(obj)
    };
    let mut manifest = HashMap::new();
    manifest.insert(
        "packages".to_string(),
        JsonValue::Array(vec![
            entry("good"),
            entry("../evil"),
            entry(""),
            entry("a/b"),
            entry("also_ok"),
        ]),
    );
    let names: Vec<String> = package_dependencies(&manifest)
        .into_iter()
        .map(|dep| dep.name)
        .collect();
    assert_eq!(names, vec!["good".to_string(), "also_ok".to_string()]);
}

#[test]
fn read_mfp_header_rejects_missing_file() {
    let dir = tempfile::tempdir().unwrap();
    let err = header_err(&dir.path().join("absent.mfp"));
    assert!(err.contains("failed to read"));
}

#[test]
fn read_mfp_header_rejects_too_small() {
    let (_dir, path) = write_temp(&[0u8; 4]);
    let err = header_err(&path);
    assert!(err.contains("too small"));
}

#[test]
fn read_mfp_header_rejects_bad_magic() {
    let mut bytes = build_mfp("p", "1");
    bytes[0] = 0xff;
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(err.contains("magic"));
}

#[test]
fn read_mfp_header_rejects_bad_container_version() {
    let mut bytes = build_mfp("p", "1");
    bytes[8] = 2; // container major 2.0
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(err.contains("unsupported MFP container version"));
}

#[test]
fn read_mfp_header_rejects_empty_required_name() {
    let (_dir, path) = write_temp(&build_mfp("", "1"));
    let err = header_err(&path);
    assert!(err.contains("name must not be empty"));
}

#[test]
fn read_mfp_header_rejects_signature_length_mismatch() {
    // Unsigned (type 0) with a non-zero signature length is rejected.
    let mut bytes = build_mfp("p", "1");
    let len = bytes.len();
    // The signature-length u32 is the last 4 bytes we appended.
    bytes[len - 4..].copy_from_slice(&5u32.to_le_bytes());
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(err.contains("unsigned .mfp package must have zero signature length"));
}

#[test]
fn read_mfp_header_rejects_unknown_signature_type() {
    let mut bytes = build_mfp("p", "1");
    let len = bytes.len();
    // signature type u16 sits 6 bytes before the end (2 type + 4 length).
    bytes[len - 6..len - 4].copy_from_slice(&9u16.to_le_bytes());
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(err.contains("unsupported .mfp signature type"));
}

#[test]
fn read_mfp_header_rejects_trailing_length_mismatch() {
    // Claim a non-zero binary_repr length but supply no body -> the final
    // `offset != bytes.len()` check fails.
    let mut bytes = build_mfp("p", "1");
    // binary_repr length u64 sits 14 bytes before the end (8 + 2 + 4).
    let at = bytes.len() - 14;
    bytes[at..at + 8].copy_from_slice(&100u64.to_le_bytes());
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(err.contains("invalid .mfp binary representation length"));
}

/// bug-37: a 64-bit body length is narrowed to `usize`. `as` would truncate a
/// hostile `0x1_0000_0000` to `0` on a 32-bit target and the structural
/// `offset != bytes.len()` check would then validate the truncated value. Any
/// oversized length must be rejected — here it fails the trailing check on a
/// 64-bit host and the `try_from` guard on a narrower one.
#[test]
fn read_mfp_header_rejects_a_length_that_cannot_address_the_body() {
    let mut bytes = build_mfp("p", "1");
    let at = bytes.len() - 14;
    bytes[at..at + 8].copy_from_slice(&0x1_0000_0000u64.to_le_bytes());
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(
        err.contains("invalid .mfp binary representation length"),
        "{err}"
    );

    // The maximum u64 cannot describe any body on any target.
    let mut bytes = build_mfp("p", "1");
    bytes[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(
        err.contains("invalid .mfp binary representation length"),
        "{err}"
    );
}

#[test]
fn read_mfp_header_rejects_truncated_field() {
    // Truncate mid-header (after the fixed 20-byte prefix) so a length-
    // prefixed string read runs off the end.
    let bytes = build_mfp("longishname", "1")[..24].to_vec();
    let (_dir, path) = write_temp(&bytes);
    assert!(read_mfp_header(&path).is_err());
}

#[test]
fn read_mfp_header_rejects_oversized_field() {
    // Name length claims 300 bytes (> 255 limit) with matching payload.
    let mut buf = Vec::new();
    buf.extend_from_slice(&MFP_MAGIC);
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&300u32.to_le_bytes()); // name length exceeds limit
    buf.extend_from_slice(&vec![b'x'; 300]);
    let (_dir, path) = write_temp(&buf);
    let err = header_err(&path);
    assert!(err.contains("exceeds the 255 byte limit"));
}

// --- percent decoding / file:// URLs ----------------------------------

#[test]
#[cfg(unix)]
fn package_file_url_path_validates_scheme_and_extension() {
    assert!(package_file_url_path("https://x/y.mfp")
        .unwrap_err()
        .contains("file://"));
    assert!(package_file_url_path("file://")
        .unwrap_err()
        .contains("absolute package path"));
    assert!(package_file_url_path("file:///a/b.mfp?x=1")
        .unwrap_err()
        .contains("query strings or fragments"));
    assert!(package_file_url_path("file://relative/path.mfp")
        .unwrap_err()
        .contains("absolute path"));
    assert!(package_file_url_path("file:///abs/path.txt")
        .unwrap_err()
        .contains("must point to a .mfp file"));
    assert!(package_file_url_path("file:///does/not/exist.mfp")
        .unwrap_err()
        .contains("does not exist"));
}

#[test]
#[cfg(windows)]
fn package_file_url_path_validates_windows_paths() {
    let dir = tempfile::tempdir().unwrap();
    for (name, expected) in [
        ("package.txt", "must point to a .mfp file"),
        ("missing.mfp", "does not exist"),
    ] {
        let url = format!("file://{}", dir.path().join(name).display());
        assert!(package_file_url_path(&url).unwrap_err().contains(expected));
    }
}

#[test]
fn package_file_url_path_accepts_existing_file_and_decodes_percent() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("my pkg.mfp");
    fs::write(&file, b"x").unwrap();
    // Percent-encode the space in the path.
    let encoded = file.display().to_string().replace(' ', "%20");
    let url = format!("file://{encoded}");
    let resolved = package_file_url_path(&url).expect("resolves");
    assert_eq!(resolved, file);
}

#[test]
fn percent_decode_rejects_malformed_escapes() {
    assert!(percent_decode_path("abc%")
        .unwrap_err()
        .contains("incomplete"));
    assert!(percent_decode_path("abc%zz")
        .unwrap_err()
        .contains("invalid percent escape"));
    assert_eq!(percent_decode_path("a%2Fb").unwrap(), "a/b");
}

#[test]
fn hex_value_covers_all_cases() {
    assert_eq!(hex_value(b'0'), Some(0));
    assert_eq!(hex_value(b'9'), Some(9));
    assert_eq!(hex_value(b'a'), Some(10));
    assert_eq!(hex_value(b'F'), Some(15));
    assert_eq!(hex_value(b'g'), None);
}

// --- project.json package dependency parsing --------------------------

fn json(src: &str) -> JsonValue {
    src.parse::<JsonValue>().expect("json")
}

#[test]
fn project_package_dependency_reads_all_fields() {
    let value = json(
        r#"{"name":"pkg","ident":"pkg.id","version":"2.0","source":"file:///p.mfp","pin":true,"identKey":"KEY"}"#,
    );
    let dep = project_package_dependency(&value).expect("dependency");
    assert_eq!(dep.name, "pkg");
    assert_eq!(dep.ident, "pkg.id");
    assert_eq!(dep.version, "2.0");
    assert!(dep.pin);
    assert_eq!(dep.source, "file:///p.mfp");
    assert_eq!(dep.ident_key, "KEY");
}

#[test]
fn project_package_dependency_defaults_and_ident_key_alias() {
    // Missing ident defaults to name; ident_key snake-case alias is honored.
    let value = json(r#"{"name":"pkg","ident_key":"K2"}"#);
    let dep = project_package_dependency(&value).expect("dependency");
    assert_eq!(dep.ident, "pkg");
    assert_eq!(dep.version, "");
    assert!(!dep.pin);
    assert_eq!(dep.ident_key, "K2");
}

#[test]
fn project_package_dependency_rejects_non_object_and_blank_name() {
    assert!(project_package_dependency(&json("42")).is_none());
    assert!(project_package_dependency(&json(r#"{"version":"1"}"#)).is_none());
    assert!(project_package_dependency(&json(r#"{"name":"   "}"#)).is_none());
}

#[test]
fn project_package_dependency_rejects_path_traversal_name() {
    // A `name` that is not a single path component would escape `packages/`
    // when interpolated into `packages/<name>.mfp` (bug-195). Reject it.
    assert!(project_package_dependency(&json(r#"{"name":"../../../../etc/passwd"}"#)).is_none());
    assert!(project_package_dependency(&json(r#"{"name":"/etc/passwd"}"#)).is_none());
    assert!(project_package_dependency(&json(r#"{"name":"sub/dep"}"#)).is_none());
    assert!(project_package_dependency(&json(r#"{"name":".hidden"}"#)).is_none());
    // A legitimate single-component name is still accepted.
    assert!(project_package_dependency(&json(r#"{"name":"legit_pkg"}"#)).is_some());
}

#[test]
fn package_metadata_and_dependencies_from_manifest() {
    let value = json(
        r#"{"name":"proj","version":"1.0","ident":"proj.id","author":"me","url":"http://x",
                "packages":[{"name":"dep","version":"3.1","pin":true},{"noname":true}]}"#,
    );
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let meta = package_metadata(manifest);
    assert_eq!(meta.name, "proj");
    assert_eq!(meta.version, "1.0");
    assert_eq!(meta.ident, "proj.id");
    assert_eq!(meta.author, "me");
    assert_eq!(meta.url, "http://x");
    // The malformed second package (no name) is filtered out.
    assert_eq!(meta.dependencies.len(), 1);
    assert_eq!(meta.dependencies[0].name, "dep");
    assert_eq!(meta.dependencies[0].version, "3.1");
    assert!(meta.dependencies[0].pin);
    // Its ident defaults to its name.
    assert_eq!(meta.dependencies[0].ident, "dep");
}

#[test]
fn package_metadata_defaults_when_optional_fields_absent() {
    let value = json(r#"{"name":"p","version":"1"}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let meta = package_metadata(manifest);
    assert_eq!(meta.ident, "");
    assert_eq!(meta.author, "");
    assert_eq!(meta.url, "");
    assert!(meta.dependencies.is_empty());
}

#[test]
fn installed_package_files_empty_when_no_packages() {
    let value = json(r#"{"name":"p","version":"1"}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let files = installed_package_files(dir.path(), manifest).expect("ok");
    assert!(files.is_empty());
}

#[test]
fn installed_package_files_reports_missing_and_pin_mismatch() {
    // A declared dependency with no installed file -> "must be installed".
    let value = json(r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"1"}]}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let err = installed_package_files(dir.path(), manifest).unwrap_err();
    assert!(err.contains("must be installed"));

    // Install a package at the wrong version with pin -> version mismatch.
    let value = json(
        r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"9.9","pin":true}]}"#,
    );
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(pkg_dir.join("dep.mfp"), build_mfp("dep", "1.0")).unwrap();
    let err = installed_package_files(dir.path(), manifest).unwrap_err();
    assert!(err.contains("is pinned to version"));
}

#[test]
fn installed_package_files_ok_when_present_and_unpinned() {
    let value = json(r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"1"}]}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pkg_dir = dir.path().join("packages");
    fs::create_dir_all(&pkg_dir).unwrap();
    fs::write(pkg_dir.join("dep.mfp"), build_mfp("dep", "2.0")).unwrap();
    let files = installed_package_files(dir.path(), manifest).expect("ok");
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("dep.mfp"));
}

/// bug-480: a dependency compiled from source has no `packages/<name>.mfp`,
/// only the build's cache entry — and that entry must resolve exactly like an
/// installed one, including the pin check.
#[test]
fn installed_package_files_resolves_the_source_package_cache() {
    let value = json(r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"1"}]}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let cache = source_package_cache_dir(dir.path());
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("dep.mfp"), build_mfp("dep", "2.0")).unwrap();
    let files = installed_package_files(dir.path(), manifest).expect("ok");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0], cache.join("dep.mfp"));

    // The pin is enforced against the cache entry too.
    let value = json(
        r#"{"name":"p","version":"1","packages":[{"name":"dep","version":"9.9","pin":true}]}"#,
    );
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let err = installed_package_files(dir.path(), manifest).unwrap_err();
    assert!(err.contains("is pinned to version"), "{err}");
}

/// The installed form wins: `mfb pkg install` put it there deliberately, and
/// a leftover cache entry must never shadow it.
#[test]
fn resolved_package_file_prefers_the_installed_mfp() {
    let dir = tempfile::tempdir().unwrap();
    let cache = source_package_cache_dir(dir.path());
    fs::create_dir_all(&cache).unwrap();
    fs::write(cache.join("dep.mfp"), build_mfp("dep", "2.0")).unwrap();
    assert_eq!(
        resolved_package_file(dir.path(), "dep"),
        Some(cache.join("dep.mfp"))
    );

    let installed = dir.path().join("packages");
    fs::create_dir_all(&installed).unwrap();
    fs::write(installed.join("dep.mfp"), build_mfp("dep", "2.0")).unwrap();
    assert_eq!(
        resolved_package_file(dir.path(), "dep"),
        Some(installed.join("dep.mfp"))
    );

    assert_eq!(resolved_package_file(dir.path(), "absent"), None);
    // A traversing name is refused before the join, not probed.
    assert_eq!(resolved_package_file(dir.path(), "../../evil"), None);
}

#[test]
fn source_dependency_classifies_every_source_spelling() {
    let root = Path::new("/proj");
    // No `source`: the conventional `packages/<name>/` directory.
    assert_eq!(
        source_dependency(root, "dep", None),
        SourceDependency::Directory(PathBuf::from("/proj/packages/dep"))
    );
    assert_eq!(
        source_dependency(root, "dep", Some("")),
        SourceDependency::Directory(PathBuf::from("/proj/packages/dep"))
    );
    // A project-relative `file:` directory.
    assert_eq!(
        source_dependency(root, "dep", Some("file:vendor/dep")),
        SourceDependency::Directory(PathBuf::from("/proj/vendor/dep"))
    );
    // A project-relative `file:` .mfp, and the absolute `file://` URL form
    // `mfb pkg add` writes: both are the compiled form.
    assert_eq!(
        source_dependency(root, "dep", Some("file:packages/dep.mfp")),
        SourceDependency::Compiled
    );
    assert_eq!(
        source_dependency(root, "dep", Some("file:///elsewhere/dep.mfp")),
        SourceDependency::Compiled
    );
    // A registry source is never built from source here.
    assert_eq!(
        source_dependency(root, "dep", Some("https://example.test/repo")),
        SourceDependency::Compiled
    );
    // `local://` must be absolute.
    assert_eq!(
        source_dependency(root, "dep", Some("local:///abs/dep")),
        SourceDependency::Directory(PathBuf::from("/abs/dep"))
    );
    assert_eq!(
        source_dependency(root, "dep", Some("local://rel/dep")),
        SourceDependency::LocalPathNotAbsolute
    );
    // Windows' drive-relative form: rooted at no directory, so it is exactly
    // the ambiguity `local://` refuses. Its verdict is the same on every host
    // — on Unix `C:dep` is simply an oddly-named relative path.
    assert_eq!(
        source_dependency(root, "dep", Some("local://C:dep")),
        SourceDependency::LocalPathNotAbsolute
    );
}

#[test]
fn external_package_function_types_lossy_swallows_errors() {
    // Missing packages array -> empty maps, no error.
    let value = json(r#"{"name":"p","version":"1"}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let dir = tempfile::tempdir().unwrap();
    assert!(external_package_function_types(dir.path(), manifest).is_empty());
}

#[test]
fn package_export_signature_decodes_and_renders_signature() {
    let export = binary_repr::BinaryReprExport {
        name: "f".to_string(),
        kind: binary_repr::BinaryReprExportKind::Func,
        isolated: false,
        params: vec![
            binary_repr::BinaryReprExportParam {
                name: "a".to_string(),
                type_: crate::types::ParameterType::Integer,
                default: binary_repr::BinaryReprExportDefault::None,
            },
            binary_repr::BinaryReprExportParam {
                name: "b".to_string(),
                type_: crate::types::ParameterType::String,
                default: binary_repr::BinaryReprExportDefault::None,
            },
        ],
        return_type: crate::types::ParameterType::Boolean,
    };
    let signature = package_export_signature("pkg", &Default::default(), &export);
    // Params and return are decoded structurally (parse-once), and render
    // back to the exact spelling the pre-plan-105 hand formatter produced.
    assert_eq!(
        signature
            .params
            .iter()
            .map(|param| (param.name.as_str(), param.type_.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("a", crate::types::ParameterType::Integer),
            ("b", crate::types::ParameterType::String),
        ]
    );
    assert_eq!(signature.returns, crate::types::ParameterType::Boolean);
    assert!(!signature.isolated);
    assert_eq!(
        signature.signature_type().name(),
        "FUNC(Integer, String) AS Boolean"
    );
    let isolated = binary_repr::BinaryReprExport {
        isolated: true,
        params: Vec::new(),
        ..export
    };
    let isolated = package_export_signature("pkg", &Default::default(), &isolated);
    assert!(isolated.isolated);
    assert_eq!(
        isolated.signature_type().name(),
        "ISOLATED FUNC() AS Boolean"
    );
}

// --- project.json rewriting -------------------------------------------

fn dependency(name: &str, ident_key: &str) -> ProjectPackageDependency {
    ProjectPackageDependency {
        name: name.to_string(),
        ident: format!("{name}.id"),
        version: "1.0".to_string(),
        pin: true,
        source: "file:///p.mfp".to_string(),
        ident_key: ident_key.to_string(),
        direct: None,
        required_by: None,
    }
}

// --- real compiled .mfp fixture ---------------------------------------

/// A committed, structurally-valid compiled package fixture.
fn fixture_mfp() -> PathBuf {
    crate::testutil::fixture_dir("package-simple")
        .join("golden")
        .join("package_simple.mfp")
}
