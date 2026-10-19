//! Scope walk and per-file classification for the file-size budget.
//!
//! A file is test-only when a directory component of its path is `tests`, it is
//! an MFBASIC `test_*.mfb`, it begins a line with `#![cfg(test)]`, or it sits at
//! or under a path a `#[cfg(test)] mod x;` (or `#[cfg(test)] #[path = "…"] mod
//! x;`) declares. A file is generated when one of its first three lines is a
//! comment carrying the uppercase word `GENERATED`; generated files are exempt.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use crate::mfb_lex::mfb_lines;
use crate::rust_lex::{ident_char, rust_code_text, rust_lines, rust_test_lines, LineKind};

/// The directories the rule covers, relative to the repository root.
pub const SCOPE: &[&str] = &["src", "packages", "examples", "repository", "tests"];
/// Directory names the walk never enters.
const SKIP_DIRS: &[&str] = &["target", ".git"];

/// The measured counts of one file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileCount {
    /// Code lines (always 0 for a test-only file).
    pub code: usize,
    /// Test lines: inline test items, or every LOC of a test-only file.
    pub test: usize,
    pub generated: bool,
    pub testfile: bool,
}

/// One budget kind and its limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Code,
    Inline,
    Test,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Code => "code",
            Kind::Inline => "inline",
            Kind::Test => "test",
        }
    }

    pub fn parse(s: &str) -> Option<Kind> {
        match s {
            "code" => Some(Kind::Code),
            "inline" => Some(Kind::Inline),
            "test" => Some(Kind::Test),
            _ => None,
        }
    }

    pub fn limit(self) -> usize {
        match self {
            Kind::Code => 1000,
            Kind::Inline => 250,
            Kind::Test => 1000,
        }
    }
}

impl FileCount {
    /// The (kind, count) pairs that apply to this file: `test` for a test-only
    /// file, `code` and `inline` otherwise, none for a generated file.
    pub fn kinds(&self) -> Vec<(Kind, usize)> {
        if self.generated {
            Vec::new()
        } else if self.testfile {
            vec![(Kind::Test, self.test)]
        } else {
            vec![(Kind::Code, self.code), (Kind::Inline, self.test)]
        }
    }

    /// This file's count for `kind`, 0 when the kind does not apply.
    pub fn count(&self, kind: Kind) -> usize {
        self.kinds()
            .into_iter()
            .find(|(k, _)| *k == kind)
            .map_or(0, |(_, n)| n)
    }
}

/// Repository-relative path with `/` separators.
fn rel_string(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every in-scope `.rs` / `.mfb` file, as (absolute path, relative path).
/// Symlinked directories are not entered.
pub fn walk(root: &Path) -> Vec<(PathBuf, String)> {
    fn visit(dir: &Path, root: &Path, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if !SKIP_DIRS.contains(&name.as_str()) {
                    visit(&path, root, out);
                }
                continue;
            }
            if ft.is_symlink() && path.is_dir() {
                continue;
            }
            if name.ends_with(".rs") || name.ends_with(".mfb") {
                out.push((path.clone(), rel_string(root, &path)));
            }
        }
    }
    let mut out = Vec::new();
    for d in SCOPE {
        visit(&root.join(d), root, &mut out);
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

pub fn read(path: &Path) -> String {
    String::from_utf8_lossy(&std::fs::read(path).unwrap_or_default()).into_owned()
}

/// Lexically normalize `.` and `..` components (no filesystem access).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn skip_ws(s: &str) -> &str {
    s.trim_start_matches(char::is_whitespace)
}

