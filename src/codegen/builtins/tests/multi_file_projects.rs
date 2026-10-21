//! Projects of several source files, lowered as projects.
//!
//! `corpus.rs` and `fixture_src` read one `src/main.mfb`. A function defined in another
//! file of the same project is a different thing to the lowerer: its source location
//! names a file the module's table interns, an inlined callee's error location must
//! resolve to ITS file rather than the caller's, a generic template declared in a LATER
//! file is instantiated from the first, and a private name in one file must not collide
//! with the same name in another. Those paths — `source_file_id` / `source_file_name`
//! for a foreign file, the inline callbacks that rewrite a callee's locations into the
//! caller's frame — are reached only by a project with more than one file.
//!
//! Each fixture below is such a project; it lowers on two backends (the file-table logic
//! is shared, the instruction selection is not what is under test) and its plan must name
//! more than one source file.

use std::collections::BTreeSet;

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_fixture_project, CodeTarget};

const MULTI_FILE_FIXTURES: &[&str] = &[
    "func_override_visibility",
    "toInt_enum_package",
    "toString_enum_package",
    "template-in-later-file-error-source-rt",
    "template-in-later-file-private-scope-rt",
    "type-template-in-later-file-private-rt",
    "local-names-distinct-from-top-level-valid",
    "private-overload-beside-export-rt",
];

#[test]
fn every_multi_file_project_lowers_and_names_its_files() {
    for fixture in MULTI_FILE_FIXTURES {
        for target in [CodeTarget::LinuxX86_64, CodeTarget::MacosAarch64] {
            let plan = try_code_for_fixture_project(fixture, target, Console)
                .unwrap_or_else(|err| panic!("{fixture} must lower on {}: {err}", target.name()));
            // Each function's source file backs its errors' `ErrorLoc.filename`, emitted as
            // a string constant; a project of several files names several of them.
            let files: BTreeSet<&str> = plan
                .data_objects
                .iter()
                .map(|object| object.value.as_str())
                .filter(|text| text.ends_with(".mfb"))
                .collect();
            assert!(
                files.len() >= 2,
                "{fixture} on {}: a project of several files must name more than one \
                 source file in its plan; it names {files:?}",
                target.name()
            );
        }
    }
}
