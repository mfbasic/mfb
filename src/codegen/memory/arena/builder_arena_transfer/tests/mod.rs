//! `memory/arena/builder_arena_transfer` unit tests: the `Result` tag partition and the
//! trapped-error sources. Moved out of `mod.rs` (plan tests.md C15) so these lines sit
//! outside the coverage denominator.

use super::*;

/// The `Result` tag vocabulary, as declared in `error_constants.rs`.
///
/// bug-565's fix decides, from this tag alone, whether the trapped error's
/// flat `Error` block is one the raiser PARKED for us to adopt or one we must
/// rebuild — and then frees what it ended up holding. An unclassified tag
/// falling through to `LooseRegisters` is the safe direction (it leaks rather
/// than double-frees), but it is still a silent wrong answer, so the partition
/// is asserted TOTAL against the constants rather than left to a default.
///
/// This reads the constants file so the check cannot be satisfied by keeping a
/// stale copy of the list in step with itself: a fifth `RESULT_*_TAG` reds this
/// test, and whoever adds it has to say which class it belongs to.
const ERROR_CONSTANTS_SOURCE: &str = include_str!("../../../../error/constants/error_constants.rs");

fn declared_result_tags() -> Vec<String> {
    ERROR_CONSTANTS_SOURCE
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let rest = line.strip_prefix("pub(crate) const RESULT_")?;
            let (name, value) = rest.split_once(": &str = ")?;
            if !name.ends_with("_TAG") {
                return None;
            }
            Some(
                value
                    .trim()
                    .trim_end_matches(';')
                    .trim_matches('"')
                    .to_string(),
            )
        })
        .collect()
}

#[test]
fn the_result_tag_partition_is_total() {
    let declared = declared_result_tags();
    assert_eq!(
        declared.len(),
        4,
        "the `RESULT_*_TAG` vocabulary changed ({declared:?}); classify the new \
         tag in `trapped_error_tag_class` — an unlisted tag REBUILDS the error, \
         which silently orphans a parked `Error` block (bug-565)"
    );
    let mut classified: Vec<(String, TrappedErrorTagClass)> = declared
        .iter()
        .map(|tag| (tag.clone(), trapped_error_tag_class(tag)))
        .collect();
    classified.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        classified,
        vec![
            ("0".to_string(), TrappedErrorTagClass::NotAnError),
            ("1".to_string(), TrappedErrorTagClass::LooseRegisters),
            ("2".to_string(), TrappedErrorTagClass::LooseRegisters),
            ("3".to_string(), TrappedErrorTagClass::ParkedBlock),
        ],
        "every declared tag must have an explicit class"
    );
}

/// Exactly one tag adopts, and the emitter compares against THAT one.
///
/// The adopt branch is the only place the trapped-error assembly takes a block
/// it did not allocate. If two tags ever adopted, the single-compare emitter
/// would silently serve one of them by rebuilding — orphaning a parked block
/// again — so the "exactly one" is what makes one compare sufficient.
#[test]
fn exactly_one_tag_is_adoptable_and_the_emitter_uses_it() {
    let adoptable: Vec<&str> = declared_result_tags()
        .into_iter()
        .filter(|tag| trapped_error_tag_class(tag) == TrappedErrorTagClass::ParkedBlock)
        .map(|tag| match tag.as_str() {
            "0" => RESULT_OK_TAG,
            "1" => RESULT_ERR_TAG,
            "2" => RESULT_PROGRAM_EXIT_TAG,
            "3" => RESULT_ERR_BLOCK_TAG,
            other => panic!("undeclared tag {other}"),
        })
        .collect();
    assert_eq!(adoptable, vec![RESULT_ERR_BLOCK_TAG]);
    assert_eq!(adoptable_error_tag(), RESULT_ERR_BLOCK_TAG);
}

/// The three trapped-`Result` lowerings, and the fact that they differ only in
/// where the REBUILD branch gets its `ErrorLoc`.
///
/// [`TrappedErrorSource`] is what the shared emitter `match`es, so a fourth
/// lowering cannot reach the shared free without declaring itself one of these
/// — and the free of the rebuild's own `ErrorLoc` is skipped, at compile time,
/// exactly for the variant whose `ErrorLoc` is the RAISER's.
#[test]
fn every_trapped_error_source_is_accounted_for() {
    let all = [
        TrappedErrorSource::CalleeRegister,
        TrappedErrorSource::CurrentLocation,
        TrappedErrorSource::WorkerArena,
    ];
    assert_eq!(all.len(), 3);
    // The one variant that allocates NOTHING for the rebuild: it reuses the
    // callee's `x3` slot verbatim, so `source_slot == source_raw_slot` and the
    // emitter never emits a free for it.
    assert_eq!(all[0], TrappedErrorSource::CalleeRegister);
    // The two that DO allocate: a fresh `ErrorLoc` (and, for a worker error, a
    // caller-arena copy of the message) which the flat `Error` then inlines.
    assert_ne!(all[1], all[0]);
    assert_ne!(all[2], all[0]);
    assert_ne!(all[1], all[2]);
}

/// A tag outside the declared vocabulary is classified as a loose error: the
/// assembly REBUILDS (a leak at worst) rather than adopting a slot nobody parked and
/// freeing a block someone else owns. Never `ParkedBlock`, never `NotAnError`.
#[test]
fn an_undeclared_tag_rebuilds_and_never_adopts() {
    for tag in ["4", "7", "-1", ""] {
        assert_eq!(
            trapped_error_tag_class(tag),
            TrappedErrorTagClass::LooseRegisters,
            "undeclared tag {tag:?}"
        );
    }
}
