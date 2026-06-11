use crate::bytecode::{
    self, NativeConst, NativeFunctionKind, NativeProgram, NativeType, NATIVE_OPCODE_ADD,
    NATIVE_OPCODE_CALL_RESULT, NATIVE_OPCODE_CONCAT, NATIVE_OPCODE_COPY, NATIVE_OPCODE_DIV,
    NATIVE_OPCODE_LOAD_CONST, NATIVE_OPCODE_LOAD_DEFAULT, NATIVE_OPCODE_MOVE, NATIVE_OPCODE_MUL,
    NATIVE_OPCODE_RETURN_OK, NATIVE_OPCODE_SUB, NATIVE_OPCODE_UNWRAP_RESULT,
};
use crate::ir::IrProject;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const SLOT_SIZE: usize = 24;
const SCRATCH_SIZE: usize = 64;
const ERR_INVALID_ARGUMENT: u64 = 10002;

pub struct Arm64Image {
    pub code: Vec<u8>,
    pub data: Vec<u8>,
}

pub fn write_arm64_dump(project_dir: &Path, ir: &IrProject) -> Result<PathBuf, String> {
    let program = bytecode::native_program(ir)?;
    let image = encode(&program, 0)?;
    let path = project_dir.join(format!("{}.arm64.bin", ir.name));
    let mut bytes = image.code;
    bytes.extend_from_slice(&image.data);
    fs::write(&path, bytes)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    Ok(path)
}

pub fn encode(program: &NativeProgram, code_vmaddr: u64) -> Result<Arm64Image, String> {
    let data = NativeData::new(program);
    let code_len = NativeEmitter::new(program, &data, code_vmaddr, code_vmaddr)
        .emit()?
        .len();
    let data_base = code_vmaddr + code_len as u64;
    let code = NativeEmitter::new(program, &data, code_vmaddr, data_base).emit()?;
    Ok(Arm64Image {
        code,
        data: data.bytes,
    })
}

struct NativeData {
    bytes: Vec<u8>,
    constants: HashMap<u32, usize>,
    newline: usize,
}

impl NativeData {
    fn new(program: &NativeProgram) -> Self {
        let mut bytes = Vec::new();
        let mut constants = HashMap::new();
        for (index, constant) in program.constants.iter().enumerate() {
            if let NativeConst::String(value) = constant {
                constants.insert(index as u32, bytes.len());
                bytes.extend_from_slice(value.as_bytes());
            }
        }
        let newline = bytes.len();
        bytes.push(b'\n');
        Self {
            bytes,
            constants,
            newline,
        }
    }
}

struct NativeEmitter<'a> {
    program: &'a NativeProgram,
    data: &'a NativeData,
    code_vmaddr: u64,
    data_base: u64,
    code: Code,
    functions: Vec<Label>,
    current_scratch: usize,
}

impl<'a> NativeEmitter<'a> {
    fn new(
        program: &'a NativeProgram,
        data: &'a NativeData,
        code_vmaddr: u64,
        data_base: u64,
    ) -> Self {
        let mut code = Code::new();
        let functions = (0..program.functions.len())
            .map(|_| code.new_label())
            .collect();
        Self {
            program,
            data,
            code_vmaddr,
            data_base,
            code,
            functions,
            current_scratch: 0,
        }
    }

    fn emit(mut self) -> Result<Vec<u8>, String> {
        self.emit_entry()?;
        for (index, function) in self.program.functions.iter().enumerate() {
            if function.kind == NativeFunctionKind::Bytecode {
                self.emit_function(index)?;
            }
        }
        Ok(self.code.finish())
    }

    fn emit_entry(&mut self) -> Result<(), String> {
        let entry = self.function_label(self.program.entry_function)?;
        self.code.bl(entry);
        let exit = self.code.new_label();
        let ok = self.code.new_label();
        self.code.cbz(0, ok);
        self.code.b(exit);
        self.code.bind(ok);
        if self.program.entry_returns_integer {
            self.code.mov_reg(0, 1);
        } else {
            self.code.mov_imm(0, 0);
        }
        self.code.bind(exit);
        self.emit_exit_syscall();
        Ok(())
    }

