//! The inline-call lowerings (`engine/value/builder_values/inline_calls.rs`) — the
//! runtime-helper call, the inline-`TRAP` forms of the inline built-ins, and the
//! `abi_inline` dispatch.
//!
//! Two halves. The refusals: each names the call and the fact it is missing — a
//! queue call's handle, a write's payload type, a raw lowering for a call's shape, a
//! padded `Fill` default — instead of selecting a code form by guess or indexing
//! past an argument list. The front end keeps every such call out of a real
//! program, so they are driven directly on a builder. The accepting half: the
//! ownership answer an inline `TRAP` gives its success value, read off whole
//! programs.

use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::engine::tests::test_support::{BuilderHarness, TestPlatform};
use crate::target::shared::nir::{NirSourceLoc, NirValue};
use crate::target::shared::runtime;
use crate::testutil::{code_for_src_on, code_function, CodeTarget};
use crate::types::ParameterType;

fn constant(type_: ParameterType, value: &str) -> NirValue {
    NirValue::Const {
        type_,
        value: value.to_string(),
    }
}

fn integer() -> NirValue {
    constant(ParameterType::Integer, "3")
}

fn boolean() -> NirValue {
    constant(ParameterType::Boolean, "true")
}

/// A local nothing binds: no oracle can type it.
fn unbound() -> NirValue {
    NirValue::Local("unbound".to_string())
}

