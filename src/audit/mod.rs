//! `mfb audit` — reports fallible call sites, auto-propagation, `TRAP` recovery,
//! resource cleanup, native links, permissions, dependency versions, lockfile
//! mismatches, and verifier status for an MFBASIC project.
//!
//! Audit reuses the same project loader, front-end pipeline, package reader, and
//! `.mfp` helpers that builds use, so its output matches real build behavior. It
//! never executes user code.

mod collect;
mod json;
mod report;
mod text;

/// The canonical hash of a project's declared dependency request set, reused
/// by the `mfb.lock` writer (plan-10-B2) so `mfb audit` sees a fresh lock as
/// current.
pub(crate) use collect::project_hash;

use std::path::PathBuf;

use collect::AuditInputs;
use report::Severity;

pub enum AuditFormat {
    Text,
    Json,
}

pub struct AuditOptions {
    pub location: PathBuf,
    pub format: AuditFormat,
    pub locked: bool,
}

/// Parses `audit` arguments. Unknown options and invalid `--format` values are
/// usage errors (exit code 2).
pub fn parse_options(args: Vec<String>) -> Result<AuditOptions, String> {
    let mut location = None;
    let mut format = AuditFormat::Text;
    let mut locked = false;
    let mut iter = args.into_iter();

    while let Some(arg) = iter.next() {
        if arg == "--format" {
            let Some(value) = iter.next() else {
                return Err("mfb audit --format requires text or json".to_string());
            };
            format = parse_format(&value)?;
        } else if let Some(value) = arg.strip_prefix("--format=") {
            format = parse_format(value)?;
        } else if arg == "--locked" {
            locked = true;
        } else if arg.starts_with('-') {
            return Err(format!("unknown audit option `{arg}`"));
        } else if location.replace(PathBuf::from(&arg)).is_some() {
            return Err("mfb audit accepts at most one [path]".to_string());
        }
    }

    Ok(AuditOptions {
        location: location.unwrap_or_else(|| PathBuf::from(".")),
        format,
        locked,
    })
}

fn parse_format(value: &str) -> Result<AuditFormat, String> {
    match value {
        "text" => Ok(AuditFormat::Text),
        "json" => Ok(AuditFormat::Json),
        other => Err(format!(
            "invalid --format value `{other}` (expected text or json)"
        )),
    }
}

/// Runs the audit and returns the process exit code:
/// `0` clean, `1` error-severity findings, `3` unreadable/malformed input.
pub fn run(options: &AuditOptions) -> i32 {
    let project_path = options.location.join("project.json");
    let Ok(manifest) = crate::manifest::validate_project_manifest(&project_path) else {
        return 3;
    };

    let Some(project_name) = manifest
        .get("name")
        .and_then(|value| value.get::<String>())
        .cloned()
    else {
        return 3;
    };

    let Ok(ast) = crate::ast::parse_project(&project_name, &options.location, &manifest) else {
        return 3;
    };
    if crate::resolver::resolve_project(&options.location, &manifest, &ast).is_err() {
        return 3;
    }
    let Ok(concrete_ast) = crate::monomorph::monomorphize_project(&options.location, &ast) else {
        return 3;
    };
    if crate::resolver::resolve_project_with(&options.location, &manifest, &concrete_ast, false)
        .is_err()
    {
        return 3;
    }
    let Ok(entry) =
        crate::manifest::entry::validate_entry_point(&options.location, &manifest, &concrete_ast)
    else {
        return 3;
    };
    if crate::syntaxcheck::check_project(&options.location, &concrete_ast).is_err() {
        return 3;
    }

    let inputs = AuditInputs {
        project_dir: &options.location,
        root_display: options.location.to_string_lossy().replace('\\', "/"),
        manifest: &manifest,
        ast: &ast,
        kind: crate::manifest::project_kind(&manifest).to_string(),
        entry: entry.map(|entry| entry.name),
        locked: options.locked,
    };

    let report = collect::collect(&inputs);
    let output = match options.format {
        AuditFormat::Text => text::render(&report),
        AuditFormat::Json => json::render(&report),
    };
    print!("{output}");

    if report
        .findings
        .iter()
        .any(|finding| finding.severity == Severity::Error)
    {
        1
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
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
        let opts =
            parse_options(args(&["sub/dir", "--format", "json", "--locked"])).expect("options");
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
}
