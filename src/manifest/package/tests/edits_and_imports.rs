//! plan-176-K: `manifest::package::tests` — the edits_and_imports tests. The shared `.mfp` builders live\n//! in `package/tests/mod.rs`.

use super::*;

/// plan-105-A: the `.mfp` decode is the ONE parse-in boundary for imported
/// signatures, and its typed form must render back byte-identically to the
/// `FUNC(…) AS R` spelling the driver used to build (and then re-parse).
/// A shape that does not round-trip here would silently change every
/// `function_types` entry an importer sees.
#[test]
fn package_export_signature_round_trips_every_export_shape() {
    let param = |type_: &str| binary_repr::BinaryReprExportParam {
        name: "p".to_string(),
        type_: crate::types::ParameterType::parse(type_),
        default: binary_repr::BinaryReprExportDefault::None,
    };
    // (param types, return type, isolated) -> the hand-formatted spelling.
    let cases: Vec<(Vec<&str>, &str, bool)> = vec![
        (vec![], "Nothing", false),
        (vec!["Integer"], "String", false),
        (vec!["Integer", "String", "Boolean"], "Fixed", false),
        (vec!["String"], "SoundFile", false),
        // A stateful resource return: the STATE clause rides inside the
        // nominal leaf, which is exactly why `rsplit_once(" AS ")` existed.
        (vec!["String"], "SoundFile STATE FileInfo", false),
        (vec!["List OF RES fs.File"], "Nothing", false),
        (
            vec!["Map OF String TO List OF Integer"],
            "Set OF Byte",
            false,
        ),
        (
            vec!["MapEntry OF String TO Integer"],
            "#Result OF Integer",
            false,
        ),
        (vec!["RES fs.File"], "RES tcp.Socket", false),
        // A higher-order parameter — the shape whose embedded `) AS ` broke
        // the old first-match `split_once(") AS ")` splitter.
        (vec!["FUNC(Integer) AS String"], "Boolean", false),
        (
            vec!["FUNC(Integer, String) AS List OF Byte", "Integer"],
            "Nothing",
            false,
        ),
        (vec![], "FUNC(Integer) AS String", false),
        (
            vec!["ISOLATED FUNC(ThreadWorker OF Integer TO String) AS Nothing"],
            "Thread OF Integer TO String",
            true,
        ),
        (
            vec!["Thread OF Integer RES fs.File TO String"],
            "Nothing",
            false,
        ),
        (vec!["Pair OF (Integer, String)"], "Nothing", false),
    ];
    for (param_types, return_type, isolated) in cases {
        let export = binary_repr::BinaryReprExport {
            name: "f".to_string(),
            kind: binary_repr::BinaryReprExportKind::Func,
            isolated,
            params: param_types.iter().map(|t| param(t)).collect(),
            return_type: crate::types::ParameterType::parse(return_type),
        };
        let expected = format!(
            "{}FUNC({}) AS {return_type}",
            if isolated { "ISOLATED " } else { "" },
            param_types.join(", "),
        );
        assert_eq!(
            package_export_signature("pkg", &Default::default(), &export)
                .signature_type()
                .name(),
            expected,
            "signature round-trip lost information for {expected}"
        );
    }
}

#[test]
fn project_json_with_package_inserts_new_array() {
    let contents = "{\n  \"name\": \"proj\",\n  \"version\": \"1\"\n}\n";
    let value = json(contents);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let out = project_json_with_package(contents, manifest, &dependency("dep", "KEY")).unwrap();
    assert!(out.contains("\"packages\": ["));
    assert!(out.contains("\"name\": \"dep\""));
    assert!(out.contains("\"identKey\": \"KEY\""));
    // Result is still valid JSON declaring the dependency.
    let reparsed = out.parse::<JsonValue>().expect("valid json");
    assert!(reparsed
        .get::<HashMap<String, JsonValue>>()
        .unwrap()
        .contains_key("packages"));
}

