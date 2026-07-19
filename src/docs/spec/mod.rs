//! `mfb spec` — the MFBASIC language specification, embedded in the compiler and
//! version-locked to it (the spec you read always matches the binary you have).
//!
//! This mirrors `src/docs/man`: build.rs walks `src/docs/spec`, and any directory holding
//! a `spec.md` overview is a spec package named after the directory. Topic pages
//! sit beside it as `*.md`. The whole tree is embedded via `include_str!` (zero
//! runtime I/O); the only thing the filesystem cannot express — display order —
//! lives in `PACKAGE_ORDER` below.
//!
//! Unlike `man`, spec pages are Markdown (so they keep rendering in GitHub and
//! editors during review) and are turned into width-aware terminal text by
//! [`render`].

use std::sync::LazyLock;

use super::render;

// The generated `SPEC_PACKAGES` table is a nested tuple slice by nature.
#[allow(clippy::type_complexity)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/spec_generated.rs"));
}

/// One spec package: an overview plus its topic pages, all raw Markdown.
pub(crate) struct SpecPackage {
    pub(crate) name: &'static str,
    pub(crate) summary: String,
    pub(crate) overview: &'static str,
    pub(crate) topics: Vec<SpecTopic>,
}

/// One topic page within a spec package.
pub(crate) struct SpecTopic {
    pub(crate) name: &'static str,
    pub(crate) summary: String,
    pub(crate) page: &'static str,
}

/// Display order for `mfb spec`, listing packages in the order shown. A new
/// topic file needs no edit here; a brand-new package needs one row. Names not
/// listed are appended afterwards in discovery (alphabetical) order, so a
/// forgotten row degrades gracefully instead of hiding a package.
const PACKAGE_ORDER: &[&str] = &[
    "architecture",
    "language",
    "memory",
    "linker",
    "threading",
    "package",
    "diagnostics",
    "tooling",
    "package-manager",
    "unicode",
    "app",
    "stdlib",
];

static PACKAGES: LazyLock<Vec<SpecPackage>> = LazyLock::new(|| {
    let mut packages: Vec<SpecPackage> = generated::SPEC_PACKAGES
        .iter()
        .map(|&(name, overview, topics)| SpecPackage {
            name,
            summary: render::plain(summary_line(overview)),
            overview,
            topics: topics
                .iter()
                .map(|&(topic_name, page)| SpecTopic {
                    name: topic_name,
                    summary: render::plain(summary_line(page)),
                    page,
                })
                .collect(),
        })
        .collect();

    packages.sort_by_key(|package| {
        PACKAGE_ORDER
            .iter()
            .position(|name| *name == package.name)
            .unwrap_or(usize::MAX)
    });
    packages
});

pub(crate) fn packages() -> &'static [SpecPackage] {
    PACKAGES.as_slice()
}

pub(crate) fn package(name: &str) -> Option<&'static SpecPackage> {
    packages().iter().find(|package| package.name == name)
}

pub(crate) fn topic<'a>(package: &'a SpecPackage, name: &str) -> Option<&'a SpecTopic> {
    package.topics.iter().find(|topic| topic.name == name)
}

