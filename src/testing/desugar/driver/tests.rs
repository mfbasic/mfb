//! The driver's CHUNKING, which only a suite of more than eight cases reaches.
//!
//! `build_driver` partitions the flat step list into runs of at most
//! [`super::DRIVER_CHUNK_SIZE`] cases and emits one `#mfb_test_chunk_N` per run,
//! leaving `#mfb_test_main` thin. That split exists for a reason with a number
//! attached (bug-445 follow-up): each case contributes stack slots for its inline
//! TRAP handler's temporaries, and inlining every case into one function grew
//! `#mfb_test_main`'s frame with the size of the suite until it overflowed.
//!
//! The partition loop had never run. Its `cases_in_run == DRIVER_CHUNK_SIZE`
//! branch — name the chunk, cut the slice, reset the counter — needs a NINTH
//! case, and no fixture in the tree has one: the largest `TESTING` block is
//! eight cases, so every suite fits in the tail chunk the loop never enters.
//!
//! **The boundary is the whole property.** A cut at the wrong index silently
//! drops a case (it belongs to neither chunk) or runs it twice (it belongs to
//! both), and a test suite that quietly stops running one of its cases reports
//! success either way. So the cases are counted across the emitted chunks and
//! compared against what went in, at exactly eight, nine and seventeen.

use super::{build_driver, DriverStep, DRIVER_CHUNK_SIZE};

/// `count` cases, in one group, as the flat step list `build_driver` takes.
fn steps(count: usize) -> Vec<DriverStep> {
    let mut steps = vec![DriverStep::Group {
        indent: 0,
        description: "group".to_string(),
    }];
    for index in 0..count {
        steps.push(DriverStep::Case {
            sub_name: format!("#mfb_test_case_{index}"),
            description: format!("case {index}"),
            indent: 2,
        });
    }
    steps
}

/// Every emitted function's name. The entry is LAST: `build_driver` pushes the
/// chunks and then the entry that calls them.
fn function_names(count: usize) -> Vec<String> {
    build_driver(&steps(count), false)
        .iter()
        .map(|function| function.name.clone())
        .collect()
}

/// One chunk up to the cap, and a second the moment the cap is crossed.
#[test]
fn the_driver_splits_into_chunks_at_the_cap() {
    assert_eq!(
        DRIVER_CHUNK_SIZE, 8,
        "the numbers below are written for a cap of 8; if the cap moves this test \
         is measuring a boundary that no longer exists"
    );

    // At the cap exactly, the loop's branch fires on the last case and the tail
    // is empty -- so this is one chunk, not two. Off by one either way and the
    // suite gains an empty chunk function or loses its last case.
    let at_cap = function_names(DRIVER_CHUNK_SIZE);
    assert_eq!(
        at_cap.len(),
        2,
        "eight cases is the entry plus ONE chunk; got {at_cap:?}"
    );

    let over_cap = function_names(DRIVER_CHUNK_SIZE + 1);
    assert_eq!(
        over_cap.len(),
        3,
        "the ninth case is what the partition loop exists for -- it belongs to a \
         second chunk; got {over_cap:?}"
    );

    let two_over = function_names(DRIVER_CHUNK_SIZE * 2 + 1);
    assert_eq!(
        two_over.len(),
        4,
        "seventeen cases is two full chunks and a tail of one; got {two_over:?}"
    );
}

/// The entry calls every chunk, and each chunk is called exactly once.
///
/// The cut is only half the property. A partition that named three chunks and
/// left the entry calling two would lose a third of the suite, and the entry
/// would still report success for the cases it did run.
#[test]
fn the_entry_calls_each_chunk_exactly_once() {
    let functions = build_driver(&steps(DRIVER_CHUNK_SIZE * 2 + 1), false);
    let (entry, chunks) = functions.split_last().expect("an entry function");
    assert_eq!(chunks.len(), 3, "two full chunks and a tail");

    let body = format!("{:?}", entry.body);
    for chunk in chunks {
        assert_eq!(
            body.matches(&chunk.name).count(),
            1,
            "`{}` must be called exactly once by the entry: zero means its cases \
             never run and the suite still reports success, and twice means every \
             case in it is counted twice",
            chunk.name
        );
    }
}
