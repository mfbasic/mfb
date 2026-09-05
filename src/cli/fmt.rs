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
mod tests {
    use super::*;

    fn s(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| value.to_string()).collect()
    }

    // ---- bug-502: nesting cap and atomic write-back ----

    fn tower(opens: usize) -> String {
        let mut source = String::from("SUB main()\n");
        for _ in 0..opens {
            source.push_str("IF TRUE THEN\n");
        }
        source.push_str("io::print(\"x\")\n");
        for _ in 0..opens {
            source.push_str("END IF\n");
        }
        source.push_str("END SUB\n");
        source
    }

    #[test]
    fn format_path_refuses_a_too_deep_file_and_leaves_it_intact() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.mfb");
        let hostile = tower(2000);
        std::fs::write(&file, &hostile).expect("write");
        let err = format_path(&file, 2, false).unwrap_err();
        assert!(err.contains("nested too deeply"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), hostile);
        // Check mode reports the same refusal without writing.
        let err = format_path(&file, 2, true).unwrap_err();
        assert!(err.contains("nested too deeply"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), hostile);
        // No temporary was left beside the source.
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("main.mfb")]);
    }

    #[test]
    fn replace_contents_leaves_no_temporary_and_keeps_permissions() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.mfb");
        std::fs::write(&file, "SUB main()\nio::print(\"hi\")\nEND SUB\n").expect("write");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
        }
        assert_eq!(format_path(&file, 2, false), Ok(true));
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "SUB main()\n  io::print(\"hi\")\nEND SUB\n"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o640, "the replacement must keep the original's mode");
        }
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("main.mfb")]);
    }

    #[cfg(unix)]
    #[test]
    fn replace_contents_refuses_a_read_only_file_untouched() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.mfb");
        let original = "SUB main()\nio::print(\"hi\")\nEND SUB\n";
        std::fs::write(&file, original).expect("write");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o444)).unwrap();
        let err = format_path(&file, 2, false).unwrap_err();
        assert!(err.contains("failed to write"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
        let entries: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(entries, vec![std::ffi::OsString::from("main.mfb")]);
    }

    #[test]
    fn parse_indent_accepts_non_negative_and_rejects_junk() {
        assert_eq!(parse_indent("4"), Ok(4));
        assert_eq!(parse_indent("0"), Ok(0));
        let err = parse_indent("-1").unwrap_err();
        assert!(err.contains("non-negative integer"));
        assert!(parse_indent("abc").is_err());
    }

    #[test]
    fn run_fmt_rejects_bad_indent_value() {
        assert_eq!(run_fmt_command(&s(&["--indent=xx"])), 2);
        assert_eq!(run_fmt_command(&s(&["--indent", "xx"])), 2);
    }

    #[test]
    fn run_fmt_indent_flag_requires_a_value() {
        assert_eq!(run_fmt_command(&s(&["--indent"])), 2);
    }

    #[test]
    fn run_fmt_rejects_unknown_flag() {
        assert_eq!(run_fmt_command(&s(&["--nope"])), 2);
    }

    #[test]
    fn run_fmt_rejects_two_locations() {
        assert_eq!(run_fmt_command(&s(&["a.mfb", "b.mfb"])), 2);
    }

    #[test]
    fn run_fmt_reports_missing_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("does-not-exist");
        assert_eq!(run_fmt_command(&s(&[missing.to_str().unwrap()])), 1);
    }

    #[test]
    fn format_path_rejects_non_mfb_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("notes.txt");
        std::fs::write(&file, "hello").expect("write");
        let err = format_path(&file, 2, false).unwrap_err();
        assert!(err.contains("is not a .mfb source file"));
    }

    #[test]
    fn format_path_reports_missing_target() {
        let dir = tempfile::tempdir().expect("temp dir");
        let missing = dir.path().join("nope.mfb");
        let err = format_path(&missing, 2, false).unwrap_err();
        assert!(err.contains("no such file or directory"));
    }

    #[test]
    fn format_path_rewrites_a_single_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.mfb");
        // Poorly indented source that the formatter will change.
        std::fs::write(&file, "SUB main()\nio::print(\"hi\")\nEND SUB\n").expect("write");
        // Check mode reports a change without rewriting (Ok(false)).
        assert_eq!(format_path(&file, 2, true), Ok(false));
        let before = std::fs::read_to_string(&file).unwrap();
        // Apply mode rewrites (Ok(true)) and the file changes.
        assert_eq!(format_path(&file, 2, false), Ok(true));
        let after = std::fs::read_to_string(&file).unwrap();
        assert_ne!(before, after);
        // Re-running finds nothing to do (already formatted).
        assert_eq!(format_path(&file, 2, false), Ok(true));
        assert_eq!(format_path(&file, 2, true), Ok(true));
    }

    #[test]
    fn run_fmt_command_formats_a_file_end_to_end() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.mfb");
        std::fs::write(&file, "SUB main()\nio::print(\"hi\")\nEND SUB\n").expect("write");
        // First pass changes the file; check mode would then exit 1 -> 0.
        assert_eq!(run_fmt_command(&s(&[file.to_str().unwrap()])), 0);
        // Now formatted: --check passes (exit 0).
        assert_eq!(run_fmt_command(&s(&["--check", file.to_str().unwrap()])), 0);
    }

    #[test]
    fn run_fmt_accepts_valid_indent_both_forms() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.mfb");
        std::fs::write(&file, "SUB main()\nio::print(\"hi\")\nEND SUB\n").expect("write");
        // `--indent=4` form.
        assert_eq!(
            run_fmt_command(&s(&["--indent=4", file.to_str().unwrap()])),
            0
        );
        std::fs::write(&file, "SUB main()\nio::print(\"hi\")\nEND SUB\n").expect("write");
        // `--indent 4` split form.
        assert_eq!(
            run_fmt_command(&s(&["--indent", "4", file.to_str().unwrap()])),
            0
        );
    }

    #[test]
    fn format_path_formats_a_project_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let project = dir.path();
        std::fs::write(
            project.join("project.json"),
            concat!(
                "{\n",
                "  \"name\": \"app\",\n",
                "  \"version\": \"0.1.0\",\n",
                "  \"mfb\": \"1.0\",\n",
                "  \"kind\": \"executable\",\n",
                "  \"entry\": \"main\",\n",
                "  \"sources\": [{ \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }]\n",
                "}\n"
            ),
        )
        .expect("manifest");
        std::fs::create_dir_all(project.join("src")).expect("src dir");
        std::fs::write(
            project.join("src").join("main.mfb"),
            "SUB main()\nio::print(\"hi\")\nEND SUB\n",
        )
        .expect("source");
        // Formats every selected file in the project (apply mode).
        assert_eq!(format_path(project, 2, false), Ok(true));
        // In check mode, a now-formatted project reports Ok(true).
        assert_eq!(format_path(project, 2, true), Ok(true));
    }

    #[test]
    fn run_fmt_check_reports_unformatted_file() {
        let dir = tempfile::tempdir().expect("temp dir");
        let file = dir.path().join("main.mfb");
        std::fs::write(&file, "SUB main()\nio::print(\"hi\")\nEND SUB\n").expect("write");
        // Unformatted file in --check mode exits 1.
        assert_eq!(run_fmt_command(&s(&["--check", file.to_str().unwrap()])), 1);
    }
}
