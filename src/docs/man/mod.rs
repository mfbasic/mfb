//! `mfb man <topic>` guide pages — the prose guides that document the language
//! itself rather than a single built-in package (`errors`, `flow`, `lambda`,
//! `link`, `tooling`, `tour`, `types`, `unicode`).
//!
//! This mirrors `src/docs/spec`: build.rs walks `src/docs/man`, and any directory
//! holding a `package.md` overview is a guide topic named after the directory.
//! Sub-pages sit beside it as `*.md` (a leading `<digits>_` ordering prefix is
//! stripped from the page name — `01_c.md` becomes the sub-page `c`). The whole
//! tree is embedded via `include_str!` (zero runtime I/O).
//!
//! The built-in `mfb man <package>` / `mfb man <package> <function>` pages are
//! unaffected — those are rendered from the descriptor registry
//! (`crate::codegen::registry`) by `crate::cli::man`; this module is only the
//! fallback consulted when the first positional is not a known package.

use std::sync::LazyLock;

// The generated `MAN_TOPICS` table is a nested tuple slice by nature.
#[allow(clippy::type_complexity)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/man_generated.rs"));
}

/// One guide topic: an overview page plus its sub-pages, all raw Markdown.
pub(crate) struct ManTopic {
    pub(crate) name: &'static str,
    pub(crate) overview: &'static str,
    pub(crate) pages: Vec<ManPage>,
}

/// One sub-page within a guide topic.
pub(crate) struct ManPage {
    pub(crate) name: &'static str,
    pub(crate) page: &'static str,
}

static TOPICS: LazyLock<Vec<ManTopic>> = LazyLock::new(|| {
    generated::MAN_TOPICS
        .iter()
        .map(|&(name, overview, pages)| ManTopic {
            name,
            overview,
            pages: pages
                .iter()
                .map(|&(page_name, page)| ManPage {
                    name: page_name,
                    page,
                })
                .collect(),
        })
        .collect()
});

pub(crate) fn topics() -> &'static [ManTopic] {
    TOPICS.as_slice()
}

pub(crate) fn topic(name: &str) -> Option<&'static ManTopic> {
    topics().iter().find(|topic| topic.name == name)
}

pub(crate) fn page<'a>(topic: &'a ManTopic, name: &str) -> Option<&'a ManPage> {
    topic.pages.iter().find(|page| page.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_guide_topics_are_discovered() {
        // Every topic advertised by `mfb man <topic>` resolves to a non-empty
        // overview with a one-line summary.
        for name in [
            "errors", "flow", "lambda", "link", "tooling", "tour", "types", "unicode",
        ] {
            let topic = topic(name).unwrap_or_else(|| panic!("guide topic `{name}` present"));
            assert!(!topic.overview.is_empty(), "{name} overview is non-empty");
        }
    }

    #[test]
    #[should_panic(expected = "guide topic `does-not-exist` present")]
    fn a_missing_guide_topic_names_itself() {
        let name = "does-not-exist";
        topic(name).unwrap_or_else(|| panic!("guide topic `{name}` present"));
    }

    #[test]
    fn sub_pages_are_looked_up_by_name_with_the_order_prefix_stripped() {
        // `flow` ships sub-pages (`for`, `if`, ...); an unknown page is None.
        let flow = topic("flow").expect("flow topic present");
        assert!(!flow.pages.is_empty(), "flow ships sub-pages");
        assert!(page(flow, "for").is_some(), "flow/for.md resolves as `for`");
        assert!(page(flow, "does-not-exist").is_none());

        // `tour` files carry a `NN_` ordering prefix that is stripped from the
        // command-line name (`01_c.md` -> `c`).
        let tour = topic("tour").expect("tour topic present");
        assert!(page(tour, "c").is_some(), "tour/01_c.md resolves as `c`");
    }
}
