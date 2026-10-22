use super::*;

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

// ---- bug-502: nesting cap and atomic write-back ----

fn tower(opens: usize) -> String {
    let mut source = String::from("FUNC main() AS Nothing\n");
    for _ in 0..opens {
        source.push_str("IF TRUE THEN\n");
    }
    source.push_str("io::print(\"x\")\n");
    for _ in 0..opens {
        source.push_str("END IF\n");
    }
    source.push_str("END FUNC\n");
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
    std::fs::write(
        &file,
        "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n",
    )
    .expect("write");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    assert_eq!(format_path(&file, 2, false), Ok(true));
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "FUNC main() AS Nothing\n  io::print(\"hi\")\nEND FUNC\n"
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
    let original = "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n";
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
    std::fs::write(
        &file,
        "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n",
    )
    .expect("write");
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
    std::fs::write(
        &file,
        "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n",
    )
    .expect("write");
    // First pass changes the file; check mode would then exit 1 -> 0.
    assert_eq!(run_fmt_command(&s(&[file.to_str().unwrap()])), 0);
    // Now formatted: --check passes (exit 0).
    assert_eq!(run_fmt_command(&s(&["--check", file.to_str().unwrap()])), 0);
}

#[test]
fn run_fmt_accepts_valid_indent_both_forms() {
    let dir = tempfile::tempdir().expect("temp dir");
    let file = dir.path().join("main.mfb");
    std::fs::write(
        &file,
        "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n",
    )
    .expect("write");
    // `--indent=4` form.
    assert_eq!(
        run_fmt_command(&s(&["--indent=4", file.to_str().unwrap()])),
        0
    );
    std::fs::write(
        &file,
        "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n",
    )
    .expect("write");
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
        "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n",
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
    std::fs::write(
        &file,
        "FUNC main() AS Nothing\nio::print(\"hi\")\nEND FUNC\n",
    )
    .expect("write");
    // Unformatted file in --check mode exits 1.
    assert_eq!(run_fmt_command(&s(&["--check", file.to_str().unwrap()])), 1);
}

/// bug-220: an indent wider than the cap is refused (both ends of the range
/// are accepted).
#[test]
fn parse_indent_refuses_a_width_past_the_cap() {
    assert_eq!(parse_indent("256"), Ok(MAX_INDENT));
    assert_eq!(
        parse_indent("257"),
        Err("mfb fmt --indent must be between 0 and 256 (got `257`)".to_string())
    );
}