    fn emit_function(&mut self, index: usize) -> Result<(), String> {
        let function = &self.program.functions[index];
        self.code.bind(self.functions[index]);

        self.current_scratch = align(function.registers.len() * SLOT_SIZE, 16);
        let frame = align(self.current_scratch + SCRATCH_SIZE, 16);
        let epilogue = self.code.new_label();
        self.code.stp_fp_lr_pre();
        self.code.mov_fp_sp();
        self.code.sub_sp(frame)?;

        for index in 0..function.param_count {
            let slot = slot_offset(index as u32);
            self.code.str_imm((index * 2) as u8, 31, slot)?;
            self.code.str_imm((index * 2 + 1) as u8, 31, slot + 8)?;
        }

        for instruction in &function.code {
            match instruction.opcode {
                NATIVE_OPCODE_LOAD_CONST => {
                    let dst = operand(instruction, 0)?;
                    let constant_id = operand(instruction, 1)?;
                    self.emit_load_const(dst, constant_id)?;
                }
                NATIVE_OPCODE_LOAD_DEFAULT => {
                    let dst = operand(instruction, 0)?;
                    self.store_zero_slot(dst)?;
                }
                NATIVE_OPCODE_MOVE | NATIVE_OPCODE_COPY => {
                    let dst = operand(instruction, 0)?;
                    let src = operand(instruction, 1)?;
                    self.copy_slot(dst, src)?;
                }
                NATIVE_OPCODE_ADD | NATIVE_OPCODE_SUB | NATIVE_OPCODE_MUL | NATIVE_OPCODE_DIV => {
                    self.emit_integer_arithmetic(instruction.opcode, instruction, epilogue)?;
                }
                NATIVE_OPCODE_CONCAT => {
                    return Err(
                        "native bytecode execution does not support string concatenation yet"
                            .to_string(),
                    );
                }
                NATIVE_OPCODE_CALL_RESULT => {
                    self.emit_call_result(index, instruction)?;
                }
                NATIVE_OPCODE_UNWRAP_RESULT => {
                    let dst = operand(instruction, 0)?;
                    let result = operand(instruction, 1)?;
                    self.emit_unwrap_result(dst, result, epilogue)?;
                }
                NATIVE_OPCODE_RETURN_OK => {
                    let src = operand(instruction, 0)?;
                    self.code.mov_imm(0, 0);
                    self.load_value_to_return(src)?;
                    self.code.b(epilogue);
                }
                opcode => {
                    return Err(format!("native bytecode execution does not support opcode {opcode}"));
                }
            }
        }

        self.code.bind(epilogue);
        self.code.add_sp(frame)?;
        self.code.ldp_fp_lr_post();
        self.code.ret();
        Ok(())
    }

    fn emit_load_const(&mut self, dst: u32, constant_id: u32) -> Result<(), String> {
        let constant = self
            .program
            .constants
            .get(constant_id as usize)
            .ok_or_else(|| format!("bytecode references missing constant {constant_id}"))?;
        let slot = slot_offset(dst);
        match constant {
            NativeConst::Nothing | NativeConst::Other => self.store_zero_slot(dst),
            NativeConst::Boolean(value) => {
                self.code.mov_imm(9, u64::from(*value));
                self.code.str_imm(9, 31, slot)
            }
            NativeConst::Integer(value) => {
                self.code.mov_imm(9, *value as u64);
                self.code.str_imm(9, 31, slot)
            }
            NativeConst::String(value) => {
                let offset = *self
                    .data
                    .constants
                    .get(&constant_id)
                    .ok_or_else(|| format!("missing native data for string constant {constant_id}"))?;
                self.emit_data_addr(9, offset);
                self.code.mov_imm(10, value.len() as u64);
                self.code.str_imm(9, 31, slot)?;
                self.code.str_imm(10, 31, slot + 8)
            }
            NativeConst::Float | NativeConst::Fixed => Err(
                "native bytecode execution supports Integer, String, Boolean, and Nothing constants"
                    .to_string(),
            ),
        }
    }

