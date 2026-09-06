//! The corpus, lowered at every optimization level.
//!
//! `src/optimizer/**` is 36 files and ~1,250 lines below the floor, and the
//! reason is structural rather than a missing test: `optimizer::active_opt_level`
//! defaults to `-O1`, so every program any test lowers runs the Level-1 rows and
//! nothing above them. The `-O2` and `-O3` catalog rows — the loop plans, the
//! range plans, jump threading, tail duplication — are reached only by a build
//! that asked for them, and no in-process test ever did.
//!
//! The dial is a *thread*-local under `cfg(test)` (deliberately: a write-once
//! process global would let the first test decide the level for every other),
//! and this harness lowers on a thread it spawns — so `code_for_src_at` pushes
//! the level INSIDE that thread. Without that the level is silently ignored and
//! this whole file would measure `-O1` four times.
//!
//! The contract is the one an optimizer has to keep and the one its own per-pass
//! tests cannot state over a whole corpus: **raising the level does not change
//! what the program IS.** Concretely, at every level the program still lowers,
//! still validates — `code::lower_module` rejects a branch to a label nothing
//! defines and a relocation against a symbol nothing emits, which is exactly
//! what a pass that rewrites a CFG gets wrong — and still contains its own
//! functions with non-empty bodies. A pass that drops a live function, empties a
//! body, or leaves a dangling edge is a miscompile, and each of those is caught
//! here rather than by a golden that only runs at one level.

use std::collections::BTreeSet;

use crate::codegen::engine::types::NativeCodePlan;
use crate::testutil::{code_for_src_at, fixture_src, CodeTarget};

/// Every level the dial accepts.
const LEVELS: [u8; 4] = [0, 1, 2, 3];

/// Programs chosen for the SHAPES the higher rows act on: counted loops, nested
/// loops with an invariant, index arithmetic the range plans can bound, long
/// branch chains for jump threading, and a join block worth tail-duplicating.
const SHAPES: &[&str] = &[
    "bounds-elim-rt",
    "control-flow-behavior",
    "control-flow-if",
    "bug118_match_guard_helper",
    "bug361_match_oneof_literals",
    "bulk-append-inplace",
    "mut-append-grow",
    "inplace-grow-free",
    "list-ops-codegen-rt",
    "nested-fixed-list-rt",
    "map-set-grow-rt",
    "set-algebra-rt",
    "reduce-accumulator-reclaim-rt",
    "json-behavior",
    "json-number-rendering-rt",
    "regex-posix-classes-rt",
    "float-fma-fusion",
    "record-field",
    "user-generic-nested-rt",
    "call-function-value-rt",
    "p121d-state-ops-rt",
    "func-bare-trap-loop-leak-rt",
    "get-borrow-match-rt",
    "recursive-get-then-grow-rt",
];

/// The program's OWN functions with a body — everything the source declared.
///
/// Runtime helpers (`runtime.*`), package bodies (`#pkg_*`) and constructors are
/// excluded because a higher level may legitimately delete one: folding every
/// `&` in a program at -O3 removes the last call to `runtime.stringConcat`, and
/// then the helper is dead. That is the optimizer working. A function the SOURCE
/// declared is a different matter.
fn program_functions(plan: &NativeCodePlan) -> BTreeSet<String> {
    plan.functions
        .iter()
        .filter(|f| f.instructions.len() > 1)
        .map(|f| f.name.clone())
        .filter(|name| {
            !name.starts_with("runtime.")
                && !name.starts_with("construct.")
                && !name.starts_with('#')
                && !name.starts_with("program.")
                && !name.starts_with('_')
        })
        .collect()
}

/// Every level lowers, validates, and keeps the program's own functions.
#[test]
fn the_corpus_survives_every_optimization_level() {
    for fixture in SHAPES {
        let source = fixture_src(fixture);
        let mut baseline: Option<BTreeSet<String>> = None;
        for level in LEVELS {
            let plan = code_for_src_at(
                &source,
                CodeTarget::LinuxX86_64,
                crate::target::NativeBuildMode::Console,
                level,
            )
            .unwrap_or_else(|err| panic!("{fixture} at -O{level}: {err}"));

            let functions = program_functions(&plan);
            assert!(
                functions.contains("main"),
                "{fixture} at -O{level}: the entry function must survive with a \
                 non-empty body"
            );
            match &baseline {
                None => baseline = Some(functions),
                Some(at_zero) => {
                    // The direction that matters is what went MISSING: a pass
                    // that drops a function the source declared, or empties its
                    // body, is a miscompile. (Gaining one is not: an inliner may
                    // split or clone.)
                    let missing: Vec<&String> = at_zero.difference(&functions).collect();
                    assert!(
                        missing.is_empty(),
                        "{fixture}: -O{level} lost {missing:?}, which -O0 emitted \
                         with a body. A pass that drops or empties a live function \
                         is a miscompile, not an optimization"
                    );
                }
            }
        }
    }
}

