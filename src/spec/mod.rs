//! `mfb spec` — the MFBASIC language specification, embedded in the compiler and
//! version-locked to it (the spec you read always matches the binary you have).
//!
//! This mirrors `src/man`: build.rs walks `src/spec`, and any directory holding
//! a `spec.md` overview is a spec package named after the directory. Topic pages
//! sit beside it as `*.md`. The whole tree is embedded via `include_str!` (zero
//! runtime I/O); the only thing the filesystem cannot express — display order —
//! lives in `PACKAGE_ORDER` below.
//!
//! Unlike `man`, spec pages are Markdown (so they keep rendering in GitHub and
//! editors during review) and are turned into width-aware terminal text by
//! [`render`].

use std::sync::LazyLock;

pub(crate) mod render;

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
const PACKAGE_ORDER: &[&str] = &["architecture", "memory", "linker", "threading"];

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

    #[test]
    fn summary_skips_headings() {
        assert_eq!(summary_line("# Title\n\nA summary line.\n"), "A summary line.");
        assert_eq!(summary_line("# Only a heading"), "");
    }
}