    fn emit_integer_arithmetic(
        &mut self,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let left = operand(instruction, 1)?;
        let right = operand(instruction, 2)?;
        self.code.ldr_imm(9, 31, slot_offset(left))?;
        self.code.ldr_imm(10, 31, slot_offset(right))?;
        match opcode {
            NATIVE_OPCODE_ADD => self.code.add_reg(11, 9, 10),
            NATIVE_OPCODE_SUB => self.code.sub_reg(11, 9, 10),
            NATIVE_OPCODE_MUL => self.code.mul(11, 9, 10),
            NATIVE_OPCODE_DIV => {
                let nonzero = self.code.new_label();
                self.code.cbnz(10, nonzero);
                self.code.mov_imm(0, ERR_INVALID_ARGUMENT);
                self.code.mov_imm(1, 0);
                self.code.mov_imm(2, 0);
                self.code.b(epilogue);
                self.code.bind(nonzero);
                self.code.sdiv(11, 9, 10);
            }
            _ => unreachable!(),
        }
        self.code.str_imm(11, 31, slot_offset(dst))?;
        Ok(())
    }

    fn emit_call_result(
        &mut self,
        caller_id: usize,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let function_id = operand(instruction, 1)?;
        let target = self
            .program
            .functions
            .get(function_id as usize)
            .ok_or_else(|| format!("bytecode calls missing function {function_id}"))?;
        if target.kind == NativeFunctionKind::Builtin {
            if target.name != "io.print" {
                return Err(format!("native bytecode execution does not support built-in `{}`", target.name));
            }
            let arg = operand(instruction, 2)?;
            let arg_type = self
                .program
                .functions
                .get(caller_id)
                .and_then(|caller| caller.registers.get(arg as usize))
                .map(|register| register.type_)
                .ok_or_else(|| format!("built-in argument references missing register {arg}"))?;
            self.emit_io_print(arg, arg_type, dst)?;
            return Ok(());
        }

        for (arg_index, arg) in instruction.operands.iter().skip(2).enumerate() {
            if arg_index >= 4 {
                return Err("native bytecode execution supports at most four call arguments".to_string());
            }
            let slot = slot_offset(*arg);
            self.code.ldr_imm((arg_index * 2) as u8, 31, slot)?;
            self.code.ldr_imm((arg_index * 2 + 1) as u8, 31, slot + 8)?;
        }

        self.code.bl(self.function_label(function_id)?);
        self.store_result(dst)
    }

    fn emit_io_print(
        &mut self,
        arg: u32,
        arg_type: NativeType,
        dst_result: u32,
    ) -> Result<(), String> {
        match arg_type {
            NativeType::String => {
                self.code.ldr_imm(1, 31, slot_offset(arg))?;
                self.code.ldr_imm(2, 31, slot_offset(arg) + 8)?;
                self.emit_write_buffer()?;
            }
            NativeType::Integer => {
                self.code.ldr_imm(9, 31, slot_offset(arg))?;
                self.emit_print_integer()?;
            }
            _ => {
                return Err("native io.print supports String and Integer values in this backend".to_string());
            }
        }
        self.emit_newline_write()?;
        self.code.mov_imm(0, 0);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.store_result(dst_result)?;
        Ok(())
    }