#[test]
fn project_json_with_package_appends_to_existing_array() {
    let contents = "{\n  \"name\": \"proj\",\n  \"packages\": [\n    { \"name\": \"a\" }\n  ]\n}\n";
    let value = json(contents);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    // No identKey -> the field is omitted.
    let out = project_json_with_package(contents, manifest, &dependency("b", "")).unwrap();
    assert!(out.contains("\"name\": \"a\""));
    assert!(out.contains("\"name\": \"b\""));
    assert!(!out.contains("identKey"));
    out.parse::<JsonValue>().expect("valid json");
}

#[test]
fn project_json_with_package_appends_to_empty_array() {
    let contents = "{\n  \"name\": \"proj\",\n  \"packages\": []\n}\n";
    let value = json(contents);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let out = project_json_with_package(contents, manifest, &dependency("b", "")).unwrap();
    assert!(out.contains("\"name\": \"b\""));
    out.parse::<JsonValue>().expect("valid json");
}

#[test]
fn project_json_with_package_rejects_duplicate() {
    let contents = "{\n  \"packages\": [\n    { \"name\": \"dep\" }\n  ]\n}\n";
    let value = json(contents);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let err = project_json_with_package(contents, manifest, &dependency("dep", "")).unwrap_err();
    assert!(err.contains("already declares package"));
}

fn three_package_manifest() -> String {
    "{\n  \"name\": \"app\",\n  \"packages\": [\n    {\n      \"name\": \"a\",\n      \"ident\": \"o#a\",\n      \"version\": \"1.0.0\"\n    },\n    {\n      \"name\": \"b\",\n      \"ident\": \"o#b\",\n      \"version\": \"2.0.0\"\n    },\n    {\n      \"name\": \"c\",\n      \"ident\": \"o#c\",\n      \"version\": \"3.0.0\"\n    }\n  ]\n}\n".to_string()
}

/// plan-60-F §4.3: removing the first, middle, last and only entry. Each
/// result must re-parse as valid JSON — a dangling comma is the failure mode
/// this class of surgical edit gets wrong, and it differs per position.
#[test]
fn project_json_without_packages_removes_at_every_position() {
    let contents = three_package_manifest();
    for (target, survivors) in [
        ("o#a", vec!["o#b", "o#c"]),
        ("o#b", vec!["o#a", "o#c"]),
        ("o#c", vec!["o#a", "o#b"]),
    ] {
        let out = project_json_without_packages(&contents, &[target]).unwrap();
        let parsed = out.parse::<JsonValue>().expect("valid json");
        let packages = parsed
            .get::<HashMap<String, JsonValue>>()
            .and_then(|m| m.get("packages"))
            .and_then(|v| v.get::<Vec<JsonValue>>())
            .expect("packages array");
        assert_eq!(packages.len(), 2, "removing {target}: {out}");
        assert!(!out.contains(target), "removing {target}: {out}");
        for survivor in survivors {
            assert!(
                out.contains(survivor),
                "removing {target} lost {survivor}: {out}"
            );
        }
    }
}

/// Removing every entry leaves an empty array, which is what plan-60-B
/// §4.3's zero-dependency path expects to find.
#[test]
fn project_json_without_packages_can_empty_the_array() {
    let out =
        project_json_without_packages(&three_package_manifest(), &["o#a", "o#b", "o#c"]).unwrap();
    let parsed = out.parse::<JsonValue>().expect("valid json");
    let packages = parsed
        .get::<HashMap<String, JsonValue>>()
        .and_then(|m| m.get("packages"))
        .and_then(|v| v.get::<Vec<JsonValue>>())
        .expect("packages array");
    assert!(packages.is_empty(), "{out}");
    // ...and the rest of the manifest survives.
    assert!(out.contains("\"name\": \"app\""), "{out}");
}

