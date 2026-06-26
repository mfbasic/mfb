use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let error_codes_doc = manifest_dir.join("specifications/error_codes.md");
    println!("cargo:rerun-if-changed={}", error_codes_doc.display());

    // Man pages and spec pages share one discovery model: walk a tree, and any
    // directory holding an index file (`package.txt` / `spec.md`) is a package
    // named after the directory. Adding a topic is "drop a file"; adding a
    // package is "create a directory" — no edits here. Display order (and, for
    // man, the usage synopsis) lives in the runtime module, the only editorial
    // bits the filesystem can't express.
    //
    // Man pages are plain text; spec pages are Markdown rendered to the terminal
    // by `src/spec/render.rs`. Both embed via `include_str!` (zero runtime I/O).
    generate_doc_table(
        &manifest_dir.join("src/man"),
        "package.txt",
        "txt",
        "MAN_PACKAGES",
        &out_dir.join("man_generated.rs"),
    );
    generate_doc_table(
        &manifest_dir.join("src/spec"),
        "spec.md",
        "md",
        "SPEC_PACKAGES",
        &out_dir.join("spec_generated.rs"),
    );

    generate_errorcode_table(&error_codes_doc, &out_dir);
}

/// Discover every package under `root`, emit `cargo:rerun-if-changed` lines for
/// the tree, and write the generated `(name, index-text, &[(page, text)])`
/// table to `out_path` as `const_name`.
fn generate_doc_table(
    root: &Path,
    index_name: &str,
    page_ext: &str,
    const_name: &str,
    out_path: &Path,
) {
    let packages = doc_packages(root, index_name, page_ext);
    println!("cargo:rerun-if-changed={}", root.display());
    for package in &packages {
        println!("cargo:rerun-if-changed={}", package.dir.display());
        for page in std::iter::once(&package.index).chain(&package.pages) {
            println!("cargo:rerun-if-changed={}", page.display());
        }
    }

    let mut output = fs::File::create(out_path).expect("create generated doc source");
    write_doc_packages(&mut output, const_name, &packages);
}

/// Parse the "Runtime and Standard Package Errors" table in
/// `specifications/error_codes.md` and emit a generated `(name, integer)`
/// table for the built-in `errorCode` package. The doc is the single source of
/// truth (plan-06-errorcodes.md §4a); this keeps the package from drifting from
/// the canonical registry. Only the runtime `Err*` rows are exported — those are
/// the program-visible `Error.code` values, matching `errorCode::Err*` usage.
fn generate_errorcode_table(doc_path: &PathBuf, out_dir: &PathBuf) {
    let doc = fs::read_to_string(doc_path).expect("read specifications/error_codes.md");

    let mut in_section = false;
    let mut rows: Vec<(String, String)> = Vec::new();
    for line in doc.lines() {
        if line.starts_with("## ") {
            // The runtime registry table lives under this one heading; any other
            // top-level heading ends it.
            in_section = line.contains("Runtime and Standard Package Errors");
            continue;
        }
        if !in_section || !line.trim_start().starts_with("| `") {
            continue;
        }
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        // | `code` | `integer` | `Name` | meaning | notes |  -> cells[1..4]
        if cells.len() < 4 {
            continue;
        }
        let code = cells[1].trim_matches('`');
        let integer = cells[2].trim_matches('`');
        let name = cells[3].trim_matches('`');
        if code.is_empty() || integer.is_empty() || name.is_empty() {
            continue;
        }
        // Defend against doc drift: hyphen-stripping the canonical code must equal
        // the integer column, and the integer must be a bare number.
        assert_eq!(
            code.replace('-', ""),
            integer,
            "error_codes.md row `{name}`: code `{code}` does not match integer `{integer}`",
        );
        assert!(
            integer.chars().all(|c| c.is_ascii_digit()),
            "error_codes.md row `{name}`: integer `{integer}` is not numeric",
        );
        rows.push((name.to_string(), integer.to_string()));
    }

    assert!(
        !rows.is_empty(),
        "no runtime error-code rows parsed from {}",
        doc_path.display()
    );

    let out_path = out_dir.join("errorcode_generated.rs");
    let mut output = fs::File::create(out_path).expect("create generated errorcode source");
    writeln!(
        output,
        "/// `(name, integer-literal)` for every runtime registry row, generated\n\
         /// from specifications/error_codes.md by build.rs. Do not edit by hand.\n\
         pub(crate) const ERRORCODE_CONSTANTS: &[(&str, &str)] = &["
    )
    .expect("write generated errorcode source");
    for (name, integer) in &rows {
        writeln!(output, "    ({name:?}, {integer:?}),").expect("write generated errorcode source");
    }
    writeln!(output, "];").expect("write generated errorcode source");
}

