use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let man_dir = manifest_dir.join("src/man");
    let error_codes_doc = manifest_dir.join("specifications/error_codes.md");
    println!("cargo:rerun-if-changed={}", error_codes_doc.display());

    // Discover every man package by walking the tree: any directory holding a
    // `package.txt` is a package, named after the directory. Adding a topic is
    // "drop a `.txt` file"; adding a package is "create a directory" — no edits
    // here. The display order and usage synopsis stay in `src/man/mod.rs`, the
    // only editorial bits the filesystem can't express.
    let packages = man_packages(&man_dir);
    println!("cargo:rerun-if-changed={}", man_dir.display());
    for package in &packages {
        println!("cargo:rerun-if-changed={}", package.dir.display());
        for page in std::iter::once(&package.package_txt).chain(&package.pages) {
            println!("cargo:rerun-if-changed={}", page.display());
        }
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let out_path = out_dir.join("man_generated.rs");
    let mut output = fs::File::create(out_path).expect("create generated man source");
    write_man_packages(&mut output, &packages);

    generate_errorcode_table(&error_codes_doc, &out_dir);
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

/// A documented man package discovered on disk: the directory, its `package.txt`
/// overview, and the topic/function pages beside it (sorted, `package.txt`
/// excluded).
struct ManPackage {
    name: String,
    dir: PathBuf,
    package_txt: PathBuf,
    pages: Vec<PathBuf>,
}

/// Walk `src/man` and collect every package, sorted by name so the generated
/// table is deterministic. The runtime imposes its own display order on top.
fn man_packages(root: &Path) -> Vec<ManPackage> {
    let mut packages = Vec::new();
    collect_man_packages(root, &mut packages);
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    packages
}

fn collect_man_packages(dir: &Path, out: &mut Vec<ManPackage>) {
    let package_txt = dir.join("package.txt");
    if package_txt.is_file() {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("man package directory name")
            .to_string();
        let mut pages = fs::read_dir(dir)
            .unwrap_or_else(|_| panic!("read {name} man directory"))
            .map(|entry| entry.expect("read man entry").path())
            .filter(|path| path.extension().is_some_and(|extension| extension == "txt"))
            .filter(|path| path.file_name().is_some_and(|name| name != "package.txt"))
            .collect::<Vec<_>>();
        pages.sort();
        out.push(ManPackage {
            name,
            dir: dir.to_path_buf(),
            package_txt,
            pages,
        });
    }

    let mut subdirs = fs::read_dir(dir)
        .unwrap_or_else(|_| panic!("read man directory {}", dir.display()))
        .map(|entry| entry.expect("read man entry").path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    subdirs.sort();
    for subdir in subdirs {
        collect_man_packages(&subdir, out);
    }
}

/// Emit a single self-contained table the runtime indexes by package name:
/// `(name, package.txt, &[(page_name, page_text)])`. This replaces the former
/// per-package constants plus their `match` arm in `mod.rs`.
fn write_man_packages(output: &mut fs::File, packages: &[ManPackage]) {
    writeln!(
        output,
        "/// `(name, package-overview, &[(page-name, page-text)])` for every man\n\
         /// package, generated from `src/man` by build.rs. Do not edit by hand.\n\
         pub(crate) const MAN_PACKAGES: &[(&str, &str, &[(&str, &str)])] = &["
    )
    .expect("write generated man source");

    for package in packages {
        writeln!(
            output,
            "    ({name:?}, include_str!({package_txt:?}), &[",
            name = package.name,
            package_txt = package.package_txt.display().to_string(),
        )
        .expect("write generated man source");
        for page in &package.pages {
            let page_name = page
                .file_stem()
                .and_then(|name| name.to_str())
                .expect("man page file stem");
            writeln!(
                output,
                "        ({page_name:?}, include_str!({path:?})),",
                path = page.display().to_string(),
            )
            .expect("write generated man source");
        }
        writeln!(output, "    ]),").expect("write generated man source");
    }

    writeln!(output, "];").expect("write generated man source");
}
