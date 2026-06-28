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
    let Ok(entry) = crate::manifest::entry::validate_entry_point(&options.location, &manifest, &concrete_ast) else {
        return 3;
    };
    if crate::typecheck::check_project(&options.location, &concrete_ast).is_err() {
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
