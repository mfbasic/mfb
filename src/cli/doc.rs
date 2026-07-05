use std::fs;
use std::path::{Path, PathBuf};

use crate::ast;
use crate::doc;
use crate::manifest::validate_project_manifest;
use crate::resolver;
use crate::USAGE;

/// `mfb doc <path> [--out <file>]` — render HTML documentation from source
/// (plan-09-doc.md §6.1). Returns a process exit code.
pub(crate) fn run_doc_command(args: &[String]) -> i32 {
    let mut path: Option<&String> = None;
    let mut out: Option<&String> = None;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--out" => {
                index += 1;
                let Some(file) = args.get(index) else {
                    eprintln!("error: mfb doc --out requires a file\n\n{USAGE}");
                    return 2;
                };
                out = Some(file);
            }
            flag if flag.starts_with("--") => {
                eprintln!("error: unknown flag `{flag}`\n\n{USAGE}");
                return 2;
            }
            _ => {
                if path.is_some() {
                    eprintln!("error: mfb doc accepts exactly one <path>\n\n{USAGE}");
                    return 2;
                }
                path = Some(&args[index]);
            }
        }
        index += 1;
    }
    // Like `mfb build`, the path defaults to the current directory.
    let path = path
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let out_path = out
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("doc.html"));

    match build_source_doc_page(&path) {
        Ok((page, valid)) => {
            let html = doc::render_html(&page);
            if let Err(err) = fs::write(&out_path, html) {
                eprintln!("error: failed to write '{}': {err}", out_path.display());
                return 1;
            }
            println!("Wrote documentation to {}", out_path.display());
            // Diagnostics for invalid blocks were already printed to stderr.
            if valid {
                0
            } else {
                1
            }
        }
        Err(err) => {
            eprintln!("error: {err}");
            1
        }
    }
}

/// Parse a source path (project directory or single `.mfb` file), validate its
/// `DOC` blocks, and build a renderable page. The bool is `false` when any block
/// failed validation (diagnostics already emitted).
fn build_source_doc_page(path: &Path) -> Result<(doc::DocPage, bool), String> {
    if path.is_dir() {
        let project_path = path.join("project.json");
        let manifest = validate_project_manifest(&project_path)
            .map_err(|_| "project validation failed".to_string())?;
        let name = manifest
            .get("name")
            .and_then(|value| value.get::<String>())
            .cloned()
            .unwrap_or_else(|| "package".to_string());
        let ast = ast::parse_project(&name, path, &manifest)
            .map_err(|_| "failed to parse project source".to_string())?;
        let valid = resolver::resolve_project(path, &manifest, &ast).is_ok();
        Ok((doc::from_source(&ast), valid))
    } else {
        let contents = fs::read_to_string(path)
            .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
        let stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("doc")
            .to_string();
        let display = path.to_string_lossy().replace('\\', "/");
        let file = ast::parse_source(path, &display, &contents)
            .map_err(|_| "failed to parse source file".to_string())?;
        let project = ast::AstProject {
            name: stem,
            files: vec![file],
        };
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let valid = resolver::validate_project_docs(parent, &project);
        Ok((doc::from_source(&project), valid))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn s(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    const PROJECT_MANIFEST: &str = concat!(
        "{\n",
        "  \"name\": \"docapp\",\n",
        "  \"version\": \"0.1.0\",\n",
        "  \"mfb\": \"1.0\",\n",
        "  \"kind\": \"executable\",\n",
        "  \"sources\": [\n",
        "    { \"root\": \"src\", \"role\": \"main\", \"include\": [\"**/*.mfb\"] }\n",
        "  ],\n",
        "  \"entry\": \"main\",\n",
        "  \"targets\": [\"native\"]\n",
        "}\n"
    );

    #[test]
    fn build_source_doc_page_single_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("prog.mfb");
        fs::write(&file, "SUB main()\nEND SUB\n").unwrap();
        let (_, valid) = build_source_doc_page(&file).expect("page");
        assert!(valid);
    }

    #[test]
    fn build_source_doc_page_unparseable_file_errors() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("bad.mfb");
        fs::write(&file, "FUNC main( AS\n").unwrap();
        assert!(build_source_doc_page(&file).is_err());
    }

    #[test]
    fn build_source_doc_page_missing_file_errors() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("nope.mfb");
        match build_source_doc_page(&missing) {
            Err(err) => assert!(err.contains("failed to read")),
            Ok(_) => panic!("missing file must error"),
        }
    }

    #[test]
    fn build_source_doc_page_project_dir() {
        let dir = tempdir().unwrap();
        let src = dir.path().join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(dir.path().join("project.json"), PROJECT_MANIFEST).unwrap();
        fs::write(src.join("main.mfb"), "SUB main()\nEND SUB\n").unwrap();
        let (_, valid) = build_source_doc_page(dir.path()).expect("page");
        assert!(valid);
    }

    #[test]
    fn build_source_doc_page_bad_manifest_errors() {
        let dir = tempdir().unwrap();
        // A directory with no project.json fails validation.
        match build_source_doc_page(dir.path()) {
            Err(err) => assert!(err.contains("project validation failed")),
            Ok(_) => panic!("missing manifest must error"),
        }
    }

    #[test]
    fn run_doc_command_writes_html_for_file() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("prog.mfb");
        fs::write(&file, "SUB main()\nEND SUB\n").unwrap();
        let out = dir.path().join("out.html");
        let code = run_doc_command(&s(&[
            file.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]));
        assert_eq!(code, 0);
        assert!(out.is_file());
    }

    #[test]
    fn run_doc_command_reports_build_error() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("bad.mfb");
        fs::write(&file, "FUNC main( AS\n").unwrap();
        let out = dir.path().join("out.html");
        let code = run_doc_command(&s(&[
            file.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
        ]));
        assert_eq!(code, 1);
    }

    #[test]
    fn run_doc_command_out_requires_a_value() {
        assert_eq!(run_doc_command(&s(&["--out"])), 2);
    }

    #[test]
    fn run_doc_command_rejects_unknown_flag() {
        assert_eq!(run_doc_command(&s(&["--bogus"])), 2);
    }

    #[test]
    fn run_doc_command_rejects_two_paths() {
        assert_eq!(run_doc_command(&s(&["a.mfb", "b.mfb"])), 2);
    }

    #[test]
    fn run_doc_command_write_failure_returns_one() {
        let dir = tempdir().unwrap();
        let file = dir.path().join("prog.mfb");
        fs::write(&file, "SUB main()\nEND SUB\n").unwrap();
        // Point --out at a path whose parent is not a directory.
        let bad_out = file.join("cannot").join("write.html");
        let code = run_doc_command(&s(&[
            file.to_str().unwrap(),
            "--out",
            bad_out.to_str().unwrap(),
        ]));
        assert_eq!(code, 1);
    }
}