/// The WHOLE corpus at the top of the dial, not just the shapes chosen for it.
///
/// `SHAPES` above is 25 programs picked for the constructs the higher rows act
/// on, and it is the right list for the level-by-level comparison — four
/// lowerings each, and a level that changes nothing is a level that is not
/// arriving. It is the wrong list for *reach*: an `-O3` row fires on whatever
/// shape the program happens to have, and 25 hand-picked programs cannot stand
/// in for 630. The rows that were still unreached after `SHAPES` landed are the
/// ones no chosen program happened to contain — a sunk store, a redundant
/// bounds check on a shape the range plans recognise, a branch worth threading.
///
/// One level rather than four, because `level_enabled(row) = row <= active`:
/// `-O3` enables every row `-O2` does and more, so `-O3` alone reaches the whole
/// catalog. One lowering per program rather than two, because at 630 programs
/// the second is 125 seconds bought for a comparison `SHAPES` already makes at
/// four levels; what only this can say is that the top of the dial reaches the
/// other 605.
///
/// The contract is `no_corpus_function_lowers_to_an_empty_body`'s, asserted
/// where it has never been asserted: at `-O3` the program still lowers, still
/// validates (`code::lower_module` rejects a branch to a label nothing defines
/// and a relocation against a symbol nothing emits — precisely what a pass that
/// rewrites a CFG gets wrong), and still has no function whose body is only its
/// entry label. An empty body is what a pass that deleted a live function's
/// contents looks like from here.
#[test]
fn the_whole_corpus_survives_the_top_of_the_dial() {
    // Report every failure rather than the first: at 630 programs, finding them
    // one run at a time is the difference between a minute and an afternoon.
    let mut failed = Vec::new();
    for fixture in super::corpus::CORPUS {
        let source = fixture_src(fixture);
        let plan = match code_for_src_at(
            &source,
            CodeTarget::LinuxX86_64,
            crate::target::NativeBuildMode::Console,
            3,
        ) {
            Ok(plan) => plan,
            Err(err) => {
                failed.push(format!(
                    "{fixture} at -O3: {}",
                    err.lines().next().unwrap_or(&err)
                ));
                continue;
            }
        };
        for f in &plan.functions {
            if f.instructions.len() <= 1 {
                failed.push(format!(
                    "{fixture} at -O3: `{}` lowered to {} instruction(s) — a body \
                     that is only its entry label is a function the optimizer \
                     emptied",
                    f.name,
                    f.instructions.len()
                ));
            }
        }
        if !program_functions(&plan).contains("main") {
            failed.push(format!(
                "{fixture} at -O3: the entry function did not survive with a body"
            ));
        }
    }
    assert!(
        failed.is_empty(),
        "{} corpus program(s) did not survive -O3:\n  {}",
        failed.len(),
        failed.join("\n  ")
    );
}

/// Raising the level actually changes the emitted code.
///
/// The other half, and the reason it is here: a level that never reaches the
/// lowering would pass every assertion above while optimizing nothing. That is
/// not hypothetical — it is exactly what the first draft of this file did,
/// because the dial is thread-local and the lowering runs on a thread the
/// harness spawns. Half the programs must lower differently at `-O3` than at
/// `-O0`, or the level is not arriving.
#[test]
fn raising_the_level_changes_the_emitted_code() {
    let mut changed = 0;
    for fixture in SHAPES {
        let source = fixture_src(fixture);
        let at = |level: u8| {
            code_for_src_at(
                &source,
                CodeTarget::LinuxX86_64,
                crate::target::NativeBuildMode::Console,
                level,
            )
            .unwrap_or_else(|err| panic!("{fixture} at -O{level}: {err}"))
            .functions
            .iter()
            .map(|f| f.instructions.len())
            .sum::<usize>()
        };
        if at(0) != at(3) {
            changed += 1;
        }
    }
    assert!(
        changed * 2 >= SHAPES.len(),
        "only {changed} of {} programs lower differently at -O3 than at -O0; the \
         optimization dial is not reaching the lowering",
        SHAPES.len()
    );
}