    fn emit_print_integer(&mut self) -> Result<(), String> {
        let scratch = self.current_scratch;
        self.code.add_imm(12, 31, scratch + 63)?;
        self.code.mov_imm(13, 10);
        let positive = self.code.new_label();
        let convert = self.code.new_label();
        self.code.cmp_imm(9, 0);
        self.code.b_cond(Cond::Ge, positive);
        self.code.neg(9, 9);
        self.code.mov_imm(14, 1);
        self.code.b(convert);
        self.code.bind(positive);
        self.code.mov_imm(14, 0);
        self.code.bind(convert);

        let nonzero = self.code.new_label();
        let digits_done = self.code.new_label();
        self.code.cbnz(9, nonzero);
        self.code.mov_imm(15, b'0' as u64);
        self.code.sub_imm(12, 12, 1)?;
        self.code.strb_imm(15, 12, 0)?;
        self.code.b(digits_done);

        self.code.bind(nonzero);
        let digit_loop = self.code.new_label();
        self.code.bind(digit_loop);
        self.code.sdiv(16, 9, 13);
        self.code.msub(17, 16, 13, 9);
        self.code.add_imm(17, 17, b'0' as usize)?;
        self.code.sub_imm(12, 12, 1)?;
        self.code.strb_imm(17, 12, 0)?;
        self.code.mov_reg(9, 16);
        self.code.cbnz(9, digit_loop);

        self.code.bind(digits_done);
        let no_sign = self.code.new_label();
        self.code.cbz(14, no_sign);
        self.code.mov_imm(15, b'-' as u64);
        self.code.sub_imm(12, 12, 1)?;
        self.code.strb_imm(15, 12, 0)?;
        self.code.bind(no_sign);

        self.code.add_imm(13, 31, scratch + 63)?;
        self.code.sub_reg(2, 13, 12);
        self.code.mov_reg(1, 12);
        self.emit_write_buffer()
    }

    fn emit_newline_write(&mut self) -> Result<(), String> {
        self.emit_data_addr(1, self.data.newline);
        self.code.mov_imm(2, 1);
        self.emit_write_buffer()
    }

    fn emit_write_buffer(&mut self) -> Result<(), String> {
        self.code.mov_imm(0, 1);
        self.code.mov_imm(16, 0x0200_0004);
        self.code.svc();
        Ok(())
    }

    fn emit_unwrap_result(
        &mut self,
        dst: u32,
        result: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let ok = self.code.new_label();
        let slot = slot_offset(result);
        self.code.ldr_imm(0, 31, slot)?;
        self.code.cbz(0, ok);
        self.code.ldr_imm(1, 31, slot + 8)?;
        self.code.ldr_imm(2, 31, slot + 16)?;
        self.code.b(epilogue);
        self.code.bind(ok);
        self.code.ldr_imm(9, 31, slot + 8)?;
        self.code.ldr_imm(10, 31, slot + 16)?;
        self.code.str_imm(9, 31, slot_offset(dst))?;
        self.code.str_imm(10, 31, slot_offset(dst) + 8)?;
        Ok(())
    }

    fn store_result(&mut self, dst: u32) -> Result<(), String> {
        let slot = slot_offset(dst);
        self.code.str_imm(0, 31, slot)?;
        self.code.str_imm(1, 31, slot + 8)?;
        self.code.str_imm(2, 31, slot + 16)
    }

    fn load_value_to_return(&mut self, src: u32) -> Result<(), String> {
        let slot = slot_offset(src);
        self.code.ldr_imm(1, 31, slot)?;
        self.code.ldr_imm(2, 31, slot + 8)
    }

    fn copy_slot(&mut self, dst: u32, src: u32) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.ldr_imm(10, 31, slot_offset(src) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(src) + 16)?;
        self.code.str_imm(9, 31, slot_offset(dst))?;
        self.code.str_imm(10, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(11, 31, slot_offset(dst) + 16)
    }