/// Removing the ONLY entry — the single-entry array is its own case.
#[test]
fn project_json_without_packages_removes_the_only_entry() {
    let contents = "{\n  \"name\": \"app\",\n  \"packages\": [\n    {\n      \"name\": \"a\",\n      \"ident\": \"o#a\",\n      \"version\": \"1.0.0\"\n    }\n  ]\n}\n";
    let out = project_json_without_packages(contents, &["o#a"]).unwrap();
    out.parse::<JsonValue>().expect("valid json");
    assert!(!out.contains("o#a"), "{out}");
}

/// Multiple removals in one call — the cascade's shape.
#[test]
fn project_json_without_packages_removes_several_at_once() {
    let out = project_json_without_packages(&three_package_manifest(), &["o#a", "o#c"]).unwrap();
    out.parse::<JsonValue>().expect("valid json");
    assert!(!out.contains("o#a") && !out.contains("o#c"), "{out}");
    assert!(out.contains("o#b"), "{out}");
}

/// Surviving entries keep their original text, so per-entry formatting is
/// preserved rather than re-serialized.
#[test]
fn project_json_without_packages_preserves_survivor_formatting() {
    let out = project_json_without_packages(&three_package_manifest(), &["o#a"]).unwrap();
    assert!(
        out.contains("{\n      \"name\": \"b\",\n      \"ident\": \"o#b\",\n      \"version\": \"2.0.0\"\n    }"),
        "survivor text must be byte-identical: {out}"
    );
}

#[test]
fn project_json_without_packages_errors_when_none_match() {
    let err = project_json_without_packages(&three_package_manifest(), &["o#missing"]).unwrap_err();
    assert!(err.contains("declares none of"), "{err}");
}

/// plan-60-E §4.1: pin state is PRESERVED unless a flag was passed. This is
/// the property that separates the targeted form from `add` — bumping a
/// floating dependency's ABI floor must not silently pin it.
#[test]
fn project_json_with_updated_version_preserves_pin_by_default() {
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"shape\",\n      \"ident\": \"ada#shape\",\n      \"version\": \"1.0.0\",\n      \"pin\": false\n    }\n  ]\n}\n";
    let out = project_json_with_updated_version(contents, "ada#shape", "1.4.0", None).unwrap();
    assert!(out.contains("\"version\": \"1.4.0\""), "{out}");
    assert!(
        out.contains("\"pin\": false"),
        "pin must survive untouched: {out}"
    );
    // A pinned entry likewise keeps its pin.
    let pinned = contents.replace("\"pin\": false", "\"pin\": true");
    let out = project_json_with_updated_version(&pinned, "ada#shape", "1.4.0", None).unwrap();
    assert!(out.contains("\"pin\": true"), "{out}");
}

#[test]
fn project_json_with_updated_version_rewrites_pin_when_asked() {
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"shape\",\n      \"ident\": \"ada#shape\",\n      \"version\": \"1.0.0\",\n      \"pin\": false\n    }\n  ]\n}\n";
    let out =
        project_json_with_updated_version(contents, "ada#shape", "1.4.0", Some(true)).unwrap();
    assert!(out.contains("\"pin\": true"), "{out}");
    let out =
        project_json_with_updated_version(contents, "ada#shape", "1.4.0", Some(false)).unwrap();
    assert!(out.contains("\"pin\": false"), "{out}");
}

/// A dependency with no `pin` field gets one appended, mirroring the
/// sibling `identKey` editor's behavior.
#[test]
fn project_json_with_updated_version_appends_a_missing_pin() {
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"shape\",\n      \"ident\": \"ada#shape\",\n      \"version\": \"1.0.0\"\n    }\n  ]\n}\n";
    let out =
        project_json_with_updated_version(contents, "ada#shape", "1.4.0", Some(true)).unwrap();
    assert!(out.contains("\"pin\": true"), "{out}");
    assert!(out.contains("\"version\": \"1.4.0\""), "{out}");
}

