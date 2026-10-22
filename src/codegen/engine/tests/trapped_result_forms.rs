//! The heap (`{tag, size, payload}`) form of an inline-`TRAP` result, as
//! `materialize_current_result` builds it (`memory/arena/builder_arena_transfer/result.rs`).
//!
//! Since bug-778 every trapped value whose payload the frame form can own is an
//! unboxed temporary, so a program reaches the heap form only for the payloads
//! `unboxed_trap_payload_is_block` declines (a graph, a `Result`, a resource) — and
//! no runtime helper returns one of those as a block this frame owns, nor is a
//! discard-error conversion ever declined. The two arms below are still the
//! contract of the function every one of its six call sites relies on, so these
//! rows drive it directly, on the type model of a real program:
//!
//! * `RawSuccessBlock::OwnedByThisFrame` with a block payload frees the producer's
//!   block after the copy (bug-566), behind a run-time pointer compare against the
//!   copy (a double-free guard), and a graph payload is dropped as a graph
//!   (plan-134). `OwnedElsewhere` frees nothing of the producer's.
//! * `raw_result_discard_error` (plan-64-I) builds a bare tag-only `Result` on the
//!   error branch: no `ErrorLoc`, no flat `Error` block, no adopt.
//!
//! And the unboxed form refuses, by name, the two producers its eligibility keeps
//! out (a worker-arena error and a borrowed resource) rather than emitting a frame
//! form that would free a block another owner holds.

use crate::arch::ops::CodeOp;
use crate::codegen::engine::builder::{TypeModel, UnboxedTrapResult};
use crate::codegen::engine::tests::test_support::{BuilderHarness, Stream, TestPlatform};
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::memory::arena::builder_arena_transfer::RawSuccessBlock;
use crate::testutil::{nir_for_src, CodeTarget};
use crate::types::ParameterType;

const PROGRAM: &str = "\
IMPORT io

TYPE Leaf
  v AS Integer
END TYPE

TYPE Node2
  left AS Tree
  right AS Tree
END TYPE

UNION Tree
  Leaf
  Node2
END UNION

FUNC main() AS Integer
  io::print(\"ok\")
  RETURN 0
END FUNC
";

/// How the probe drives `materialize_current_result`.
#[derive(Clone, Copy)]
struct Probe {
    raw_success: RawSuccessBlock,
    discard_error: bool,
    worker_error_source: bool,
    unboxed: bool,
}

const HEAP: Probe = Probe {
    raw_success: RawSuccessBlock::OwnedElsewhere,
    discard_error: false,
    worker_error_source: false,
    unboxed: false,
};

fn materialize(success: &ParameterType, probe: Probe) -> Result<Vec<CodeInstruction>, String> {
    let module = nir_for_src(
        PROGRAM,
        CodeTarget::LinuxX86_64,
        crate::target::NativeBuildMode::Console,
    )
    .expect("the program lowers");
    let mut harness = BuilderHarness {
        type_model: TypeModel::from_module(&module).expect("the model builds"),
        ..BuilderHarness::default()
    };
    // The rodata strings the error paths name (a real plan interns them up front).
    for (n, text) in ["Allocation failed."].iter().enumerate() {
        harness
            .string_symbols
            .insert(text.to_string(), format!("_mfb_str_probe{n}"));
    }
    let platform = TestPlatform;
    let mut builder = harness.builder("_mfb_trapped_result_probe", &platform);
    if probe.unboxed {
        let tag_slot = builder.allocate_stack_object("probe_tag", 8);
        let payload_slot = builder.allocate_stack_object("probe_payload", 8);
        let error_slot = builder.allocate_stack_object("probe_error", 8);
        builder.unboxed_trap_active = Some(UnboxedTrapResult {
            tag_slot,
            payload_slot,
            error_slot,
            payload: success.clone(),
            block: builder.result_payload_is_block(success),
            error_read: true,
        });
    }
    builder.raw_result_discard_error = probe.discard_error;
    builder.materialize_current_result(
        success,
        "callResult probe".to_string(),
        probe.worker_error_source,
        probe.raw_success,
    )?;
    Ok(builder.instructions)
}

fn emitted(success: &ParameterType, probe: Probe) -> Vec<CodeInstruction> {
    materialize(success, probe).unwrap_or_else(|err| panic!("`{success}` materializes: {err}"))
}

fn stream(instructions: &[CodeInstruction]) -> Stream<'_> {
    Stream {
        name: "_mfb_trapped_result_probe",
        instructions,
    }
}