    fn store_zero_slot(&mut self, dst: u32) -> Result<(), String> {
        self.code.str_imm(31, 31, slot_offset(dst))?;
        self.code.str_imm(31, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn function_label(&self, function_id: u32) -> Result<Label, String> {
        self.functions
            .get(function_id as usize)
            .copied()
            .ok_or_else(|| format!("bytecode references missing function {function_id}"))
    }

    fn emit_data_addr(&mut self, rd: u8, offset: usize) {
        let instruction_addr = self.code_vmaddr + self.code.position() as u64;
        self.code
            .adr(rd, instruction_addr, self.data_base + offset as u64);
    }

    fn emit_exit_syscall(&mut self) {
        self.code.mov_imm(16, 0x0200_0001);
        self.code.svc();
        self.code.branch_self();
    }
}

fn operand(instruction: &bytecode::NativeInstruction, index: usize) -> Result<u32, String> {
    instruction
        .operands
        .get(index)
        .copied()
        .ok_or_else(|| {
            format!(
                "opcode {} is missing operand {}",
                instruction.opcode, index
            )
        })
}

fn slot_offset(register: u32) -> usize {
    register as usize * SLOT_SIZE
}

fn align(value: usize, alignment: usize) -> usize {
    (value + alignment - 1) & !(alignment - 1)
}

#[derive(Clone, Copy)]
struct Label(usize);

struct Code {
    bytes: Vec<u8>,
    labels: Vec<Option<usize>>,
    patches: Vec<Patch>,
}

struct Patch {
    at: usize,
    label: Label,
    kind: PatchKind,
}

enum PatchKind {
    B,
    Bl,
    BCond(Cond),
    Cbz { rt: u8, nonzero: bool },
}

#[derive(Clone, Copy)]
enum Cond {
    Ge = 10,
}

impl Code {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            labels: Vec::new(),
            patches: Vec::new(),
        }
    }

    fn position(&self) -> usize {
        self.bytes.len()
    }

    fn new_label(&mut self) -> Label {
        let label = Label(self.labels.len());
        self.labels.push(None);
        label
    }

    fn bind(&mut self, label: Label) {
        self.labels[label.0] = Some(self.bytes.len());
    }

    fn finish(mut self) -> Vec<u8> {
        for patch in &self.patches {
            let target = self.labels[patch.label.0].expect("unbound ARM64 label");
            let source = patch.at;
            match patch.kind {
                PatchKind::B => {
                    let imm = branch_imm26(source, target);
                    write_u32(&mut self.bytes, source, 0x1400_0000 | imm);
                }
                PatchKind::Bl => {
                    let imm = branch_imm26(source, target);
                    write_u32(&mut self.bytes, source, 0x9400_0000 | imm);
                }
                PatchKind::BCond(cond) => {
                    let imm = branch_imm19(source, target);
                    write_u32(
                        &mut self.bytes,
                        source,
                        0x5400_0000 | (imm << 5) | cond as u32,
                    );
                }
                PatchKind::Cbz { rt, nonzero } => {
                    let imm = branch_imm19(source, target);
                    write_u32(
                        &mut self.bytes,
                        source,
                        if nonzero { 0xb500_0000 } else { 0xb400_0000 }
                            | (imm << 5)
                            | rt as u32,
                    );
                }
            }
        }
        self.bytes
    }

