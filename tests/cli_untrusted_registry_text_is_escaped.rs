//! bug-489: registry-authored text must not forge the operator's trust decision,
//! and sanitizing it must not mangle the compiler's own multi-line messages.
//!
//! These two assertions belong together because they are the two halves of one
//! design decision, and the second is what makes the first non-obvious.
//!
//! The bug: a malicious or MITM'd registry (anything `MFB_REPO_URL` points at)
//! authors the `error` field of its JSON response, and `mfb` printed it raw.
//! `\x1b[2K` erases the line and `\r` returns to column 0, so the `error: `
//! prefix the operator relies on is wiped and the attacker's own text — e.g.
//! `ok: uses toolbox - [Verified]` — is what renders. U+202E then visually
//! reorders whatever follows. That is a forged trust report, the exact threat
//! `terminal_safe` was written for (bug-24, bug-210), at a source their censuses
//! did not cover.
//!
//! The fix sanitizes at the TRUST BOUNDARY — where `mfb_repository::client`
//! creates the error — and NOT at the print site, which is what the bug report
//! originally proposed. `terminal_safe::safe` escapes `\n`, correctly, because a
//! server-authored newline forges whole rows. But 19 of the compiler's own
//! `CommandError` messages carry deliberate newlines (the multi-line usage
//! hints), so wrapping `dispatch_command_error` would have rendered every one of
//! them as a single `\u{000a}`-littered line. Only the source knows which strings
//! are untrusted.
//!
//! So the second test here is not decoration: it is the pin that fails if
//! someone later "simplifies" this by moving the sanitizer to the printer.

mod common;
use std::process::Command;

/// A multi-line usage error must reach the terminal with REAL newlines.
///
/// `mfb org` with no arguments is one of the 19; its message is
/// `"mfb org grant …\n       mfb org remove …"`. If the sanitizer ever moves to
/// the print site, this collapses to one line containing `\u{000a}`.
#[test]
fn a_multiline_usage_error_keeps_its_real_newlines() {
    let out = Command::new(common::mfb_exe())
        .arg("org")
        .output()
        .expect("run mfb org");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains("mfb org grant"),
        "expected the org usage hint, got: {stderr:?}"
    );
    assert!(
        !stderr.contains("\\u{000a}"),
        "the multi-line usage hint was escaped as if it were untrusted — the \
         sanitizer has moved to the print site, where it cannot tell the \
         compiler's own text from a registry's. stderr: {stderr:?}"
    );
    assert!(
        stderr.trim_end().contains('\n'),
        "the usage hint must still be genuinely multi-line: {stderr:?}"
    );
}

/// And the same for the other shape: a subcommand hint separated by a blank line.
#[test]
fn a_usage_hint_with_a_blank_line_is_unescaped_too() {
    let out = Command::new(common::mfb_exe())
        .arg("repo")
        .output()
        .expect("run mfb repo");
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        stderr.contains("requires a subcommand"),
        "expected the repo usage hint, got: {stderr:?}"
    );
    assert!(
        !stderr.contains("\\u{000a}"),
        "the repo usage hint was escaped: {stderr:?}"
    );
    assert!(
        stderr.lines().count() >= 3,
        "the hint's blank line and follow-up must survive: {stderr:?}"
    );
}
