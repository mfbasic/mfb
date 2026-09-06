//! An `OUT` parameter's value is narrowed on the way back, by its C type.
//!
//! A `LINK` function may `RETURN` an `OUT` parameter rather than the call's
//! direct result. The callee wrote that slot with a C value of the declared
//! width; MFBASIC reads it as an 8-byte word, so the thunk has to convert —
//! and `link_thunk.rs`'s comment records what happens when it does not:
//!
//! > otherwise a `CInt32` OUT writing -1 surfaced as 4294967295
//! > (zero-extended) and a `CDouble` OUT bypassed the finiteness rejection an
//! > MFBASIC `Float` requires.
//!
//! Four arms, one per width class, and none of them had ever run. Every
//! committed `OUT` fixture declares `CPtr` — a pointer is already a word and
//! takes the untouched default — so the sign-extend, the boolean normalization,
//! the byte mask and the finiteness gate were all dead.
//!
//! Each is a WRONG VALUE rather than a failure to build, and each is wrong in a
//! way that reads as the C library's fault: a negative status that comes back as
//! four billion, a boolean that is `TRUE` for every non-zero byte pattern
//! including padding, a `Float` that is `Inf` where MFBASIC guarantees finite.

use crate::testutil::{try_code_for_linking_src, CodeTarget};

/// A one-function `LINK` program whose ABI has an `OUT` parameter of `ctype`,
/// returned as the function's result.
///
/// `getpid` is the symbol — it takes no arguments and exists everywhere, and
/// nothing is executed. What is under test is the thunk the compiler builds
/// around the call.
fn program(ctype: &str, mfb_type: &str) -> String {
    format!(
        "IMPORT io\n\
         \n\
         LINK \"c\" AS libc\n\
         \x20 FUNC fetch() AS {mfb_type}\n\
         \x20   SYMBOL \"getpid\"\n\
         \x20   ABI (slot OUT {ctype}) AS status CInt32\n\
         \x20   RETURN slot\n\
         \x20 END FUNC\n\
         END LINK\n\
         \n\
         FUNC main() AS Integer\n\
         \x20 io::print(toString(libc::fetch()))\n\
         \x20 RETURN 0\n\
         END FUNC\n"
    )
}

/// Every operand string of every instruction `link_thunk.rs` emitted.
///
/// Filtered by the emitting file, for C10's reason: these programs carry
/// hundreds of instructions from string handling and the entry stub, so "the
/// plan contains a compare" is true of every program ever compiled and says
/// nothing about the narrowing. `CodeInstruction::source` is the
/// `#[track_caller]` location of the builder call — audit-only metadata that
/// never reaches emitted bytes — so it attributes an instruction to whoever
/// asked for it without keying on a line number that drifts.
fn thunk_operands(plan: &crate::codegen::engine::types::NativeCodePlan) -> Vec<String> {
    plan.functions
        .iter()
        .flat_map(|function| function.instructions.iter())
        .filter(|instruction| {
            instruction
                .source
                .is_some_and(|location| location.file().ends_with("link_thunk.rs"))
        })
        .flat_map(|instruction| {
            // The mnemonic as well as the operands: the `CInt32` arm's marker is
            // the sign-extend INSTRUCTION, which carries no distinctive operand
            // of its own.
            std::iter::once(instruction.op.mnemonic().to_string()).chain(
                instruction
                    .fields
                    .iter()
                    .map(|(_, operand)| operand.render()),
            )
        })
        .collect()
}

/// `(ctype, MFBASIC type, a marker the narrowing emits and nothing else does)`.
///
/// The marker is a label name or an immediate the arm is the only source of, so
/// the assertion names the ARM rather than "some instruction appeared". The
/// table also asserts each marker is ABSENT from the other rows, which is what
/// turns four presence checks into a statement about which arm ran.
const OUT_TYPES: &[(&str, &str, &str)] = &[
    // The whole point: a C `int` of -1 is 0xFFFFFFFF in the slot, and reading it
    // as a word without sign-extending gives 4294967295.
    ("CInt32", "Integer", "sxtw"),
    // Normalized to 0 or 1 through a branch, so any non-zero byte pattern in the
    // slot -- including one the callee never wrote -- becomes exactly TRUE.
    ("CBool", "Boolean", "_out_bool_true"),
    // Masked to its low byte, because the other seven belong to whatever was
    // there before.
    ("CByte", "Integer", "255"),
    // An MFBASIC `Float` is finite by construction, so an Inf or NaN the callee
    // wrote has to be refused here rather than propagated.
    ("CDouble", "Float", "_out_float_finite"),
];

/// Each `OUT` width emits its own narrowing, and only its own.
#[test]
fn an_out_parameter_is_narrowed_by_its_c_type() {
    let mut emitted: Vec<(&str, Vec<String>)> = Vec::new();
    for (ctype, mfb_type, _) in OUT_TYPES {
        let plan =
            try_code_for_linking_src(&program(ctype, mfb_type), CodeTarget::MacosAarch64, &["c"])
                .unwrap_or_else(|err| panic!("an OUT {ctype} must lower: {err}"));
        emitted.push((ctype, thunk_operands(&plan)));
    }

    for (ctype, _, marker) in OUT_TYPES {
        for (other, operands) in &emitted {
            let present = operands.iter().any(|operand| operand.contains(marker));
            if other == ctype {
                assert!(
                    present,
                    "an OUT {ctype} must narrow through {marker:?}; without it the \
                     value MFBASIC reads is the raw 8 bytes of a slot the callee \
                     wrote {ctype} into"
                );
            } else {
                assert!(
                    !present,
                    "an OUT {other} emitted {marker:?}, which belongs to {ctype}. \
                     Two widths sharing a narrowing means one of them is being \
                     converted as the other."
                );
            }
        }
    }
}

/// Every `OUT` width lowers on every backend.
///
/// The narrowing is decided by shared codegen and emitted per-ISA: the
/// sign-extend, the byte mask and the finiteness compare are all instructions a
/// backend has to have, and one that did not would have to synthesise them.
#[test]
fn every_out_width_lowers_on_every_backend() {
    for (ctype, mfb_type, _) in OUT_TYPES {
        for target in CodeTarget::ALL {
            try_code_for_linking_src(&program(ctype, mfb_type), target, &["c"])
                .unwrap_or_else(|err| panic!("an OUT {ctype} on {}: {err}", target.name()));
        }
    }
}