/// The first non-blank, non-heading line of a Markdown page, used as its
/// one-line summary in listings. Falls back to the empty string.
fn summary_line(markdown: &str) -> &str {
    markdown
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architecture_package_is_discovered() {
        let package = package("architecture").expect("architecture spec package present");
        assert!(!package.overview.is_empty());
        assert!(!package.summary.is_empty());
        assert!(
            !package.topics.is_empty(),
            "architecture package should ship topic pages"
        );
    }

    #[test]
    fn topics_are_looked_up_by_name() {
        let package = package("architecture").expect("architecture spec package present");
        let first = &package.topics[0];
        let found = topic(package, first.name).expect("topic resolves by name");
        assert_eq!(found.name, first.name);
        assert!(topic(package, "does-not-exist").is_none());
    }

    /// Drift guard (bug-338): every `./mfb spec <package> <topic>` cross-link in
    /// an embedded page must resolve to a package/topic that exists.
    ///
    /// The spec is written to be reimplementable and is version-locked to the
    /// binary, so a link that goes nowhere is a defect in the shipped artifact,
    /// not a broken doc link in a repo. bug-338 found 51 drifts by hand; this is
    /// the half a machine can keep checking.
    #[test]
    fn spec_links_resolve() {
        let mut broken = Vec::new();
        for pkg in packages() {
            let pages = std::iter::once(("spec", pkg.overview))
                .chain(pkg.topics.iter().map(|t| (t.name, t.page)));
            for (page, body) in pages {
                for target in cross_links(body) {
                    let mut parts = target.split_whitespace();
                    let (Some(pkg_name), topic_name) = (parts.next(), parts.next()) else {
                        continue;
                    };
                    let Some(found) = package(pkg_name) else {
                        broken.push(format!(
                            "{}/{page} -> unknown package `{pkg_name}`",
                            pkg.name
                        ));
                        continue;
                    };
                    if let Some(topic_name) = topic_name {
                        if topic(found, topic_name).is_none() {
                            broken.push(format!(
                                "{}/{page} -> `{pkg_name} {topic_name}` (no such topic)",
                                pkg.name
                            ));
                        }
                    }
                }
            }
        }
        assert!(
            broken.is_empty(),
            "unresolvable spec cross-links:\n{}",
            broken.join("\n")
        );
    }

    /// Drift guard (bug-338): the **file** component of every `[[path:Symbol]]`
    /// provenance citation must exist in the tree.
    ///
    /// Deliberately file-level only. The symbol half needs a language-aware
    /// resolver, and a naive grep would false-positive on re-exports — bug-338-H2
    /// is exactly that shape, where `verify_semantics` exists one module up from
    /// the file cited. File breakage is unambiguous and is what this catches.
    #[test]
    fn spec_citations_resolve() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut broken = Vec::new();
        for pkg in packages() {
            let pages = std::iter::once(("spec", pkg.overview))
                .chain(pkg.topics.iter().map(|t| (t.name, t.page)));
            for (page, body) in pages {
                for cite in citations(body) {
                    let file = cite.split(':').next().unwrap_or(&cite);
                    if file.is_empty() || !root.join(file).exists() {
                        broken.push(format!("{}/{page} -> [[{cite}]]", pkg.name));
                    }
                }
            }
        }
        assert!(
            broken.is_empty(),
            "unresolvable spec citations:\n{}",
            broken.join("\n")
        );
    }

    /// `./mfb spec <package> [<topic>]` targets named in a page's prose.
    fn cross_links(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        for (index, _) in body.match_indices("./mfb spec ") {
            let rest = &body[index + "./mfb spec ".len()..];
            let end = rest
                .find(|c: char| {
                    c == '\n' || c == '`' || c == ')' || c == ',' || c == ';' || c == '—'
                })
                .unwrap_or(rest.len());
            let target = rest[..end].trim().trim_end_matches('.');
            // `./mfb spec` with no argument is the index; a `--flag` example and
            // the `language *` glob (prose for "the language topics") are not
            // cross-links.
            if target.is_empty() || target.starts_with('-') || target.ends_with('*') {
                continue;
            }
            out.push(target.to_string());
        }
        out
    }

    /// `[[path]]` / `[[path:Symbol]]` provenance citations in a page.
    fn citations(body: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut rest = body;
        while let Some(start) = rest.find("[[") {
            let after = &rest[start + 2..];
            let Some(end) = after.find("]]") else { break };
            let inner = &after[..end];
            rest = &after[end + 2..];
            // POSIX character classes (`[[:alpha:]]`) are not citations.
            if inner.starts_with(':') || inner.contains(' ') || !inner.contains('/') {
                continue;
            }
            out.push(inner.to_string());
        }
        out
    }

    #[test]
    fn summary_skips_headings() {
        assert_eq!(
            summary_line("# Title\n\nA summary line.\n"),
            "A summary line."
        );
        assert_eq!(summary_line("# Only a heading"), "");
    }
}
