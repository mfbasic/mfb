//! Byte-exact encoding tests — the correctness gate for the x86-64 encoder.
//! Each expected sequence is hand-verified against the x86-64 instruction
//! reference. These also implicitly verify size==emit consistency, since
//! `encode_one` exercises the same `encode_instruction` path `instruction_size`
//! uses.

use super::emitter::{encode_instruction, Encoder};
use super::sizing::instruction_size;
use crate::codegen::engine::types::CodeDataObject;
use crate::codegen::engine::types::CodeFrame;
use crate::codegen::engine::types::CodeFunction;
use crate::codegen::engine::types::CodeImport;
use crate::codegen::engine::types::CodeInstruction;
use crate::codegen::engine::types::NativeCodePlan;
use std::collections::HashMap;
// From a part, `super` is `encode::tests`; part bodies spell `super::encode`,
// `super::operand` and `super::{EncodedSection, EncodedSymbol}`, which live in `encode`.
use super::{encode, operand, EncodedSection, EncodedSymbol};

mod float_v128_and_ranges;
mod image_and_coverage;
mod integer;
mod lanes_plans_and_aliasing;

fn fresh_encoder() -> Encoder {
    // Delegates to the one shared constructor (bug-341-C2); the struct literal
    // lives only in `InstructionEncoder::new` now (bug-341-B1).
    <Encoder as crate::arch::encode_plan::InstructionEncoder>::new(Vec::new(), HashMap::new())
}

/// Encode a single instruction and return its bytes, asserting the reported size
/// matches exactly.
fn bytes(op: &str, fields: &[(&'static str, &str)]) -> Vec<u8> {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.physical_field(k, v);
    }
    let encoded = encode_instruction(&ins).expect("encode");
    let size = instruction_size(&ins).expect("size");
    assert_eq!(
        encoded.bytes_len(),
        size,
        "size/emit mismatch for op '{op}'"
    );
    encoded.into_bytes()
}

/// Encode and return the error string (the `Encoded` Ok value has no `Debug`).
fn enc_err(ins: &CodeInstruction) -> String {
    match encode_instruction(ins) {
        Ok(_) => panic!("expected an encoding error"),
        Err(err) => err,
    }
}

/// Encode one instruction (no size assertion), returning the raw bytes. Used for
/// the arm-coverage sweeps where the exact byte sequence is verified only for a
/// representative subset and the rest assert successful, non-empty encoding.
fn enc(op: &str, fields: &[(&'static str, &str)]) -> Vec<u8> {
    let mut ins = CodeInstruction::new(op);
    for (k, v) in fields {
        ins = ins.physical_field(k, v);
    }
    encode_instruction(&ins).expect("encode").into_bytes()
}
