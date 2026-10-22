//! The three collection payload matches (`emit_collection_payload_match_branch`,
//! `..._matches_value_branch`, `..._payloads_match_branch`) compare only what the
//! language calls comparable, and refuse anything else by name.
//!
//! Every caller hands them a `Map` key, a `Set` element, or the element of a
//! `contains`/`find`/`replace`/`distinct` list, and `ir::verify::is_comparable` gates
//! each of those: a primitive, an enum, or a record of comparables. So a nested
//! collection, a resource or a union never arrives from a program. These rows drive
//! the emitters directly, on the type model of a real program, with exactly those
//! types — the shapes the checker stops — and pin that the emitter answers `Err`
//! naming the type rather than comparing a pointer word (identity, not equality, for
//! a nested list) or a union's raw bytes (which include a pointer for a `String`
//! field). A record whose field is one of them is refused the same way, from the
//! field comparison inside the record arm.

use crate::codegen::engine::builder::TypeModel;
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT io

TYPE Circle
  r AS Integer
END TYPE

TYPE Label
  text AS String
END TYPE

UNION Shape
  Circle
  Label
END UNION

TYPE Holder
  shape AS Shape
  n AS Integer
END TYPE

TYPE Pt
  x AS Integer
  name AS String
END TYPE

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

/// Which of the three entry points.
#[derive(Clone, Copy, Debug)]
enum Entry {
    /// `emit_collection_payload_match_branch` — a packed payload against a value
    /// (map get/set, set membership, `contains`, in-place `distinct`).
    PayloadMatch,
    /// `emit_collection_payload_matches_value_branch` — the same, through private
    /// scratch (map find/remove, list `find`/`replace`).
    MatchesValue,
    /// `emit_collection_payloads_match_branch` — two packed payloads (sublist `find`).
    PayloadsMatch,
}

const ENTRIES: [Entry; 3] = [
    Entry::PayloadMatch,
    Entry::MatchesValue,
    Entry::PayloadsMatch,
];

/// Drive `entry` for a payload of `type_`; the instructions it emitted, or its refusal.
fn emit(
    entry: Entry,
    type_: &ParameterType,
) -> Result<Vec<crate::codegen::engine::types::CodeInstruction>, String> {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let harness = BuilderHarness {
        type_model: TypeModel::from_module(&module).expect("the model builds"),
        ..BuilderHarness::default()
    };
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_payload_match_probe", &platform);
    let left = builder.temporary_vreg();
    let left_offset = builder.temporary_vreg();
    let left_length = builder.temporary_vreg();
    let right = builder.temporary_vreg();
    let right_offset = builder.temporary_vreg();
    let right_length = builder.temporary_vreg();
    let stride = ParameterType::named("");
    match entry {
        Entry::PayloadMatch => builder.emit_collection_payload_match_branch(
            type_,
            &stride,
            left,
            left_offset,
            left_length,
            right,
            "probe_equal",
            "probe_unequal",
        )?,
        Entry::MatchesValue => builder.emit_collection_payload_matches_value_branch(
            type_,
            &stride,
            left,
            left_offset,
            left_length,
            right,
            "probe_equal",
            "probe_unequal",
        )?,
        Entry::PayloadsMatch => builder.emit_collection_payloads_match_branch(
            type_,
            &stride,
            left,
            left_offset,
            left_length,
            right,
            right_offset,
            right_length,
            "probe_equal",
            "probe_unequal",
        )?,
    }
    Ok(builder.instructions)
}

/// The refusal `entry` gives for a payload of `type_`, which it must refuse.
fn refusal(entry: Entry, type_: &ParameterType) -> String {
    match emit(entry, type_) {
        Err(err) => err,
        Ok(_) => panic!("{entry:?} must refuse a `{type_}` payload, not compare it"),
    }
}

fn branches_to(
    instructions: &[crate::codegen::engine::types::CodeInstruction],
    label: &str,
) -> bool {
    instructions
        .iter()
        .any(|i| i.get("target").as_deref() == Some(label))
}

/// A nested collection that is not flat (its element is a `String`), a resource and
/// a union are not comparable, and no entry point compares one.
#[test]
fn an_incomparable_payload_is_refused_by_name_at_every_entry() {
    let incomparable = [
        ParameterType::list_of(ParameterType::String),
        ParameterType::named("fs.File"),
        ParameterType::declared("Shape"),
    ];
    for entry in ENTRIES {
        for type_ in &incomparable {
            let err = refusal(entry, type_);
            assert!(
                err.contains("packed payload does not support type")
                    && err.contains(&type_.to_string()),
                "{entry:?} names the refused type `{type_}`: {err}"
            );
        }
    }
}

/// A record is compared field by field; a field that is not comparable is refused by
/// the field comparison, and the refusal reaches the caller instead of being dropped.
#[test]
fn a_record_with_an_incomparable_field_is_refused_at_every_entry() {
    for entry in ENTRIES {
        let err = refusal(entry, &ParameterType::declared("Holder"));
        assert!(
            err.contains("does not support type 'Shape'"),
            "{entry:?} names the field's type: {err}"
        );
    }
}

/// The refusals are about the type, not blanket: a comparable record lowers at every
/// entry point and branches to both outcomes.
#[test]
fn a_comparable_record_is_compared_at_every_entry() {
    for entry in ENTRIES {
        let instructions = emit(entry, &ParameterType::declared("Pt"))
            .unwrap_or_else(|err| panic!("{entry:?} compares a record of comparables: {err}"));
        assert!(
            branches_to(&instructions, "probe_equal")
                && branches_to(&instructions, "probe_unequal"),
            "{entry:?} branches to both outcomes"
        );
    }
}
