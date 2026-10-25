use super::*;
use crate::arch::ops::CodeOp;
use crate::codegen::engine::operand::Operand;
use crate::codegen::engine::types::CodeInstruction;

mod branches_image_operands;
mod forms;
mod pairs;
mod vector_and_relax_edges;

fn fresh_encoder() -> Encoder {
    // Delegates to the one shared constructor (bug-341-C2); the struct literal
    // lives only in `InstructionEncoder::new` now (bug-341-B1).
    <Encoder as crate::arch::encode_plan::InstructionEncoder>::new(Vec::new(), HashMap::new())
}

fn encode_one(instruction: &CodeInstruction) -> u32 {
    let mut encoder = fresh_encoder();
    encoder.emit_instruction(instruction).unwrap();
    assert_eq!(encoder.text.len(), 4, "expected a single 4-byte word");
    u32::from_le_bytes(encoder.text[..4].try_into().unwrap())
}

/// Emit one instruction and return its 4-byte words (asserting size==emit).
fn emit_words(op: &str, fields: &[(&'static str, &str)]) -> Vec<u32> {
    let mut inst = CodeInstruction::new(op);
    for (k, v) in fields {
        inst = inst.physical_field(k, v);
    }
    let mut enc = fresh_encoder();
    enc.emit_instruction(&inst).unwrap();
    assert_eq!(
        enc.text.len(),
        instruction_size(&inst).unwrap(),
        "size/emit mismatch for '{op}'"
    );
    enc.text
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

fn plan(
    functions: Vec<crate::codegen::engine::types::CodeFunction>,
    data_objects: Vec<crate::codegen::engine::types::CodeDataObject>,
    imports: Vec<crate::codegen::engine::types::CodeImport>,
    entry: Option<&str>,
) -> crate::codegen::engine::types::NativeCodePlan {
    crate::codegen::engine::types::NativeCodePlan {
        target: "linux-aarch64".to_string(),
        build_mode: crate::target::NativeBuildMode::Console,
        arch: "aarch64".to_string(),
        project: "t".to_string(),
        entry_symbol: entry.map(str::to_string),
        imports,
        data_objects,
        functions,
    }
}