    fn emit(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_le_bytes());
    }

    fn b(&mut self, label: Label) {
        let at = self.bytes.len();
        self.emit(0);
        self.patches.push(Patch {
            at,
            label,
            kind: PatchKind::B,
        });
    }

    fn bl(&mut self, label: Label) {
        let at = self.bytes.len();
        self.emit(0);
        self.patches.push(Patch {
            at,
            label,
            kind: PatchKind::Bl,
        });
    }

    fn b_cond(&mut self, cond: Cond, label: Label) {
        let at = self.bytes.len();
        self.emit(0);
        self.patches.push(Patch {
            at,
            label,
            kind: PatchKind::BCond(cond),
        });
    }

    fn cbz(&mut self, rt: u8, label: Label) {
        let at = self.bytes.len();
        self.emit(0);
        self.patches.push(Patch {
            at,
            label,
            kind: PatchKind::Cbz { rt, nonzero: false },
        });
    }

    fn cbnz(&mut self, rt: u8, label: Label) {
        let at = self.bytes.len();
        self.emit(0);
        self.patches.push(Patch {
            at,
            label,
            kind: PatchKind::Cbz { rt, nonzero: true },
        });
    }

    fn stp_fp_lr_pre(&mut self) {
        self.emit(0xa9bf_7bfd);
    }

    fn ldp_fp_lr_post(&mut self) {
        self.emit(0xa8c1_7bfd);
    }

    fn mov_fp_sp(&mut self) {
        self.emit(0x9100_03fd);
    }

    fn ret(&mut self) {
        self.emit(0xd65f_03c0);
    }

    fn branch_self(&mut self) {
        self.emit(0x1400_0000);
    }

    fn svc(&mut self) {
        self.emit(0xd400_1001);
    }

    fn mov_imm(&mut self, rd: u8, value: u64) {
        let mut first = true;
        for shift in [0, 16, 32, 48] {
            let part = ((value >> shift) & 0xffff) as u16;
            if first {
                self.emit(movz(rd, part, shift));
                first = false;
            } else if part != 0 {
                self.emit(movk(rd, part, shift));
            }
        }
    }

    fn mov_reg(&mut self, rd: u8, rn: u8) {
        let rm = if rn == 31 { 31 } else { rn };
        self.emit(0xaa00_03e0 | ((rm as u32) << 16) | rd as u32);
    }

    fn add_sp(&mut self, value: usize) -> Result<(), String> {
        self.add_imm(31, 31, value)
    }

    fn sub_sp(&mut self, value: usize) -> Result<(), String> {
        self.sub_imm(31, 31, value)
    }

    fn add_imm(&mut self, rd: u8, rn: u8, value: usize) -> Result<(), String> {
        let imm = checked_imm12(value)?;
        self.emit(0x9100_0000 | (imm << 10) | ((rn as u32) << 5) | rd as u32);
        Ok(())
    }

    fn sub_imm(&mut self, rd: u8, rn: u8, value: usize) -> Result<(), String> {
        let imm = checked_imm12(value)?;
        self.emit(0xd100_0000 | (imm << 10) | ((rn as u32) << 5) | rd as u32);
        Ok(())
    }

    fn add_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x8b00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn sub_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0xcb00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn neg(&mut self, rd: u8, rm: u8) {
        self.emit(0xcb00_03e0 | ((rm as u32) << 16) | rd as u32);
    }

    fn mul(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x9b00_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn sdiv(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x9ac0_0c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn msub(&mut self, rd: u8, rn: u8, rm: u8, ra: u8) {
        self.emit(
            0x9b00_8000
                | ((rm as u32) << 16)
                | ((ra as u32) << 10)
                | ((rn as u32) << 5)
                | rd as u32,
        );
    }

    fn cmp_imm(&mut self, rn: u8, value: usize) {
        self.emit(0xf100_001f | ((value as u32) << 10) | ((rn as u32) << 5));
    }

    fn ldr_imm(&mut self, rt: u8, rn: u8, offset: usize) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned ARM64 load offset {offset}"));
        }
        let imm = checked_imm12(offset / 8)?;
        self.emit(0xf940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        Ok(())
    }

    fn str_imm(&mut self, rt: u8, rn: u8, offset: usize) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned ARM64 store offset {offset}"));
        }
        let imm = checked_imm12(offset / 8)?;
        self.emit(0xf900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        Ok(())
    }

    fn strb_imm(&mut self, rt: u8, rn: u8, offset: usize) -> Result<(), String> {
        let imm = checked_imm12(offset)?;
        self.emit(0x3900_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        Ok(())
    }

    fn adr(&mut self, rd: u8, instruction_addr: u64, target_addr: u64) {
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
        self.emit(0x1000_0000 | (immlo << 29) | (immhi << 5) | rd as u32);
    }
}

fn checked_imm12(value: usize) -> Result<u32, String> {
    if value > 4095 {
        return Err(format!("ARM64 immediate {value} exceeds 12-bit encoding"));
    }
    Ok(value as u32)
}

fn branch_imm26(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x03ff_ffff
}

fn branch_imm19(source: usize, target: usize) -> u32 {
    let delta = target as isize - source as isize;
    ((delta / 4) as i32 as u32) & 0x0007_ffff
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

fn movz(rd: u8, value: u16, shift: u64) -> u32 {
    0xd280_0000 | (((shift / 16) as u32) << 21) | ((value as u32) << 5) | rd as u32
}

fn movk(rd: u8, value: u16, shift: u64) -> u32 {
    0xf280_0000 | (((shift / 16) as u32) << 21) | ((value as u32) << 5) | rd as u32
}