/// A documented package discovered on disk: the directory, its index page
/// (`package.txt` for man / `spec.md` for spec), and the topic/function pages
/// beside it (sorted, index excluded).
struct DocPackage {
    name: String,
    dir: PathBuf,
    index: PathBuf,
    pages: Vec<PathBuf>,
}

/// Walk `root` and collect every package, sorted by name so the generated table
/// is deterministic. The runtime imposes its own display order on top.
fn doc_packages(root: &Path, index_name: &str, page_ext: &str) -> Vec<DocPackage> {
    let mut packages = Vec::new();
    collect_doc_packages(root, index_name, page_ext, &mut packages);
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    packages
}

fn collect_doc_packages(dir: &Path, index_name: &str, page_ext: &str, out: &mut Vec<DocPackage>) {
    let index = dir.join(index_name);
    if index.is_file() {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("doc package directory name")
            .to_string();
        let mut pages = fs::read_dir(dir)
            .unwrap_or_else(|_| panic!("read {name} doc directory"))
            .map(|entry| entry.expect("read doc entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == page_ext))
            .filter(|path| path.file_name().is_some_and(|name| name != index_name))
            .collect::<Vec<_>>();
        pages.sort();
        out.push(DocPackage {
            name,
            dir: dir.to_path_buf(),
            index,
            pages,
        });
    }

    let mut subdirs = fs::read_dir(dir)
        .unwrap_or_else(|_| panic!("read doc directory {}", dir.display()))
        .map(|entry| entry.expect("read doc entry").path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    subdirs.sort();
    for subdir in subdirs {
        collect_doc_packages(&subdir, index_name, page_ext, out);
    }
}

/// Strip a leading `<digits>_` ordering prefix from a page file stem. The digits
/// set the on-disk sort order (and thus the listing/`--all` reading order) but
/// are not part of the topic name used on the command line — `06_native.md`
/// becomes the topic `native`. A stem without a numeric prefix is returned
/// unchanged.
fn strip_order_prefix(stem: &str) -> &str {
    let rest = stem.trim_start_matches(|c: char| c.is_ascii_digit());
    if rest.len() < stem.len() {
        if let Some(name) = rest.strip_prefix('_') {
            if !name.is_empty() {
                return name;
            }
        }
    }
    stem
}

/// Emit a single self-contained table the runtime indexes by package name:
/// `(name, index-text, &[(page_name, page_text)])`.
fn write_doc_packages(output: &mut fs::File, const_name: &str, packages: &[DocPackage]) {
    writeln!(
        output,
        "/// `(name, package-overview, &[(page-name, page-text)])` for every\n\
         /// package, generated by build.rs. Do not edit by hand.\n\
         pub(crate) const {const_name}: &[(&str, &str, &[(&str, &str)])] = &["
    )
    .expect("write generated doc source");

    for package in packages {
        writeln!(
            output,
            "    ({name:?}, include_str!({index:?}), &[",
            name = package.name,
            index = package.index.display().to_string(),
        )
        .expect("write generated doc source");
        for page in &package.pages {
            let page_stem = page
                .file_stem()
                .and_then(|name| name.to_str())
                .expect("doc page file stem");
            let page_name = strip_order_prefix(page_stem);
            writeln!(
                output,
                "        ({page_name:?}, include_str!({path:?})),",
                path = page.display().to_string(),
            )
            .expect("write generated doc source");
        }
        writeln!(output, "    ]),").expect("write generated doc source");
    }

    writeln!(output, "];").expect("write generated doc source");
}
