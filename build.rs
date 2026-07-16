use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let error_codes_doc = manifest_dir.join("src/docs/spec/diagnostics/02_error-codes.md");
    println!("cargo:rerun-if-changed={}", error_codes_doc.display());

    emit_build_metadata(&manifest_dir);

    // Man pages and spec pages share one discovery model: walk a tree, and any
    // directory holding an index file (`package.{txt,md}` / `spec.md`) is a
    // package named after the directory. Adding a topic is "drop a file"; adding
    // a package is "create a directory" — no edits here. Display order (and, for
    // man, the usage synopsis) lives in the runtime module, the only editorial
    // bits the filesystem can't express.
    //
    // Man pages may be plain text (`.txt`) or Markdown (`.md`); spec pages are
    // always Markdown. A Markdown page renders to the terminal through
    // `src/docs/render.rs` (the runtime picks the renderer by sniffing a leading
    // ATX heading). All pages embed via `include_str!` (zero runtime I/O).
    generate_doc_table(
        &manifest_dir.join("src/docs/man"),
        &["package.txt", "package.md"],
        &["txt", "md"],
        "MAN_PACKAGES",
        &out_dir.join("man_generated.rs"),
    );
    generate_doc_table(
        &manifest_dir.join("src/docs/spec"),
        &["spec.md"],
        &["md"],
        "SPEC_PACKAGES",
        &out_dir.join("spec_generated.rs"),
    );

    generate_errorcode_table(&error_codes_doc, &out_dir);
}

/// Stamp the `mfb --version` block's build metadata into the binary (plan-42
/// §4.7): the UTC build time, the short commit, and whether that commit is one a
/// reader could actually fetch.
///
/// Provenance is captured here rather than in the shipped binary because that
/// binary may run far from the tree it was built in. Every probe is best-effort
/// and never fails the build: a tree with no `.git` (a vendored tarball) or a
/// host with no `git` still compiles, it just reports `Local Development`.
///
/// The claim `Commit: <hash>` is only made when git proves the tree is both
/// clean and pushed. Every unprovable case — no git, no upstream, a probe that
/// errored — falls to `Local Development`, so the version block can understate
/// provenance but never overstate it.
fn emit_build_metadata(manifest_dir: &Path) {
    // `date -u` keeps this std-only (a formatting crate would be a new build
    // dependency); an unavailable `date` renders as "unknown build date".
    let build_date = capture("date", &["-u", "+%Y-%m-%d %H:%M:%S UTC"], manifest_dir);
    println!(
        "cargo:rustc-env=MFB_BUILD_DATE={}",
        build_date.unwrap_or_default()
    );

    watch_build_state(manifest_dir);

    let commit =
        capture("git", &["rev-parse", "--short", "HEAD"], manifest_dir).unwrap_or_default();
    // Clean: no uncommitted work of any kind (`--porcelain` prints one line per
    // modified, staged, or untracked path, and nothing at all when clean).
    let clean = capture("git", &["status", "--porcelain"], manifest_dir)
        .is_some_and(|status| status.is_empty());
    // Pushed: no commit on HEAD that the upstream lacks. A missing or
    // unresolvable `@{u}` exits non-zero — no upstream means nothing to fetch
    // the commit from, which is exactly local development.
    let pushed = capture("git", &["rev-list", "@{u}..HEAD"], manifest_dir)
        .is_some_and(|ahead| ahead.is_empty());
    let local_dev = if !commit.is_empty() && clean && pushed {
        "0"
    } else {
        "1"
    };
    println!("cargo:rustc-env=MFB_COMMIT={commit}");
    println!("cargo:rustc-env=MFB_LOCAL_DEV={local_dev}");
}

