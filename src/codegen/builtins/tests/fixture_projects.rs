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

/// Every `tests/rt-behavior/**` fixture that needs its PROJECT, not just a
/// source string.
///
/// 87 of them, and not one was reachable in process before: each has a
/// `packages/` directory its `project.json` declares, and `fixture_src` reads
/// `src/main.mfb` alone. They are the thread and native-`LINK` surface almost
/// in its entirety — a worker's entry lives in a package, so nearly every
/// `thread::` fixture is one of these.
///
/// Named rather than discovered, for the reason `corpus.rs` gives: a list that
/// silently skips what it cannot lower is a gate that stops measuring. Nothing
/// is deliberately absent: the four `libsnd-*` fixtures declare their
/// `LINK "libsnd"` INSIDE the package, so its locators live in the package's own
/// section-10 table rather than in the consumer's `libraries` section — and the
/// loader below merges the two tables (`read_package_native_libraries` per
/// package, after `native_libraries_for_test` for the project), which is what
/// lets them in.
const PACKAGE_FIXTURES: &[&str] = &[
    "allocator-04-thread-arena-init",
    "bug104_aliased_overload_import",
    "bug221_transfer_accept_named_args",
    "func_thread_closeStdIn_valid",
    "func_thread_result_valid",
    "func_thread_transfer_valid",
    "libsnd-load-sound-rt",
    "libsnd-playback-rt",
    "native-link-alias-collision-rt",
    "native-link-import-sqlite-rt",
    "native-resource-import-valid",
    "native-resource-state-import-rt",
    "os-env-thread-race-rt",
    "os-sleep-worker-cancel-rt",
    "os-sleep-worker-rt",
    "p121d-state-reach-rt",
    "project-fs-createTempFile-package-valid",
    "project-record-comparable-package-valid",
    "project-with-package-import-as",
    "record-res-field-export-rt",
    "resource-state-import-rt",
    "thread-bounded-queues",
    "thread-drop-cleanup",
    "thread-dual-cancel",
    "thread-error-source-rt",
    "thread-fixed-list-transfer-rt",
    "thread-fs-close-rt",
    "thread-fs-listdir-order-rt",
    "thread-fs-pathjoin-rt",
    "thread-fs-read-return",
    "thread-fs-readtext-return",
    "thread-import-package-print",
    "thread-import-pkg-receive-rt",
    "thread-link-worker-rt",
    "thread-main-poll",
    "thread-package-fanout-rt",
    "thread-package-globals-rt",
    "thread-print-count",
    "thread-queue-timeout-cancel",
    "thread-receive-print",
    "thread-regex-rt",
    "thread-resource-transfer-fail-leak",
    "thread-resource-transfer-fail-reclaim",
    "thread-return-byte",
    "thread-return-fixed",
    "thread-return-float",
    "thread-return-integer",
    "thread-return-list-of-string",
    "thread-return-map-of-string-to-string",
    "thread-return-string",
    "thread-return-type",
    "thread-return-union",
    "thread-send-file-ownership-rt",
    "thread-start-invalid-limit-trapped",
    "thread-strings-split-return",
    "thread-timeout-convention-rt",
    "thread-transfer-bidirectional-rt",
    "thread-transfer-state-rt",
    "thread-transfer-tcp-listener-rt",
    "thread-transfer-tls-socket-rt",
    "thread-transfer-union-state-rt",
    "thread-transfer-union-stateless-rt",
    "trap-builtin-consumer",
    // --- not package-bearing, but still needs its PROJECT ---
    //
    // Each of these fails from `src/main.mfb` alone for a reason the manifest
    // answers. The `native-*` and `libsnd-*` fixtures declare a `LINK` whose
    // locators live in the project's own `libraries` section; the
    // `project-entry-*-foobar-*` ones name an entry that is not `main`, which
    // the source-string path hardcodes; and `func_override_visibility` has a
    // second source file.
    "func_override_visibility",
    "libsnd-open-file-info-rt",
    "libsnd-read-samples-rt",
    "native-cbuffer-read-rt",
    "native-closed-guard-rt",
    "native-link-and-truthy-rt",
    "native-link-const-64bit-rt",
    "native-link-free-rt",
    "native-link-inline-trap-rt",
    "native-link-nested-success-rt",
    "native-link-sqlite-rt",
    "native-private-resource-rt",
    "native-resource-in-collection-rt",
    "native-resource-scope-drop-rt",
    "native-resource-state-rt",
    "native-resource-thread-accept-rt",
    "native-resource-transfer-fail-usable-rt",
    "native-stateless-record-rt",
    "native-struct-cstring-rt",
    "native-struct-scalar-rt",
    "project-entry-func-args-foobar-trap",
    "project-entry-func-foobar-trap",
    "project-entry-sub-args-foobar-trap",
    "project-entry-sub-foobar-trap",
];