/// The edit is surgical: sibling dependencies and the file's formatting are
/// untouched. Re-serializing the manifest would destroy both.
#[test]
fn project_json_with_updated_version_leaves_siblings_and_formatting_alone() {
    let contents = "{\n  \"name\": \"app\",\n  \"packages\": [\n    {\n      \"name\": \"other\",\n      \"ident\": \"ada#other\",\n      \"version\": \"9.9.9\",\n      \"pin\": true\n    },\n    {\n      \"name\": \"shape\",\n      \"ident\": \"ada#shape\",\n      \"version\": \"1.0.0\",\n      \"pin\": false\n    }\n  ]\n}\n";
    let out = project_json_with_updated_version(contents, "ada#shape", "1.4.0", None).unwrap();
    assert!(
        out.contains(
            "\"ident\": \"ada#other\",\n      \"version\": \"9.9.9\",\n      \"pin\": true"
        ),
        "the sibling entry must be byte-identical: {out}"
    );
    assert!(out.starts_with("{\n  \"name\": \"app\","), "{out}");
    assert!(out.ends_with("]\n}\n"), "{out}");
    // Exactly one version changed.
    assert_eq!(out.matches("\"version\": \"1.4.0\"").count(), 1, "{out}");
    assert_eq!(out.matches("\"version\": \"9.9.9\"").count(), 1, "{out}");
}

/// Matched on `ident`, which defaults to `name` when absent — so a
/// bare-name entry is still reachable.
#[test]
fn project_json_with_updated_version_matches_a_bare_name_entry() {
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"shape\",\n      \"version\": \"1.0.0\"\n    }\n  ]\n}\n";
    let out = project_json_with_updated_version(contents, "shape", "2.0.0", None).unwrap();
    assert!(out.contains("\"version\": \"2.0.0\""), "{out}");
}

#[test]
fn project_json_with_updated_version_errors_when_absent() {
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"shape\",\n      \"version\": \"1.0.0\"\n    }\n  ]\n}\n";
    let err =
        project_json_with_updated_version(contents, "ada#missing", "2.0.0", None).unwrap_err();
    assert!(err.contains("does not declare package"), "{err}");
}

#[test]
fn project_json_with_updated_ident_key_rewrites_existing() {
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"dep\",\n      \"identKey\": \"OLD\"\n    }\n  ]\n}\n";
    let out = project_json_with_updated_ident_key(contents, "dep", "NEW").unwrap();
    assert!(out.contains("\"identKey\": \"NEW\""));
    assert!(!out.contains("OLD"));
    out.parse::<JsonValue>().expect("valid json");
}

#[test]
fn project_json_with_updated_ident_key_inserts_when_absent() {
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"dep\"\n    }\n  ]\n}\n";
    let out = project_json_with_updated_ident_key(contents, "dep", "NEW").unwrap();
    assert!(out.contains("\"identKey\": \"NEW\""));
    out.parse::<JsonValue>().expect("valid json");
}

#[test]
fn project_json_with_updated_ident_key_errors() {
    // No packages array at all.
    let err = project_json_with_updated_ident_key("{}", "dep", "K").unwrap_err();
    assert!(err.contains("could not locate project.json `packages` array"));
    // Array present but package not declared.
    let contents = "{\n  \"packages\": [\n    { \"name\": \"other\" }\n  ]\n}\n";
    let err = project_json_with_updated_ident_key(contents, "dep", "K").unwrap_err();
    assert!(err.contains("does not declare package"));
}

#[test]
fn insert_helpers_report_missing_structures() {
    // insert_package_dependency with no packages array.
    assert!(insert_package_dependency("{}", "entry")
        .unwrap_err()
        .contains("could not locate project.json `packages` array"));
    // insert_packages_array with no root object.
    assert!(insert_packages_array("", "entry")
        .unwrap_err()
        .contains("could not locate end of project.json object"));
}

