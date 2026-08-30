//! Every `canvas::` man example must actually compile and run.
//!
//! Nothing in the tree verified man examples before this, and writing plan-98-B's
//! turned up two that could not compile: a `LET img AS Image` (a resource is bound
//! with `RES` and named package-qualified) and a list literal spanning source lines
//! (which is `MFB_PARSE_UNEXPECTED_STATEMENT`). Both render in `mfb man` as the
//! recommended way to use the call, so a broken one is worse than no example.
//!
//! Scoped to `canvas` deliberately. A tree-wide version is the obviously desirable
//! thing, but the rest of the corpus predates any such check and would need its own
//! audit; this covers what plan-98 adds without taking that on.
//!
//! The example is read back out of `mfb man` rather than duplicated here, so it
//! cannot drift from what a user is actually shown.

mod common;
use std::fs;
use std::process::Command;

/// Every `canvas::` member that ships an example. Kept explicit rather than
/// discovered, so *removing* a member's example is a visible edit here rather than
/// a silently shrinking test.
const MEMBERS: &[&str] = &[
    "rgb",
    "rgba",
    "fill",
    "stroke",
    "fillStroke",
    "present",
    "createImage",
    "destroyImage",
    "imageRef",
    "getSize",
    "getBytes",
    "setBytes",
];

/// Pull the fenced example out of `mfb man canvas <member>`.
///
/// The renderer indents an example by two spaces and follows it with a `See also`
/// section or end of output, so the block is the indented run between the `Examples`
/// heading and whichever comes first. Prose lines inside the section are *not*
/// indented, which is what separates them from code.
fn example_source(member: &str) -> String {
    let output = Command::new(common::mfb_exe())
        .arg("man")
        .arg("canvas")
        .arg(member)
        .output()
        .unwrap_or_else(|e| panic!("run mfb man canvas {member}: {e}"));
    assert!(
        output.status.success(),
        "mfb man canvas {member} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8_lossy(&output.stdout).into_owned();

    let body = text
        .split_once("\nExamples\n")
        .unwrap_or_else(|| panic!("canvas::{member} has no Examples section"))
        .1;
    let body = body.split("\nSee also\n").next().unwrap_or(body);

    let mut lines: Vec<String> = Vec::new();
    for line in body.lines() {
        if line.trim().is_empty() {
            // Blank lines inside the code block are meaningful; blanks before it are
            // not. Only keep them once code has started.
            if !lines.is_empty() {
                lines.push(String::new());
            }
            continue;
        }
        if let Some(code) = line.strip_prefix("  ") {
            lines.push(code.to_string());
        } else if !lines.is_empty() {
            // An unindented line after code started: prose introducing a *second*
            // example. One compilable program per member is enough coverage.
            break;
        }
    }
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }
    assert!(
        !lines.is_empty(),
        "canvas::{member}'s example section contained no code"
    );
    format!("{}\n", lines.join("\n"))
}

#[test]
fn every_canvas_man_example_compiles() {
    let mut failures = Vec::new();
    for member in MEMBERS {
        let source = example_source(member);
        let project = common::temp_project(&format!("canvas_ex_{member}"), &source);
        let output = Command::new(common::mfb_exe())
            .arg("build")
            .arg("-app")
            .arg(&project)
            .output()
            .expect("run mfb build -app");
        if !output.status.success() {
            failures.push(format!(
                "--- canvas::{member} ---\n{source}\n{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            ));
        }
        let _ = fs::remove_dir_all(&project);
    }
    assert!(
        failures.is_empty(),
        "{} canvas man example(s) do not compile:\n\n{}",
        failures.len(),
        failures.join("\n")
    );
}
