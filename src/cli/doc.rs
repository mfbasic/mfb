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
    let path = path.map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
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