#[test]
fn json_scanning_primitives() {
    let src = r#"{"a": "x\"y", "b": [1, 2]}"#;
    // Field-name lookup skips string contents (the escaped quote in "x\"y").
    let a = json_field_name_position(src, "a").unwrap();
    assert_eq!(&src[a..a + 3], "\"a\"");
    let b = json_field_name_position(src, "b").unwrap();
    assert!(b > a);
    assert!(json_field_name_position(src, "missing").is_none());
    // bug-212: a string *value* equal to a field name must not match as a key;
    // only the real `"packages":` key (followed by `:`) is located.
    let src2 = r#"{"source": "packages", "packages": [1]}"#;
    let pkgs = json_field_name_position(src2, "packages").unwrap();
    // The located position is the key, so a `:` (after optional whitespace)
    // follows its closing quote.
    assert!(src2[pkgs + "\"packages\"".len()..]
        .trim_start()
        .starts_with(':'));
    // Array bounds around "b".
    let (start, end) = json_array_bounds(src, "b").unwrap();
    assert_eq!(src.as_bytes()[start], b'[');
    assert_eq!(src.as_bytes()[end], b']');
    // find_json_punct returns None when a non-space non-target byte appears.
    assert!(find_json_punct("x:", 0, b':').is_none());
    // root_object_end and matching delimiter.
    assert!(root_object_end("  { }").is_some());
    assert!(root_object_end("no object").is_none());
    // Unbalanced braces -> None.
    assert!(matching_json_delimiter("{", 0, b'{', b'}').is_none());
    // json_string_end on a non-string start.
    assert!(json_string_end("abc", 0).is_none());
    // Unterminated string.
    assert!(json_string_end("\"abc", 0).is_none());
}

#[test]
fn read_int_helpers_reject_truncation() {
    assert!(read_u16(&[0], 0).is_err());
    assert!(read_u32(&[0, 0], 0).is_err());
    assert!(read_u64(&[0, 0, 0], 0).is_err());
    assert_eq!(read_u16(&[1, 0], 0).unwrap(), 1);
    assert_eq!(read_u32(&[1, 0, 0, 0], 0).unwrap(), 1);
    assert_eq!(read_u64(&[1, 0, 0, 0, 0, 0, 0, 0], 0).unwrap(), 1);
}

#[test]
fn read_mfp_header_reads_real_package_fixture() {
    let header = read_mfp_header(&fixture_mfp()).expect("valid fixture header");
    assert_eq!(header.name, "package_simple");
    assert_eq!(header.container_major, 1);
    assert_eq!(header.container_minor, 0);
    assert!(header.binary_repr_length > 0);
}

#[test]
fn external_package_function_types_from_files_reads_exports() {
    // The strict variant reads the real fixture's exported function types.
    let signatures =
        external_package_function_types_from_files(&[fixture_mfp()]).expect("reads exports");
    assert!(
        !signatures.is_empty(),
        "fixture should export at least one function"
    );
    // Every exported function name is package-qualified.
    for name in signatures.keys() {
        assert!(name.starts_with("package_simple."), "{name}");
    }
    // Each decodes to a typed FUNC signature that renders back as one
    // (plan-105-A: the decode is the only parse, and it is lossless).
    for signature in signatures.values() {
        assert!(matches!(
            signature.signature_type(),
            crate::types::ParameterType::Func(..)
        ));
        assert!(signature.signature_type().name().contains("FUNC("));
    }
}

#[test]
fn external_package_function_types_lossy_matches_strict_for_valid_files() {
    let strict = external_package_function_types_from_files(&[fixture_mfp()]).unwrap();
    let lossy = external_package_function_types_from_files_lossy(&[fixture_mfp()]);
    assert_eq!(strict.len(), lossy.len());
}

#[test]
fn external_package_function_types_lossy_skips_bad_files() {
    // A path that is not a valid .mfp is silently skipped by the lossy path.
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.mfp");
    fs::write(&bad, b"not an mfp").unwrap();
    assert!(external_package_function_types_from_files_lossy(&[bad]).is_empty());
}

#[test]
fn external_package_function_types_from_files_propagates_error() {
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.mfp");
    fs::write(&bad, b"not an mfp").unwrap();
    assert!(external_package_function_types_from_files(&[bad]).is_err());
}