/// Force this script to re-run whenever anything that decides the commit line
/// changes: a new commit (`HEAD` / the branch ref), a staged edit (the index), a
/// push (the upstream's remote-tracking ref), or an edit to a tracked source
/// file — which makes the tree dirty without touching `.git` at all, so watching
/// git alone would leave a stale `Commit:` line on a modified tree.
///
/// Only existing paths are emitted: cargo treats a `rerun-if-changed` path that
/// does not exist as perpetually dirty, which would re-run this script on every
/// build.
///
/// Known caveat (plan-42 §4.7): cargo caches build-script output, so
/// `MFB_BUILD_DATE` is when this script last re-ran, not the instant of the
/// final link. That is accepted for a `--version` stamp; the provenance line,
/// which must not go stale, is what this watch set covers.
fn watch_build_state(manifest_dir: &Path) {
    let mut watched = vec![manifest_dir.join("src"), manifest_dir.join("Cargo.toml")];

    if let Some(git_dir) = capture("git", &["rev-parse", "--absolute-git-dir"], manifest_dir) {
        let git_dir = PathBuf::from(git_dir);
        watched.push(git_dir.join("HEAD"));
        watched.push(git_dir.join("index"));
        // A loose branch/upstream ref is its own file; when refs are packed they
        // live in `packed-refs` instead. Watch whichever exist.
        watched.push(git_dir.join("packed-refs"));
        for rev in ["HEAD", "@{u}"] {
            if let Some(name) = capture(
                "git",
                &["rev-parse", "--symbolic-full-name", rev],
                manifest_dir,
            ) {
                if !name.is_empty() {
                    watched.push(git_dir.join(name));
                }
            }
        }
    }

    for path in watched {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
}

/// Run `program` in `dir` and return its trimmed stdout, or `None` if the
/// program is absent, fails to spawn, exits non-zero, or emits non-UTF-8. A
/// successful command that prints nothing yields `Some("")` — the difference
/// between "clean tree" and "no git" that `emit_build_metadata` turns on.
fn capture(program: &str, args: &[&str], dir: &Path) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8(output.stdout).ok()?.trim().to_string())
}

/// Discover every package under `root`, emit `cargo:rerun-if-changed` lines for
/// the tree, and write the generated `(name, index-text, &[(page, text)])`
/// table to `out_path` as `const_name`.
fn generate_doc_table(
    root: &Path,
    index_names: &[&str],
    page_exts: &[&str],
    const_name: &str,
    out_path: &Path,
) {
    let packages = doc_packages(root, index_names, page_exts);
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

/// Parse the "Constant Registry" table in the embedded spec topic
/// `src/docs/spec/diagnostics/02_error-codes.md` and emit a generated `(name, integer)`
/// table for the built-in `errorCode` package. The spec topic is the single
/// source of truth (`mfb spec diagnostics error-codes`); this keeps the package
/// from drifting from the canonical registry. Only the runtime `Err*` rows are
/// exported — those are the program-visible `Error.code` values, matching
/// `errorCode::Err*` usage.
fn generate_errorcode_table(doc_path: &PathBuf, out_dir: &PathBuf) {
    let doc =
        fs::read_to_string(doc_path).expect("read src/docs/spec/diagnostics/02_error-codes.md");

    let mut in_section = false;
    let mut rows: Vec<(String, String)> = Vec::new();
    for line in doc.lines() {
        if line.starts_with("## ") {
            // The runtime registry table lives under this one heading; any other
            // top-level heading ends it (notably the Subsystem Partitioning table,
            // whose rows also start with "| `7-...").
            in_section = line.contains("Constant Registry");
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
         /// from src/docs/spec/diagnostics/02_error-codes.md by build.rs. Do not edit by hand.\n\
         pub(crate) const ERRORCODE_CONSTANTS: &[(&str, &str)] = &["
    )
    .expect("write generated errorcode source");
    for (name, integer) in &rows {
        writeln!(output, "    ({name:?}, {integer:?}),").expect("write generated errorcode source");
    }
    writeln!(output, "];").expect("write generated errorcode source");
}

/// A documented package discovered on disk: the directory, its index page
/// (`package.{txt,md}` for man / `spec.md` for spec), and the topic/function
/// pages beside it (sorted, index excluded).
struct DocPackage {
    name: String,
    dir: PathBuf,
    index: PathBuf,
    pages: Vec<PathBuf>,
}

/// Walk `root` and collect every package, sorted by name so the generated table
/// is deterministic. The runtime imposes its own display order on top.
fn doc_packages(root: &Path, index_names: &[&str], page_exts: &[&str]) -> Vec<DocPackage> {
    let mut packages = Vec::new();
    collect_doc_packages(root, index_names, page_exts, &mut packages);
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    packages
}

fn collect_doc_packages(
    dir: &Path,
    index_names: &[&str],
    page_exts: &[&str],
    out: &mut Vec<DocPackage>,
) {
    // The first index candidate that exists names the package; a directory holds
    // at most one (`package.txt` or `package.md`, never both).
    if let Some(index) = index_names
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.is_file())
    {
        let name = dir
            .file_name()
            .and_then(|name| name.to_str())
            .expect("doc package directory name")
            .to_string();
        let mut pages = fs::read_dir(dir)
            .unwrap_or_else(|_| panic!("read {name} doc directory"))
            .map(|entry| entry.expect("read doc entry").path())
            .filter(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| page_exts.contains(&extension))
            })
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| !index_names.contains(&name))
            })
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
        collect_doc_packages(&subdir, index_names, page_exts, out);
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
