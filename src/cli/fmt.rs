use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::ast;
use crate::cli::help::USAGE;
use crate::fmt;
use crate::manifest::validate_project_manifest;
use crate::rules;

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
        // `--check` found files that are not formatted (22_test-framework.md).
        Ok(false) => 1,
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

/// Upper bound on `--indent`. `indent_str` builds `" ".repeat(level * width)`, so
/// an unbounded width drives a multiply-/capacity-overflow panic or a multi-GB
/// allocation on any nested source (bug-220); no real style uses a deep indent.
const MAX_INDENT: usize = 256;

fn parse_indent(value: &str) -> Result<usize, String> {
    let width = value
        .parse::<usize>()
        .map_err(|_| format!("mfb fmt --indent requires a non-negative integer (got `{value}`)"))?;
    if width > MAX_INDENT {
        return Err(format!(
            "mfb fmt --indent must be between 0 and {MAX_INDENT} (got `{value}`)"
        ));
    }
    Ok(width)
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
        let formatted = match fmt::format_source(&original, indent) {
            Ok(formatted) => formatted,
            // bug-502: past the nesting cap the output would grow quadratically
            // in the input; report where the source crossed it and leave the
            // file exactly as it was. The parser's block-depth rule names the
            // same condition, so no new rule code is minted.
            Err(fmt::FormatError::NestingTooDeep { line }) => {
                rules::show_diagnostic(
                    "MFB_PARSE_BLOCK_TOO_DEEP",
                    &format!(
                        "Statement block nesting is too deep; `mfb fmt` re-indents at most {} \
                         nested blocks and left the file unchanged.",
                        fmt::MAX_NESTING_DEPTH
                    ),
                    file,
                    line,
                    1,
                    1,
                );
                return Err(format!(
                    "'{}' is nested too deeply to format",
                    file.display()
                ));
            }
        };
        if formatted == original {
            continue;
        }
        changed += 1;
        if check {
            println!("Not formatted: {}", file.display());
        } else {
            replace_contents(file, &formatted)
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

/// Replace `path`'s contents with `contents` without ever exposing a partial
/// file: the text is written in full (and synced) to a sibling temporary file,
/// which is then renamed over the original. A failure at any step leaves the
/// original byte-for-byte intact and removes the temporary, so an interrupted
/// or out-of-memory format can never truncate the user's source (bug-502). The
/// original's permission bits carry over to the replacement.
///
/// The path is canonicalized first so a symlinked source keeps its write-through
/// semantics (the target is replaced, as `fs::write` replaced it) rather than
/// the link itself becoming a regular file. The original is opened for writing
/// (without truncation) before anything is created, so a read-only file still
/// fails with the same permission error it always did instead of being swapped
/// out from under its mode.
fn replace_contents(path: &Path, contents: &str) -> io::Result<()> {
    let target = fs::canonicalize(path)?;
    let permissions = fs::OpenOptions::new()
        .write(true)
        .open(&target)?
        .metadata()?
        .permissions();
    let directory = target.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "source file has no parent directory",
        )
    })?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("source.mfb");
    let temp = directory.join(format!(".{name}.mfb-fmt-{}.tmp", std::process::id()));
    let written = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        file.set_permissions(permissions)?;
        drop(file);
        fs::rename(&temp, &target)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&temp);
    }
    written
}

#[cfg(test)]
mod tests;