#[test]
fn read_mfp_header_rejects_ed25519_wrong_signature_length() {
    // signature type 1 (Ed25519) but a length other than 64 is rejected.
    let mut bytes = build_mfp("p", "1");
    let len = bytes.len();
    bytes[len - 6..len - 4].copy_from_slice(&1u16.to_le_bytes()); // type = 1
    bytes[len - 4..].copy_from_slice(&10u32.to_le_bytes()); // length = 10
    let (_dir, path) = write_temp(&bytes);
    let err = header_err(&path);
    assert!(err.contains("Ed25519 .mfp package must have a 64 byte signature"));
}

#[test]
fn read_mfp_header_rejects_truncated_binary_hash() {
    // Cut the buffer right after the last variable-length field so the fixed
    // 32-byte packageBinaryHash read runs past the end.
    let full = build_mfp("p", "1");
    // The trailing fixed section is 32 (hash) + 8 (len) + 2 + 4 = 46 bytes.
    let bytes = full[..full.len() - 46 + 4].to_vec();
    let (_dir, path) = write_temp(&bytes);
    assert!(read_mfp_header(&path).is_err());
}

#[test]
fn project_json_with_updated_ident_key_rewrites_snake_case_field() {
    // A pre-existing snake_case `ident_key` field is rewritten in place.
    let contents = "{\n  \"packages\": [\n    {\n      \"name\": \"dep\",\n      \"ident_key\": \"OLD\"\n    }\n  ]\n}\n";
    let out = project_json_with_updated_ident_key(contents, "dep", "NEW").unwrap();
    assert!(out.contains("\"NEW\""));
    assert!(!out.contains("OLD"));
    out.parse::<JsonValue>().expect("valid json");
}

#[test]
fn project_json_with_updated_ident_key_rejects_malformed_entry() {
    // The packages array bounds resolve, but the first `{` inside never
    // closes (no `}` anywhere after it), so matching_json_delimiter returns
    // None -> "malformed project.json `packages` entry".
    let contents = "{ \"packages\": [ { \"name\": \"dep\" ] ";
    let err = project_json_with_updated_ident_key(contents, "dep", "K").unwrap_err();
    assert!(
        err.contains("malformed project.json `packages` entry"),
        "got: {err}"
    );
}

#[test]
fn read_mfp_string_rejects_invalid_utf8() {
    // A name field with invalid UTF-8 bytes is rejected.
    let mut buf = Vec::new();
    buf.extend_from_slice(&MFP_MAGIC);
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&0u32.to_le_bytes());
    push_field(&mut buf, &[0xff, 0xfe]); // invalid UTF-8 name
    let (_dir, path) = write_temp(&buf);
    let err = header_err(&path);
    assert!(err.contains("not valid UTF-8"));
}

// --- imported type-def conversion (pure) ------------------------------

fn type_field(name: &str, ty: &str) -> binary_repr::BinaryReprTypeField {
    binary_repr::BinaryReprTypeField {
        name: name.to_string(),
        type_: ty.to_string(),
        visibility: binary_repr::BinaryReprTypeVisibility::Export,
    }
}

fn type_export(
    name: &str,
    kind: binary_repr::BinaryReprExportKind,
) -> binary_repr::BinaryReprTypeExport {
    binary_repr::BinaryReprTypeExport {
        name: name.to_string(),
        kind,
        fields: Vec::new(),
        variants: Vec::new(),
        members: Vec::new(),
        foreign_owner: None,
    }
}

/// The package's own names, as `package_type_owners` would report them:
/// each declared by `pkg`.
fn owned(names: &[&str]) -> PackageTypeOwners {
    names
        .iter()
        .map(|name| (name.to_string(), "pkg".to_string()))
        .collect()
}

