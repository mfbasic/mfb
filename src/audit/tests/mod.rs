use super::*;

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

fn err(items: &[&str]) -> String {
    match parse_options(args(items)) {
        Ok(_) => panic!("expected error for {items:?}"),
        Err(message) => message,
    }
}

#[test]
fn default_options_are_current_dir_text_unlocked() {
    let opts = parse_options(Vec::new()).expect("defaults");
    assert_eq!(opts.location, PathBuf::from("."));
    assert!(matches!(opts.format, AuditFormat::Text));
    assert!(!opts.locked);
}

#[test]
fn parses_path_and_flags() {
    let opts = parse_options(args(&["sub/dir", "--format", "json", "--locked"])).expect("options");
    assert_eq!(opts.location, PathBuf::from("sub/dir"));
    assert!(matches!(opts.format, AuditFormat::Json));
    assert!(opts.locked);
}

#[test]
fn parses_inline_format() {
    let opts = parse_options(args(&["--format=text"])).expect("options");
    assert!(matches!(opts.format, AuditFormat::Text));
    let opts = parse_options(args(&["--format=json"])).expect("options");
    assert!(matches!(opts.format, AuditFormat::Json));
}

#[test]
fn format_requires_value() {
    assert!(err(&["--format"]).contains("requires text or json"));
}

#[test]
fn invalid_format_value_is_error() {
    assert!(err(&["--format", "yaml"]).contains("invalid --format value"));
    assert!(err(&["--format=xml"]).contains("invalid --format value"));
}

#[test]
fn unknown_option_is_error() {
    assert!(err(&["--nope"]).contains("unknown audit option"));
}

#[test]
fn at_most_one_path() {
    assert!(err(&["a", "b"]).contains("at most one"));
}

#[test]
fn parse_format_helper_direct() {
    assert!(matches!(parse_format("text"), Ok(AuditFormat::Text)));
    assert!(matches!(parse_format("json"), Ok(AuditFormat::Json)));
    assert!(parse_format("other").is_err());
}

#[test]
fn run_on_missing_project_returns_three() {
    let dir = tempfile::tempdir().unwrap();
    let options = AuditOptions {
        location: dir.path().to_path_buf(),
        format: AuditFormat::Text,
        locked: false,
    };
    assert_eq!(run(&options), 3);
}

#[test]
fn run_on_manifest_without_required_fields_returns_three() {
    let dir = tempfile::tempdir().unwrap();
    // A syntactically valid manifest that omits required fields fails
    // validation and yields exit code 3.
    std::fs::write(dir.path().join("project.json"), "{}").unwrap();
    let options = AuditOptions {
        location: dir.path().to_path_buf(),
        format: AuditFormat::Text,
        locked: false,
    };
    assert_eq!(run(&options), 3);
}

#[test]
fn run_on_unparseable_source_returns_three() {
    let dir = tempfile::tempdir().unwrap();
    let src = dir.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        concat!(
            "{\n",
            "  \"name\": \"app\",\n",
            "  \"version\": \"0.1.0\",\n",
            "  \"mfb\": \"1.0\",\n",
            "  \"kind\": \"executable\",\n",
            "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }],\n",
            "  \"entry\": \"main\"\n",
            "}\n"
        ),
    )
    .unwrap();
    // Unparseable source fails the front-end and yields exit code 3.
    std::fs::write(src.join("main.mfb"), "FUNC main( AS\n").unwrap();
    let options = AuditOptions {
        location: dir.path().to_path_buf(),
        format: AuditFormat::Text,
        locked: false,
    };
    assert_eq!(run(&options), 3);
}

#[test]
fn run_on_valid_project_emits_report_and_returns_zero() {
    // Copy a package-free single-file fixture project into a temp dir and
    // audit it end-to-end in both formats.
    let src_root = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/rt-behavior/math/math_package_valid"
    ));
    if !src_root.exists() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let dest = dir.path();
    std::fs::create_dir_all(dest.join("src")).unwrap();
    std::fs::copy(src_root.join("project.json"), dest.join("project.json")).unwrap();
    std::fs::copy(src_root.join("src/main.mfb"), dest.join("src/main.mfb")).unwrap();

    let text_options = AuditOptions {
        location: dest.to_path_buf(),
        format: AuditFormat::Text,
        locked: false,
    };
    // A clean program has no error findings.
    assert_eq!(run(&text_options), 0);

    let json_options = AuditOptions {
        location: dest.to_path_buf(),
        format: AuditFormat::Json,
        locked: false,
    };
    assert_eq!(run(&json_options), 0);
}
