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

mod edges;
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

/// `description` is a capped string, **required** on a `kind: "package"`
/// project and optional on an executable (plan-61-D added it as a warning;
/// plan-61-F flipped it once all 91 manifests carried one).
#[test]
fn description_is_capped_and_required_only_for_packages() {
    // Present and well-formed: accepted, and carried into the metadata.
    let with_description = VALID.replace(
        "\"author\": \"me\"",
        "\"description\": \"A demo package.\", \"author\": \"me\"",
    );
    let (_dir, path) = write_manifest(&with_description);
    let manifest = validate_project_manifest(&path).expect("a description is valid");
    assert_eq!(
        manifest.get("description").and_then(|v| v.get::<String>()),
        Some(&"A demo package.".to_string()),
    );

    // Absent on an *executable*: silent. Executables are never published,
    // so the field is inert there.
    let (_dir, path) = write_manifest(VALID);
    assert!(validate_project_manifest(&path).is_ok());

    // Absent on a *package*: a hard error since plan-61-F. This assertion
    // was inverted when the rule was a warning; the flip is the point of
    // that sub-plan, so the test asserts the new contract rather than
    // being deleted.
    let package = VALID.replace("\"kind\": \"executable\"", "\"kind\": \"package\"");
    let (_dir, path) = write_manifest(&package);
    assert!(
        validate_project_manifest(&path).is_err(),
        "a package without a description must fail to build",
    );

    // Wrong type: a real error.
    let wrong_type = VALID.replace(
        "\"author\": \"me\"",
        "\"description\": 42, \"author\": \"me\"",
    );
    let (_dir, path) = write_manifest(&wrong_type);
    assert!(validate_project_manifest(&path).is_err());

    // Over the cap: a real error. One byte over, so the boundary itself is
    // what is tested rather than something comfortably past it.
    let over = "x".repeat(MAX_DESCRIPTION_BYTES + 1);
    let too_long = VALID.replace(
        "\"author\": \"me\"",
        &format!("\"description\": \"{over}\", \"author\": \"me\""),
    );
    let (_dir, path) = write_manifest(&too_long);
    assert!(validate_project_manifest(&path).is_err());

    // Exactly at the cap: accepted.
    let at_cap = "x".repeat(MAX_DESCRIPTION_BYTES);
    let exact = VALID.replace(
        "\"author\": \"me\"",
        &format!("\"description\": \"{at_cap}\", \"author\": \"me\""),
    );
    let (_dir, path) = write_manifest(&exact);
    assert!(
        validate_project_manifest(&path).is_ok(),
        "the cap is inclusive",
    );
}

#[test]
fn valid_manifest_parses() {
    let (_dir, path) = write_manifest(VALID);
    let manifest = validate_project_manifest(&path).expect("valid");
    assert_eq!(project_kind(&manifest), "executable");
    assert_eq!(entry_point(&manifest), "main");
    validate_packages_array(&manifest).expect("no packages ok");
}