/// bug-632: an importer sees the record under its package-qualified identity
/// (spec §13), and a field typed with another of the package's own types is
/// qualified too; a scalar field is untouched.
#[test]
fn imported_type_def_maps_a_record_export() {
    let mut export = type_export("Point", binary_repr::BinaryReprExportKind::Type);
    export.fields = vec![
        type_field("x", "Integer"),
        type_field("y", "Integer"),
        type_field("next", "List OF Point"),
    ];
    let def = imported_type_def(export, "pkg", &owned(&["Point"])).expect("record maps to a def");
    assert_eq!(def.name, "pkg.Point");
    assert!(matches!(def.kind, ir::ImportedTypeKind::Record));
    assert_eq!(def.fields.len(), 3);
    assert_eq!(def.fields[0].name, "x");
    assert_eq!(def.fields[0].type_.name(), "Integer");
    assert_eq!(def.fields[1].name, "y");
    assert_eq!(def.fields[2].type_.name(), "List OF pkg.Point");
    assert!(def.variants.is_empty());
    assert!(def.members.is_empty());
}

#[test]
fn imported_type_def_maps_a_union_export_with_variant_fields() {
    let mut export = type_export("Shape", binary_repr::BinaryReprExportKind::Union);
    export.variants = vec![
        binary_repr::BinaryReprTypeVariant {
            name: "Circle".to_string(),
            fields: vec![type_field("r", "Float")],
        },
        binary_repr::BinaryReprTypeVariant {
            name: "Empty".to_string(),
            fields: Vec::new(),
        },
    ];
    let def = imported_type_def(export, "pkg", &owned(&["Shape", "Circle", "Empty"]))
        .expect("union maps to a def");
    assert!(matches!(def.kind, ir::ImportedTypeKind::Union));
    assert_eq!(def.name, "pkg.Shape");
    assert_eq!(def.variants.len(), 2);
    assert_eq!(def.variants[0].name, "pkg.Circle");
    assert_eq!(def.variants[0].fields.len(), 1);
    assert_eq!(def.variants[0].fields[0].name, "r");
    assert_eq!(def.variants[0].fields[0].type_.name(), "Float");
    assert!(def.variants[1].fields.is_empty());
}

#[test]
fn imported_type_def_maps_an_enum_export() {
    let mut export = type_export("Color", binary_repr::BinaryReprExportKind::Enum);
    export.members = vec!["Red".to_string(), "Green".to_string()];
    let def = imported_type_def(export, "pkg", &owned(&["Color"])).expect("enum maps to a def");
    assert!(matches!(def.kind, ir::ImportedTypeKind::Enum));
    assert_eq!(def.members, vec!["Red".to_string(), "Green".to_string()]);
    assert!(def.fields.is_empty());
    assert!(def.variants.is_empty());
}

#[test]
fn imported_type_def_drops_function_exports() {
    // Only record/union/enum exports become type defs; a callable export is
    // dropped (it reaches the build through `external_package_function_types`).
    let kind = binary_repr::BinaryReprExportKind::Func;
    assert!(imported_type_def(type_export("callable", kind), "pkg", &owned(&[])).is_none());
}

// --- imported type defs / resource closers from real fixtures ---------

#[test]
fn imported_type_defs_from_files_reads_a_record_export_fixture() {
    let dir = crate::testutil::fixture_dir("project-record-comparable-package-valid");
    let pkg = dir.join("packages").join("package_record_comparable.mfp");
    let defs = imported_type_defs_from_files(&[pkg]);
    assert!(!defs.is_empty(), "fixture exports at least one type");
    assert!(
        defs.iter()
            .any(|def| matches!(def.kind, ir::ImportedTypeKind::Record)),
        "fixture exports a record type"
    );
}

#[test]
fn imported_type_defs_from_files_skips_unreadable_packages() {
    // A file that is not a valid `.mfp` is silently skipped (line 456).
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("bad.mfp");
    fs::write(&bad, b"not an mfp").unwrap();
    assert!(imported_type_defs_from_files(&[bad]).is_empty());
}

