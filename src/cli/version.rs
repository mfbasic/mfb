//! `mfb --version` — the three-line version/build block (plan-42 §4.6).
//!
//! The build metadata is stamped in at compile time by the root `build.rs`
//! (`MFB_BUILD_DATE`/`MFB_COMMIT`/`MFB_LOCAL_DEV`), never resolved at runtime:
//! the shipped binary may run far from the tree it was built in. A build with no
//! git metadata (no `.git`, or git unavailable) still renders a valid block —
//! it just reports `Local Development`.

/// The provenance line shown for any build that is not a clean, pushed commit.
const LOCAL_DEVELOPMENT: &str = "Local Development";

/// Render the block from the metadata baked in by `build.rs`.
pub(crate) fn version_text() -> String {
    format_version(
        env!("CARGO_PKG_VERSION"),
        option_env!("MFB_BUILD_DATE"),
        option_env!("MFB_COMMIT"),
        option_env!("MFB_LOCAL_DEV"),
    )
}

/// ```text
/// MFBasic Compiler <version>
/// <UTC build date/time>
/// Commit: <short-hash>   |   Local Development
/// ```
///
/// Line 3 is a commit only when `build.rs` proved the tree was both clean and
/// fully pushed (`local_dev == "0"`) *and* handed us a hash. Anything else —
/// dirty tree, unpushed commit, absent metadata — is `Local Development`, so the
/// commit line never claims a provenance the binary cannot back up.
fn format_version(
    version: &str,
    build_date: Option<&str>,
    commit: Option<&str>,
    local_dev: Option<&str>,
) -> String {
    let build_date = build_date
        .filter(|date| !date.is_empty())
        .unwrap_or("unknown build date");
    let provenance = match (commit, local_dev) {
        (Some(commit), Some("0")) if !commit.is_empty() => format!("Commit: {commit}"),
        _ => LOCAL_DEVELOPMENT.to_string(),
    };
    format!("MFBasic Compiler {version}\n{build_date}\n{provenance}")
}

pub(crate) fn print_version() {
    println!("{}", version_text());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_pushed_build_reports_the_commit() {
        assert_eq!(
            format_version(
                "0.1.0",
                Some("2026-07-16 12:00:00 UTC"),
                Some("abc1234"),
                Some("0")
            ),
            "MFBasic Compiler 0.1.0\n2026-07-16 12:00:00 UTC\nCommit: abc1234"
        );
    }

    #[test]
    fn dirty_or_unpushed_build_reports_local_development() {
        assert_eq!(
            format_version(
                "0.1.0",
                Some("2026-07-16 12:00:00 UTC"),
                Some("abc1234"),
                Some("1")
            ),
            "MFBasic Compiler 0.1.0\n2026-07-16 12:00:00 UTC\nLocal Development"
        );
    }

    #[test]
    fn absent_metadata_still_renders_three_lines() {
        // A build with no git at all: no commit, no state, no date.
        let text = format_version("0.1.0", None, None, None);
        assert_eq!(
            text,
            "MFBasic Compiler 0.1.0\nunknown build date\nLocal Development"
        );
        assert_eq!(text.lines().count(), 3);
    }

    #[test]
    fn a_commit_without_a_clean_state_is_not_trusted() {
        // MFB_COMMIT present but MFB_LOCAL_DEV missing/empty must not print a
        // commit line — provenance is only claimed when build.rs proved it.
        for state in [None, Some(""), Some("1")] {
            let text = format_version("0.1.0", Some("date"), Some("abc1234"), state);
            assert!(
                text.ends_with(LOCAL_DEVELOPMENT),
                "state {state:?} must render Local Development, got {text}"
            );
        }
        // An empty commit with a clean state is equally untrustworthy.
        assert!(
            format_version("0.1.0", Some("date"), Some(""), Some("0")).ends_with(LOCAL_DEVELOPMENT)
        );
    }

    #[test]
    fn the_shipped_block_is_three_nonempty_lines() {
        // Whatever this build's metadata is, the rendered block is well-formed.
        let text = version_text();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("MFBasic Compiler "));
        assert!(lines.iter().all(|line| !line.is_empty()));
        assert!(lines[2] == LOCAL_DEVELOPMENT || lines[2].starts_with("Commit: "));
    }
}
