//! plan-146-G's shared `String` capacity shadow, in process
//! (`engine/control/builder_control/string_shadow.rs`).
//!
//! A frame that self-appends `s` (`s = s & …`) leaves spare bytes in `s`'s block
//! and records how many in its capacity shadow. A lambda that captures `s` by
//! reference and REASSIGNS it frees that block through the reference, so the free
//! must be sized by the owner's shadow, read through the closure environment, and
//! the shadow reset to 0 for the fresh tight block (an under-sized free plus a
//! stale shadow is the heap overflow `tests/runtime/rt_byref_string_capture_capacity.rs`
//! was written for — that suite runs the programs; this one lowers them, which is
//! what reaches the emitter in process). Asserted through the stack objects the
//! lambda allocates for the two words.

use crate::target::NativeBuildMode::Console;
use crate::testutil::{code_for_src_cached, CodeTarget};

fn program(owner_appends: bool, lambda: &str) -> String {
    let appends = if owner_appends {
        "  FOR i = 1 TO 100\n    s = s & \"b\"\n  NEXT\n"
    } else {
        ""
    };
    format!(
        "IMPORT collections\nIMPORT io\nIMPORT strings\n\nFUNC main() AS Integer\n  MUT s AS String = \"a\"\n{appends}  collections::forEach([\"x\", \"y\"], {lambda})\n  io::print(toString(len(s)))\n  RETURN 0\nEND FUNC\n"
    )
}

/// Every lowered function's stack-object names, for each backend.
fn slot_names(source: &str, target: CodeTarget) -> Vec<String> {
    code_for_src_cached(source, target, Console)
        .functions
        .iter()
        .flat_map(|function| function.stack_slots.iter().map(|slot| slot.name.clone()))
        .collect()
}

fn has_slot(names: &[String], prefix: &str) -> bool {
    names.iter().any(|name| name.starts_with(prefix))
}

/// A reassignment through the reference, of a `String` whose owner self-appends,
/// reads the owner's shadow (`ref_strcap_free`) and keeps the owner's address to
/// reset it (`ref_strcap_owner`), on every backend.
#[test]
fn a_reassignment_through_a_reference_frees_by_the_owners_shadow() {
    for lambda in [
        "LAMBDA(v AS String) -> s = v & \"q\"",
        "LAMBDA(v AS String) -> s = strings::repeat(v, 3)",
    ] {
        let source = program(true, lambda);
        for target in CodeTarget::ALL {
            let names = slot_names(&source, target);
            for slot in ["strcap_s", "ref_strcap_free", "ref_strcap_owner"] {
                assert!(
                    has_slot(&names, slot),
                    "{}: `{lambda}` has no `{slot}` stack object: {names:?}",
                    target.name()
                );
            }
        }
    }
}

/// With no self-append anywhere there is no shadow to share: the reassignment
/// frees the tight size and allocates neither word.
#[test]
fn with_no_self_append_there_is_no_shared_shadow() {
    let source = program(false, "LAMBDA(v AS String) -> s = v & \"q\"");
    let names = slot_names(&source, CodeTarget::LinuxX86_64);
    for slot in ["strcap_s", "ref_strcap_free", "ref_strcap_owner"] {
        assert!(
            !has_slot(&names, slot),
            "`{slot}` without a self-append: {names:?}"
        );
    }
}