#[test]
fn imported_type_defs_empty_when_no_packages() {
    let value = json(r#"{"name":"p","version":"1"}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let dir = tempfile::tempdir().unwrap();
    assert!(imported_type_defs(dir.path(), manifest).is_empty());
}

#[test]
fn imported_type_defs_reads_from_an_installed_package() {
    let dir = crate::testutil::fixture_dir("project-record-comparable-package-valid");
    let value =
        json(r#"{"name":"p","version":"1","packages":[{"name":"package_record_comparable"}]}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let defs = imported_type_defs(&dir, manifest);
    assert!(
        defs.iter()
            .any(|def| matches!(def.kind, ir::ImportedTypeKind::Record)),
        "the installed package's record export is surfaced"
    );
}

#[test]
fn imported_resource_closers_skips_builtin_backed_resources() {
    // `resource_state_export_valid` exports only builtin-backed resources
    // (a `UNION Stream` of `File`/`Socket`). A built-in is authoritative and
    // already seeded, so the importer registers no closer for it — the loop
    // hits the `is_resource_type` skip and yields nothing.
    let dir = crate::testutil::fixture_dir("resource-state-import-rt");
    let value =
        json(r#"{"name":"p","version":"1","packages":[{"name":"resource_state_export_valid"}]}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    assert!(imported_resource_closers(&dir, manifest).is_empty());
}

#[test]
fn imported_resource_closers_registers_a_native_resource() {
    let dir = crate::testutil::fixture_dir("native-resource-import-valid");
    let value =
        json(r#"{"name":"p","version":"1","packages":[{"name":"native_resource_link_valid"}]}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let closers = imported_resource_closers(&dir, manifest);
    assert!(
        !closers.is_empty(),
        "native package exports a closable resource"
    );
    // bug-632: registered ONLY under `<pkg>.<Type>`. Spec §13 refuses the bare
    // spelling of an imported type, and the bare row this used to assert is
    // what let `RES h AS Db` resolve to an imported resource.
    assert!(
        closers.iter().all(|r| r.type_name.contains('.')),
        "every resource is registered package-qualified: {closers:?}"
    );
    // A native resource's bare close alias is resolved to a dotted
    // `<pkg>.<alias>` op, and no close op is empty.
    assert!(
        closers.iter().any(|r| r.close_function.contains('.')),
        "native close op is dotted: {closers:?}"
    );
    assert!(closers.iter().all(|r| !r.close_function.is_empty()));
}

#[test]
fn imported_resource_closers_empty_when_packages_unreadable() {
    // installed_package_files errors (a declared package is not installed),
    // so the lossy closer reader returns no pairs (lines 295-297).
    let value = json(r#"{"name":"p","version":"1","packages":[{"name":"absent_dep"}]}"#);
    let manifest = value.get::<HashMap<String, JsonValue>>().unwrap();
    let dir = tempfile::tempdir().unwrap();
    assert!(imported_resource_closers(dir.path(), manifest).is_empty());
}

#[test]
fn common_dependency_fields_rejects_a_non_string_name() {
    // `name` is present but is a JSON number, not a string, so the
    // `.get::<String>()?` short-circuits and the entry is dropped (line 607).
    let mut obj = HashMap::new();
    obj.insert("name".to_string(), JsonValue::Number(42.0));
    assert!(project_package_dependency(&JsonValue::Object(obj)).is_none());
}

#[test]
fn external_package_function_types_lossy_skips_a_corrupt_body() {
    // The container header is valid but the binary-representation body is
    // corrupt: read_mfp_header succeeds while read_package_exports fails, so
    // the lossy reader skips the package (line 516) instead of propagating.
    let header = read_mfp_header(&fixture_mfp()).unwrap();
    let mut bytes = fs::read(fixture_mfp()).unwrap();
    let body_start = bytes.len() - header.binary_repr_length;
    for byte in &mut bytes[body_start..body_start + 8] {
        *byte ^= 0xff;
    }
    let (_dir, path) = write_temp(&bytes);
    // The fixed header is untouched, so it still parses.
    assert!(read_mfp_header(&path).is_ok());
    assert!(
        external_package_function_types_from_files_lossy(&[path]).is_empty(),
        "corrupt body must yield no signatures"
    );
}