/// The tail of a test `mod` declaration that follows its attributes:
/// `[pub[(…)] ]mod <name> ;`. Returns the module name.
fn mod_decl(s: &str) -> Option<&str> {
    let mut rest = s;
    if let Some(r) = rest.strip_prefix("pub") {
        let mut r2 = r;
        if let Some(inner) = r2.strip_prefix('(') {
            if let Some(close) = inner.find(')') {
                r2 = &inner[close + 1..];
            }
        }
        if r2.starts_with(char::is_whitespace) {
            rest = skip_ws(r2);
        }
    }
    let r = rest.strip_prefix("mod")?;
    if !r.starts_with(char::is_whitespace) {
        return None;
    }
    let r = skip_ws(r);
    let len = r
        .find(|c: char| !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'))
        .unwrap_or(r.len());
    if len == 0 {
        return None;
    }
    skip_ws(&r[len..]).starts_with(';').then(|| &r[..len])
}

/// The path of a `path = "…"` attribute body, if that is all the body holds.
fn path_attr(body: &str) -> Option<&str> {
    let r = skip_ws(body.strip_prefix("path")?);
    let r = skip_ws(r.strip_prefix('=')?);
    let r = r.strip_prefix('"')?;
    let end = r.find('"')?;
    (end > 0 && r[end + 1..].is_empty()).then(|| &r[..end])
}

/// Every `#[cfg(test)]`-gated module declaration in `text`: the declared name,
/// and the path of its last `#[path = "…"]` attribute if it has one.
pub fn test_mod_decls(text: &str) -> Vec<(String, Option<String>)> {
    const CFG: &str = "#[cfg(test)]";
    let mut out = Vec::new();
    let mut from = 0;
    while let Some(off) = text[from..].find(CFG) {
        let start = from + off;
        from = start + 1;
        let mut rest = skip_ws(&text[start + CFG.len()..]);
        let mut attrs = Vec::new();
        while let Some(r) = rest.strip_prefix("#[") {
            let Some(close) = r.find(']') else { break };
            attrs.push(&r[..close]);
            rest = skip_ws(&r[close + 1..]);
        }
        if let Some(name) = mod_decl(rest) {
            let path = attrs.iter().rev().find_map(|a| path_attr(a));
            out.push((name.to_string(), path.map(str::to_string)));
            from = text.len() - rest.len();
        }
    }
    out
}

/// Every repository-relative path (file or directory) that a `#[cfg(test)]`
/// module declaration makes test-only.
pub fn test_roots(root: &Path, files: &[(PathBuf, String)]) -> BTreeSet<String> {
    let mut roots = BTreeSet::new();
    for (path, _) in files {
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = read(path);
        let dir = path.parent().unwrap_or(root);
        let file = path.file_name().and_then(|f| f.to_str()).unwrap_or("");
        let base = if matches!(file, "mod.rs" | "main.rs" | "lib.rs") {
            dir.to_path_buf()
        } else {
            dir.join(file.trim_end_matches(".rs"))
        };
        for (name, attr_path) in test_mod_decls(&text) {
            for cand in [base.join(format!("{name}.rs")), base.join(&name)] {
                if cand.exists() {
                    roots.insert(rel_string(root, &cand));
                }
            }
            if let Some(p) = attr_path {
                let cand = normalize(&dir.join(p));
                if cand.exists() {
                    roots.insert(rel_string(root, &cand));
                }
            }
        }
    }
    let derived: Vec<String> = roots
        .iter()
        .filter_map(|r| {
            r.strip_suffix("/mod.rs")
                .or_else(|| r.strip_suffix(".rs"))
                .map(str::to_string)
        })
        .collect();
    roots.extend(derived);
    roots
}

/// Whether the whole file is test-only.
pub fn is_test_file(rel: &str, text: &str, roots: &BTreeSet<String>) -> bool {
    let parts: Vec<&str> = rel.split('/').collect();
    let (name, dirs) = parts.split_last().expect("a path has a file name");
    if dirs.contains(&"tests") {
        return true;
    }
    if name.ends_with(".mfb") && name.starts_with("test_") {
        return true;
    }
    if roots.contains(rel) || roots.iter().any(|r| rel.starts_with(&format!("{r}/"))) {
        return true;
    }
    name.ends_with(".rs") && text.split('\n').any(|l| l.starts_with("#![cfg(test)]"))
}

/// `line` is a comment (`//`, `//!`, `/*`, `'`, `REM`) carrying the word `GENERATED`.
fn generated_line(line: &str) -> bool {
    let s = line.trim_start_matches(char::is_whitespace);
    let markers = ["//!", "//", "/*", "'", "REM"];
    markers.iter().any(|m| {
        let Some(rest) = s.strip_prefix(m) else {
            return false;
        };
        if rest.starts_with(char::is_whitespace) {
            let word = "GENERATED";
            rest.match_indices(word).any(|(at, _)| {
                let before = rest[..at].chars().next_back();
                let after = rest[at + word.len()..].chars().next();
                !before.is_some_and(ident_char) && !after.is_some_and(ident_char)
            })
        } else {
            skip_ws(rest)
                .strip_prefix("GENERATED")
                .is_some_and(|r| !r.chars().next().is_some_and(ident_char))
        }
    })
}

pub fn is_generated(text: &str) -> bool {
    text.split('\n').take(3).any(generated_line)
}

/// Per line: whether it is LOC, and whether that LOC is test.
pub fn line_kinds(rel: &str, text: &str, roots: &BTreeSet<String>) -> Vec<(bool, bool)> {
    let lines: Vec<(bool, bool)> = if rel.ends_with(".rs") {
        let tests = rust_test_lines(&rust_code_text(text));
        rust_lines(text)
            .into_iter()
            .zip(tests)
            .map(|(k, t)| (k == LineKind::Code, t))
            .collect()
    } else {
        mfb_lines(text)
            .into_iter()
            .map(|(k, t)| (k == LineKind::Code, t))
            .collect()
    };
    if is_test_file(rel, text, roots) {
        lines.into_iter().map(|(loc, _)| (loc, loc)).collect()
    } else {
        lines
    }
}

/// Counts for one file's text.
pub fn count_text(rel: &str, text: &str, roots: &BTreeSet<String>) -> FileCount {
    let lines = line_kinds(rel, text, roots);
    FileCount {
        code: lines.iter().filter(|(loc, t)| *loc && !*t).count(),
        test: lines.iter().filter(|(loc, t)| *loc && *t).count(),
        generated: is_generated(text),
        testfile: is_test_file(rel, text, roots),
    }
}

/// Counts for every in-scope file, keyed by repository-relative path.
pub fn census(root: &Path) -> BTreeMap<String, FileCount> {
    let files = walk(root);
    let roots = test_roots(root, &files);
    files
        .iter()
        .map(|(path, rel)| (rel.clone(), count_text(rel, &read(path), &roots)))
        .collect()
}