fn has_label_starting(instructions: &[CodeInstruction], prefix: &str) -> bool {
    stream(instructions)
        .labels()
        .iter()
        .any(|(_, name)| name.starts_with(prefix))
}

/// The producer's block is freed after the copy, but only when the pointer the
/// copy produced differs from it: the compare and its skip label come first, then
/// the free, then the label.
#[test]
fn an_owned_block_payload_frees_the_producer_behind_a_pointer_compare() {
    let owned = emitted(
        &ParameterType::String,
        Probe {
            raw_success: RawSuccessBlock::OwnedByThisFrame,
            ..HEAP
        },
    );
    let s = stream(&owned);
    let kept = s.label_starting("raw_helper_result_kept");
    let guard = s.index_of("the branch to the kept label", |i| {
        i.get("target").as_deref() == Some(kept.as_str())
    });
    assert_eq!(
        s.branch_condition_to(&kept),
        "eq",
        "an identical pointer skips the free (it would be a double free)"
    );
    assert_eq!(
        owned[guard - 1].op,
        CodeOp::Cmp,
        "the skip is decided by comparing the produced and copied pointers"
    );
    let label = s.label_at(&kept);
    assert!(
        !s.calls_between(guard, label).is_empty(),
        "between the guard and its label the producer's block is freed: {:?}",
        s.calls_between(guard, label)
    );

    let elsewhere = emitted(&ParameterType::String, HEAP);
    assert!(
        !has_label_starting(&elsewhere, "raw_helper_result_kept"),
        "a block another owner frees gets no second owner here"
    );
}

/// A graph payload (a recursive union) is dropped as a graph, not freed as one
/// flat block: the guarded region calls the graph drop.
#[test]
fn an_owned_graph_payload_is_dropped_as_a_graph() {
    let tree = ParameterType::declared("Tree");
    let owned = emitted(
        &tree,
        Probe {
            raw_success: RawSuccessBlock::OwnedByThisFrame,
            ..HEAP
        },
    );
    let s = stream(&owned);
    let kept = s.label_starting("raw_helper_result_kept");
    let guard = s.index_of("the branch to the kept label", |i| {
        i.get("target").as_deref() == Some(kept.as_str())
    });
    let calls = s.calls_between(guard, s.label_at(&kept));
    assert!(
        calls.iter().any(|c| c.contains("graph_drop")),
        "the producer's graph is dropped by the graph walker: {calls:?}"
    );
}

/// plan-64-I: with the error provably unused, the error branch wraps a bare tag:
/// no `ErrorLoc`/`Error` rebuild, no adoption. Without the flag the same call
/// rebuilds the error.
#[test]
fn a_discarded_error_builds_a_tag_only_result() {
    let discard = emitted(
        &ParameterType::Integer,
        Probe {
            discard_error: true,
            ..HEAP
        },
    );
    assert!(
        !has_label_starting(&discard, "raw_result_rebuild"),
        "a discarded error builds no Error block"
    );
    let kept = emitted(&ParameterType::Integer, HEAP);
    assert!(
        has_label_starting(&kept, "raw_result_rebuild"),
        "a read error is adopted or rebuilt"
    );
    // The tag-only branch still allocates its `Result`: one arena allocation per
    // wrap, the Ok wrap and the error wrap.
    let allocations = |instructions: &[CodeInstruction]| {
        stream(instructions)
            .labels()
            .iter()
            .filter(|(_, name)| name.starts_with("result_inline_alloc_ok"))
            .count()
    };
    assert_eq!(allocations(&discard), 2, "an Ok wrap and a bare error wrap");
}

/// The unboxed form's eligibility keeps a worker-arena error and a borrowed
/// resource out; reaching it with either is refused by name.
#[test]
fn the_unboxed_form_refuses_a_worker_error_and_a_borrowed_resource() {
    for probe in [
        Probe {
            worker_error_source: true,
            unboxed: true,
            ..HEAP
        },
        Probe {
            raw_success: RawSuccessBlock::BorrowedResource,
            unboxed: true,
            ..HEAP
        },
    ] {
        let Err(err) = materialize(&ParameterType::Integer, probe) else {
            panic!("the unboxed form must refuse this producer");
        };
        assert!(
            err.contains("callResult probe reached the unboxed inline-TRAP form")
                && err.contains("worker-arena error or a borrowed resource"),
            "the refusal names the producer and why: {err}"
        );
    }
}
