use crate::bytecode;
use crate::ir::IrProject;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Arm64Image {
    pub code: Vec<u8>,
    pub data: Vec<u8>,
}

pub fn write_arm64_dump(project_dir: &Path, ir: &IrProject) -> Result<PathBuf, String> {
    let plan = bytecode::native_plan(ir)?;
    let image = encode(&plan, 0);
    let path = project_dir.join(format!("{}.arm64.bin", ir.name));
    let mut bytes = image.code;
    bytes.extend_from_slice(&image.data);
    fs::write(&path, bytes)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    Ok(path)
}

pub fn encode(plan: &bytecode::NativePlan, code_vmaddr: u64) -> Arm64Image {
    let mut instruction_count = 0usize;
    for _ in &plan.prints {
        instruction_count += 6;
    }
    instruction_count += 5;

    let data_base = code_vmaddr + (instruction_count * 4) as u64;
    let mut code = Vec::new();
    let mut data = Vec::new();

    for bytes in &plan.prints {
        let data_addr = data_base + data.len() as u64;
        emit_mov_imm(&mut code, 0, 1);
        let instruction_addr = code_vmaddr + code.len() as u64;
        emit_adr(&mut code, 1, instruction_addr, data_addr);
        emit_mov_imm(&mut code, 2, bytes.len() as u64);
        emit_mov_imm(&mut code, 16, 0x0200_0004);
        emit_svc(&mut code);
        data.extend_from_slice(bytes);
    }

    emit_mov_imm(&mut code, 0, plan.exit_code as u64);
    emit_mov_imm(&mut code, 16, 0x0200_0001);
    emit_svc(&mut code);
    emit_branch_self(&mut code);

    Arm64Image { code, data }
}

fn emit_mov_imm(code: &mut Vec<u8>, rd: u8, value: u64) {
    let mut first = true;
    for shift in [0, 16, 32, 48] {
        let part = ((value >> shift) & 0xffff) as u16;
        if first {
            emit_u32(code, movz(rd, part, shift));
            first = false;
        } else if part != 0 {
            emit_u32(code, movk(rd, part, shift));
        }
    }
}

fn emit_adr(code: &mut Vec<u8>, rd: u8, instruction_addr: u64, target_addr: u64) {
    let offset = target_addr as i64 - instruction_addr as i64;
    assert!(
        (-(1 << 20)..(1 << 20)).contains(&offset),
        "ARM64 ADR target out of range"
    );
    let encoded = if offset < 0 {
        ((1 << 21) + offset) as u32
    } else {
        offset as u32
    };
    let immlo = encoded & 0b11;
    let immhi = (encoded >> 2) & 0x7ffff;
    emit_u32(code, 0x1000_0000 | (immlo << 29) | (immhi << 5) | rd as u32);
}

fn emit_svc(code: &mut Vec<u8>) {
    emit_u32(code, 0xd400_1001);
}

fn emit_branch_self(code: &mut Vec<u8>) {
    emit_u32(code, 0x1400_0000);
}

fn movz(rd: u8, value: u16, shift: u64) -> u32 {
    0xd280_0000 | (((shift / 16) as u32) << 21) | ((value as u32) << 5) | rd as u32
}

fn movk(rd: u8, value: u16, shift: u64) -> u32 {
    0xf280_0000 | (((shift / 16) as u32) << 21) | ((value as u32) << 5) | rd as u32
}

fn emit_u32(code: &mut Vec<u8>, value: u32) {
    code.extend_from_slice(&value.to_le_bytes());
}
