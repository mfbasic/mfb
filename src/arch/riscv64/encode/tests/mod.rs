use super::*;
use crate::codegen::engine::types::CodeDataObject;
use crate::codegen::engine::types::CodeFrame;
use crate::codegen::engine::types::CodeFunction;
use crate::codegen::engine::types::CodeImport;
use crate::codegen::engine::types::NativeCodePlan;
use crate::target::NativeBuildMode;

mod emitter_and_rvv;
mod encoder;

/// Encode a single function's instructions and return its text bytes.
fn encode_text(instructions: Vec<CodeInstruction>) -> Vec<u8> {
    let plan = NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("f".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![CodeFunction {
            name: "f".to_string(),
            symbol: "f".to_string(),
            params: Vec::new(),
            returns: "Integer".to_string(),
            frame: CodeFrame {
                stack_size: 0,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations: Vec::new(),
            stack_slots: Vec::new(),
        }],
    };
    encode(&plan).expect("encode").text
}

fn words(bytes: &[u8]) -> Vec<u32> {
    bytes
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn ci(op: &str, fields: &[(&'static str, &str)]) -> CodeInstruction {
    let mut inst = CodeInstruction::new(op);
    for (k, v) in fields {
        inst = inst.physical_field(k, v);
    }
    inst
}

// --- Shared plan builders for the broader op/error coverage below --------------

/// A minimal one-function plan whose body is `instructions`, entry `f`.
fn plan_of(instructions: Vec<CodeInstruction>) -> NativeCodePlan {
    NativeCodePlan {
        target: "linux-riscv64".to_string(),
        build_mode: NativeBuildMode::Console,
        arch: "riscv64".to_string(),
        project: "t".to_string(),
        entry_symbol: Some("f".to_string()),
        imports: Vec::new(),
        data_objects: Vec::new(),
        functions: vec![func("f", instructions)],
    }
}

fn func(name: &str, instructions: Vec<CodeInstruction>) -> CodeFunction {
    CodeFunction {
        name: name.to_string(),
        symbol: name.to_string(),
        params: Vec::new(),
        returns: "Integer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        instructions,
        relocations: Vec::new(),
        stack_slots: Vec::new(),
    }
}

/// Encode a one-function body, returning the encoder's `Result` so error paths
/// can be asserted (mirrors `encode_text` but does not `.expect`).
fn try_encode(instructions: Vec<CodeInstruction>) -> Result<Vec<u8>, String> {
    encode(&plan_of(instructions)).map(|image| image.text)
}