/// Each one lowers, on every backend.
///
/// The same assertion `corpus.rs` makes for its own list. These are here rather
/// than there because they need a different entry point, not because they are
/// held to a different standard.
#[test]
fn every_package_bearing_fixture_lowers_on_every_backend() {
    for fixture in PACKAGE_FIXTURES {
        for target in CodeTarget::ALL {
            // A fixture whose `LINK` names a vendored or system library can
            // only be built for the targets that library declares a locator
            // for, and several of these declare macOS and Linux and no
            // Windows. That refusal is the product working: a vendored library
            // is a file per (os, arch, libc), and a build for a triple the
            // binding never declared has to stop with the list of what IS
            // declared rather than emit a binary that fails to `dlopen` on the
            // user's machine.
            //
            // Derived rather than listed. A hand-maintained skip list would
            // grow a row every time a LINK fixture is added, and each row would
            // be an unexamined exclusion; this admits exactly one refusal and
            // still fails on every other.
            let plan = match try_code_for_fixture_project(fixture, target, Console) {
                Ok(plan) => plan,
                Err(err) if is_no_locator_for_target(&err, target) => continue,
                Err(err) => panic!("{fixture} on {}: {err}", target.name()),
            };
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

/// True when `err` is the one refusal a target-limited `LINK` library produces.
///
/// Deliberately narrow: it must name the target it could not resolve for, so a
/// generic "cannot resolve native library" from some other cause does not slip
/// through as an expected skip.
fn is_no_locator_for_target(err: &str, target: CodeTarget) -> bool {
    let (os, arch) = match target {
        CodeTarget::MacosAarch64 => ("macos", "aarch64"),
        CodeTarget::WindowsX86_64 => ("windows", "x86_64"),
        CodeTarget::LinuxAarch64 => ("linux", "aarch64"),
        CodeTarget::LinuxX86_64 => ("linux", "x86_64"),
        CodeTarget::LinuxRiscv64 => ("linux", "riscv64"),
    };
    err.contains("cannot resolve native library") && err.contains(os) && err.contains(arch)
}

/// A `LINK` library with no locator for the target refuses, and names the target.
///
/// The refusal is the feature. A vendored library is a file per `(os, arch,
/// libc)`, and a build for a triple the binding never declared cannot invent
/// one — so it has to stop at build time with the list of what IS declared,
/// rather than emit a binary that fails to `dlopen` on the user's machine.
///
/// This is also what keeps the skip above honest: without it, `supports`
/// would be an unexamined exclusion, and a `libsnd` that silently started
/// resolving to nothing on Windows would look like progress.
#[test]
fn a_link_library_with_no_locator_for_the_target_refuses() {
    let Err(message) =
        try_code_for_fixture_project("libsnd-load-sound-rt", CodeTarget::WindowsX86_64, Console)
    else {
        panic!(
            "`libsnd.mfp` declares locators for macOS/aarch64 and six Linux \
             flavors and none for Windows, so a windows-x86_64 build must \
             refuse rather than emit a binary that fails to load"
        );
    };
    assert!(
        message.contains("libsnd"),
        "the refusal must name the library that could not be resolved: {message}"
    );
    assert!(
        message.contains("windows") && message.contains("x86_64"),
        "the refusal must name the TARGET it could not resolve for -- that is \
         what tells the reader to add a locator rather than to look at the \
         source: {message}"
    );
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
