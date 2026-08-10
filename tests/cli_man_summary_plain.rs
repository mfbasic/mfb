//! bug-214: `mfb man` one-line summaries were printed as raw Markdown, so inline
//! markup (backticks/bold/citations) leaked verbatim into the package listing —
//! unlike `mfb spec`, which runs `summary_line` through `render::plain`. The man
//! summary print sites now strip markup the same way.
//!
//! This drives `mfb man datetime` (the package listing, where each function's
//! one-line summary is shown) and asserts `between`'s summary renders as plain
//! prose (`The signed Duration span ...`) with no literal backtick around
//! `Duration` — the exact leak the doc names.

use std::process::Command;

mod common;
use common::*;

#[test]
fn man_listing_summaries_are_plain_rendered() {
    let output = Command::new(mfb_exe())
        .args(["man", "datetime"])
        .output()
        .expect("run mfb man datetime");
    assert!(
        output.status.success(),
        "mfb man datetime failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);

    // The plain one-line summary must appear in the listing...
    assert!(
        stdout.contains("The signed Duration span between two instants."),
        "expected the plain-rendered `between` summary in the listing, got:\n{stdout}"
    );
    // ...and its raw Markdown form (backtick-wrapped `Duration`) must NOT — that
    // is exactly the leak bug-214 fixed.
    assert!(
        !stdout.contains("The signed `Duration` span"),
        "man listing leaked raw Markdown backticks (bug-214 regressed):\n{stdout}"
    );
}