fn call_result(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::CallResult {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

fn call(target: &str, args: Vec<NirValue>) -> NirValue {
    NirValue::Call {
        target: target.to_string(),
        args,
        loc: NirSourceLoc::default(),
    }
}

/// What `lower` produces on a fresh builder: the lowered value's type, or the
/// refusal.
fn outcome(
    lower: impl FnOnce(&mut CodeBuilder<'_>) -> Result<ValueResult, String>,
) -> Result<ParameterType, String> {
    let harness = BuilderHarness::default();
    let mut builder = harness.builder("inline_call_refusals", &TestPlatform);
    lower(&mut builder).map(|value| value.type_)
}

/// The refusal a runtime-helper call of `target` over `args` produces.
fn helper_refusal(target: &str, args: Vec<NirValue>) -> String {
    let helper = runtime::helper_for_call(target)
        .unwrap_or_else(|| panic!("`{target}` is a runtime-helper call"));
    match outcome(|b| b.lower_runtime_helper_call(helper, target, &args, false)) {
        Ok(type_) => panic!("{target}: expected a refusal, lowered to a {type_}"),
        Err(error) => error,
    }
}

/// The refusal lowering `value` produces.
fn value_refusal(value: NirValue) -> String {
    match outcome(|b| b.lower_value(&value)) {
        Ok(type_) => panic!("expected a refusal, lowered to a {type_}"),
        Err(error) => error,
    }
}

/// A `thread::send`/`thread::transfer` picks its queue direction from its handle's
/// kind, and its `Nothing` return type is resolved without looking at the handle —
/// so this is where a missing or untyped handle is caught, by name, rather than
/// defaulting to one direction.
#[test]
fn a_queue_write_without_a_typed_handle_is_refused_by_name() {
    for (target, member) in [
        ("thread.send", "send"),
        ("thread.transferResource", "transfer"),
    ] {
        assert_eq!(
            helper_refusal(target, vec![]),
            format!("native runtime thread.{member} missing handle argument")
        );
        assert_eq!(
            helper_refusal(target, vec![unbound(), integer()]),
            format!("native runtime thread.{member} handle has unknown type")
        );
    }
}

/// A `thread::receive`/`thread::accept` types its RESULT from the handle (the
/// message or resource plane of the `ThreadHandle`), so an untyped handle is
/// refused there, before the direction is chosen — which is why the direction
/// step's own refusal is shared with the writes and cannot fire for these two.
#[test]
fn a_queue_read_without_a_typed_handle_has_no_result_type() {
    for target in ["thread.receive", "thread.acceptResource"] {
        for args in [vec![], vec![unbound()]] {
            assert_eq!(
                helper_refusal(target, args),
                format!("native runtime call '{target}' has no return type")
            );
        }
    }
}

/// bug-497: the text and byte forms of a write read the payload block with
/// different layouts, so a payload that is neither a `String` nor a `List` is a
/// build failure naming the code form — never the byte form by default, which is
/// what turned a socket echo into a memory-disclosure oracle.
#[test]
fn a_write_payload_that_is_neither_text_nor_bytes_is_refused() {
    assert_eq!(
        helper_refusal("tcp.write", vec![integer()]),
        "native runtime tcp.write is missing its payload argument"
    );
    assert_eq!(
        helper_refusal("tcp.write", vec![integer(), boolean()]),
        "native runtime tcp.write: payload static type Boolean is neither String nor \
         List OF Byte; refusing to select a lowering (bug-497)"
    );
    assert_eq!(
        helper_refusal("tcp.write", vec![integer(), unbound()]),
        "native runtime tcp.write: payload static type <unresolved> is neither String \
         nor List OF Byte; refusing to select a lowering (bug-497)"
    );
    // bug-809: `fs::openData` chooses its bytes form by the same rule, from its
    // FIRST argument.
    assert_eq!(
        helper_refusal("fs.openData", vec![]),
        "native runtime fs.openDataBytes is missing its payload argument"
    );
    assert_eq!(
        helper_refusal("fs.openData", vec![boolean(), integer()]),
        "native runtime fs.openDataBytes: payload static type Boolean is neither String \
         nor List OF Byte; refusing to select a lowering (bug-497)"
    );
}

/// An inline `TRAP` on an inline built-in lowers through the raw (fallible) or the
/// infallible wrapper, each of which knows only the shapes it was written for. A
/// call outside them — an arity the front end never lets through, an argument no
/// oracle types — is refused by name rather than indexed past.
#[test]
fn an_inline_trap_on_an_unlowerable_builtin_shape_is_refused() {
    assert_eq!(
        value_refusal(call_result("len", vec![integer(), integer()])),
        "native infallible inline builtin 'len' is not supported"
    );
    assert_eq!(
        value_refusal(call_result("typeName", vec![integer(), integer()])),
        "native infallible inline builtin 'typeName' is not supported"
    );
    assert_eq!(
        value_refusal(call_result("typeName", vec![unbound()])),
        "native code cannot determine typeName argument type"
    );
    assert_eq!(
        value_refusal(call_result(
            "toString",
            vec![integer(), integer(), integer()]
        )),
        "native raw inline builtin 'toString' (None) is not supported"
    );
}

/// bug-863: lowering pads every omitted `Fill` parameter with its default, so a
/// call that reaches an `abi_inline` body without one is a front-end defect. The
/// arity guard admits it (a `Fill` parameter is not required), and this is the
/// check that names it — on the plain call and the trapped one alike.
#[test]
fn an_inline_call_missing_a_fill_default_is_an_internal_error() {
    let expected = "internal error: call 'collections.repeat' reached code generation with 3 \
                    argument(s) and its default parameters unpadded";
    for value in [
        call("collections.repeat", vec![integer(), integer(), integer()]),
        call_result("collections.repeat", vec![integer(), integer(), integer()]),
    ] {
        assert_eq!(value_refusal(value), expected);
    }
}

/// The `_mfb_arena_alloc` calls a function makes.
fn arena_allocs(source: &str, function: &str) -> usize {
    let plan = code_for_src_on(source, CodeTarget::LinuxX86_64);
    code_function(&plan, function)
        .relocations
        .iter()
        .filter(|relocation| relocation.to == "_mfb_arena_alloc")
        .count()
}

/// plan-165-D / bug-778: a trapped `a & b` is a `Checked` whose result is a fresh
/// block of this frame, and the unboxed temporary TAKES IT OVER, as the untrapped
/// binding claims it. A trapped value the frame does not own — a parameter — has to
/// be copied into the temporary instead, so the fresh form makes exactly one
/// allocation fewer; copying the fresh block too would cost one per evaluation.
#[test]
fn a_trapped_concatenation_takes_over_its_fresh_block() {
    const FRESH: &str = "\
IMPORT io

FUNC cat(a AS String, b AS String) AS String
  LET s = a & b TRAP(e)
    RETURN \"trapped\"
  END TRAP
  RETURN s
END FUNC

FUNC main() AS Integer
  io::print(cat(\"x\", \"y\"))
  RETURN 0
END FUNC
";
    let borrowed = FRESH.replace("LET s = a & b TRAP(e)", "LET s = a TRAP(e)");
    assert_ne!(borrowed, FRESH, "the twin traps the parameter");
    assert_eq!(
        arena_allocs(FRESH, "cat") + 1,
        arena_allocs(&borrowed, "cat"),
        "the trapped concatenation copied its own fresh block"
    );
}

/// The instructions `main` of `source` takes from the hand-over copy
/// (`builder_arena_transfer/transfer.rs`).
fn transfer_instructions(source: &str) -> usize {
    let plan = code_for_src_on(source, CodeTarget::LinuxX86_64);
    code_function(&plan, "main")
        .instructions
        .iter()
        .filter(|instruction| {
            instruction.source.is_some_and(|at| {
                at.file()
                    .replace('\\', "/")
                    .ends_with("builder_arena_transfer/transfer.rs")
            })
        })
        .count()
}

/// bug-648: an element `collections::get` reads out of a `List OF RES` is still
/// its list's to close. A trapped read carries the pointer into the `Result`; it
/// must not run the hand-over copy, which would tombstone the element the list
/// still holds `moved|closed`.
#[test]
fn a_trapped_read_of_a_resource_element_borrows_it() {
    const SOURCE: &str = "\
IMPORT collections
IMPORT fs
IMPORT io

FUNC main() AS Integer
  RES a AS fs::File = fs::createTempFile()
  RES b AS fs::File = fs::createTempFile()
  LET hs AS List OF RES fs::File = [a, b]
  RES h = collections::get(hs, 1) TRAP(e)
    io::print(\"trapped\")
    RETURN 1
  END TRAP
  io::print(\"ok\")
  RETURN 0
END FUNC
";
    assert_eq!(transfer_instructions(SOURCE), 0, "the borrowed element was copied");
}
