use crate::bytecode::{
    self, NativeConst, NativeProgram, NativeType, NATIVE_OPCODE_ADD, NATIVE_OPCODE_BRANCH,
    NATIVE_OPCODE_BRANCH_IF_FALSE, NATIVE_OPCODE_BRANCH_IF_TRUE, NATIVE_OPCODE_CALL_RESULT,
    NATIVE_OPCODE_CONCAT, NATIVE_OPCODE_CONSTRUCT_LIST, NATIVE_OPCODE_CONSTRUCT_MAP,
    NATIVE_OPCODE_CONSTRUCT_RECORD, NATIVE_OPCODE_CONSTRUCT_VARIANT, NATIVE_OPCODE_COPY,
    NATIVE_OPCODE_DIV, NATIVE_OPCODE_EQUAL, NATIVE_OPCODE_GREATER, NATIVE_OPCODE_GREATER_EQUAL,
    NATIVE_OPCODE_LESS, NATIVE_OPCODE_LESS_EQUAL, NATIVE_OPCODE_LOAD_CONST,
    NATIVE_OPCODE_LOAD_DEFAULT, NATIVE_OPCODE_LOAD_ENUM_MEMBER, NATIVE_OPCODE_LOAD_FIELD,
    NATIVE_OPCODE_MOD, NATIVE_OPCODE_MOVE, NATIVE_OPCODE_MUL, NATIVE_OPCODE_NEG, NATIVE_OPCODE_NOT,
    NATIVE_OPCODE_NOT_EQUAL, NATIVE_OPCODE_POW, NATIVE_OPCODE_RETURN_OK, NATIVE_OPCODE_SUB,
    NATIVE_OPCODE_UNWRAP_RESULT, NATIVE_OPCODE_VARIANT_MATCH, NATIVE_OPCODE_WRITE_STDOUT,
    NATIVE_OPCODE_XOR,
};
use crate::ir::IrProject;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const SLOT_SIZE: usize = 24;
const SCRATCH_SIZE: usize = 64;
const HEAP_HEADER_SIZE: usize = 32;
const ARENA_STATE_SIZE: usize = 64;
const ARENA_DEFAULT_BLOCK_SIZE: u64 = 4096;
const ARENA_BLOCK_HEADER_SIZE: u64 = 32;
const ERR_INVALID_ARGUMENT: u64 = 10002;
const ERR_OUT_OF_MEMORY: u64 = 10010;
const DARWIN_PROT_READ_WRITE: u64 = 0x3;
const DARWIN_MAP_PRIVATE_ANON: u64 = 0x1002;
const HEAP_KIND_STRING: u64 = 1;
const HEAP_KIND_LIST: u64 = 2;
const HEAP_KIND_RECORD: u64 = 3;
const HEAP_KIND_VARIANT: u64 = 4;
const HEAP_KIND_MAP: u64 = 5;

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
                bytes.resize(align(bytes.len(), 8), 0);
                constants.insert(index as u32, bytes.len());
                put_u64(&mut bytes, HEAP_KIND_STRING);
                put_u64(&mut bytes, value.len() as u64);
                put_u64(&mut bytes, value.len() as u64);
                put_u64(&mut bytes, 0);
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
    arena_alloc: Label,
    arena_destroy: Label,
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
        let arena_alloc = code.new_label();
        let arena_destroy = code.new_label();
        Self {
            program,
            data,
            code_vmaddr,
            data_base,
            code,
            functions,
            arena_alloc,
            arena_destroy,
            current_scratch: 0,
        }
    }

    fn emit(mut self) -> Result<Vec<u8>, String> {
        self.emit_entry()?;
        for index in 0..self.program.functions.len() {
            self.emit_function(index)?;
        }
        self.emit_arena_alloc_runtime();
        self.emit_arena_destroy_runtime();
        Ok(self.code.finish())
    }

    fn emit_entry(&mut self) -> Result<(), String> {
        let entry = self.function_label(self.program.entry_function)?;
        self.code.sub_sp(ARENA_STATE_SIZE)?;
        self.code.add_imm(19, 31, 0)?;
        self.code.str_imm(31, 19, 0)?;
        self.code.str_imm(31, 19, 8)?;
        self.code.str_imm(31, 19, 16)?;
        self.code.str_imm(31, 19, 24)?;
        self.code.bl(entry);
        self.code.str_imm(0, 19, 32)?;
        self.code.str_imm(1, 19, 40)?;
        self.code.str_imm(2, 19, 48)?;
        self.code.bl(self.arena_destroy);
        self.code.ldr_imm(0, 19, 32)?;
        self.code.ldr_imm(1, 19, 40)?;
        self.code.ldr_imm(2, 19, 48)?;
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

        let aggregate_offsets = self.aggregate_offsets(function);
        self.current_scratch = align(self.aggregate_storage_end(function, &aggregate_offsets), 16);
        let frame = align(self.current_scratch + SCRATCH_SIZE, 16);
        let epilogue = self.code.new_label();
        self.code.stp_fp_lr_pre();
        self.code.mov_fp_sp();
        self.code.sub_sp(frame)?;
        let instruction_labels = (0..=function.code.len())
            .map(|_| self.code.new_label())
            .collect::<Vec<_>>();

        for index in 0..function.param_count {
            let slot = slot_offset(index as u32);
            self.code.str_imm((index * 2) as u8, 31, slot)?;
            self.code.str_imm((index * 2 + 1) as u8, 31, slot + 8)?;
        }

        for (instruction_index, instruction) in function.code.iter().enumerate() {
            self.code.bind(instruction_labels[instruction_index]);
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
                NATIVE_OPCODE_ADD | NATIVE_OPCODE_SUB | NATIVE_OPCODE_MUL | NATIVE_OPCODE_DIV
                | NATIVE_OPCODE_MOD | NATIVE_OPCODE_POW => {
                    self.emit_integer_arithmetic(instruction.opcode, instruction, epilogue)?;
                }
                NATIVE_OPCODE_EQUAL
                | NATIVE_OPCODE_NOT_EQUAL
                | NATIVE_OPCODE_LESS
                | NATIVE_OPCODE_LESS_EQUAL
                | NATIVE_OPCODE_GREATER
                | NATIVE_OPCODE_GREATER_EQUAL => {
                    self.emit_comparison(instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_NOT | NATIVE_OPCODE_NEG => {
                    self.emit_unary(instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_XOR => {
                    self.emit_xor(instruction)?;
                }
                NATIVE_OPCODE_CONCAT => {
                    self.emit_concat(instruction, epilogue)?;
                }
                NATIVE_OPCODE_WRITE_STDOUT => {
                    self.emit_write_stdout(function, instruction)?;
                }
                NATIVE_OPCODE_CALL_RESULT => {
                    self.emit_call_result(instruction)?;
                }
                NATIVE_OPCODE_UNWRAP_RESULT => {
                    let dst = operand(instruction, 0)?;
                    let result = operand(instruction, 1)?;
                    self.emit_unwrap_result(function, dst, result, epilogue)?;
                }
                NATIVE_OPCODE_CONSTRUCT_RECORD => {
                    self.emit_construct_record(instruction, epilogue)?;
                }
                NATIVE_OPCODE_CONSTRUCT_VARIANT => {
                    self.emit_construct_variant(instruction, epilogue)?;
                }
                NATIVE_OPCODE_CONSTRUCT_LIST | NATIVE_OPCODE_CONSTRUCT_MAP => {
                    self.emit_construct_sequence(instruction.opcode, instruction, epilogue)?;
                }
                NATIVE_OPCODE_LOAD_FIELD => {
                    let target = operand(instruction, 1)?;
                    let target_type = function.registers[target as usize].type_id;
                    self.emit_load_field(instruction, target_type)?;
                }
                NATIVE_OPCODE_LOAD_ENUM_MEMBER => {
                    let dst = operand(instruction, 0)?;
                    let ordinal = operand(instruction, 3)?;
                    self.code.mov_imm(9, ordinal as u64);
                    self.code.str_imm(9, 31, slot_offset(dst))?;
                }
                NATIVE_OPCODE_BRANCH => {
                    let target = operand(instruction, 0)?;
                    self.code.b(instruction_label(&instruction_labels, target)?);
                }
                NATIVE_OPCODE_BRANCH_IF_FALSE => {
                    let condition = operand(instruction, 0)?;
                    let target = operand(instruction, 1)?;
                    self.code.ldr_imm(9, 31, slot_offset(condition))?;
                    self.code
                        .cbz(9, instruction_label(&instruction_labels, target)?);
                }
                NATIVE_OPCODE_BRANCH_IF_TRUE => {
                    let condition = operand(instruction, 0)?;
                    let target = operand(instruction, 1)?;
                    self.code.ldr_imm(9, 31, slot_offset(condition))?;
                    self.code
                        .cbnz(9, instruction_label(&instruction_labels, target)?);
                }
                NATIVE_OPCODE_VARIANT_MATCH => {
                    self.emit_variant_match(instruction)?;
                }
                NATIVE_OPCODE_RETURN_OK => {
                    let src = operand(instruction, 0)?;
                    self.code.mov_imm(0, 0);
                    self.load_value_to_return(src)?;
                    self.code.b(epilogue);
                }
                opcode => {
                    return Err(format!(
                        "native bytecode execution does not support opcode {opcode}"
                    ));
                }
            }
        }

        self.code.bind(instruction_labels[function.code.len()]);
        self.code.bind(epilogue);
        self.code.add_sp(frame)?;
        self.code.ldp_fp_lr_post();
        self.code.ret();
        Ok(())
    }

    fn aggregate_offsets(&self, function: &bytecode::NativeFunction) -> Vec<(u32, usize)> {
        let _ = function;
        Vec::new()
    }

    fn aggregate_storage_end(
        &self,
        function: &bytecode::NativeFunction,
        offsets: &[(u32, usize)],
    ) -> usize {
        offsets
            .iter()
            .map(|(register, offset)| {
                *offset
                    + self
                        .aggregate_size_slots(function.registers[*register as usize].type_id)
                        .unwrap_or(0)
                        * SLOT_SIZE
            })
            .max()
            .unwrap_or(function.registers.len() * SLOT_SIZE)
    }

    fn aggregate_size_slots(&self, type_id: u32) -> Option<usize> {
        self.program
            .types
            .records
            .get(&type_id)
            .map(|record| record.size_slots)
            .or_else(|| {
                self.program
                    .types
                    .unions
                    .get(&type_id)
                    .map(|union| union.size_slots)
            })
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
            NativeConst::Float(value) => {
                self.code.mov_imm(9, value.to_bits());
                self.code.str_imm(9, 31, slot)
            }
            NativeConst::String(value) => {
                let offset = *self.data.constants.get(&constant_id).ok_or_else(|| {
                    format!("missing native data for string constant {constant_id}")
                })?;
                self.emit_data_addr(9, offset);
                self.code.mov_imm(10, value.len() as u64);
                self.code.str_imm(9, 31, slot)?;
                self.code.str_imm(10, 31, slot + 8)?;
                self.code.str_imm(31, 31, slot + 16)
            }
            NativeConst::Fixed => {
                Err("native bytecode execution does not support Fixed constants yet".to_string())
            }
        }
    }

    fn emit_construct_record(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let type_id = operand(instruction, 1)?;
        let fields = self
            .program
            .types
            .records
            .get(&type_id)
            .ok_or_else(|| format!("record constructor references unknown type {type_id}"))?
            .ordered_fields
            .clone();
        let payload_size = fields.len() * SLOT_SIZE;
        self.emit_allocate_heap_object(
            HEAP_KIND_RECORD,
            fields.len() as u64,
            0,
            payload_size,
            epilogue,
        )?;
        self.code.mov_reg(12, 1);
        self.code.str_imm(12, 31, slot_offset(dst))?;
        self.code.mov_imm(9, fields.len() as u64);
        self.code.str_imm(9, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)?;
        for (index, field) in fields.iter().enumerate() {
            let arg = operand(instruction, 2 + index)?;
            self.copy_slot_to_address(arg, 12, HEAP_HEADER_SIZE + field.offset_slots * SLOT_SIZE)?;
        }
        Ok(())
    }

    fn emit_construct_variant(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let union_type_id = operand(instruction, 1)?;
        let variant_name = operand(instruction, 2)?;
        let fields = self
            .program
            .types
            .unions
            .get(&union_type_id)
            .and_then(|union| union.variants.get(&variant_name))
            .ok_or_else(|| {
                format!("variant constructor references unknown variant {variant_name}")
            })?
            .fields
            .clone();
        let payload_size = fields.len() * SLOT_SIZE;
        self.emit_allocate_heap_object(
            HEAP_KIND_VARIANT,
            fields.len() as u64,
            variant_name as u64,
            payload_size,
            epilogue,
        )?;
        self.code.mov_reg(12, 1);
        self.code.str_imm(12, 31, slot_offset(dst))?;
        self.code.mov_imm(9, fields.len() as u64);
        self.code.str_imm(9, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)?;
        for (index, field) in fields.iter().enumerate() {
            let arg = operand(instruction, 3 + index)?;
            self.copy_slot_to_address(arg, 12, HEAP_HEADER_SIZE + field.offset_slots * SLOT_SIZE)?;
        }
        Ok(())
    }

    fn emit_load_field(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        target_type: u32,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let target = operand(instruction, 1)?;
        let field_name = operand(instruction, 2)?;
        let field = self
            .program
            .types
            .records
            .get(&target_type)
            .and_then(|record| record.fields.get(&field_name))
            .ok_or_else(|| {
                format!("field access references unknown field {field_name} on type {target_type}")
            })?
            .clone();
        self.code.ldr_imm(12, 31, slot_offset(target))?;
        self.copy_address_to_slot(12, HEAP_HEADER_SIZE + field.offset_slots * SLOT_SIZE, dst)
    }

    fn emit_construct_sequence(
        &mut self,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let count = operand(instruction, 2)? as usize;
        let payload_slots = if opcode == NATIVE_OPCODE_CONSTRUCT_MAP {
            count.checked_mul(2)
        } else {
            Some(count)
        }
        .ok_or_else(|| "container literal payload size overflowed".to_string())?;
        let payload_size = payload_slots
            .checked_mul(SLOT_SIZE)
            .ok_or_else(|| "container literal payload size overflowed".to_string())?;
        let kind = if opcode == NATIVE_OPCODE_CONSTRUCT_MAP {
            HEAP_KIND_MAP
        } else {
            HEAP_KIND_LIST
        };
        self.emit_allocate_heap_object(kind, count as u64, count as u64, payload_size, epilogue)?;
        self.code.mov_reg(12, 1);
        self.code.str_imm(12, 31, slot_offset(dst))?;
        self.code.mov_imm(9, count as u64);
        self.code.str_imm(9, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)?;
        for index in 0..payload_slots {
            let value = operand(instruction, 3 + index)?;
            self.copy_slot_to_address(value, 12, HEAP_HEADER_SIZE + index * SLOT_SIZE)?;
        }
        Ok(())
    }

    fn emit_allocate_heap_object(
        &mut self,
        kind: u64,
        length: u64,
        aux: u64,
        payload_size: usize,
        epilogue: Label,
    ) -> Result<(), String> {
        let total_size = HEAP_HEADER_SIZE
            .checked_add(payload_size)
            .ok_or_else(|| "heap object size overflowed".to_string())?;
        self.code.mov_imm(0, total_size as u64);
        self.code.mov_imm(1, 8);
        self.code.bl(self.arena_alloc);
        let ok = self.code.new_label();
        self.code.cbz(0, ok);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);
        self.code.bind(ok);
        self.write_heap_header(1, kind, length, payload_size as u64, aux)
    }

    fn write_heap_header(
        &mut self,
        object_reg: u8,
        kind: u64,
        length: u64,
        capacity: u64,
        aux: u64,
    ) -> Result<(), String> {
        self.code.mov_imm(16, kind);
        self.code.str_imm(16, object_reg, 0)?;
        self.code.mov_imm(16, length);
        self.code.str_imm(16, object_reg, 8)?;
        self.code.mov_imm(16, capacity);
        self.code.str_imm(16, object_reg, 16)?;
        self.code.mov_imm(16, aux);
        self.code.str_imm(16, object_reg, 24)
    }

    fn emit_error(&mut self, code: u64, epilogue: Label) {
        self.code.mov_imm(0, code);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);
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
                self.emit_nonzero_or_error(10, epilogue);
                self.code.sdiv(11, 9, 10);
            }
            NATIVE_OPCODE_MOD => {
                self.emit_nonzero_or_error(10, epilogue);
                self.code.sdiv(12, 9, 10);
                self.code.msub(11, 12, 10, 9);
            }
            NATIVE_OPCODE_POW => {
                self.emit_pow(epilogue);
            }
            _ => unreachable!(),
        }
        self.code.str_imm(11, 31, slot_offset(dst))?;
        Ok(())
    }

    fn emit_nonzero_or_error(&mut self, reg: u8, epilogue: Label) {
        let nonzero = self.code.new_label();
        self.code.cbnz(reg, nonzero);
        self.code.mov_imm(0, ERR_INVALID_ARGUMENT);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);
        self.code.bind(nonzero);
    }

    fn emit_pow(&mut self, epilogue: Label) {
        let nonnegative = self.code.new_label();
        let done = self.code.new_label();
        let loop_start = self.code.new_label();
        self.code.cmp_zero(10);
        self.code.b_ge(nonnegative);
        self.code.mov_imm(0, ERR_INVALID_ARGUMENT);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);
        self.code.bind(nonnegative);
        self.code.mov_imm(11, 1);
        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.code.mul(11, 11, 9);
        self.code.sub_imm(10, 10, 1).expect("literal 1 fits imm12");
        self.code.b(loop_start);
        self.code.bind(done);
    }

    fn emit_comparison(
        &mut self,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let left = operand(instruction, 1)?;
        let right = operand(instruction, 2)?;
        let false_label = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(left))?;
        self.code.ldr_imm(10, 31, slot_offset(right))?;
        self.code.cmp_reg(9, 10);
        match opcode {
            NATIVE_OPCODE_EQUAL => self.code.b_ne(false_label),
            NATIVE_OPCODE_NOT_EQUAL => self.code.b_eq(false_label),
            NATIVE_OPCODE_LESS => self.code.b_ge(false_label),
            NATIVE_OPCODE_LESS_EQUAL => self.code.b_gt(false_label),
            NATIVE_OPCODE_GREATER => self.code.b_le(false_label),
            NATIVE_OPCODE_GREATER_EQUAL => self.code.b_lt(false_label),
            _ => unreachable!(),
        }
        self.code.mov_imm(11, 1);
        self.code.b(done);
        self.code.bind(false_label);
        self.code.mov_imm(11, 0);
        self.code.bind(done);
        self.code.str_imm(11, 31, slot_offset(dst))
    }

    fn emit_unary(
        &mut self,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let operand = operand(instruction, 1)?;
        self.code.ldr_imm(9, 31, slot_offset(operand))?;
        match opcode {
            NATIVE_OPCODE_NOT => {
                let true_label = self.code.new_label();
                let done = self.code.new_label();
                self.code.cbz(9, true_label);
                self.code.mov_imm(10, 0);
                self.code.b(done);
                self.code.bind(true_label);
                self.code.mov_imm(10, 1);
                self.code.bind(done);
            }
            NATIVE_OPCODE_NEG => self.code.neg(10, 9),
            _ => unreachable!(),
        }
        self.code.str_imm(10, 31, slot_offset(dst))
    }

    fn emit_xor(&mut self, instruction: &bytecode::NativeInstruction) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let left = operand(instruction, 1)?;
        let right = operand(instruction, 2)?;
        self.code.ldr_imm(9, 31, slot_offset(left))?;
        self.code.ldr_imm(10, 31, slot_offset(right))?;
        self.code.eor(11, 9, 10);
        self.code.str_imm(11, 31, slot_offset(dst))
    }

    fn emit_concat(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let left = operand(instruction, 1)?;
        let right = operand(instruction, 2)?;
        let scratch = self.current_scratch;
        let size_ok = self.code.new_label();
        let alloc_size_ok = self.code.new_label();
        let alloc_ok = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(left))?;
        self.code.ldr_imm(10, 31, slot_offset(left) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(right))?;
        self.code.ldr_imm(12, 31, slot_offset(right) + 8)?;
        self.code.add_reg(13, 10, 12);
        self.code.cmp_reg(13, 10);
        self.code.b_hs(size_ok);
        self.emit_error(ERR_OUT_OF_MEMORY, epilogue);
        self.code.bind(size_ok);

        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.str_imm(12, 31, scratch + 24)?;
        self.code.str_imm(13, 31, scratch + 32)?;

        self.code.add_imm(0, 13, HEAP_HEADER_SIZE)?;
        self.code.cmp_reg(0, 13);
        self.code.b_hs(alloc_size_ok);
        self.emit_error(ERR_OUT_OF_MEMORY, epilogue);
        self.code.bind(alloc_size_ok);
        self.code.mov_imm(1, 8);
        self.code.bl(self.arena_alloc);
        self.code.cbz(0, alloc_ok);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);

        self.code.bind(alloc_ok);
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.ldr_imm(12, 31, scratch + 24)?;
        self.code.ldr_imm(13, 31, scratch + 32)?;
        self.write_heap_header(1, HEAP_KIND_STRING, 13, 13, 0)?;
        self.code.add_imm(14, 1, HEAP_HEADER_SIZE)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.emit_copy_bytes(9, 14, 10, 15)?;
        self.code.add_imm(11, 11, HEAP_HEADER_SIZE)?;
        self.emit_copy_bytes(11, 14, 12, 15)?;
        self.code.str_imm(1, 31, slot_offset(dst))?;
        self.code.str_imm(13, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn emit_copy_bytes(&mut self, src: u8, dst: u8, count: u8, byte: u8) -> Result<(), String> {
        let done = self.code.new_label();
        let loop_start = self.code.new_label();
        self.code.bind(loop_start);
        self.code.cbz(count, done);
        self.code.ldrb_imm(byte, src, 0);
        self.code.strb_imm(byte, dst, 0);
        self.code.add_imm(src, src, 1)?;
        self.code.add_imm(dst, dst, 1)?;
        self.code.sub_imm(count, count, 1)?;
        self.code.b(loop_start);
        self.code.bind(done);
        Ok(())
    }

    fn emit_variant_match(
        &mut self,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let variant_name = operand(instruction, 2)?;
        let unequal = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 9, 24)?;
        self.code.mov_imm(11, variant_name as u64);
        self.code.cmp_reg(10, 11);
        self.code.b_ne(unequal);
        self.code.mov_imm(12, 1);
        self.code.b(done);
        self.code.bind(unequal);
        self.code.mov_imm(12, 0);
        self.code.bind(done);
        self.code.str_imm(12, 31, slot_offset(dst))
    }

    fn emit_call_result(
        &mut self,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let function_id = operand(instruction, 1)?;
        self.program
            .functions
            .get(function_id as usize)
            .ok_or_else(|| format!("bytecode calls missing function {function_id}"))?;

        for (arg_index, arg) in instruction.operands.iter().skip(2).enumerate() {
            if arg_index >= 4 {
                return Err(
                    "native bytecode execution supports at most four call arguments".to_string(),
                );
            }
            let slot = slot_offset(*arg);
            self.code.ldr_imm((arg_index * 2) as u8, 31, slot)?;
            self.code.ldr_imm((arg_index * 2 + 1) as u8, 31, slot + 8)?;
        }

        self.code.bl(self.function_label(function_id)?);
        self.store_result(dst)
    }

    fn emit_write_stdout(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let src = operand(instruction, 0)?;
        let append_newline = operand(instruction, 1)? != 0;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("WRITE_STDOUT references missing register {src}"))?;
        if src_type != NativeType::String {
            return Err(format!(
                "WRITE_STDOUT requires a String register, got {}",
                native_type_name(src_type)
            ));
        }

        self.code.ldr_imm(1, 31, slot_offset(src))?;
        self.code.ldr_imm(2, 31, slot_offset(src) + 8)?;
        self.code.add_imm(1, 1, HEAP_HEADER_SIZE)?;
        self.emit_write_buffer()?;
        if append_newline {
            self.emit_newline_write()?;
        }
        Ok(())
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
        function: &bytecode::NativeFunction,
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
        let _ = function;
        self.code.ldr_imm(9, 31, slot + 8)?;
        self.code.ldr_imm(10, 31, slot + 16)?;
        self.code.str_imm(9, 31, slot_offset(dst))?;
        self.code.str_imm(10, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
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

    fn copy_slot_to_address(
        &mut self,
        src: u32,
        dst_ptr_reg: u8,
        dst_offset: usize,
    ) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.ldr_imm(10, 31, slot_offset(src) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(src) + 16)?;
        self.code.str_imm(9, dst_ptr_reg, dst_offset)?;
        self.code.str_imm(10, dst_ptr_reg, dst_offset + 8)?;
        self.code.str_imm(11, dst_ptr_reg, dst_offset + 16)
    }

    fn copy_address_to_slot(
        &mut self,
        src_ptr_reg: u8,
        src_offset: usize,
        dst: u32,
    ) -> Result<(), String> {
        self.code.ldr_imm(9, src_ptr_reg, src_offset)?;
        self.code.ldr_imm(10, src_ptr_reg, src_offset + 8)?;
        self.code.ldr_imm(11, src_ptr_reg, src_offset + 16)?;
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

    fn emit_arena_alloc_runtime(&mut self) {
        let size_nonzero = self.code.new_label();
        let try_current = self.code.new_label();
        let grow = self.code.new_label();
        let success = self.code.new_label();
        let normal_block = self.code.new_label();
        let mapped = self.code.new_label();
        let invalid = self.code.new_label();
        let oom = self.code.new_label();

        self.code.bind(self.arena_alloc);
        self.code.cbz(1, invalid);
        self.code.sub_imm(9, 1, 1).expect("literal 1 fits imm12");
        self.code.and_reg(10, 1, 9);
        self.code.cbnz(10, invalid);
        self.code.cbnz(0, size_nonzero);
        self.code.mov_imm(0, 1);
        self.code.bind(size_nonzero);
        self.code.mov_reg(20, 0);
        self.code.mov_reg(21, 1);

        self.code.bind(try_current);
        self.code.ldr_imm(22, 19, 0).expect("arena state load");
        self.code.cbz(22, grow);
        self.code.ldr_imm(23, 22, 16).expect("block capacity load");
        self.code.ldr_imm(24, 22, 24).expect("block offset load");
        self.code
            .add_imm(25, 22, ARENA_BLOCK_HEADER_SIZE as usize)
            .expect("block header size fits imm12");
        self.code.add_reg(26, 25, 24);
        self.code.cmp_reg(26, 25);
        self.code.b_lo(oom);
        self.code.sub_imm(27, 21, 1).expect("literal 1 fits imm12");
        self.code.mov_reg(15, 26);
        self.code.add_reg(26, 26, 27);
        self.code.cmp_reg(26, 15);
        self.code.b_lo(oom);
        self.code.mvn(27, 27);
        self.code.and_reg(26, 26, 27);
        self.code.add_reg(28, 26, 20);
        self.code.cmp_reg(28, 26);
        self.code.b_lo(oom);
        self.code.sub_reg(28, 28, 25);
        self.code.cmp_reg(28, 23);
        self.code.b_ls(success);

        self.code.bind(grow);
        self.code.add_reg(23, 20, 21);
        self.code.cmp_reg(23, 20);
        self.code.b_lo(oom);
        self.code
            .add_imm(23, 23, ARENA_BLOCK_HEADER_SIZE as usize)
            .expect("block header size fits imm12");
        self.code.cmp_reg_imm(23, ARENA_DEFAULT_BLOCK_SIZE);
        self.code.b_hi(normal_block);
        self.code.mov_imm(23, ARENA_DEFAULT_BLOCK_SIZE);
        let map_size_ready = self.code.new_label();
        self.code.b(map_size_ready);
        self.code.bind(normal_block);
        self.code.mov_reg(15, 23);
        self.code
            .add_imm(23, 23, 4095)
            .expect("page mask fits imm12");
        self.code.cmp_reg(23, 15);
        self.code.b_lo(oom);
        self.code.mov_imm(24, !4095u64);
        self.code.and_reg(23, 23, 24);
        self.code.bind(map_size_ready);

        self.code.mov_imm(0, 0);
        self.code.mov_reg(1, 23);
        self.code.mov_imm(2, DARWIN_PROT_READ_WRITE);
        self.code.mov_imm(3, DARWIN_MAP_PRIVATE_ANON);
        self.code.mov_imm(4, u64::MAX);
        self.code.mov_imm(5, 0);
        self.code.mov_imm(16, 0x0200_00c5);
        self.code.svc();
        self.code.cmp_zero(0);
        self.code.b_ge(mapped);
        self.code.b(oom);

        self.code.bind(mapped);
        self.code.ldr_imm(24, 19, 0).expect("arena current load");
        self.code.str_imm(24, 0, 0).expect("block next store");
        self.code.str_imm(23, 0, 8).expect("block map size store");
        self.code
            .sub_imm(24, 23, ARENA_BLOCK_HEADER_SIZE as usize)
            .expect("block header size fits imm12");
        self.code.str_imm(24, 0, 16).expect("block capacity store");
        self.code.str_imm(31, 0, 24).expect("block offset store");
        self.code.str_imm(0, 19, 0).expect("arena current store");
        self.code.b(try_current);

        self.code.bind(success);
        self.code.str_imm(28, 22, 24).expect("block offset update");
        self.code.mov_imm(0, 0);
        self.code.mov_reg(1, 26);
        self.code.ret();

        self.code.bind(invalid);
        self.code.mov_imm(0, ERR_INVALID_ARGUMENT);
        self.code.mov_imm(1, 0);
        self.code.ret();

        self.code.bind(oom);
        self.code.mov_imm(0, ERR_OUT_OF_MEMORY);
        self.code.mov_imm(1, 0);
        self.code.ret();
    }

    fn emit_arena_destroy_runtime(&mut self) {
        let loop_start = self.code.new_label();
        let done = self.code.new_label();
        self.code.bind(self.arena_destroy);
        self.code.ldr_imm(20, 19, 0).expect("arena current load");
        self.code.bind(loop_start);
        self.code.cbz(20, done);
        self.code.ldr_imm(21, 20, 0).expect("block next load");
        self.code.ldr_imm(1, 20, 8).expect("block map size load");
        self.code.mov_reg(0, 20);
        self.code.mov_imm(16, 0x0200_0049);
        self.code.svc();
        self.code.mov_reg(20, 21);
        self.code.b(loop_start);
        self.code.bind(done);
        self.code.str_imm(31, 19, 0).expect("arena current clear");
        self.code.ret();
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
        .ok_or_else(|| format!("opcode {} is missing operand {}", instruction.opcode, index))
}

fn instruction_label(labels: &[Label], target: u32) -> Result<Label, String> {
    labels
        .get(target as usize)
        .copied()
        .ok_or_else(|| format!("branch target {target} is outside the function"))
}

fn slot_offset(register: u32) -> usize {
    register as usize * SLOT_SIZE
}

fn native_type_name(type_: NativeType) -> &'static str {
    match type_ {
        NativeType::Nothing => "Nothing",
        NativeType::Boolean => "Boolean",
        NativeType::Integer => "Integer",
        NativeType::Float => "Float",
        NativeType::Fixed => "Fixed",
        NativeType::String => "String",
        NativeType::Result => "Result",
        NativeType::Other => "Other",
    }
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
    BCond { cond: u8 },
    Cbz { rt: u8, nonzero: bool },
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
                PatchKind::BCond { cond } => {
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
                        if nonzero { 0xb500_0000 } else { 0xb400_0000 } | (imm << 5) | rt as u32,
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

    fn b_ne(&mut self, label: Label) {
        self.b_cond(label, 1);
    }

    fn b_eq(&mut self, label: Label) {
        self.b_cond(label, 0);
    }

    fn b_ge(&mut self, label: Label) {
        self.b_cond(label, 10);
    }

    fn b_hs(&mut self, label: Label) {
        self.b_cond(label, 2);
    }

    fn b_lo(&mut self, label: Label) {
        self.b_cond(label, 3);
    }

    fn b_hi(&mut self, label: Label) {
        self.b_cond(label, 8);
    }

    fn b_ls(&mut self, label: Label) {
        self.b_cond(label, 9);
    }

    fn b_lt(&mut self, label: Label) {
        self.b_cond(label, 11);
    }

    fn b_gt(&mut self, label: Label) {
        self.b_cond(label, 12);
    }

    fn b_le(&mut self, label: Label) {
        self.b_cond(label, 13);
    }

    fn b_cond(&mut self, label: Label, cond: u8) {
        let at = self.bytes.len();
        self.emit(0);
        self.patches.push(Patch {
            at,
            label,
            kind: PatchKind::BCond { cond },
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

    fn cmp_reg(&mut self, rn: u8, rm: u8) {
        self.emit(0xeb00_001f | ((rm as u32) << 16) | ((rn as u32) << 5));
    }

    fn cmp_zero(&mut self, rn: u8) {
        self.emit(0xf100_001f | ((rn as u32) << 5));
    }

    fn cmp_reg_imm(&mut self, rn: u8, value: u64) {
        self.mov_imm(17, value);
        self.cmp_reg(rn, 17);
    }

    fn and_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x8a00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn mvn(&mut self, rd: u8, rm: u8) {
        self.emit(0xaa20_03e0 | ((rm as u32) << 16) | rd as u32);
    }

    fn mul(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x9b00_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
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

    fn sdiv(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x9ac0_0c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn neg(&mut self, rd: u8, rm: u8) {
        self.emit(0xcb00_03e0 | ((rm as u32) << 16) | rd as u32);
    }

    fn eor(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0xca00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
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

    fn ldrb_imm(&mut self, rt: u8, rn: u8, offset: usize) {
        debug_assert!(offset <= 4095);
        self.emit(0x3940_0000 | ((offset as u32) << 10) | ((rn as u32) << 5) | rt as u32);
    }

    fn strb_imm(&mut self, rt: u8, rn: u8, offset: usize) {
        debug_assert!(offset <= 4095);
        self.emit(0x3900_0000 | ((offset as u32) << 10) | ((rn as u32) << 5) | rt as u32);
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

fn put_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
fn checked_arena_used_after_alloc(
    block_base: u64,
    current_offset: u64,
    capacity: u64,
    size: u64,
    align: u64,
) -> Result<(u64, u64), u64> {
    if align == 0 || !align.is_power_of_two() {
        return Err(ERR_INVALID_ARGUMENT);
    }
    let size = size.max(1);
    let payload_base = block_base
        .checked_add(ARENA_BLOCK_HEADER_SIZE)
        .ok_or(ERR_OUT_OF_MEMORY)?;
    let raw = payload_base
        .checked_add(current_offset)
        .ok_or(ERR_OUT_OF_MEMORY)?;
    let mask = align - 1;
    let aligned = raw
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(ERR_OUT_OF_MEMORY)?;
    let end = aligned.checked_add(size).ok_or(ERR_OUT_OF_MEMORY)?;
    let used = end.checked_sub(payload_base).ok_or(ERR_OUT_OF_MEMORY)?;
    if used > capacity {
        return Err(ERR_OUT_OF_MEMORY);
    }
    Ok((aligned, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arena_rejects_invalid_alignment() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 8, 0),
            Err(ERR_INVALID_ARGUMENT)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 8, 3),
            Err(ERR_INVALID_ARGUMENT)
        );
    }

    #[test]
    fn arena_handles_zero_size_allocations() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 0, 8),
            Ok((0x1020, 1))
        );
    }

    #[test]
    fn arena_checks_alignment_rounding_and_capacity() {
        assert_eq!(
            checked_arena_used_after_alloc(0x1003, 5, 128, 8, 16),
            Ok((0x1030, 21))
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 120, 128, 16, 16),
            Err(ERR_OUT_OF_MEMORY)
        );
    }

    #[test]
    fn arena_checks_arithmetic_overflow() {
        assert_eq!(
            checked_arena_used_after_alloc(u64::MAX - 8, 0, 128, 8, 8),
            Err(ERR_OUT_OF_MEMORY)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, u64::MAX, u64::MAX, 8),
            Err(ERR_OUT_OF_MEMORY)
        );
    }
}

fn movz(rd: u8, value: u16, shift: u64) -> u32 {
    0xd280_0000 | (((shift / 16) as u32) << 21) | ((value as u32) << 5) | rd as u32
}

fn movk(rd: u8, value: u16, shift: u64) -> u32 {
    0xf280_0000 | (((shift / 16) as u32) << 21) | ((value as u32) << 5) | rd as u32
}
