//! The fixtures that need their `.mfp` packages, lowered in process at last.
//!
//! `corpus.rs` lowers a fixture by reading `src/main.mfb` and nothing else, and
//! excludes two fixtures because they "do not lower here on ANY backend
//! (`thread.start entry point must name an ISOLATED FUNC`)". That message is not
//! about threads and not about any backend: both fixtures name their worker
//! through an imported package —
//!
//!     thread::start(fixed_list_xfer_worker::doubleIntegers, "seed")
//!
//! — and a qualified entry resolves through `imported_signatures`, which a
//! single-source project never populates. One missing input, reported as a
//! front-end limitation.
//!
//! `testutil::try_code_for_fixture_project` runs `cli/build`'s own front end
//! over the fixture's DIRECTORY instead, so the manifest's `packages` are read
//! and their signatures and type tables reach lowering. What that buys is not
//! the two fixtures: it is the shape they carry. A resource-bearing composite
//! handed to a thread is the only thing that reaches
//! `memory/arena/builder_arena_transfer.rs`'s four inline copiers —
//! `emit_thread_copy_real` sends every flat value to `copy_flat_block` and
//! leaves "resources and the collections / unions that embed them" to
//! `copy_resource_to_current_arena` (197 lines), `copy_collection_to_current_arena`,
//! `copy_union_to_current_arena` and `copy_record_to_current_arena`.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_fixture_project, CodeTarget};

/// Every package-bearing fixture the single-source path cannot reach.
///
/// Named rather than discovered, for the reason `corpus.rs` gives: a list that
/// silently skips what it cannot lower is a gate that stops measuring.
const PACKAGE_FIXTURES: &[&str] = &["thread-fixed-list-transfer-rt", "p121d-state-reach-rt"];

/// Each one lowers, on every backend.
///
/// The same assertion `corpus.rs` makes for its own list. These are here rather
/// than there because they need a different entry point, not because they are
/// held to a different standard.
#[test]
fn every_package_bearing_fixture_lowers_on_every_backend() {
    for fixture in PACKAGE_FIXTURES {
        for target in CodeTarget::ALL {
            let plan = try_code_for_fixture_project(fixture, target, Console)
                .unwrap_or_else(|err| panic!("{fixture} on {}: {err}", target.name()));
            assert_eq!(
                plan.target,
                target.name(),
                "{fixture}: the code plan must record the backend it was lowered for"
            );
            assert!(
                plan.functions.len() > 1,
                "{fixture} on {}: a plan with one function is a lowering that \
                 declined, not a program",
                target.name()
            );
        }
    }
}

/// The worker's package really did reach lowering.
///
/// Without it the test above would pass against a loader that silently dropped
/// the `packages` entry and lowered a program with an unresolved import — which
/// is exactly the state this file exists to leave behind. The worker's entry has
/// to be in the emitted program for the thread to have anything to run.
#[test]
fn the_imported_workers_entry_is_emitted() {
    let plan = try_code_for_fixture_project(
        "thread-fixed-list-transfer-rt",
        CodeTarget::LinuxX86_64,
        Console,
    )
    .expect("the fixture must lower");
    let names: Vec<&str> = plan.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(
        names.iter().any(|name| name.contains("doubleIntegers")),
        "the worker `fixed_list_xfer_worker::doubleIntegers` comes from the \
         fixture's .mfp; if the package had not been read, the program would \
         have lowered without it. Emitted: {names:?}"
    );
}
