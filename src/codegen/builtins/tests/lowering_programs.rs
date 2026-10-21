//! Every program under `tests/codegen/lowering_programs/` lowers, validates, and
//! leaves no function empty.
//!
//! The directory holds single-file programs aimed at resource ownership, closures,
//! inline `TRAP` positions, thread transfer, `typeName` folds and the collection
//! self-updates - the shapes where lowering has to decide who frees what:
//!
//! * the `bug-*.mfb` programs are the reproducers of bugs fixed in the ownership
//!   model (a resource list floated out of a loop, a list of `RES` handed to a
//!   callee, a record rebuilt under an inline `TRAP`, a capture appended to from a
//!   `forEach` lambda, ...). Each was a wrong-code or leak bug at run time, and the
//!   acceptance suite runs it; this suite lowers the same source in process, so the
//!   shared codegen it reaches is also exercised by the unit-test binary;
//! * the others are programs written for this suite.
//!
//! The contract is the corpus's (`corpus.rs`): `try_code_for_src` validates as it
//! lowers - a branch to an undefined label, a relocation against a symbol nothing
//! defines, a data object nobody emitted is rejected - and no function lowers to
//! just its entry label, which is what a lowering that silently declined looks like.
//! The program list is the directory, and its size is asserted, so a program that
//! is deleted or stops being found fails here rather than quietly dropping out.

use std::path::Path;

use crate::target::NativeBuildMode::Console;
use crate::testutil::{try_code_for_src, CodeTarget};

/// How many programs the directory held when this suite was written.
const PROGRAMS: usize = 65;

#[test]
fn every_lowering_program_lowers_and_no_function_is_empty() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/codegen/lowering_programs");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .unwrap_or_else(|err| panic!("{}: {err}", dir.display()))
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "mfb"))
        .collect();
    files.sort();
    assert!(
        files.len() >= PROGRAMS,
        "{} holds {} programs; it held {PROGRAMS} when this suite was written, and a \
         directory that shrinks silently is a gate that stops measuring",
        dir.display(),
        files.len()
    );
    let mut failed = Vec::new();
    for file in &files {
        let name = file.file_name().expect("a file name").to_string_lossy();
        let source =
            std::fs::read_to_string(file).unwrap_or_else(|err| panic!("{}: {err}", file.display()));
        let plan = match try_code_for_src(&source, CodeTarget::LinuxX86_64, Console) {
            Ok(plan) => plan,
            Err(err) => {
                failed.push(format!("{name}: {}", err.lines().next().unwrap_or(&err)));
                continue;
            }
        };
        for function in &plan.functions {
            if function.instructions.len() <= 1 {
                failed.push(format!(
                    "{name}: `{}` lowered to {} instruction(s) - a body that is only its \
                     entry label is a lowering that declined",
                    function.name,
                    function.instructions.len()
                ));
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} program(s) did not lower:\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
}