/// bug-353 A: `validate_project_manifest` is the sole manifest gate for
/// `build`/`test`/`fmt`/`doc`/`audit`, and before the fix it never called
/// `validate_packages_array`. A non-array `packages` was silently ignored and
/// every dependency vanished with no diagnostic. Every non-array shape must
/// now fail the gate; a valid array (and an absent key) must still pass.
#[test]
fn non_array_packages_is_rejected_by_the_build_gate() {
    for shape in ["{}", "\"oops\"", "42", "null"] {
        let manifest = VALID.replace(
            "\"author\": \"me\"",
            &format!("\"packages\": {shape}, \"author\": \"me\""),
        );
        let (_dir, path) = write_manifest(&manifest);
        assert!(
            validate_project_manifest(&path).is_err(),
            "packages: {shape} must be rejected by the build gate",
        );
    }
    // A valid array and an absent key both pass — no valid-manifest churn.
    let with_array = VALID.replace(
        "\"author\": \"me\"",
        "\"packages\": [{ \"name\": \"greet\" }], \"author\": \"me\"",
    );
    let (_dir, path) = write_manifest(&with_array);
    assert!(validate_project_manifest(&path).is_ok());
    let (_dir, path) = write_manifest(VALID);
    assert!(validate_project_manifest(&path).is_ok());
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
        finding.message.contains("unknown `os` \"solaris\"") && finding.message.contains("`linux`"),
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
    let finding =
        only_finding(r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": "  " } ] }"#);
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
    let finding =
        only_finding(r#"{ "sqlite3": [ { "os": "linux", "type": "system", "source": ".." } ] }"#);
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

use std::path::PathBuf;

use crate::ast;
use crate::cli::init::project_manifest;
use crate::manifest::entry::validate_entry_point;
use crate::manifest::package::{project_json_with_package, ProjectPackageDependency};

#[test]
fn package_add_manifest_insert_creates_packages_array() {
    let contents = concat!(
        "{\n",
        "  \"name\": \"app\",\n",
        "  \"version\": \"0.1.0\",\n",
        "  \"mfb\": \"1.0\",\n",
        "  \"sources\": [{ \"root\": \"src\" }]\n",
        "}\n"
    );
    let manifest = parse_project_json(contents, Path::new("project.json")).expect("manifest");
    let dependency = ProjectPackageDependency {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        version: "1.0.0".to_string(),
        pin: true,
        source: "file:///tmp/source/shape.mfp".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };

    let updated =
        project_json_with_package(contents, &manifest, &dependency).expect("updated manifest");

    assert!(updated.contains("\"packages\": ["));
    assert!(updated.contains("\"name\": \"shape\""));
    assert!(updated.contains("\"ident\": \"ada#shape\""));
    assert!(updated.contains("\"version\": \"1.0.0\""));
    assert!(updated.contains("\"pin\": true"));
    assert!(updated.contains("\"source\": \"file:///tmp/source/shape.mfp\""));
    assert!(updated.parse::<JsonValue>().is_ok());
}

#[test]
fn package_add_manifest_append_preserves_json_array_format() {
    let contents = concat!(
        "{\n",
        "  \"name\": \"app\",\n",
        "  \"version\": \"0.1.0\",\n",
        "  \"mfb\": \"1.0\",\n",
        "  \"sources\": [{ \"root\": \"src\" }],\n",
        "  \"packages\": [\n",
        "    {\n",
        "      \"name\": \"math\",\n",
        "      \"ident\": \"std#math\",\n",
        "      \"version\": \"1.0.0\",\n",
        "      \"pin\": true,\n",
        "      \"source\": \"file:packages/math.mfp\"\n",
        "    }\n",
        "  ]\n",
        "}\n"
    );
    let manifest = parse_project_json(contents, Path::new("project.json")).expect("manifest");
    let dependency = ProjectPackageDependency {
        name: "shape".to_string(),
        ident: "ada#shape".to_string(),
        version: "1.0.0".to_string(),
        pin: true,
        source: "file:///tmp/source/shape.mfp".to_string(),
        ident_key: String::new(),
        direct: None,
        required_by: None,
    };

    let updated =
        project_json_with_package(contents, &manifest, &dependency).expect("updated manifest");

    assert!(updated.contains("    },\n    {\n      \"name\": \"shape\""));
    assert!(updated.parse::<JsonValue>().is_ok());
}

#[test]
fn validate_entry_point_rejects_multiple_matching_declarations() {
    let root = test_temp_dir("validate_entry_point_rejects_multiple_matching_declarations");
    let project_dir = root.join("app");
    let src_dir = project_dir.join("src");
    fs::create_dir_all(&src_dir).expect("src dir");
    fs::write(
        project_dir.join("project.json"),
        project_manifest(&project_dir),
    )
    .expect("project manifest");
    fs::write(
        src_dir.join("main_a.mfb"),
        "FUNC main() AS Nothing\nEND FUNC\n",
    )
    .expect("main_a");
    fs::write(
        src_dir.join("main_b.mfb"),
        "FUNC main(args AS List OF String) AS Integer\n  RETURN 0\nEND FUNC\n",
    )
    .expect("main_b");

    let manifest_contents =
        fs::read_to_string(project_dir.join("project.json")).expect("manifest contents");
    let manifest = parse_project_json(&manifest_contents, &project_dir.join("project.json"))
        .expect("manifest");
    let ast = ast::parse_project("app", &project_dir, &manifest).expect("ast");

    assert!(validate_entry_point(&project_dir, &manifest, &crate::hir::elaborate(&ast)).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

#[test]
fn validate_project_manifest_rejects_missing_kind() {
    let root = test_temp_dir("validate_project_manifest_rejects_missing_kind");
    let project_dir = root.join("app");
    fs::create_dir_all(&project_dir).expect("project dir");
    fs::write(
        project_dir.join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"sources\": [{ \"root\": \"src\" }]\n",
            "}\n"
        ),
    )
    .expect("project manifest");

    assert!(validate_project_manifest(&project_dir.join("project.json")).is_err());

    fs::remove_dir_all(root).expect("remove temp dir");
}

/// bug-503: `name` is interpolated into every artifact path (`build/<name>.out`,
/// `<name>.ast`, `<name>.nir`, the `.app`/`.AppDir` bundles), so a name that is
/// not a single safe path component must be refused at the manifest gate —
/// `../evil` escapes the project, `/tmp/evil` is absolute, `.evil` hides the
/// artifact.
#[test]
fn validate_project_manifest_rejects_a_name_that_is_not_a_path_component() {
    let root = test_temp_dir("validate_project_manifest_rejects_traversing_name");
    let project_dir = root.join("app");
    fs::create_dir_all(&project_dir).expect("project dir");
    let manifest = |name: &str| {
        format!(
            concat!(
                "{{\n",
                "  \"name\": \"{}\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"sources\": [{{ \"root\": \"src\" }}]\n",
                "}}\n"
            ),
            name
        )
    };
    for bad in [
        "../evil",
        "../../../../tmp/lnk12-pwn/evil",
        "/tmp/evil",
        "sub/dir",
        "back\\slash",
        ".hidden",
        "..",
        "with space",
    ] {
        fs::write(project_dir.join("project.json"), manifest(bad)).expect("project manifest");
        assert!(
            validate_project_manifest(&project_dir.join("project.json")).is_err(),
            "name {bad:?} must be rejected"
        );
    }
    // Every name shape the tree's 1,400+ manifests actually use stays valid.
    for good in ["app", "my-app", "my_app", "app.v2", "3d", "A"] {
        fs::write(project_dir.join("project.json"), manifest(good)).expect("project manifest");
        assert!(
            validate_project_manifest(&project_dir.join("project.json")).is_ok(),
            "name {good:?} must stay valid"
        );
    }

    fs::remove_dir_all(root).expect("remove temp dir");
}

/// plan-58-C: `maxBuffer` is the `OUT CBuffer` allocation ceiling in MiB.
/// Whole number, 1..=4096; anything else is rejected rather than silently
/// clamped, because a manifest that says 0 or "big" means the author has a
/// belief about the ceiling that the build would otherwise quietly ignore.
#[test]
fn validate_project_manifest_checks_max_buffer() {
    let root = test_temp_dir("validate_project_manifest_checks_max_buffer");
    let project_dir = root.join("app");
    fs::create_dir_all(&project_dir).expect("project dir");
    let manifest = |max_buffer: &str| {
        format!(
            concat!(
                "{{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"options\": {{ \"maxBuffer\": {} }},\n",
                "  \"sources\": [{{ \"root\": \"src\" }}]\n",
                "}}\n"
            ),
            max_buffer
        )
    };
    let path = project_dir.join("project.json");
    for good in ["1", "64", "128", "4096"] {
        fs::write(&path, manifest(good)).expect("project manifest");
        assert!(
            validate_project_manifest(&path).is_ok(),
            "maxBuffer {good} must be accepted"
        );
    }
    for bad in ["0", "-1", "4097", "64.5", "\"64\"", "true"] {
        fs::write(&path, manifest(bad)).expect("project manifest");
        assert!(
            validate_project_manifest(&path).is_err(),
            "maxBuffer {bad} must be rejected"
        );
    }
    // Omitted entirely is fine — it defaults to 64 MiB.
    fs::write(
        &path,
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"sources\": [{ \"root\": \"src\" }]\n",
            "}\n"
        ),
    )
    .expect("project manifest");
    assert!(validate_project_manifest(&path).is_ok());

    fs::remove_dir_all(root).expect("remove temp dir");
}

fn test_temp_dir(name: &str) -> PathBuf {
    let current_thread = std::thread::current();
    let thread = current_thread.name().unwrap_or("test");
    let thread: String = thread
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '_' })
        .collect();
    let root = std::env::temp_dir().join(format!("mfb_{name}_{}_{}", std::process::id(), thread));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("temp dir");
    root
}
