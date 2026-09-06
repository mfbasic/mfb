//! The `-nir` and `-nplan` dumps render every module the compiler produces.
//!
//! Both are developer artifacts with exactly one caller apiece — each backend's
//! `write_nir` / `write_nplan` — and no test ever read one. That makes them the
//! one part of the pipeline where a node kind the renderer does not know how to
//! print is invisible: the compiler still builds the program correctly, and the
//! hole shows up only when somebody dumps a program that happens to contain
//! that kind and reads a `"<unknown>"` (or, worse, silently valid-looking JSON
//! with a field missing).
//!
//! `src/target/shared/nir/json.rs` is one `match` arm per node kind and was at
//! 89.48%, so 46 of those arms had never rendered anything.
//!
//! The input is the corpus, because "every node kind" is not a list anyone
//! maintains — it is whatever 630 committed programs happen to contain, and
//! that list grows on its own as fixtures land.
//!
//! What is asserted is what a dump is FOR: that it parses. A renderer that
//! forgot a comma or left a trailing one produces a file no tool can read, and
//! the eight-thousand-line dumps these programs produce are not something a
//! human notices that in.

use tinyjson::JsonValue;

use crate::target::NativeBuildMode::Console;
use crate::testutil::{fixture_src, nir_for_src, CodeTarget};

/// Every corpus program's `-nir` dump is valid JSON with the fields the format
/// declares.
#[test]
fn the_nir_dump_of_every_corpus_program_is_valid_json() {
    // Report every failure rather than the first: at 630 programs, finding them
    // one run at a time is the difference between a minute and an afternoon.
    let mut failed = Vec::new();
    let mut rendered = 0usize;

    for fixture in super::corpus::CORPUS {
        let source = fixture_src(fixture);
        let module = match nir_for_src(&source, CodeTarget::LinuxX86_64, Console) {
            Ok(module) => module,
            Err(err) => {
                failed.push(format!("{fixture}: {}", err.lines().next().unwrap_or(&err)));
                continue;
            }
        };
        let dump = module.to_json();
        rendered += dump.len();
        let parsed: Result<JsonValue, _> = dump.parse();
        match parsed {
            Ok(JsonValue::Object(root)) => {
                // The three fields the format's own header declares. A dump that
                // parses but has lost its function list is a dump of nothing.
                for field in ["format", "target", "functions"] {
                    if !root.contains_key(field) {
                        failed.push(format!("{fixture}: the dump has no `{field}`"));
                    }
                }
            }
            Ok(_) => failed.push(format!("{fixture}: the dump is not a JSON object")),
            Err(err) => failed.push(format!("{fixture}: the dump is not valid JSON: {err}")),
        }
    }

    assert!(
        failed.is_empty(),
        "{} corpus program(s) did not render a readable -nir dump:\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
    // The bound that says the loop RAN. A renderer returning `""` would satisfy
    // every assertion above by never entering the `Ok(Object)` arm at all.
    assert!(
        rendered > 1_000_000,
        "the corpus rendered only {rendered} bytes of -nir; it measured tens of \
         megabytes, and a renderer that stopped emitting would show up here"
    );
}

/// And the `-nplan` dump, one stage below.
///
/// Same argument, different renderer: `NativePlan::to_json` prints the plan the
/// backend lays out — its sections, its data objects, its symbols — and nothing
/// read one either.
#[test]
fn the_nplan_dump_of_every_corpus_program_is_valid_json() {
    let mut failed = Vec::new();
    let mut rendered = 0usize;

    for fixture in super::corpus::CORPUS {
        let source = fixture_src(fixture);
        let plan =
            match crate::testutil::native_plan_for_src(&source, CodeTarget::LinuxX86_64, Console) {
                Ok(plan) => plan,
                Err(err) => {
                    failed.push(format!("{fixture}: {}", err.lines().next().unwrap_or(&err)));
                    continue;
                }
            };
        let dump = plan.to_json();
        rendered += dump.len();
        let parsed: Result<JsonValue, _> = dump.parse();
        match parsed {
            Ok(JsonValue::Object(_)) => {}
            Ok(_) => failed.push(format!("{fixture}: the dump is not a JSON object")),
            Err(err) => failed.push(format!("{fixture}: the dump is not valid JSON: {err}")),
        }
    }

    assert!(
        failed.is_empty(),
        "{} corpus program(s) did not render a readable -nplan dump:\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
    assert!(
        rendered > 100_000,
        "the corpus rendered only {rendered} bytes of -nplan; a renderer that \
         stopped emitting would show up here"
    );
}
