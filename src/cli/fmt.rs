use std::fs;
use std::path::{Path, PathBuf};

use crate::ast;
use crate::fmt;
use crate::manifest::validate_project_manifest;
use crate::rules;
use crate::USAGE;

/// `mfb fmt [--indent N] [location]` — format MFBASIC source in place. Like
/// `mfb build` and `mfb doc`, the location defaults to the current directory and
/// may be a project directory (formats every selected `.mfb` file) or a single
/// `.mfb` file. Returns a process exit code.
pub(crate) fn run_fmt_command(args: &[String]) -> i32 {
    let mut location: Option<&String> = None;
    let mut indent: usize = 2;
    let mut check = false;
    let mut index = 0;
    while index < args.len() {
        let arg = &args[index];
        if arg == "--check" {
            check = true;
        } else if let Some(value) = arg.strip_prefix("--indent=") {
            match parse_indent(value) {
                Ok(width) => indent = width,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    return 2;
                }
            }
        } else if arg == "--indent" {
            index += 1;
            let Some(value) = args.get(index) else {
                eprintln!("error: mfb fmt --indent requires a value\n\n{USAGE}");
                return 2;
            };
            match parse_indent(value) {
                Ok(width) => indent = width,
                Err(err) => {
                    eprintln!("error: {err}\n\n{USAGE}");
                    return 2;
                }
            }
        } else if arg.starts_with("--") {
            eprintln!("error: unknown flag `{arg}`\n\n{USAGE}");
            return 2;
        } else if location.replace(arg).is_some() {
            eprintln!("error: mfb fmt accepts exactly one [location]\n\n{USAGE}");
            return 2;
        }
        index += 1;
    }

    let path = location
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    match format_path(&path, indent, check) {
        Ok(true) => 0,
        // `--check` found files that are not formatted (mfbasic.md §22).
        Ok(false) => 1,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

fn parse_indent(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .map_err(|_| format!("mfb fmt --indent requires a non-negative integer (got `{value}`)"))
}

/// Format the source selected by `path`. Without `check`, rewrites files in
/// place and prints one line per file changed. With `check`, writes nothing and
/// reports files that are not formatted. Returns `Ok(false)` only in check mode
/// when at least one file would change (so the caller exits non-zero).
fn format_path(path: &Path, indent: usize, check: bool) -> Result<bool, String> {
    let files = if path.is_file() {
        if path.extension().and_then(|ext| ext.to_str()) != Some("mfb") {
            return Err(format!("`{}` is not a .mfb source file", path.display()));
        }
        vec![path.to_path_buf()]
    } else if path.is_dir() {
        let project_path = path.join("project.json");
        let manifest = validate_project_manifest(&project_path)
            .map_err(|_| "project validation failed".to_string())?;
        ast::selected_source_paths(path, &manifest)
            .map_err(|_| "failed to enumerate project source files".to_string())?
    } else {
        return Err(format!("no such file or directory: `{}`", path.display()));
    };

    let mut changed = 0;
    for file in &files {
        let original = fs::read_to_string(file)
            .map_err(|err| format!("failed to read '{}': {err}", file.display()))?;
        let formatted = fmt::format_source(&original, indent);
        if formatted == original {
            continue;
        }
        changed += 1;
        if check {
            println!("Not formatted: {}", file.display());
        } else {
            fs::write(file, &formatted)
                .map_err(|err| format!("failed to write '{}': {err}", file.display()))?;
            println!("Formatted {}", file.display());
        }
    }

    if check {
        if changed > 0 {
            rules::show_general_diagnostic(
                "FMT_CHECK_FAILED",
                &format!("{changed} file(s) are not formatted; run `mfb fmt` to fix."),
            );
            return Ok(false);
        }
        println!("All {} file(s) already formatted", files.len());
    } else if changed == 0 {
        println!("Already formatted: {} file(s) unchanged", files.len());
    }
    Ok(true)
}
