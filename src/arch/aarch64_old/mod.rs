use crate::builtins;
use crate::bytecode::{
    self, NativeConst, NativeProgram, NativeType, NATIVE_OPCODE_ADD, NATIVE_OPCODE_BRANCH,
    NATIVE_OPCODE_BRANCH_IF_FALSE, NATIVE_OPCODE_BRANCH_IF_TRUE, NATIVE_OPCODE_CALL_RESULT,
    NATIVE_OPCODE_CALL_VALUE_RESULT, NATIVE_OPCODE_CLOSE_RESOURCE, NATIVE_OPCODE_COLLECTION_APPEND,
    NATIVE_OPCODE_COLLECTION_CONTAINS, NATIVE_OPCODE_COLLECTION_FILTER,
    NATIVE_OPCODE_COLLECTION_FIND, NATIVE_OPCODE_COLLECTION_FOR_EACH, NATIVE_OPCODE_COLLECTION_GET,
    NATIVE_OPCODE_COLLECTION_GET_OR, NATIVE_OPCODE_COLLECTION_HAS_KEY,
    NATIVE_OPCODE_COLLECTION_INSERT, NATIVE_OPCODE_COLLECTION_KEYS, NATIVE_OPCODE_COLLECTION_MID,
    NATIVE_OPCODE_COLLECTION_PREPEND, NATIVE_OPCODE_COLLECTION_REDUCE,
    NATIVE_OPCODE_COLLECTION_REMOVE_AT, NATIVE_OPCODE_COLLECTION_REMOVE_KEY,
    NATIVE_OPCODE_COLLECTION_REPLACE, NATIVE_OPCODE_COLLECTION_SET, NATIVE_OPCODE_COLLECTION_SUM,
    NATIVE_OPCODE_COLLECTION_TRANSFORM, NATIVE_OPCODE_COLLECTION_VALUES, NATIVE_OPCODE_CONCAT,
    NATIVE_OPCODE_CONSTRUCT_LIST, NATIVE_OPCODE_CONSTRUCT_MAP, NATIVE_OPCODE_CONSTRUCT_RECORD,
    NATIVE_OPCODE_CONSTRUCT_VARIANT, NATIVE_OPCODE_COPY, NATIVE_OPCODE_DIV, NATIVE_OPCODE_EQUAL,
    NATIVE_OPCODE_FS_APPEND_TEXT, NATIVE_OPCODE_FS_CANONICAL_PATH, NATIVE_OPCODE_FS_CLOSE,
    NATIVE_OPCODE_FS_CREATE_DIRECTORIES, NATIVE_OPCODE_FS_CREATE_DIRECTORY,
    NATIVE_OPCODE_FS_CREATE_TEMP_FILE, NATIVE_OPCODE_FS_CURRENT_DIRECTORY,
    NATIVE_OPCODE_FS_DELETE_DIRECTORY, NATIVE_OPCODE_FS_DELETE_FILE,
    NATIVE_OPCODE_FS_DIRECTORY_EXISTS, NATIVE_OPCODE_FS_EOF, NATIVE_OPCODE_FS_EXISTS,
    NATIVE_OPCODE_FS_FILE_EXISTS, NATIVE_OPCODE_FS_IS_WITHIN, NATIVE_OPCODE_FS_LIST_DIRECTORY,
    NATIVE_OPCODE_FS_OPEN, NATIVE_OPCODE_FS_OPEN_NO_FOLLOW, NATIVE_OPCODE_FS_PATH_BASE_NAME,
    NATIVE_OPCODE_FS_PATH_DIR_NAME, NATIVE_OPCODE_FS_PATH_EXTENSION, NATIVE_OPCODE_FS_PATH_JOIN,
    NATIVE_OPCODE_FS_PATH_NORMALIZE, NATIVE_OPCODE_FS_READ_ALL, NATIVE_OPCODE_FS_READ_LINE,
    NATIVE_OPCODE_FS_READ_TEXT, NATIVE_OPCODE_FS_SET_CURRENT_DIRECTORY, NATIVE_OPCODE_FS_WRITE_ALL,
    NATIVE_OPCODE_FS_WRITE_TEXT, NATIVE_OPCODE_FS_WRITE_TEXT_ATOMIC, NATIVE_OPCODE_GENERAL_FIND,
    NATIVE_OPCODE_GENERAL_IS_EMPTY, NATIVE_OPCODE_GENERAL_IS_EVEN,
    NATIVE_OPCODE_GENERAL_IS_NEGATIVE, NATIVE_OPCODE_GENERAL_IS_NOT_EMPTY,
    NATIVE_OPCODE_GENERAL_IS_NUMERIC, NATIVE_OPCODE_GENERAL_IS_ODD,
    NATIVE_OPCODE_GENERAL_IS_POSITIVE, NATIVE_OPCODE_GENERAL_IS_ZERO, NATIVE_OPCODE_GENERAL_LEN,
    NATIVE_OPCODE_GENERAL_MID, NATIVE_OPCODE_GENERAL_REPLACE, NATIVE_OPCODE_GENERAL_TO_BYTE,
    NATIVE_OPCODE_GENERAL_TO_FIXED, NATIVE_OPCODE_GENERAL_TO_FLOAT, NATIVE_OPCODE_GENERAL_TO_INT,
    NATIVE_OPCODE_GENERAL_TO_STRING, NATIVE_OPCODE_GREATER, NATIVE_OPCODE_GREATER_EQUAL,
    NATIVE_OPCODE_IO_CLOSE, NATIVE_OPCODE_IO_FLUSH, NATIVE_OPCODE_IO_IS_TERMINAL,
    NATIVE_OPCODE_IO_OPEN, NATIVE_OPCODE_IO_READ_BYTE, NATIVE_OPCODE_IO_READ_CHAR,
    NATIVE_OPCODE_IO_READ_LINE, NATIVE_OPCODE_IO_TERMINAL_SIZE, NATIVE_OPCODE_IO_WRITE,
    NATIVE_OPCODE_LESS, NATIVE_OPCODE_LESS_EQUAL, NATIVE_OPCODE_LOAD_CONST,
    NATIVE_OPCODE_LOAD_DEFAULT, NATIVE_OPCODE_LOAD_ENUM_MEMBER, NATIVE_OPCODE_LOAD_FIELD,
    NATIVE_OPCODE_LOAD_FUNCTION, NATIVE_OPCODE_MATH_ABS, NATIVE_OPCODE_MATH_ACOS,
    NATIVE_OPCODE_MATH_ASIN, NATIVE_OPCODE_MATH_ATAN, NATIVE_OPCODE_MATH_ATAN2,
    NATIVE_OPCODE_MATH_CEIL, NATIVE_OPCODE_MATH_CLAMP, NATIVE_OPCODE_MATH_COS,
    NATIVE_OPCODE_MATH_DEGREES, NATIVE_OPCODE_MATH_E, NATIVE_OPCODE_MATH_EXP,
    NATIVE_OPCODE_MATH_FLOOR, NATIVE_OPCODE_MATH_IS_FINITE, NATIVE_OPCODE_MATH_LOG,
    NATIVE_OPCODE_MATH_LOG10, NATIVE_OPCODE_MATH_MAX, NATIVE_OPCODE_MATH_MIN,
    NATIVE_OPCODE_MATH_PI, NATIVE_OPCODE_MATH_POW, NATIVE_OPCODE_MATH_RADIANS,
    NATIVE_OPCODE_MATH_ROUND, NATIVE_OPCODE_MATH_SIGN, NATIVE_OPCODE_MATH_SIN,
    NATIVE_OPCODE_MATH_SQRT, NATIVE_OPCODE_MATH_TAN, NATIVE_OPCODE_MATH_TRUNC, NATIVE_OPCODE_MOD,
    NATIVE_OPCODE_MOVE, NATIVE_OPCODE_MUL, NATIVE_OPCODE_NEG, NATIVE_OPCODE_NOT,
    NATIVE_OPCODE_NOT_EQUAL, NATIVE_OPCODE_POW, NATIVE_OPCODE_RETURN_OK,
    NATIVE_OPCODE_STRING_BYTE_LEN, NATIVE_OPCODE_STRING_CASE_FOLD, NATIVE_OPCODE_STRING_CONTAINS,
    NATIVE_OPCODE_STRING_ENDS_WITH, NATIVE_OPCODE_STRING_GRAPHEMES, NATIVE_OPCODE_STRING_JOIN,
    NATIVE_OPCODE_STRING_LOWER, NATIVE_OPCODE_STRING_NORMALIZE_NFC,
    NATIVE_OPCODE_STRING_REGEX_FIND, NATIVE_OPCODE_STRING_REGEX_MATCH,
    NATIVE_OPCODE_STRING_REGEX_REPLACE, NATIVE_OPCODE_STRING_SPLIT,
    NATIVE_OPCODE_STRING_STARTS_WITH, NATIVE_OPCODE_STRING_TRIM, NATIVE_OPCODE_STRING_TRIM_END,
    NATIVE_OPCODE_STRING_TRIM_START, NATIVE_OPCODE_STRING_UPPER, NATIVE_OPCODE_SUB,
    NATIVE_OPCODE_THREAD_CANCEL, NATIVE_OPCODE_THREAD_EMIT, NATIVE_OPCODE_THREAD_IS_CANCELLED,
    NATIVE_OPCODE_THREAD_IS_RUNNING, NATIVE_OPCODE_THREAD_POLL, NATIVE_OPCODE_THREAD_READ,
    NATIVE_OPCODE_THREAD_RECEIVE, NATIVE_OPCODE_THREAD_SEND, NATIVE_OPCODE_THREAD_START,
    NATIVE_OPCODE_THREAD_WAIT_FOR, NATIVE_OPCODE_UNWRAP_RESULT, NATIVE_OPCODE_USING_ENTER,
    NATIVE_OPCODE_USING_LEAVE, NATIVE_OPCODE_VARIANT_MATCH, NATIVE_OPCODE_XOR,
};
use crate::ir::IrProject;
use crate::target::BuildTarget;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

const SLOT_SIZE: usize = 24;
const SCRATCH_SIZE: usize = 160;
const HEAP_HEADER_SIZE: usize = 32;
const ARENA_STATE_SIZE: usize = 64;
const ARENA_DEFAULT_BLOCK_SIZE: u64 = 4096;
const ARENA_BLOCK_HEADER_SIZE: u64 = 32;
const ERR_INVALID_ARGUMENT: u64 = 10002;
const ERR_OVERFLOW: u64 = 10028;
const ERR_NOT_TERMINAL: u64 = 10007;
const ERR_PARSE: u64 = 10003;
const ERR_NOT_FOUND: u64 = 10004;
const ERR_INDEX_OUT_OF_RANGE: u64 = 10001;
const ERR_OUT_OF_MEMORY: u64 = 10010;
const ERR_OUTPUT_FAILURE: u64 = 10015;
const ERR_EOF: u64 = 10016;
const ERR_INVALID_UTF8: u64 = 10019;
const ERR_INPUT_FAILURE: u64 = 10020;
const ERR_RESOURCE_CLOSED: u64 = 10017;
const DARWIN_PROT_READ_WRITE: u64 = 0x3;
const DARWIN_MAP_PRIVATE_ANON: u64 = 0x1002;
const DARWIN_SYSCALL_EXIT: u64 = 0x0200_0001;
const DARWIN_SYSCALL_OPEN: u64 = 0x0200_0005;
const DARWIN_SYSCALL_CLOSE: u64 = 0x0200_0006;
const DARWIN_SYSCALL_READ: u64 = 0x0200_0003;
const DARWIN_SYSCALL_WRITE: u64 = 0x0200_0004;
const DARWIN_SYSCALL_IOCTL: u64 = 0x0200_0036;
const DARWIN_SYSCALL_MUNMAP: u64 = 0x0200_0049;
const DARWIN_SYSCALL_MMAP: u64 = 0x0200_00c5;
const DARWIN_TIOCGETA: u64 = 0x4048_7413;
const DARWIN_TIOCGWINSZ: u64 = 0x4008_7468;
const DARWIN_O_WRONLY: u64 = 0x0001;
const DARWIN_O_RDWR: u64 = 0x0002;
const DARWIN_O_APPEND: u64 = 0x0008;
const DARWIN_O_CREAT: u64 = 0x0200;
const DARWIN_O_TRUNC: u64 = 0x0400;
const DARWIN_OPEN_PERMISSIONS: u64 = 0o666;
const HEAP_KIND_STRING: u64 = 1;
const HEAP_KIND_LIST: u64 = 2;
const HEAP_KIND_RECORD: u64 = 3;
const HEAP_KIND_VARIANT: u64 = 4;
const HEAP_KIND_MAP: u64 = 5;

pub struct Aarch64Image {
    pub code: Vec<u8>,
    pub data: Vec<u8>,
}

pub fn write_binary_dump(
    project_dir: &Path,
    ir: &IrProject,
    target: &BuildTarget,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    if target.arch != "aarch64" {
        return Err(format!(
            "AArch64 binary output cannot write {} binaries",
            target.arch
        ));
    }

    let program = if packages.is_empty() {
        bytecode::native_program(ir)?
    } else {
        bytecode::native_program_with_packages(ir, packages)?
    };
    let image = encode(&program, 0)?;
    let path = project_dir.join(format!("{}.{}.bin", ir.name, target.arch));
    let mut bytes = image.code;
    bytes.extend_from_slice(&image.data);
    fs::write(&path, bytes)
        .map_err(|err| format!("failed to write '{}': {err}", path.display()))?;
    Ok(path)
}

pub fn encode(program: &NativeProgram, code_vmaddr: u64) -> Result<Aarch64Image, String> {
    let data = NativeData::new(program);
    let code_len = NativeEmitter::new(program, &data, code_vmaddr, code_vmaddr)
        .emit()?
        .len();
    let data_base = code_vmaddr + code_len as u64;
    let code = NativeEmitter::new(program, &data, code_vmaddr, data_base).emit()?;
    Ok(Aarch64Image {
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
                NATIVE_OPCODE_LOAD_FUNCTION => {
                    let dst = operand(instruction, 0)?;
                    let function_id = operand(instruction, 1)?;
                    self.code.mov_imm(9, function_id as u64);
                    self.code.str_imm(9, 31, slot_offset(dst))?;
                    self.code.str_imm(31, 31, slot_offset(dst) + 8)?;
                    self.code.str_imm(31, 31, slot_offset(dst) + 16)?;
                }
                NATIVE_OPCODE_MOVE | NATIVE_OPCODE_COPY => {
                    let dst = operand(instruction, 0)?;
                    let src = operand(instruction, 1)?;
                    self.copy_slot(dst, src)?;
                }
                NATIVE_OPCODE_ADD | NATIVE_OPCODE_SUB | NATIVE_OPCODE_MUL | NATIVE_OPCODE_DIV
                | NATIVE_OPCODE_MOD | NATIVE_OPCODE_POW => {
                    self.emit_numeric_arithmetic(
                        function,
                        instruction.opcode,
                        instruction,
                        epilogue,
                    )?;
                }
                NATIVE_OPCODE_EQUAL
                | NATIVE_OPCODE_NOT_EQUAL
                | NATIVE_OPCODE_LESS
                | NATIVE_OPCODE_LESS_EQUAL
                | NATIVE_OPCODE_GREATER
                | NATIVE_OPCODE_GREATER_EQUAL => {
                    self.emit_comparison(function, instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_NOT | NATIVE_OPCODE_NEG => {
                    self.emit_unary(function, instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_XOR => {
                    self.emit_xor(instruction)?;
                }
                NATIVE_OPCODE_CONCAT => {
                    self.emit_concat(instruction, epilogue)?;
                }
                NATIVE_OPCODE_IO_WRITE => {
                    self.emit_io_write(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_IO_FLUSH => {
                    self.emit_io_flush(instruction)?;
                }
                NATIVE_OPCODE_IO_READ_LINE => {
                    self.emit_io_read_line(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_IO_READ_CHAR => {
                    self.emit_io_read_char(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_IO_READ_BYTE => {
                    self.emit_io_read_byte(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_IO_IS_TERMINAL => {
                    self.emit_io_is_terminal(function, instruction)?;
                }
                NATIVE_OPCODE_IO_TERMINAL_SIZE => {
                    self.emit_io_terminal_size(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_IO_OPEN => {
                    self.emit_io_open(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_IO_CLOSE => {
                    self.emit_io_close(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_FS_OPEN | NATIVE_OPCODE_FS_OPEN_NO_FOLLOW => {
                    self.emit_io_open(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_FS_CLOSE => {
                    self.emit_io_close(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_FS_WRITE_ALL => {
                    self.emit_fs_write_all(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_FS_FILE_EXISTS
                | NATIVE_OPCODE_FS_DIRECTORY_EXISTS
                | NATIVE_OPCODE_FS_EXISTS
                | NATIVE_OPCODE_FS_READ_TEXT
                | NATIVE_OPCODE_FS_WRITE_TEXT
                | NATIVE_OPCODE_FS_WRITE_TEXT_ATOMIC
                | NATIVE_OPCODE_FS_APPEND_TEXT
                | NATIVE_OPCODE_FS_CREATE_TEMP_FILE
                | NATIVE_OPCODE_FS_READ_LINE
                | NATIVE_OPCODE_FS_READ_ALL
                | NATIVE_OPCODE_FS_EOF
                | NATIVE_OPCODE_FS_CANONICAL_PATH
                | NATIVE_OPCODE_FS_IS_WITHIN
                | NATIVE_OPCODE_FS_PATH_JOIN
                | NATIVE_OPCODE_FS_PATH_DIR_NAME
                | NATIVE_OPCODE_FS_PATH_BASE_NAME
                | NATIVE_OPCODE_FS_PATH_EXTENSION
                | NATIVE_OPCODE_FS_PATH_NORMALIZE
                | NATIVE_OPCODE_FS_DELETE_FILE
                | NATIVE_OPCODE_FS_CREATE_DIRECTORY
                | NATIVE_OPCODE_FS_CREATE_DIRECTORIES
                | NATIVE_OPCODE_FS_DELETE_DIRECTORY
                | NATIVE_OPCODE_FS_LIST_DIRECTORY
                | NATIVE_OPCODE_FS_CURRENT_DIRECTORY
                | NATIVE_OPCODE_FS_SET_CURRENT_DIRECTORY => {
                    self.emit_fs_default_result(function, instruction)?;
                }
                NATIVE_OPCODE_THREAD_START
                | NATIVE_OPCODE_THREAD_IS_RUNNING
                | NATIVE_OPCODE_THREAD_WAIT_FOR
                | NATIVE_OPCODE_THREAD_CANCEL
                | NATIVE_OPCODE_THREAD_SEND
                | NATIVE_OPCODE_THREAD_POLL
                | NATIVE_OPCODE_THREAD_READ
                | NATIVE_OPCODE_THREAD_RECEIVE
                | NATIVE_OPCODE_THREAD_EMIT
                | NATIVE_OPCODE_THREAD_IS_CANCELLED => {
                    self.emit_thread_default_result(function, instruction)?;
                }
                NATIVE_OPCODE_USING_ENTER | NATIVE_OPCODE_USING_LEAVE => {}
                NATIVE_OPCODE_CLOSE_RESOURCE => {
                    self.emit_close_resource(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_CALL_RESULT => {
                    self.emit_call_result(instruction)?;
                }
                NATIVE_OPCODE_CALL_VALUE_RESULT => {
                    self.emit_call_value_result(instruction)?;
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
                NATIVE_OPCODE_GENERAL_LEN => {
                    self.emit_general_len(instruction)?;
                }
                NATIVE_OPCODE_GENERAL_FIND => {
                    self.emit_general_find(instruction, epilogue)?;
                }
                NATIVE_OPCODE_GENERAL_MID => {
                    self.emit_general_mid(instruction, epilogue)?;
                }
                NATIVE_OPCODE_GENERAL_REPLACE => {
                    self.emit_general_replace(instruction, epilogue)?;
                }
                NATIVE_OPCODE_GENERAL_TO_STRING => {
                    self.emit_general_to_string(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_GENERAL_TO_INT => {
                    self.emit_general_to_int(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_GENERAL_TO_FLOAT | NATIVE_OPCODE_GENERAL_TO_FIXED => {
                    self.emit_general_numeric_widen_or_copy(
                        function,
                        instruction.opcode,
                        instruction,
                        epilogue,
                    )?;
                }
                NATIVE_OPCODE_GENERAL_TO_BYTE => {
                    self.emit_general_to_byte(instruction, epilogue)?;
                }
                NATIVE_OPCODE_GENERAL_IS_NUMERIC => {
                    self.emit_general_is_numeric(instruction)?;
                }
                NATIVE_OPCODE_GENERAL_IS_EVEN | NATIVE_OPCODE_GENERAL_IS_ODD => {
                    self.emit_general_integer_parity(instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_GENERAL_IS_POSITIVE
                | NATIVE_OPCODE_GENERAL_IS_NEGATIVE
                | NATIVE_OPCODE_GENERAL_IS_ZERO => {
                    self.emit_general_numeric_predicate(function, instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_GENERAL_IS_EMPTY | NATIVE_OPCODE_GENERAL_IS_NOT_EMPTY => {
                    self.emit_general_length_predicate(instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_MATH_PI | NATIVE_OPCODE_MATH_E => {
                    self.emit_math_constant(function, instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_MATH_ABS | NATIVE_OPCODE_MATH_SIGN => {
                    self.emit_math_unary(function, instruction.opcode, instruction, epilogue)?;
                }
                NATIVE_OPCODE_MATH_MIN | NATIVE_OPCODE_MATH_MAX => {
                    self.emit_math_min_max(function, instruction.opcode, instruction)?;
                }
                NATIVE_OPCODE_MATH_CLAMP => {
                    self.emit_math_clamp(function, instruction)?;
                }
                NATIVE_OPCODE_MATH_IS_FINITE => {
                    self.emit_math_is_finite(function, instruction)?;
                }
                NATIVE_OPCODE_MATH_FLOOR
                | NATIVE_OPCODE_MATH_CEIL
                | NATIVE_OPCODE_MATH_ROUND
                | NATIVE_OPCODE_MATH_TRUNC
                | NATIVE_OPCODE_MATH_SQRT
                | NATIVE_OPCODE_MATH_POW
                | NATIVE_OPCODE_MATH_EXP
                | NATIVE_OPCODE_MATH_LOG
                | NATIVE_OPCODE_MATH_LOG10
                | NATIVE_OPCODE_MATH_SIN
                | NATIVE_OPCODE_MATH_COS
                | NATIVE_OPCODE_MATH_TAN
                | NATIVE_OPCODE_MATH_ASIN
                | NATIVE_OPCODE_MATH_ACOS
                | NATIVE_OPCODE_MATH_ATAN
                | NATIVE_OPCODE_MATH_ATAN2
                | NATIVE_OPCODE_MATH_RADIANS
                | NATIVE_OPCODE_MATH_DEGREES => {
                    self.emit_math_float_intrinsic(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_GET => {
                    self.emit_collection_get(instruction, epilogue, false)?;
                }
                NATIVE_OPCODE_COLLECTION_GET_OR => {
                    self.emit_collection_get(instruction, epilogue, true)?;
                }
                NATIVE_OPCODE_COLLECTION_FIND => {
                    self.emit_collection_find(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_MID => {
                    self.emit_collection_mid(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_REPLACE => {
                    self.emit_collection_replace(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_SET => {
                    self.emit_collection_set(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_APPEND => {
                    self.emit_collection_append_prepend(instruction, epilogue, false)?;
                }
                NATIVE_OPCODE_COLLECTION_PREPEND => {
                    self.emit_collection_append_prepend(instruction, epilogue, true)?;
                }
                NATIVE_OPCODE_COLLECTION_INSERT => {
                    self.emit_collection_insert(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_REMOVE_AT => {
                    self.emit_collection_remove_at(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_REMOVE_KEY => {
                    self.emit_collection_remove_key(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_KEYS | NATIVE_OPCODE_COLLECTION_VALUES => {
                    self.emit_collection_keys_values(instruction, epilogue, instruction.opcode)?;
                }
                NATIVE_OPCODE_COLLECTION_HAS_KEY => {
                    self.emit_collection_has_key(instruction)?;
                }
                NATIVE_OPCODE_COLLECTION_CONTAINS => {
                    self.emit_collection_contains(instruction)?;
                }
                NATIVE_OPCODE_COLLECTION_SUM => {
                    self.emit_collection_sum(function, instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_FOR_EACH => {
                    self.emit_collection_for_each(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_TRANSFORM => {
                    self.emit_collection_transform(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_FILTER => {
                    self.emit_collection_filter(instruction, epilogue)?;
                }
                NATIVE_OPCODE_COLLECTION_REDUCE => {
                    self.emit_collection_reduce(instruction, epilogue)?;
                }
                NATIVE_OPCODE_STRING_TRIM
                | NATIVE_OPCODE_STRING_TRIM_START
                | NATIVE_OPCODE_STRING_TRIM_END => {
                    self.emit_string_trim(instruction, epilogue, instruction.opcode)?;
                }
                NATIVE_OPCODE_STRING_UPPER
                | NATIVE_OPCODE_STRING_LOWER
                | NATIVE_OPCODE_STRING_CASE_FOLD => {
                    self.emit_string_case(instruction, epilogue, instruction.opcode)?;
                }
                NATIVE_OPCODE_STRING_NORMALIZE_NFC => {
                    let dst = operand(instruction, 0)?;
                    let src = operand(instruction, 1)?;
                    self.copy_slot(dst, src)?;
                }
                NATIVE_OPCODE_STRING_GRAPHEMES => {
                    self.emit_string_graphemes(instruction, epilogue)?;
                }
                NATIVE_OPCODE_STRING_STARTS_WITH
                | NATIVE_OPCODE_STRING_ENDS_WITH
                | NATIVE_OPCODE_STRING_CONTAINS
                | NATIVE_OPCODE_STRING_REGEX_MATCH => {
                    self.emit_string_predicate(instruction, instruction.opcode)?;
                }
                NATIVE_OPCODE_STRING_SPLIT => {
                    self.emit_string_split(instruction, epilogue)?;
                }
                NATIVE_OPCODE_STRING_JOIN => {
                    self.emit_string_join(instruction, epilogue)?;
                }
                NATIVE_OPCODE_STRING_BYTE_LEN => {
                    self.emit_general_len(instruction)?;
                }
                NATIVE_OPCODE_STRING_REGEX_FIND => {
                    self.emit_general_find(instruction, epilogue)?;
                }
                NATIVE_OPCODE_STRING_REGEX_REPLACE => {
                    self.emit_general_replace(instruction, epilogue)?;
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
            NativeConst::Fixed(value) => {
                self.code.mov_imm(9, *value as u64);
                self.code.str_imm(9, 31, slot)
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

    fn emit_general_len(
        &mut self,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        self.code.ldr_imm(9, 31, slot_offset(value) + 8)?;
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_general_find(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let needle = operand(instruction, 2)?;
        let start = instruction.operands.get(3).copied();
        let search_loop = self.code.new_label();
        let compare_loop = self.code.new_label();
        let found = self.code.new_label();
        let next = self.code.new_label();
        let not_found = self.code.new_label();
        let empty_needle = self.code.new_label();
        let valid_start = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(needle))?;
        self.code.ldr_imm(12, 31, slot_offset(needle) + 8)?;
        if let Some(start) = start {
            self.code.ldr_imm(13, 31, slot_offset(start))?;
        } else {
            self.code.mov_imm(13, 0);
        }
        self.code.cmp_zero(13);
        self.code.b_ge(valid_start);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_start);
        self.code.cmp_reg(13, 10);
        self.code.b_gt(not_found);
        self.code.cbz(12, empty_needle);
        self.code.sub_reg(14, 10, 12);
        self.code.cmp_reg(13, 14);
        self.code.b_gt(not_found);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_reg(9, 9, 13);
        self.code.add_imm(11, 11, HEAP_HEADER_SIZE)?;

        self.code.bind(search_loop);
        self.code.mov_imm(15, 0);
        self.code.bind(compare_loop);
        self.code.cmp_reg(15, 12);
        self.code.b_eq(found);
        self.code.add_reg(16, 13, 15);
        self.code.ldrb_imm(17, 9, 0);
        self.code.ldrb_imm(18, 11, 0);
        self.code.cmp_reg(17, 18);
        self.code.b_ne(next);
        self.code.add_imm(9, 9, 1)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.add_imm(15, 15, 1)?;
        self.code.b(compare_loop);

        self.code.bind(next);
        self.code.sub_reg(9, 9, 15);
        self.code.sub_reg(11, 11, 15);
        self.code.add_imm(9, 9, 1)?;
        self.code.add_imm(13, 13, 1)?;
        self.code.cmp_reg(13, 14);
        self.code.b_le(search_loop);
        self.code.b(not_found);

        self.code.bind(empty_needle);
        self.code.str_imm(13, 31, slot_offset(dst))?;
        self.code.b(done);

        self.code.bind(found);
        self.code.str_imm(13, 31, slot_offset(dst))?;
        self.code.b(done);

        self.code.bind(not_found);
        self.emit_error(ERR_NOT_FOUND, epilogue);
        self.code.bind(done);
        Ok(())
    }

    fn emit_general_mid(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let start = operand(instruction, 2)?;
        let count = operand(instruction, 3)?;
        let valid_start = self.code.new_label();
        let valid_count = self.code.new_label();
        let valid_range = self.code.new_label();
        let copy_loop = self.code.new_label();
        let range_ok = self.code.new_label();
        let copy_done = self.code.new_label();
        let scratch = self.current_scratch;

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(start))?;
        self.code.ldr_imm(12, 31, slot_offset(count))?;
        self.code.cmp_zero(11);
        self.code.b_ge(valid_start);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_start);
        self.code.cmp_zero(12);
        self.code.b_ge(valid_count);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_count);
        self.code.add_reg(13, 11, 12);
        self.code.cmp_reg(13, 11);
        self.code.b_hs(valid_range);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_range);
        self.code.cmp_reg(13, 10);
        self.code.b_le(range_ok);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(range_ok);

        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_reg(9, 9, 11);
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(12, 31, scratch + 8)?;
        self.emit_allocate_string_from_len_reg(12, epilogue)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.add_imm(11, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(copy_loop);
        self.code.cbz(10, copy_done);
        self.code.ldrb_imm(12, 9, 0);
        self.code.strb_imm(12, 11, 0);
        self.code.add_imm(9, 9, 1)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_done);
        self.store_string_slot(dst, 1, scratch + 8)
    }

    fn emit_general_replace(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let old = operand(instruction, 2)?;
        let new = operand(instruction, 3)?;
        let scratch = self.current_scratch;
        let copy_original = self.code.new_label();
        let first_loop = self.code.new_label();
        let first_compare = self.code.new_label();
        let first_match = self.code.new_label();
        let first_next = self.code.new_label();
        let first_done = self.code.new_label();
        let second_loop = self.code.new_label();
        let second_compare = self.code.new_label();
        let second_match = self.code.new_label();
        let second_copy_new = self.code.new_label();
        let second_copy_one = self.code.new_label();
        let second_done = self.code.new_label();
        let new_copy_loop = self.code.new_label();
        let replace_done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(old))?;
        self.code.ldr_imm(12, 31, slot_offset(old) + 8)?;
        self.code.ldr_imm(13, 31, slot_offset(new))?;
        self.code.ldr_imm(14, 31, slot_offset(new) + 8)?;
        self.code.cbz(12, copy_original);
        self.code.cmp_reg(12, 10);
        self.code.b_hi(copy_original);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(11, 11, HEAP_HEADER_SIZE)?;
        self.code.add_imm(13, 13, HEAP_HEADER_SIZE)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.str_imm(12, 31, scratch + 24)?;
        self.code.str_imm(13, 31, scratch + 32)?;
        self.code.str_imm(14, 31, scratch + 40)?;

        self.code.mov_imm(20, 0);
        self.code.mov_reg(21, 10);
        self.code.sub_reg(22, 10, 12);
        self.code.bind(first_loop);
        self.code.cmp_reg(20, 22);
        self.code.b_gt(first_done);
        self.code.mov_imm(23, 0);
        self.code.add_reg(24, 9, 20);
        self.code.mov_reg(25, 11);
        self.code.bind(first_compare);
        self.code.cmp_reg(23, 12);
        self.code.b_eq(first_match);
        self.code.ldrb_imm(26, 24, 0);
        self.code.ldrb_imm(27, 25, 0);
        self.code.cmp_reg(26, 27);
        self.code.b_ne(first_next);
        self.code.add_imm(24, 24, 1)?;
        self.code.add_imm(25, 25, 1)?;
        self.code.add_imm(23, 23, 1)?;
        self.code.b(first_compare);
        self.code.bind(first_match);
        self.code.sub_reg(21, 21, 12);
        self.code.add_reg(21, 21, 14);
        self.code.add_reg(20, 20, 12);
        self.code.b(first_loop);
        self.code.bind(first_next);
        self.code.add_imm(20, 20, 1)?;
        self.code.b(first_loop);

        self.code.bind(first_done);
        self.code.str_imm(21, 31, scratch + 48)?;
        self.emit_allocate_string_from_len_reg(21, epilogue)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.ldr_imm(12, 31, scratch + 24)?;
        self.code.ldr_imm(13, 31, scratch + 32)?;
        self.code.ldr_imm(14, 31, scratch + 40)?;
        self.code.add_imm(15, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(20, 0);
        self.code.sub_reg(22, 10, 12);

        self.code.bind(second_loop);
        self.code.cmp_reg(20, 10);
        self.code.b_ge(second_done);
        self.code.cmp_reg(20, 22);
        self.code.b_gt(second_copy_one);
        self.code.mov_imm(23, 0);
        self.code.add_reg(24, 9, 20);
        self.code.mov_reg(25, 11);
        self.code.bind(second_compare);
        self.code.cmp_reg(23, 12);
        self.code.b_eq(second_match);
        self.code.ldrb_imm(26, 24, 0);
        self.code.ldrb_imm(27, 25, 0);
        self.code.cmp_reg(26, 27);
        self.code.b_ne(second_copy_one);
        self.code.add_imm(24, 24, 1)?;
        self.code.add_imm(25, 25, 1)?;
        self.code.add_imm(23, 23, 1)?;
        self.code.b(second_compare);

        self.code.bind(second_match);
        self.code.mov_imm(23, 0);
        self.code.mov_reg(24, 13);
        self.code.bind(new_copy_loop);
        self.code.cmp_reg(23, 14);
        self.code.b_eq(second_copy_new);
        self.code.ldrb_imm(26, 24, 0);
        self.code.strb_imm(26, 15, 0);
        self.code.add_imm(24, 24, 1)?;
        self.code.add_imm(15, 15, 1)?;
        self.code.add_imm(23, 23, 1)?;
        self.code.b(new_copy_loop);
        self.code.bind(second_copy_new);
        self.code.add_reg(20, 20, 12);
        self.code.b(second_loop);

        self.code.bind(second_copy_one);
        self.code.add_reg(24, 9, 20);
        self.code.ldrb_imm(26, 24, 0);
        self.code.strb_imm(26, 15, 0);
        self.code.add_imm(15, 15, 1)?;
        self.code.add_imm(20, 20, 1)?;
        self.code.b(second_loop);

        self.code.bind(second_done);
        self.store_string_slot(dst, 1, scratch + 48)?;
        self.code.b(replace_done);

        self.code.bind(copy_original);
        self.copy_slot(dst, value)?;
        self.code.bind(replace_done);
        Ok(())
    }

    fn emit_string_trim(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
        opcode: u16,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let scan_start = self.code.new_label();
        let scan_end = self.code.new_label();
        let end_done = self.code.new_label();
        let range_done = self.code.new_label();
        let copy_loop = self.code.new_label();
        let copy_done = self.code.new_label();
        let scratch = self.current_scratch;

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.add_imm(11, 9, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(12, 0);
        self.code.mov_reg(13, 10);

        if opcode != NATIVE_OPCODE_STRING_TRIM_END {
            self.code.bind(scan_start);
            self.code.cmp_reg(12, 13);
            self.code.b_ge(range_done);
            self.code.add_reg(14, 11, 12);
            self.code.ldrb_imm(15, 14, 0);
            self.emit_ascii_whitespace_flag(15, 16)?;
            self.code.cbz(16, range_done);
            self.code.add_imm(12, 12, 1)?;
            self.code.b(scan_start);
        }

        self.code.bind(range_done);
        if opcode != NATIVE_OPCODE_STRING_TRIM_START {
            self.code.bind(scan_end);
            self.code.cmp_reg(13, 12);
            self.code.b_le(end_done);
            self.code.sub_imm(14, 13, 1)?;
            self.code.add_reg(14, 11, 14);
            self.code.ldrb_imm(15, 14, 0);
            self.emit_ascii_whitespace_flag(15, 16)?;
            self.code.cbz(16, end_done);
            self.code.sub_imm(13, 13, 1)?;
            self.code.b(scan_end);
        }

        self.code.bind(end_done);
        self.code.sub_reg(10, 13, 12);
        self.code.add_reg(11, 11, 12);
        self.code.str_imm(11, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_string_from_len_reg(10, epilogue)?;
        self.code.ldr_imm(11, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.add_imm(12, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(copy_loop);
        self.code.cbz(10, copy_done);
        self.code.ldrb_imm(13, 11, 0);
        self.code.strb_imm(13, 12, 0);
        self.code.add_imm(11, 11, 1)?;
        self.code.add_imm(12, 12, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_done);
        self.store_string_slot(dst, 1, scratch + 8)
    }

    fn emit_string_case(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
        opcode: u16,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let loop_start = self.code.new_label();
        let store = self.code.new_label();
        let done = self.code.new_label();
        let scratch = self.current_scratch;
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_string_from_len_reg(10, epilogue)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.add_imm(11, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.code.ldrb_imm(12, 9, 0);
        if opcode == NATIVE_OPCODE_STRING_UPPER {
            self.code.cmp_reg_imm(12, b'a' as u64);
            self.code.b_lt(store);
            self.code.cmp_reg_imm(12, b'z' as u64);
            self.code.b_gt(store);
            self.code.sub_imm(12, 12, 32)?;
        } else {
            self.code.cmp_reg_imm(12, b'A' as u64);
            self.code.b_lt(store);
            self.code.cmp_reg_imm(12, b'Z' as u64);
            self.code.b_gt(store);
            self.code.add_imm(12, 12, 32)?;
        }
        self.code.bind(store);
        self.code.strb_imm(12, 11, 0);
        self.code.add_imm(9, 9, 1)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);
        self.code.bind(done);
        self.store_string_slot(dst, 1, scratch + 8)
    }

    fn emit_string_predicate(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        opcode: u16,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let needle = operand(instruction, 2)?;
        let search_loop = self.code.new_label();
        let compare_loop = self.code.new_label();
        let found = self.code.new_label();
        let next = self.code.new_label();
        let false_label = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(needle))?;
        self.code.ldr_imm(12, 31, slot_offset(needle) + 8)?;
        self.code.cbz(12, found);
        self.code.cmp_reg(12, 10);
        self.code.b_hi(false_label);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(11, 11, HEAP_HEADER_SIZE)?;
        if opcode == NATIVE_OPCODE_STRING_ENDS_WITH {
            self.code.sub_reg(13, 10, 12);
        } else {
            self.code.mov_imm(13, 0);
        }
        self.code.sub_reg(14, 10, 12);

        self.code.bind(search_loop);
        self.code.cmp_reg(13, 14);
        self.code.b_gt(false_label);
        self.code.mov_imm(15, 0);
        self.code.add_reg(16, 9, 13);
        self.code.mov_reg(17, 11);
        self.code.bind(compare_loop);
        self.code.cmp_reg(15, 12);
        self.code.b_eq(found);
        self.code.ldrb_imm(18, 16, 0);
        self.code.ldrb_imm(19, 17, 0);
        self.code.cmp_reg(18, 19);
        self.code.b_ne(next);
        self.code.add_imm(16, 16, 1)?;
        self.code.add_imm(17, 17, 1)?;
        self.code.add_imm(15, 15, 1)?;
        self.code.b(compare_loop);
        self.code.bind(next);
        if opcode == NATIVE_OPCODE_STRING_STARTS_WITH || opcode == NATIVE_OPCODE_STRING_ENDS_WITH {
            self.code.b(false_label);
        } else {
            self.code.add_imm(13, 13, 1)?;
            self.code.b(search_loop);
        }

        self.code.bind(found);
        self.code.mov_imm(20, 1);
        self.code.b(done);
        self.code.bind(false_label);
        self.code.mov_imm(20, 0);
        self.code.bind(done);
        self.code.str_imm(20, 31, slot_offset(dst))
    }

    fn emit_string_graphemes(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let outer_loop = self.code.new_label();
        let done = self.code.new_label();
        let scratch = self.current_scratch;

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 10, 10, epilogue)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(11, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(outer_loop);
        self.code.cbz(10, done);
        self.code.str_imm(9, 11, 0)?;
        self.code.mov_imm(12, 1);
        self.code.str_imm(12, 11, 8)?;
        self.code.str_imm(31, 11, 16)?;
        self.code.add_imm(9, 9, 1)?;
        self.code.add_imm(11, 11, SLOT_SIZE)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(outer_loop);
        self.code.bind(done);
        self.code.str_imm(1, 31, slot_offset(dst))?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.str_imm(10, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn emit_string_split(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let delimiter = operand(instruction, 2)?;
        let scratch = self.current_scratch;
        let count_loop = self.code.new_label();
        let count_compare = self.code.new_label();
        let count_match = self.code.new_label();
        let count_next = self.code.new_label();
        let count_done = self.code.new_label();
        let emit_loop = self.code.new_label();
        let emit_compare = self.code.new_label();
        let emit_match = self.code.new_label();
        let emit_next = self.code.new_label();
        let emit_done = self.code.new_label();
        let delimiter_ok = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(delimiter))?;
        self.code.ldr_imm(12, 31, slot_offset(delimiter) + 8)?;
        self.code.cbnz(12, delimiter_ok);
        self.emit_error(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(delimiter_ok);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(11, 11, HEAP_HEADER_SIZE)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.str_imm(12, 31, scratch + 24)?;
        self.code.mov_imm(13, 1);
        self.code.mov_imm(14, 0);
        self.code.cmp_reg(12, 10);
        self.code.b_hi(count_done);
        self.code.sub_reg(15, 10, 12);
        self.code.bind(count_loop);
        self.code.cmp_reg(14, 15);
        self.code.b_gt(count_done);
        self.code.mov_imm(16, 0);
        self.code.add_reg(17, 9, 14);
        self.code.mov_reg(18, 11);
        self.code.bind(count_compare);
        self.code.cmp_reg(16, 12);
        self.code.b_eq(count_match);
        self.code.ldrb_imm(19, 17, 0);
        self.code.ldrb_imm(20, 18, 0);
        self.code.cmp_reg(19, 20);
        self.code.b_ne(count_next);
        self.code.add_imm(17, 17, 1)?;
        self.code.add_imm(18, 18, 1)?;
        self.code.add_imm(16, 16, 1)?;
        self.code.b(count_compare);
        self.code.bind(count_match);
        self.code.add_imm(13, 13, 1)?;
        self.code.add_reg(14, 14, 12);
        self.code.b(count_loop);
        self.code.bind(count_next);
        self.code.add_imm(14, 14, 1)?;
        self.code.b(count_loop);

        self.code.bind(count_done);
        self.code.str_imm(13, 31, scratch + 32)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 13, 13, epilogue)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.ldr_imm(12, 31, scratch + 24)?;
        self.code.add_imm(21, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(13, 0);
        self.code.mov_imm(14, 0);
        self.code.cmp_reg(12, 10);
        self.code.b_hi(emit_done);
        self.code.sub_reg(15, 10, 12);
        self.code.bind(emit_loop);
        self.code.cmp_reg(14, 15);
        self.code.b_gt(emit_done);
        self.code.mov_imm(16, 0);
        self.code.add_reg(17, 9, 14);
        self.code.mov_reg(18, 11);
        self.code.bind(emit_compare);
        self.code.cmp_reg(16, 12);
        self.code.b_eq(emit_match);
        self.code.ldrb_imm(19, 17, 0);
        self.code.ldrb_imm(20, 18, 0);
        self.code.cmp_reg(19, 20);
        self.code.b_ne(emit_next);
        self.code.add_imm(17, 17, 1)?;
        self.code.add_imm(18, 18, 1)?;
        self.code.add_imm(16, 16, 1)?;
        self.code.b(emit_compare);
        self.code.bind(emit_match);
        self.code.sub_reg(16, 14, 13);
        self.code.add_reg(17, 9, 13);
        self.code.str_imm(17, 21, 0)?;
        self.code.str_imm(16, 21, 8)?;
        self.code.str_imm(31, 21, 16)?;
        self.code.add_imm(21, 21, SLOT_SIZE)?;
        self.code.add_reg(14, 14, 12);
        self.code.mov_reg(13, 14);
        self.code.b(emit_loop);
        self.code.bind(emit_next);
        self.code.add_imm(14, 14, 1)?;
        self.code.b(emit_loop);
        self.code.bind(emit_done);
        self.code.sub_reg(16, 10, 13);
        self.code.add_reg(17, 9, 13);
        self.code.str_imm(17, 21, 0)?;
        self.code.str_imm(16, 21, 8)?;
        self.code.str_imm(31, 21, 16)?;
        self.code.str_imm(1, 31, slot_offset(dst))?;
        self.code.ldr_imm(13, 31, scratch + 32)?;
        self.code.str_imm(13, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn emit_string_join(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let parts = operand(instruction, 1)?;
        let delimiter = operand(instruction, 2)?;
        let scratch = self.current_scratch;
        let len_loop = self.code.new_label();
        let len_done = self.code.new_label();
        let copy_part = self.code.new_label();
        let copy_delim = self.code.new_label();
        let part_done = self.code.new_label();
        let delim_done = self.code.new_label();
        let copy_loop = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(parts))?;
        self.code.ldr_imm(10, 31, slot_offset(parts) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(delimiter))?;
        self.code.ldr_imm(12, 31, slot_offset(delimiter) + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.str_imm(12, 31, scratch + 24)?;
        self.code.mov_imm(13, 0);
        self.code.mov_reg(14, 10);
        self.code.bind(len_loop);
        self.code.cbz(14, len_done);
        self.code.ldr_imm(15, 9, 8)?;
        self.code.add_reg(13, 13, 15);
        self.code.sub_imm(14, 14, 1)?;
        self.code.cbz(14, len_done);
        self.code.add_reg(13, 13, 12);
        self.code.add_imm(9, 9, SLOT_SIZE)?;
        self.code.b(len_loop);
        self.code.bind(len_done);
        self.code.str_imm(13, 31, scratch + 32)?;
        self.emit_allocate_string_from_len_reg(13, epilogue)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.ldr_imm(12, 31, scratch + 24)?;
        self.code.add_imm(13, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(copy_loop);
        self.code.cbz(10, done);
        self.code.ldr_imm(14, 9, 0)?;
        self.code.ldr_imm(15, 9, 8)?;
        self.code.bind(copy_part);
        self.code.cbz(15, part_done);
        self.code.ldrb_imm(16, 14, 0);
        self.code.strb_imm(16, 13, 0);
        self.code.add_imm(14, 14, 1)?;
        self.code.add_imm(13, 13, 1)?;
        self.code.sub_imm(15, 15, 1)?;
        self.code.b(copy_part);
        self.code.bind(part_done);
        self.code.sub_imm(10, 10, 1)?;
        self.code.cbz(10, done);
        self.code.mov_reg(14, 11);
        self.code.mov_reg(15, 12);
        self.code.bind(copy_delim);
        self.code.cbz(15, delim_done);
        self.code.ldrb_imm(16, 14, 0);
        self.code.strb_imm(16, 13, 0);
        self.code.add_imm(14, 14, 1)?;
        self.code.add_imm(13, 13, 1)?;
        self.code.sub_imm(15, 15, 1)?;
        self.code.b(copy_delim);
        self.code.bind(delim_done);
        self.code.add_imm(9, 9, SLOT_SIZE)?;
        self.code.b(copy_loop);
        self.code.bind(done);
        self.store_string_slot(dst, 1, scratch + 32)
    }

    fn emit_collection_get(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
        has_default: bool,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let key = operand(instruction, 2)?;
        let default = instruction.operands.get(3).copied();
        if has_default && default.is_none() {
            return Err("getOr is missing its default value".to_string());
        }

        let map = self.code.new_label();
        let found = self.code.new_label();
        let missing = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 9, 0)?;
        self.code.cmp_reg_imm(11, HEAP_KIND_MAP);
        self.code.b_eq(map);

        self.code.ldr_imm(11, 31, slot_offset(key))?;
        self.code.cmp_zero(11);
        self.code.b_lt(missing);
        self.code.cmp_reg(11, 10);
        self.code.b_ge(missing);
        self.code.mov_imm(12, SLOT_SIZE as u64);
        self.code.mul(12, 11, 12);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_reg(9, 9, 12);
        self.copy_address_to_slot(9, 0, dst)?;
        self.code.b(done);

        self.code.bind(map);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.ldr_imm(11, 31, slot_offset(key))?;
        self.code.mov_imm(12, 0);
        let loop_start = self.code.new_label();
        self.code.bind(loop_start);
        self.code.cmp_reg(12, 10);
        self.code.b_ge(missing);
        self.code.ldr_imm(13, 9, 0)?;
        self.code.cmp_reg(13, 11);
        self.code.b_eq(found);
        self.code.add_imm(9, 9, SLOT_SIZE * 2)?;
        self.code.add_imm(12, 12, 1)?;
        self.code.b(loop_start);
        self.code.bind(found);
        self.copy_address_to_slot(9, SLOT_SIZE, dst)?;
        self.code.b(done);

        self.code.bind(missing);
        if let Some(default) = default {
            self.copy_slot(dst, default)?;
        } else {
            self.emit_error(ERR_NOT_FOUND, epilogue);
        }
        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_find(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let needle = operand(instruction, 2)?;
        let start = instruction.operands.get(3).copied();
        let valid_start = self.code.new_label();
        let loop_start = self.code.new_label();
        let found = self.code.new_label();
        let not_found = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        if let Some(start) = start {
            self.code.ldr_imm(11, 31, slot_offset(start))?;
        } else {
            self.code.mov_imm(11, 0);
        }
        self.code.cmp_zero(11);
        self.code.b_ge(valid_start);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_start);
        self.code.cmp_reg(11, 10);
        self.code.b_gt(not_found);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(12, SLOT_SIZE as u64);
        self.code.mul(12, 11, 12);
        self.code.add_reg(9, 9, 12);
        self.code.ldr_imm(13, 31, slot_offset(needle))?;
        self.code.bind(loop_start);
        self.code.cmp_reg(11, 10);
        self.code.b_ge(not_found);
        self.code.ldr_imm(14, 9, 0)?;
        self.code.cmp_reg(14, 13);
        self.code.b_eq(found);
        self.code.add_imm(9, 9, SLOT_SIZE)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.b(loop_start);
        self.code.bind(found);
        self.code.str_imm(11, 31, slot_offset(dst))?;
        self.code.b(done);
        self.code.bind(not_found);
        self.emit_error(ERR_NOT_FOUND, epilogue);
        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_mid(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let start = operand(instruction, 2)?;
        let count = operand(instruction, 3)?;
        let valid = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(start))?;
        self.code.ldr_imm(12, 31, slot_offset(count))?;
        self.validate_list_range(11, 12, 10, valid, epilogue)?;
        self.code.bind(valid);
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 12, 12, epilogue)?;
        self.store_collection_slot(dst, 1, 12)?;
        self.code.mov_imm(13, SLOT_SIZE as u64);
        self.code.mul(13, 11, 13);
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.add_reg(9, 9, 13);
        self.code.add_imm(14, 1, HEAP_HEADER_SIZE)?;
        self.copy_slots_dynamic(9, 14, 12)
    }

    fn emit_collection_replace(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let old = operand(instruction, 2)?;
        let new = operand(instruction, 3)?;
        self.clone_list_with_replacement(dst, value, Some((old, new)), None, epilogue)
    }

    fn emit_collection_set(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let key = operand(instruction, 2)?;
        let item = operand(instruction, 3)?;
        let map = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 9, 0)?;
        self.code.cmp_reg_imm(10, HEAP_KIND_MAP);
        self.code.b_eq(map);
        self.clone_list_with_replacement(dst, value, None, Some((key, item)), epilogue)?;
        let done = self.code.new_label();
        self.code.b(done);
        self.code.bind(map);
        self.emit_collection_map_set(dst, value, key, item, epilogue)?;
        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_append_prepend(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
        prepend: bool,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let item = operand(instruction, 2)?;
        let list_items = instruction.operands.get(3).copied().unwrap_or(0) != 0;
        let list_arg = self.code.new_label();
        let done = self.code.new_label();
        if list_items {
            self.code.b(list_arg);
        }
        self.emit_collection_insert_at_end(dst, value, item, epilogue, prepend)?;
        self.code.b(done);
        self.code.bind(list_arg);
        if prepend {
            self.emit_collection_concat(dst, item, value, epilogue)?;
        } else {
            self.emit_collection_concat(dst, value, item, epilogue)?;
        }
        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_insert(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let index = operand(instruction, 2)?;
        let item = operand(instruction, 3)?;
        self.emit_collection_insert_at_index(dst, value, index, item, epilogue)
    }

    fn emit_collection_remove_at(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let index = operand(instruction, 2)?;
        self.clone_list_with_removed_index(dst, value, index, epilogue)
    }

    fn emit_collection_remove_key(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let key = operand(instruction, 2)?;
        let scratch = self.current_scratch;
        let count_loop = self.code.new_label();
        let skip = self.code.new_label();
        let counted = self.code.new_label();
        let copy_loop = self.code.new_label();
        let copy_skip = self.code.new_label();
        let copy_done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(key))?;
        self.code.add_imm(12, 9, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(13, 0);
        self.code.mov_imm(14, 0);
        self.code.bind(count_loop);
        self.code.cmp_reg(13, 10);
        self.code.b_ge(counted);
        self.code.ldr_imm(15, 12, 0)?;
        self.code.cmp_reg(15, 11);
        self.code.b_eq(skip);
        self.code.add_imm(14, 14, 1)?;
        self.code.bind(skip);
        self.code.add_imm(12, 12, SLOT_SIZE * 2)?;
        self.code.add_imm(13, 13, 1)?;
        self.code.b(count_loop);
        self.code.bind(counted);
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_MAP, 14, 14, epilogue)?;
        self.store_collection_slot(dst, 1, 14)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.add_imm(12, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(16, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(13, 0);
        self.code.bind(copy_loop);
        self.code.cmp_reg(13, 10);
        self.code.b_ge(copy_done);
        self.code.ldr_imm(15, 12, 0)?;
        self.code.cmp_reg(15, 11);
        self.code.b_eq(copy_skip);
        self.copy_dynamic_slot(12, 16)?;
        self.code.add_imm(12, 12, SLOT_SIZE)?;
        self.code.add_imm(16, 16, SLOT_SIZE)?;
        self.copy_dynamic_slot(12, 16)?;
        self.code.add_imm(12, 12, SLOT_SIZE)?;
        self.code.add_imm(16, 16, SLOT_SIZE)?;
        self.code.add_imm(13, 13, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_skip);
        self.code.add_imm(12, 12, SLOT_SIZE * 2)?;
        self.code.add_imm(13, 13, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_done);
        Ok(())
    }

    fn emit_collection_keys_values(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
        opcode: u16,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 10, 10, epilogue)?;
        self.store_collection_slot(dst, 1, 10)?;
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.add_imm(11, 9, HEAP_HEADER_SIZE)?;
        if opcode == NATIVE_OPCODE_COLLECTION_VALUES {
            self.code.add_imm(11, 11, SLOT_SIZE)?;
        }
        self.code.add_imm(12, 1, HEAP_HEADER_SIZE)?;
        let loop_start = self.code.new_label();
        let done = self.code.new_label();
        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.copy_dynamic_slot(11, 12)?;
        self.code.add_imm(11, 11, SLOT_SIZE * 2)?;
        self.code.add_imm(12, 12, SLOT_SIZE)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);
        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_has_key(
        &mut self,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let key = operand(instruction, 2)?;
        self.emit_collection_membership(dst, value, key, true)
    }

    fn emit_collection_contains(
        &mut self,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let item = operand(instruction, 2)?;
        self.emit_collection_membership(dst, value, item, false)
    }

    fn emit_collection_sum(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let dst_type = function
            .registers
            .get(dst as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("collection sum references missing register {dst}"))?;
        let loop_start = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        if dst_type == NativeType::Float {
            self.code.mov_imm(11, 0);
            self.code.fmov_d_from_x(0, 11);
            self.code.bind(loop_start);
            self.code.cbz(10, done);
            self.code.ldr_imm(12, 9, 0)?;
            self.code.fmov_d_from_x(1, 12);
            self.code.fadd_d(0, 0, 1);
            self.code.add_imm(9, 9, SLOT_SIZE)?;
            self.code.sub_imm(10, 10, 1)?;
            self.code.b(loop_start);
            self.code.bind(done);
            return self.store_double(dst, 0);
        }
        self.code.mov_imm(11, 0);
        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.code.ldr_imm(12, 9, 0)?;
        if dst_type == NativeType::Fixed {
            let ok = self.code.new_label();
            self.code.adds_reg(11, 11, 12);
            self.code.b_vc(ok);
            self.fail_current_function(ERR_OVERFLOW, epilogue);
            self.code.bind(ok);
        } else {
            self.code.add_reg(11, 11, 12);
        }
        self.code.add_imm(9, 9, SLOT_SIZE)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);
        self.code.bind(done);
        self.code.str_imm(11, 31, slot_offset(dst))
    }

    fn emit_collection_for_each(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let action = operand(instruction, 2)?;
        let scratch = self.current_scratch;
        let loop_start = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(31, 31, scratch + 16)?;

        self.code.bind(loop_start);
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.cmp_reg(11, 10);
        self.code.b_ge(done);
        self.list_element_ptr(12, 9, 11)?;
        self.code.ldr_imm(0, 12, 0)?;
        self.code.ldr_imm(1, 12, 8)?;
        self.emit_call_function_value_slot(action, epilogue)?;
        self.code.cbnz(0, epilogue);
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.b(loop_start);

        self.code.bind(done);
        self.store_zero_slot(dst)
    }

    fn emit_collection_transform(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let mapper = operand(instruction, 2)?;
        let scratch = self.current_scratch;
        let loop_start = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 10, 10, epilogue)?;
        self.store_collection_slot(dst, 1, 10)?;
        self.code.str_imm(1, 31, scratch + 24)?;
        self.code.str_imm(31, 31, scratch + 16)?;

        self.code.bind(loop_start);
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.cmp_reg(11, 10);
        self.code.b_ge(done);
        self.list_element_ptr(12, 9, 11)?;
        self.code.ldr_imm(0, 12, 0)?;
        self.code.ldr_imm(1, 12, 8)?;
        self.emit_call_function_value_slot(mapper, epilogue)?;
        self.code.cbnz(0, epilogue);
        self.code.ldr_imm(13, 31, scratch + 24)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.list_element_ptr(14, 13, 11)?;
        self.code.str_imm(1, 14, 0)?;
        self.code.str_imm(2, 14, 8)?;
        self.code.str_imm(31, 14, 16)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.b(loop_start);

        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_filter(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let predicate = operand(instruction, 2)?;
        let scratch = self.current_scratch;
        let loop_start = self.code.new_label();
        let skip = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 10, 10, epilogue)?;
        self.store_collection_slot(dst, 1, 10)?;
        self.code.str_imm(1, 31, scratch + 24)?;
        self.code.str_imm(31, 31, scratch + 16)?;
        self.code.str_imm(31, 31, scratch + 32)?;

        self.code.bind(loop_start);
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.cmp_reg(11, 10);
        self.code.b_ge(done);
        self.list_element_ptr(12, 9, 11)?;
        self.code.str_imm(12, 31, scratch + 40)?;
        self.code.ldr_imm(0, 12, 0)?;
        self.code.ldr_imm(1, 12, 8)?;
        self.emit_call_function_value_slot(predicate, epilogue)?;
        self.code.cbnz(0, epilogue);
        self.code.cbz(1, skip);
        self.code.ldr_imm(12, 31, scratch + 40)?;
        self.code.ldr_imm(13, 31, scratch + 24)?;
        self.code.ldr_imm(14, 31, scratch + 32)?;
        self.list_element_ptr(15, 13, 14)?;
        self.copy_dynamic_slot(12, 15)?;
        self.code.add_imm(14, 14, 1)?;
        self.code.str_imm(14, 31, scratch + 32)?;

        self.code.bind(skip);
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.b(loop_start);

        self.code.bind(done);
        self.code.ldr_imm(13, 31, scratch + 24)?;
        self.code.ldr_imm(14, 31, scratch + 32)?;
        self.code.str_imm(14, 13, 8)?;
        self.code.str_imm(14, 31, slot_offset(dst) + 8)?;
        Ok(())
    }

    fn emit_collection_reduce(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let initial = operand(instruction, 2)?;
        let reducer = operand(instruction, 3)?;
        let scratch = self.current_scratch;
        let loop_start = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(31, 31, scratch + 16)?;
        self.copy_slot_to_address(initial, 31, scratch + 24)?;

        self.code.bind(loop_start);
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.cmp_reg(11, 10);
        self.code.b_ge(done);
        self.code.ldr_imm(0, 31, scratch + 24)?;
        self.code.ldr_imm(1, 31, scratch + 32)?;
        self.list_element_ptr(12, 9, 11)?;
        self.code.ldr_imm(2, 12, 0)?;
        self.code.ldr_imm(3, 12, 8)?;
        self.emit_call_function_value_slot(reducer, epilogue)?;
        self.code.cbnz(0, epilogue);
        self.code.str_imm(1, 31, scratch + 24)?;
        self.code.str_imm(2, 31, scratch + 32)?;
        self.code.str_imm(31, 31, scratch + 40)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.b(loop_start);

        self.code.bind(done);
        self.copy_address_to_slot(31, scratch + 24, dst)
    }

    fn validate_list_range(
        &mut self,
        start_reg: u8,
        count_reg: u8,
        len_reg: u8,
        valid: Label,
        epilogue: Label,
    ) -> Result<(), String> {
        let valid_start = self.code.new_label();
        let valid_count = self.code.new_label();
        let valid_add = self.code.new_label();
        self.code.cmp_zero(start_reg);
        self.code.b_ge(valid_start);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_start);
        self.code.cmp_zero(count_reg);
        self.code.b_ge(valid_count);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_count);
        self.code.add_reg(15, start_reg, count_reg);
        self.code.cmp_reg(15, start_reg);
        self.code.b_hs(valid_add);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid_add);
        self.code.cmp_reg(15, len_reg);
        self.code.b_le(valid);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        Ok(())
    }

    fn emit_allocate_collection_from_len_reg(
        &mut self,
        kind: u64,
        len_reg: u8,
        payload_len_reg: u8,
        epilogue: Label,
    ) -> Result<(), String> {
        let scratch = self.current_scratch + 72;
        let size_ok = self.code.new_label();
        let total_ok = self.code.new_label();
        let alloc_ok = self.code.new_label();
        self.code.str_imm(len_reg, 31, scratch)?;
        self.code.mov_imm(17, SLOT_SIZE as u64);
        if kind == HEAP_KIND_MAP {
            self.code.add_reg(18, payload_len_reg, payload_len_reg);
            self.code.mul(0, 18, 17);
        } else {
            self.code.mul(0, payload_len_reg, 17);
        }
        self.code.mov_reg(18, 0);
        self.code.add_imm(0, 0, HEAP_HEADER_SIZE)?;
        self.code.cmp_reg(0, 18);
        self.code.b_hs(size_ok);
        self.emit_error(ERR_OUT_OF_MEMORY, epilogue);
        self.code.bind(size_ok);
        self.code.mov_imm(1, 8);
        self.code.bl(self.arena_alloc);
        self.code.cbz(0, alloc_ok);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);
        self.code.bind(alloc_ok);
        self.code.ldr_imm(10, 31, scratch)?;
        self.code.mov_imm(17, SLOT_SIZE as u64);
        if kind == HEAP_KIND_MAP {
            self.code.add_reg(18, payload_len_reg, payload_len_reg);
            self.code.mul(18, 18, 17);
        } else {
            self.code.mul(18, payload_len_reg, 17);
        }
        self.code.add_imm(0, 18, HEAP_HEADER_SIZE)?;
        self.code.cmp_reg(0, 18);
        self.code.b_hs(total_ok);
        self.emit_error(ERR_OUT_OF_MEMORY, epilogue);
        self.code.bind(total_ok);
        self.write_heap_header_from_regs(1, kind, 10, 10, 0)
    }

    fn store_collection_slot(
        &mut self,
        dst: u32,
        object_reg: u8,
        len_reg: u8,
    ) -> Result<(), String> {
        self.code.str_imm(object_reg, 31, slot_offset(dst))?;
        self.code.str_imm(len_reg, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn copy_dynamic_slot(&mut self, src_ptr_reg: u8, dst_ptr_reg: u8) -> Result<(), String> {
        self.code.ldr_imm(20, src_ptr_reg, 0)?;
        self.code.ldr_imm(21, src_ptr_reg, 8)?;
        self.code.ldr_imm(22, src_ptr_reg, 16)?;
        self.code.str_imm(20, dst_ptr_reg, 0)?;
        self.code.str_imm(21, dst_ptr_reg, 8)?;
        self.code.str_imm(22, dst_ptr_reg, 16)
    }

    fn copy_register_slot_to_ptr(&mut self, src: u32, dst_ptr_reg: u8) -> Result<(), String> {
        self.code.ldr_imm(20, 31, slot_offset(src))?;
        self.code.ldr_imm(21, 31, slot_offset(src) + 8)?;
        self.code.ldr_imm(22, 31, slot_offset(src) + 16)?;
        self.code.str_imm(20, dst_ptr_reg, 0)?;
        self.code.str_imm(21, dst_ptr_reg, 8)?;
        self.code.str_imm(22, dst_ptr_reg, 16)
    }

    fn copy_slots_dynamic(
        &mut self,
        src_ptr_reg: u8,
        dst_ptr_reg: u8,
        count_reg: u8,
    ) -> Result<(), String> {
        let loop_start = self.code.new_label();
        let done = self.code.new_label();
        self.code.bind(loop_start);
        self.code.cbz(count_reg, done);
        self.copy_dynamic_slot(src_ptr_reg, dst_ptr_reg)?;
        self.code.add_imm(src_ptr_reg, src_ptr_reg, SLOT_SIZE)?;
        self.code.add_imm(dst_ptr_reg, dst_ptr_reg, SLOT_SIZE)?;
        self.code.sub_imm(count_reg, count_reg, 1)?;
        self.code.b(loop_start);
        self.code.bind(done);
        Ok(())
    }

    fn list_element_ptr(
        &mut self,
        dst_reg: u8,
        list_ptr_reg: u8,
        index_reg: u8,
    ) -> Result<(), String> {
        self.code.mov_imm(17, SLOT_SIZE as u64);
        self.code.mul(dst_reg, index_reg, 17);
        self.code.add_imm(dst_reg, dst_reg, HEAP_HEADER_SIZE)?;
        self.code.add_reg(dst_reg, list_ptr_reg, dst_reg);
        Ok(())
    }

    fn emit_call_function_value_slot(
        &mut self,
        function_value: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        self.code.ldr_imm(20, 31, slot_offset(function_value))?;
        let done = self.code.new_label();
        let mut labels = Vec::new();
        for index in 0..self.program.functions.len() {
            labels.push(self.code.new_label());
            self.code.mov_imm(21, index as u64);
            self.code.cmp_reg(20, 21);
            self.code.b_eq(labels[index]);
        }
        let builtin_predicates = [
            builtins::general::BUILTIN_FUNCTION_IS_EVEN,
            builtins::general::BUILTIN_FUNCTION_IS_ODD,
            builtins::general::BUILTIN_FUNCTION_IS_POSITIVE,
            builtins::general::BUILTIN_FUNCTION_IS_NEGATIVE,
            builtins::general::BUILTIN_FUNCTION_IS_ZERO,
            builtins::general::BUILTIN_FUNCTION_IS_EMPTY,
            builtins::general::BUILTIN_FUNCTION_IS_NOT_EMPTY,
            builtins::general::BUILTIN_FUNCTION_IS_POSITIVE_FLOAT,
            builtins::general::BUILTIN_FUNCTION_IS_POSITIVE_FIXED,
            builtins::general::BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT,
            builtins::general::BUILTIN_FUNCTION_IS_NEGATIVE_FIXED,
            builtins::general::BUILTIN_FUNCTION_IS_ZERO_FLOAT,
            builtins::general::BUILTIN_FUNCTION_IS_ZERO_FIXED,
        ];
        let mut builtin_labels = Vec::new();
        for function_id in builtin_predicates {
            let label = self.code.new_label();
            builtin_labels.push((function_id, label));
            self.code.mov_imm(21, function_id as u64);
            self.code.cmp_reg(20, 21);
            self.code.b_eq(label);
        }
        self.code.mov_imm(0, ERR_INVALID_ARGUMENT);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);
        for (index, label) in labels.into_iter().enumerate() {
            self.code.bind(label);
            self.code.bl(self.function_label(index as u32)?);
            self.code.b(done);
        }
        for (function_id, label) in builtin_labels {
            self.code.bind(label);
            self.emit_builtin_predicate_function(function_id)?;
            self.code.b(done);
        }
        self.code.bind(done);
        Ok(())
    }

    fn emit_builtin_predicate_function(&mut self, function_id: u32) -> Result<(), String> {
        let false_label = self.code.new_label();
        let done = self.code.new_label();

        match function_id {
            builtins::general::BUILTIN_FUNCTION_IS_EVEN
            | builtins::general::BUILTIN_FUNCTION_IS_ODD => {
                self.code.mov_imm(10, 2);
                self.code.sdiv(11, 0, 10);
                self.code.msub(12, 11, 10, 0);
                self.code.cmp_zero(12);
                if function_id == builtins::general::BUILTIN_FUNCTION_IS_EVEN {
                    self.code.b_ne(false_label);
                } else {
                    self.code.b_eq(false_label);
                }
            }
            builtins::general::BUILTIN_FUNCTION_IS_POSITIVE => {
                self.code.cmp_zero(0);
                self.code.b_le(false_label);
            }
            builtins::general::BUILTIN_FUNCTION_IS_POSITIVE_FLOAT
            | builtins::general::BUILTIN_FUNCTION_IS_POSITIVE_FIXED => {
                self.emit_float_bits_positive_check(false_label);
            }
            builtins::general::BUILTIN_FUNCTION_IS_NEGATIVE => {
                self.code.cmp_zero(0);
                self.code.b_ge(false_label);
            }
            builtins::general::BUILTIN_FUNCTION_IS_NEGATIVE_FLOAT
            | builtins::general::BUILTIN_FUNCTION_IS_NEGATIVE_FIXED => {
                self.emit_float_bits_negative_check(false_label);
            }
            builtins::general::BUILTIN_FUNCTION_IS_ZERO => {
                self.code.cmp_zero(0);
                self.code.b_ne(false_label);
            }
            builtins::general::BUILTIN_FUNCTION_IS_ZERO_FLOAT
            | builtins::general::BUILTIN_FUNCTION_IS_ZERO_FIXED => {
                self.emit_float_bits_zero_check(false_label);
            }
            builtins::general::BUILTIN_FUNCTION_IS_EMPTY => {
                self.code.cmp_zero(1);
                self.code.b_ne(false_label);
            }
            builtins::general::BUILTIN_FUNCTION_IS_NOT_EMPTY => {
                self.code.cmp_zero(1);
                self.code.b_eq(false_label);
            }
            _ => return Err(format!("unsupported built-in function id {function_id}")),
        }

        self.code.mov_imm(0, 0);
        self.code.mov_imm(1, 1);
        self.code.mov_imm(2, 0);
        self.code.b(done);
        self.code.bind(false_label);
        self.code.mov_imm(0, 0);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.bind(done);
        Ok(())
    }

    fn emit_float_bits_positive_check(&mut self, false_label: Label) {
        self.code.mov_imm(10, 0x8000_0000_0000_0000);
        self.code.and_reg(11, 0, 10);
        self.code.mov_imm(12, 0x7fff_ffff_ffff_ffff);
        self.code.and_reg(13, 0, 12);
        self.code.cbnz(11, false_label);
        self.code.cbz(13, false_label);
    }

    fn emit_float_bits_negative_check(&mut self, false_label: Label) {
        self.code.mov_imm(10, 0x8000_0000_0000_0000);
        self.code.and_reg(11, 0, 10);
        self.code.mov_imm(12, 0x7fff_ffff_ffff_ffff);
        self.code.and_reg(13, 0, 12);
        self.code.cbz(11, false_label);
        self.code.cbz(13, false_label);
    }

    fn emit_float_bits_zero_check(&mut self, false_label: Label) {
        self.code.mov_imm(12, 0x7fff_ffff_ffff_ffff);
        self.code.and_reg(13, 0, 12);
        self.code.cbnz(13, false_label);
    }

    fn clone_list_with_replacement(
        &mut self,
        dst: u32,
        value: u32,
        replace_equal: Option<(u32, u32)>,
        replace_index: Option<(u32, u32)>,
        epilogue: Label,
    ) -> Result<(), String> {
        let valid_index = self.code.new_label();
        let copy_loop = self.code.new_label();
        let use_new = self.code.new_label();
        let copied = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        if let Some((index, _)) = replace_index {
            self.code.ldr_imm(11, 31, slot_offset(index))?;
            self.code.cmp_zero(11);
            self.code.b_ge(valid_index);
            self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
            self.code.bind(valid_index);
            self.code.cmp_reg(11, 10);
            self.code.b_lt(done);
            self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        }
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 10, 10, epilogue)?;
        self.store_collection_slot(dst, 1, 10)?;
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.add_imm(12, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(13, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(14, 0);
        self.code.bind(copy_loop);
        self.code.cmp_reg(14, 10);
        self.code.b_ge(done);
        if let Some((old, _)) = replace_equal {
            self.code.ldr_imm(15, 12, 0)?;
            self.code.ldr_imm(16, 31, slot_offset(old))?;
            self.code.cmp_reg(15, 16);
            self.code.b_eq(use_new);
        }
        if let Some((index, _)) = replace_index {
            self.code.ldr_imm(15, 31, slot_offset(index))?;
            self.code.cmp_reg(14, 15);
            self.code.b_eq(use_new);
        }
        self.copy_dynamic_slot(12, 13)?;
        self.code.b(copied);
        self.code.bind(use_new);
        let new_value = replace_equal
            .map(|(_, new)| new)
            .or_else(|| replace_index.map(|(_, new)| new))
            .ok_or_else(|| "list replacement is missing replacement value".to_string())?;
        self.copy_register_slot_to_ptr(new_value, 13)?;
        self.code.bind(copied);
        self.code.add_imm(12, 12, SLOT_SIZE)?;
        self.code.add_imm(13, 13, SLOT_SIZE)?;
        self.code.add_imm(14, 14, 1)?;
        self.code.b(copy_loop);
        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_insert_at_end(
        &mut self,
        dst: u32,
        value: u32,
        item: u32,
        epilogue: Label,
        prepend: bool,
    ) -> Result<(), String> {
        let scratch = self.current_scratch;
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.add_imm(11, 10, 1)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 11, 11, epilogue)?;
        self.store_collection_slot(dst, 1, 11)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.add_imm(12, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(13, 1, HEAP_HEADER_SIZE)?;
        if prepend {
            self.copy_register_slot_to_ptr(item, 13)?;
            self.code.add_imm(13, 13, SLOT_SIZE)?;
            self.copy_slots_dynamic(12, 13, 10)
        } else {
            self.copy_slots_dynamic(12, 13, 10)?;
            self.copy_register_slot_to_ptr(item, 13)
        }
    }

    fn emit_collection_concat(
        &mut self,
        dst: u32,
        left: u32,
        right: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let scratch = self.current_scratch;
        self.code.ldr_imm(9, 31, slot_offset(left))?;
        self.code.ldr_imm(10, 31, slot_offset(left) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(right))?;
        self.code.ldr_imm(12, 31, slot_offset(right) + 8)?;
        self.code.add_reg(13, 10, 12);
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.code.str_imm(12, 31, scratch + 24)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 13, 13, epilogue)?;
        self.store_collection_slot(dst, 1, 13)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.ldr_imm(12, 31, scratch + 24)?;
        self.code.add_imm(14, 1, HEAP_HEADER_SIZE)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.copy_slots_dynamic(9, 14, 10)?;
        self.code.add_imm(11, 11, HEAP_HEADER_SIZE)?;
        self.copy_slots_dynamic(11, 14, 12)
    }

    fn emit_collection_insert_at_index(
        &mut self,
        dst: u32,
        value: u32,
        index: u32,
        item: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let scratch = self.current_scratch;
        let valid = self.code.new_label();
        let copy_loop = self.code.new_label();
        let insert_item = self.code.new_label();
        let copied = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(index))?;
        self.code.cmp_zero(11);
        self.code.b_ge(valid);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid);
        self.code.cmp_reg(11, 10);
        self.code.b_le(insert_item);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(insert_item);
        self.code.add_imm(12, 10, 1)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 12, 12, epilogue)?;
        self.store_collection_slot(dst, 1, 12)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.add_imm(13, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(14, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(15, 0);
        self.code.bind(copy_loop);
        self.code.cmp_reg(15, 10);
        self.code.b_ge(done);
        self.code.cmp_reg(15, 11);
        self.code.b_ne(copied);
        self.copy_register_slot_to_ptr(item, 14)?;
        self.code.add_imm(14, 14, SLOT_SIZE)?;
        self.code.bind(copied);
        self.copy_dynamic_slot(13, 14)?;
        self.code.add_imm(13, 13, SLOT_SIZE)?;
        self.code.add_imm(14, 14, SLOT_SIZE)?;
        self.code.add_imm(15, 15, 1)?;
        self.code.b(copy_loop);
        self.code.bind(done);
        self.code.cmp_reg(10, 11);
        let after_tail = self.code.new_label();
        self.code.b_ne(after_tail);
        self.copy_register_slot_to_ptr(item, 14)?;
        self.code.bind(after_tail);
        Ok(())
    }

    fn clone_list_with_removed_index(
        &mut self,
        dst: u32,
        value: u32,
        index: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let scratch = self.current_scratch;
        let valid = self.code.new_label();
        let copy_loop = self.code.new_label();
        let skip = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(index))?;
        self.code.cmp_zero(11);
        self.code.b_ge(valid);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(valid);
        self.code.cmp_reg(11, 10);
        let in_range = self.code.new_label();
        self.code.b_lt(in_range);
        self.emit_error(ERR_INDEX_OUT_OF_RANGE, epilogue);
        self.code.bind(in_range);
        self.code.sub_imm(12, 10, 1)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch + 16)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_LIST, 12, 12, epilogue)?;
        self.store_collection_slot(dst, 1, 12)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch + 16)?;
        self.code.add_imm(13, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(14, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(15, 0);
        self.code.bind(copy_loop);
        self.code.cmp_reg(15, 10);
        self.code.b_ge(done);
        self.code.cmp_reg(15, 11);
        self.code.b_eq(skip);
        self.copy_dynamic_slot(13, 14)?;
        self.code.add_imm(14, 14, SLOT_SIZE)?;
        self.code.bind(skip);
        self.code.add_imm(13, 13, SLOT_SIZE)?;
        self.code.add_imm(15, 15, 1)?;
        self.code.b(copy_loop);
        self.code.bind(done);
        Ok(())
    }

    fn emit_collection_map_set(
        &mut self,
        dst: u32,
        value: u32,
        key: u32,
        item: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let scratch = self.current_scratch;
        let scan_loop = self.code.new_label();
        let scan_found = self.code.new_label();
        let scan_done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(13, 31, slot_offset(key))?;
        self.code.add_imm(11, 9, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(12, 0);
        self.code.mov_imm(14, 0);
        self.code.bind(scan_loop);
        self.code.cmp_reg(12, 10);
        self.code.b_ge(scan_done);
        self.code.ldr_imm(15, 11, 0)?;
        self.code.cmp_reg(15, 13);
        self.code.b_eq(scan_found);
        self.code.add_imm(11, 11, SLOT_SIZE * 2)?;
        self.code.add_imm(12, 12, 1)?;
        self.code.b(scan_loop);
        self.code.bind(scan_found);
        self.code.mov_imm(14, 1);
        self.code.bind(scan_done);
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(14, 31, scratch + 16)?;
        self.code.mov_reg(15, 10);
        let length_ready = self.code.new_label();
        self.code.cbnz(14, length_ready);
        self.code.add_imm(15, 15, 1)?;
        self.code.bind(length_ready);
        self.code.str_imm(15, 31, scratch + 24)?;
        self.emit_allocate_collection_from_len_reg(HEAP_KIND_MAP, 15, 15, epilogue)?;
        self.store_collection_slot(dst, 1, 15)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(14, 31, scratch + 16)?;
        self.code.add_imm(11, 9, HEAP_HEADER_SIZE)?;
        self.code.add_imm(12, 1, HEAP_HEADER_SIZE)?;
        let loop_start = self.code.new_label();
        let replace = self.code.new_label();
        let copied = self.code.new_label();
        let done = self.code.new_label();
        let append_missing = self.code.new_label();
        let all_done = self.code.new_label();
        self.code.ldr_imm(13, 31, slot_offset(key))?;
        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.copy_dynamic_slot(11, 12)?;
        self.code.ldr_imm(14, 11, 0)?;
        self.code.cmp_reg(14, 13);
        self.code.b_eq(replace);
        self.code.add_imm(11, 11, SLOT_SIZE)?;
        self.code.add_imm(12, 12, SLOT_SIZE)?;
        self.copy_dynamic_slot(11, 12)?;
        self.code.b(copied);
        self.code.bind(replace);
        self.copy_register_slot_to_ptr(item, 12)?;
        self.code.bind(copied);
        self.code.add_imm(11, 11, SLOT_SIZE)?;
        self.code.add_imm(12, 12, SLOT_SIZE)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);
        self.code.bind(done);
        self.code.cbnz(14, all_done);
        self.code.bind(append_missing);
        self.copy_register_slot_to_ptr(key, 12)?;
        self.code.add_imm(12, 12, SLOT_SIZE)?;
        self.copy_register_slot_to_ptr(item, 12)?;
        self.code.bind(all_done);
        Ok(())
    }

    fn emit_collection_membership(
        &mut self,
        dst: u32,
        value: u32,
        needle: u32,
        map_keys: bool,
    ) -> Result<(), String> {
        let loop_start = self.code.new_label();
        let found = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(value) + 8)?;
        self.code.ldr_imm(11, 31, slot_offset(needle))?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(12, 0);
        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.code.ldr_imm(13, 9, 0)?;
        self.code.cmp_reg(13, 11);
        self.code.b_eq(found);
        self.code
            .add_imm(9, 9, if map_keys { SLOT_SIZE * 2 } else { SLOT_SIZE })?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);
        self.code.bind(found);
        self.code.mov_imm(12, 1);
        self.code.bind(done);
        self.code.str_imm(12, 31, slot_offset(dst))
    }

    fn emit_general_to_string(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("toString references missing register {src}"))?;
        match src_type {
            NativeType::String => self.copy_slot(dst, src),
            NativeType::Boolean => self.emit_bool_to_string(dst, src, epilogue),
            NativeType::Byte | NativeType::Integer => {
                self.emit_integer_to_string(dst, src, epilogue)
            }
            NativeType::Other => self.emit_byte_list_to_string(dst, src, epilogue),
            NativeType::Fixed => self.emit_fixed_to_string(dst, src, epilogue),
            NativeType::Float => self.emit_float_to_string(dst, src, epilogue),
            _ => Err(format!(
                "toString native lowering does not support {}",
                native_type_name(src_type)
            )),
        }
    }

    fn emit_general_to_int(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("toInt references missing register {src}"))?;
        match src_type {
            NativeType::String => self.emit_parse_integer_to_slot(dst, src, epilogue),
            NativeType::Fixed => {
                self.code.ldr_imm(9, 31, slot_offset(src))?;
                self.code.asr_imm(9, 9, 32);
                self.code.str_imm(9, 31, slot_offset(dst))
            }
            NativeType::Float => self.emit_float_to_integer(dst, src),
            _ => {
                self.code.ldr_imm(9, 31, slot_offset(src))?;
                self.code.str_imm(9, 31, slot_offset(dst))
            }
        }
    }

    fn emit_general_numeric_widen_or_copy(
        &mut self,
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("numeric conversion references missing register {src}"))?;
        if opcode == NATIVE_OPCODE_GENERAL_TO_FIXED {
            return match src_type {
                NativeType::String => self.emit_parse_fixed_to_slot(dst, src, epilogue),
                NativeType::Integer => self.emit_integer_to_fixed(dst, src, epilogue),
                NativeType::Float => self.emit_float_to_fixed(dst, src),
                _ => Err(format!(
                    "toFixed native lowering does not support {}",
                    native_type_name(src_type)
                )),
            };
        }
        if src_type == NativeType::String {
            self.emit_parse_fixed_to_slot(dst, src, epilogue)?;
            return self.emit_fixed_to_float(dst, dst);
        }
        if src_type == NativeType::Integer {
            return self.emit_integer_slot_to_float(dst, src);
        }
        if src_type == NativeType::Fixed {
            return self.emit_fixed_to_float(dst, src);
        }
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_general_to_byte(
        &mut self,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let valid_low = self.code.new_label();
        let valid_high = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.cmp_zero(9);
        self.code.b_ge(valid_low);
        self.emit_error(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(valid_low);
        self.code.cmp_reg_imm(9, 255);
        self.code.b_ls(valid_high);
        self.emit_error(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(valid_high);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_general_is_numeric(
        &mut self,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let loop_start = self.code.new_label();
        let digit = self.code.new_label();
        let true_label = self.code.new_label();
        let false_label = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.ldr_imm(10, 31, slot_offset(src) + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.cbz(10, false_label);
        self.code.ldrb_imm(11, 9, 0);
        self.code.cmp_reg_imm(11, b'-' as u64);
        self.code.b_ne(loop_start);
        self.code.add_imm(9, 9, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.cbz(10, false_label);

        self.code.bind(loop_start);
        self.code.cbz(10, true_label);
        self.code.ldrb_imm(11, 9, 0);
        self.code.cmp_reg_imm(11, b'0' as u64);
        self.code.b_hs(digit);
        self.code.b(false_label);
        self.code.bind(digit);
        self.code.cmp_reg_imm(11, b'9' as u64);
        self.code.b_hi(false_label);
        self.code.add_imm(9, 9, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);

        self.code.bind(true_label);
        self.code.mov_imm(12, 1);
        self.code.b(done);
        self.code.bind(false_label);
        self.code.mov_imm(12, 0);
        self.code.bind(done);
        self.code.str_imm(12, 31, slot_offset(dst))
    }

    fn emit_general_integer_parity(
        &mut self,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let false_label = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.mov_imm(10, 2);
        self.code.sdiv(11, 9, 10);
        self.code.msub(12, 11, 10, 9);
        self.code.cmp_zero(12);
        if opcode == NATIVE_OPCODE_GENERAL_IS_EVEN {
            self.code.b_ne(false_label);
        } else {
            self.code.b_eq(false_label);
        }
        self.code.mov_imm(13, 1);
        self.code.b(done);
        self.code.bind(false_label);
        self.code.mov_imm(13, 0);
        self.code.bind(done);
        self.code.str_imm(13, 31, slot_offset(dst))
    }

    fn emit_math_constant(
        &mut self,
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let dst_type = function
            .registers
            .get(dst as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("math constant references missing register {dst}"))?;
        let float_value = match opcode {
            NATIVE_OPCODE_MATH_PI => std::f64::consts::PI,
            NATIVE_OPCODE_MATH_E => std::f64::consts::E,
            _ => unreachable!(),
        };
        let bits = match dst_type {
            NativeType::Float => float_value.to_bits(),
            NativeType::Fixed => (float_value * 4_294_967_296.0).round() as i64 as u64,
            _ => {
                return Err(format!(
                    "math constant cannot write {}",
                    native_type_name(dst_type)
                ));
            }
        };
        self.code.mov_imm(9, bits);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_math_unary(
        &mut self,
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("math unary references missing register {src}"))?;
        if opcode == NATIVE_OPCODE_MATH_ABS && matches!(src_type, NativeType::Float) {
            self.code.ldr_imm(9, 31, slot_offset(src))?;
            self.code.mov_imm(10, 0x7fff_ffff_ffff_ffff);
            self.code.and_reg(9, 9, 10);
            return self.code.str_imm(9, 31, slot_offset(dst));
        }
        if opcode == NATIVE_OPCODE_MATH_SIGN && matches!(src_type, NativeType::Float) {
            let negative = self.code.new_label();
            let zero = self.code.new_label();
            let done = self.code.new_label();
            self.load_float_as_double(0, src)?;
            self.code.fcmp_zero_d(0);
            self.code.b_lt(negative);
            self.code.b_eq(zero);
            self.code.mov_imm(9, 1);
            self.code.b(done);
            self.code.bind(negative);
            self.code.mov_imm(9, (-1_i64) as u64);
            self.code.b(done);
            self.code.bind(zero);
            self.code.mov_imm(9, 0);
            self.code.bind(done);
            return self.code.str_imm(9, 31, slot_offset(dst));
        }
        let negative = self.code.new_label();
        let zero = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.cmp_zero(9);
        match opcode {
            NATIVE_OPCODE_MATH_ABS => {
                self.code.b_ge(done);
                self.code.mov_imm(10, i64::MIN as u64);
                self.code.cmp_reg(9, 10);
                self.code.b_ne(negative);
                self.fail_current_function(ERR_OVERFLOW, epilogue);
                self.code.bind(negative);
                self.code.neg(9, 9);
            }
            NATIVE_OPCODE_MATH_SIGN => {
                self.code.b_lt(negative);
                self.code.b_eq(zero);
                self.code.mov_imm(9, 1);
                self.code.b(done);
                self.code.bind(negative);
                self.code.mov_imm(9, (-1_i64) as u64);
                self.code.b(done);
                self.code.bind(zero);
                self.code.mov_imm(9, 0);
            }
            _ => unreachable!(),
        }
        self.code.bind(done);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_math_min_max(
        &mut self,
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let left = operand(instruction, 1)?;
        let right = operand(instruction, 2)?;
        let dst_type = function
            .registers
            .get(dst as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("math min/max references missing register {dst}"))?;
        let keep_left = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(left))?;
        self.code.ldr_imm(10, 31, slot_offset(right))?;
        if dst_type == NativeType::Float {
            self.code.fmov_d_from_x(0, 9);
            self.code.fmov_d_from_x(1, 10);
            self.code.fcmp_d(0, 1);
        } else {
            self.code.cmp_reg(9, 10);
        }
        if opcode == NATIVE_OPCODE_MATH_MIN {
            self.code.b_le(keep_left);
        } else {
            self.code.b_ge(keep_left);
        }
        self.code.mov_reg(9, 10);
        self.code.b(done);
        self.code.bind(keep_left);
        self.code.bind(done);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_math_clamp(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let value = operand(instruction, 1)?;
        let low = operand(instruction, 2)?;
        let high = operand(instruction, 3)?;
        let dst_type = function
            .registers
            .get(dst as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("math clamp references missing register {dst}"))?;
        let above_low = self.code.new_label();
        let below_high = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(value))?;
        self.code.ldr_imm(10, 31, slot_offset(low))?;
        self.code.ldr_imm(11, 31, slot_offset(high))?;
        if dst_type == NativeType::Float {
            self.code.fmov_d_from_x(0, 9);
            self.code.fmov_d_from_x(1, 10);
            self.code.fcmp_d(0, 1);
        } else {
            self.code.cmp_reg(9, 10);
        }
        self.code.b_ge(above_low);
        self.code.mov_reg(9, 10);
        self.code.bind(above_low);
        if dst_type == NativeType::Float {
            self.code.fmov_d_from_x(0, 9);
            self.code.fmov_d_from_x(1, 11);
            self.code.fcmp_d(0, 1);
        } else {
            self.code.cmp_reg(9, 11);
        }
        self.code.b_le(below_high);
        self.code.mov_reg(9, 11);
        self.code.bind(below_high);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_math_is_finite(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("math isFinite references missing register {src}"))?;
        if src_type == NativeType::Float {
            let false_label = self.code.new_label();
            let done = self.code.new_label();
            self.code.ldr_imm(9, 31, slot_offset(src))?;
            self.code.lsr_imm(10, 9, 52);
            self.code.mov_imm(11, 0x7ff);
            self.code.and_reg(10, 10, 11);
            self.code.cmp_reg(10, 11);
            self.code.b_eq(false_label);
            self.code.mov_imm(9, 1);
            self.code.b(done);
            self.code.bind(false_label);
            self.code.mov_imm(9, 0);
            self.code.bind(done);
            return self.code.str_imm(9, 31, slot_offset(dst));
        }
        self.code.mov_imm(9, 1);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_math_float_intrinsic(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        if instruction.operands.iter().skip(1).any(|operand| {
            function
                .registers
                .get(*operand as usize)
                .is_some_and(|register| register.type_ == NativeType::Fixed)
        }) {
            return self.emit_fixed_math_intrinsic(instruction.opcode, dst, instruction, epilogue);
        }
        if instruction.operands.get(1).is_some() {
            self.emit_float_math_intrinsic(instruction.opcode, dst, instruction, epilogue)?;
        } else {
            self.code.mov_imm(9, 0);
            self.code.str_imm(9, 31, slot_offset(dst))?;
        }
        Ok(())
    }

    fn emit_float_math_intrinsic(
        &mut self,
        opcode: u16,
        dst: u32,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let src = operand(instruction, 1)?;
        self.load_float_as_double(0, src)?;
        match opcode {
            NATIVE_OPCODE_MATH_FLOOR
            | NATIVE_OPCODE_MATH_CEIL
            | NATIVE_OPCODE_MATH_ROUND
            | NATIVE_OPCODE_MATH_TRUNC => self.emit_float_rounding(opcode, dst),
            NATIVE_OPCODE_MATH_RADIANS | NATIVE_OPCODE_MATH_DEGREES => {
                if opcode == NATIVE_OPCODE_MATH_RADIANS {
                    self.emit_f64_const(1, std::f64::consts::PI / 180.0);
                } else {
                    self.emit_f64_const(1, 180.0 / std::f64::consts::PI);
                }
                self.code.fmul_d(0, 0, 1);
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_SQRT => {
                self.emit_domain_nonnegative_d(0, epilogue);
                self.code.fsqrt_d(0, 0);
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_EXP => {
                self.emit_exp_float_d(0, epilogue);
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_LOG | NATIVE_OPCODE_MATH_LOG10 => {
                self.emit_log_d(0, epilogue);
                if opcode == NATIVE_OPCODE_MATH_LOG10 {
                    self.emit_f64_const(1, std::f64::consts::LN_10);
                    self.code.fdiv_d(0, 0, 1);
                }
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_SIN | NATIVE_OPCODE_MATH_COS | NATIVE_OPCODE_MATH_TAN => {
                match opcode {
                    NATIVE_OPCODE_MATH_SIN => self.emit_sin_d(0),
                    NATIVE_OPCODE_MATH_COS => self.emit_cos_d(0),
                    NATIVE_OPCODE_MATH_TAN => self.emit_tan_d(0, epilogue),
                    _ => unreachable!(),
                }
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_ASIN | NATIVE_OPCODE_MATH_ACOS | NATIVE_OPCODE_MATH_ATAN => {
                match opcode {
                    NATIVE_OPCODE_MATH_ASIN => self.emit_asin_d(0, epilogue),
                    NATIVE_OPCODE_MATH_ACOS => self.emit_acos_d(0, epilogue),
                    NATIVE_OPCODE_MATH_ATAN => self.emit_atan_d(0),
                    _ => unreachable!(),
                }
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_ATAN2 => {
                let right = operand(instruction, 2)?;
                self.load_float_as_double(1, right)?;
                self.emit_atan2_d(0, 1, epilogue);
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_POW => {
                let exponent = operand(instruction, 2)?;
                self.load_float_as_double(1, exponent)?;
                self.emit_pow_float_d(0, 1, epilogue);
                self.store_double(dst, 0)
            }
            _ => Err(format!(
                "native float lowering does not support math opcode {opcode}"
            )),
        }
    }

    fn emit_fixed_rounding(
        &mut self,
        opcode: u16,
        dst: u32,
        src: u32,
        _epilogue: Label,
    ) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        match opcode {
            NATIVE_OPCODE_MATH_FLOOR => {
                self.code.asr_imm(11, 9, 32);
            }
            NATIVE_OPCODE_MATH_CEIL => {
                let done = self.code.new_label();
                self.code.asr_imm(11, 9, 32);
                self.code.cmp_zero(9);
                self.code.b_le(done);
                self.code.lsl_imm(12, 9, 32);
                self.code.cbz(12, done);
                self.code.add_imm(11, 11, 1)?;
                self.code.bind(done);
            }
            NATIVE_OPCODE_MATH_TRUNC => {
                let nonnegative = self.code.new_label();
                let done = self.code.new_label();
                self.code.cmp_zero(9);
                self.code.b_ge(nonnegative);
                self.code.neg(9, 9);
                self.code.lsr_imm(11, 9, 32);
                self.code.neg(11, 11);
                self.code.b(done);
                self.code.bind(nonnegative);
                self.code.lsr_imm(11, 9, 32);
                self.code.bind(done);
            }
            NATIVE_OPCODE_MATH_ROUND => {
                let nonnegative = self.code.new_label();
                let done = self.code.new_label();
                self.code.cmp_zero(9);
                self.code.b_ge(nonnegative);
                self.code.neg(9, 9);
                self.code.mov_imm(12, 0x8000_0000);
                self.code.add_reg(9, 9, 12);
                self.code.lsr_imm(11, 9, 32);
                self.code.neg(11, 11);
                self.code.b(done);
                self.code.bind(nonnegative);
                self.code.mov_imm(12, 0x8000_0000);
                self.code.add_reg(9, 9, 12);
                self.code.lsr_imm(11, 9, 32);
                self.code.bind(done);
            }
            _ => unreachable!(),
        }
        self.code.str_imm(11, 31, slot_offset(dst))
    }

    fn emit_float_rounding(&mut self, opcode: u16, dst: u32) -> Result<(), String> {
        match opcode {
            NATIVE_OPCODE_MATH_FLOOR => self.code.fcvtms_x_from_d(11, 0),
            NATIVE_OPCODE_MATH_CEIL => self.code.fcvtps_x_from_d(11, 0),
            NATIVE_OPCODE_MATH_ROUND => self.code.fcvtas_x_from_d(11, 0),
            NATIVE_OPCODE_MATH_TRUNC => self.code.fcvtzs_x_from_d(11, 0),
            _ => unreachable!(),
        }
        self.code.str_imm(11, 31, slot_offset(dst))
    }

    fn emit_fixed_math_intrinsic(
        &mut self,
        opcode: u16,
        dst: u32,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        match opcode {
            NATIVE_OPCODE_MATH_FLOOR
            | NATIVE_OPCODE_MATH_CEIL
            | NATIVE_OPCODE_MATH_ROUND
            | NATIVE_OPCODE_MATH_TRUNC => {
                let src = operand(instruction, 1)?;
                self.emit_fixed_rounding(opcode, dst, src, epilogue)
            }
            NATIVE_OPCODE_MATH_RADIANS | NATIVE_OPCODE_MATH_DEGREES => {
                let src = operand(instruction, 1)?;
                self.load_fixed_as_double(0, src)?;
                if opcode == NATIVE_OPCODE_MATH_RADIANS {
                    self.emit_f64_const(1, std::f64::consts::PI / 180.0);
                } else {
                    self.emit_f64_const(1, 180.0 / std::f64::consts::PI);
                }
                self.code.fmul_d(0, 0, 1);
                self.store_double(dst, 0)
            }
            NATIVE_OPCODE_MATH_SQRT => {
                let src = operand(instruction, 1)?;
                self.load_fixed_as_double(0, src)?;
                self.emit_domain_nonnegative_d(0, epilogue);
                self.code.fsqrt_d(0, 0);
                self.store_double_as_fixed(dst, 0)
            }
            NATIVE_OPCODE_MATH_EXP => {
                let src = operand(instruction, 1)?;
                self.load_fixed_as_double(0, src)?;
                self.emit_exp_d(0, epilogue);
                self.store_double_as_fixed(dst, 0)
            }
            NATIVE_OPCODE_MATH_LOG | NATIVE_OPCODE_MATH_LOG10 => {
                let src = operand(instruction, 1)?;
                self.load_fixed_as_double(0, src)?;
                self.emit_log_d(0, epilogue);
                if opcode == NATIVE_OPCODE_MATH_LOG10 {
                    self.emit_f64_const(1, std::f64::consts::LN_10);
                    self.code.fdiv_d(0, 0, 1);
                }
                self.store_double_as_fixed(dst, 0)
            }
            NATIVE_OPCODE_MATH_SIN | NATIVE_OPCODE_MATH_COS | NATIVE_OPCODE_MATH_TAN => {
                let src = operand(instruction, 1)?;
                self.load_fixed_as_double(0, src)?;
                match opcode {
                    NATIVE_OPCODE_MATH_SIN => self.emit_sin_d(0),
                    NATIVE_OPCODE_MATH_COS => self.emit_cos_d(0),
                    NATIVE_OPCODE_MATH_TAN => self.emit_tan_d(0, epilogue),
                    _ => unreachable!(),
                }
                self.store_double_as_fixed(dst, 0)
            }
            NATIVE_OPCODE_MATH_ASIN | NATIVE_OPCODE_MATH_ACOS | NATIVE_OPCODE_MATH_ATAN => {
                let src = operand(instruction, 1)?;
                self.load_fixed_as_double(0, src)?;
                match opcode {
                    NATIVE_OPCODE_MATH_ASIN => self.emit_asin_d(0, epilogue),
                    NATIVE_OPCODE_MATH_ACOS => self.emit_acos_d(0, epilogue),
                    NATIVE_OPCODE_MATH_ATAN => self.emit_atan_d(0),
                    _ => unreachable!(),
                }
                self.store_double_as_fixed(dst, 0)
            }
            NATIVE_OPCODE_MATH_ATAN2 => {
                let y = operand(instruction, 1)?;
                let x = operand(instruction, 2)?;
                self.load_fixed_as_double(0, y)?;
                self.load_fixed_as_double(1, x)?;
                self.emit_atan2_d(0, 1, epilogue);
                self.store_double_as_fixed(dst, 0)
            }
            NATIVE_OPCODE_MATH_POW => {
                let base = operand(instruction, 1)?;
                let exponent = operand(instruction, 2)?;
                self.load_fixed_as_double(0, base)?;
                self.load_fixed_as_double(1, exponent)?;
                self.emit_pow_d(0, 1, epilogue);
                self.store_double_as_fixed(dst, 0)
            }
            _ => Err(format!(
                "native fixed-point lowering does not support math opcode {opcode} yet"
            )),
        }
    }

    fn load_fixed_as_double(&mut self, dd: u8, src: u32) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.scvtf_d_from_x(dd, 9);
        self.emit_f64_const(7, 4_294_967_296.0);
        self.code.fdiv_d(dd, dd, 7);
        Ok(())
    }

    fn load_numeric_as_double(
        &mut self,
        dd: u8,
        src: u32,
        src_type: NativeType,
    ) -> Result<(), String> {
        match src_type {
            NativeType::Float => self.load_float_as_double(dd, src),
            NativeType::Fixed => self.load_fixed_as_double(dd, src),
            NativeType::Byte | NativeType::Integer => {
                self.code.ldr_imm(9, 31, slot_offset(src))?;
                self.code.scvtf_d_from_x(dd, 9);
                Ok(())
            }
            _ => Err(format!(
                "numeric Float promotion cannot load {}",
                native_type_name(src_type)
            )),
        }
    }

    fn store_double(&mut self, dst: u32, dn: u8) -> Result<(), String> {
        self.code.fmov_x_from_d(9, dn);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn store_double_as_fixed(&mut self, dst: u32, dn: u8) -> Result<(), String> {
        self.emit_f64_const(7, 4_294_967_296.0);
        self.code.fmul_d(6, dn, 7);
        self.code.fcvtzs_x_from_d(9, 6);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_f64_const(&mut self, dd: u8, value: f64) {
        self.code.mov_imm(10, value.to_bits());
        self.code.fmov_d_from_x(dd, 10);
    }

    fn emit_domain_positive_d(&mut self, dn: u8, epilogue: Label) {
        let ok = self.code.new_label();
        self.code.fcmp_zero_d(dn);
        self.code.b_gt(ok);
        self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(ok);
    }

    fn emit_domain_nonnegative_d(&mut self, dn: u8, epilogue: Label) {
        let ok = self.code.new_label();
        self.code.fcmp_zero_d(dn);
        self.code.b_ge(ok);
        self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(ok);
    }

    fn emit_domain_abs_le_one_d(&mut self, dn: u8, epilogue: Label) {
        let ok = self.code.new_label();
        self.code.fabs_d(6, dn);
        self.emit_f64_const(7, 1.0);
        self.code.fcmp_d(6, 7);
        self.code.b_le(ok);
        self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(ok);
    }

    fn emit_exp_d(&mut self, dn: u8, epilogue: Label) {
        let not_overflow = self.code.new_label();
        let not_underflow = self.code.new_label();
        let compute = self.code.new_label();
        let scale_positive = self.code.new_label();
        let scale_negative = self.code.new_label();
        let scale_done = self.code.new_label();
        let pos_loop = self.code.new_label();
        let neg_loop = self.code.new_label();

        self.emit_f64_const(7, 21.487562596892644);
        self.code.fcmp_d(dn, 7);
        self.code.b_le(not_overflow);
        self.fail_current_function(ERR_OVERFLOW, epilogue);
        self.code.bind(not_overflow);
        self.emit_f64_const(7, -64.0);
        self.code.fcmp_d(dn, 7);
        self.code.b_ge(not_underflow);
        self.emit_f64_const(dn, 0.0);
        self.code.b(scale_done);
        self.code.bind(not_underflow);

        self.emit_f64_const(7, std::f64::consts::LOG2_E);
        self.code.fmul_d(1, dn, 7);
        self.code.fcmp_zero_d(1);
        let k_negative = self.code.new_label();
        let k_ready = self.code.new_label();
        self.code.b_lt(k_negative);
        self.emit_f64_const(7, 0.5);
        self.code.fadd_d(1, 1, 7);
        self.code.b(k_ready);
        self.code.bind(k_negative);
        self.emit_f64_const(7, 0.5);
        self.code.fsub_d(1, 1, 7);
        self.code.bind(k_ready);
        self.code.fcvtzs_x_from_d(11, 1);
        self.code.scvtf_d_from_x(1, 11);
        self.emit_f64_const(7, std::f64::consts::LN_2);
        self.code.fmul_d(1, 1, 7);
        self.code.fsub_d(2, dn, 1);

        self.emit_f64_const(0, 1.0);
        self.emit_f64_const(1, 1.0);
        for i in 1..=18 {
            self.code.fmul_d(1, 1, 2);
            self.emit_f64_const(7, i as f64);
            self.code.fdiv_d(1, 1, 7);
            self.code.fadd_d(0, 0, 1);
        }

        self.code.cmp_zero(11);
        self.code.b_gt(scale_positive);
        self.code.b_lt(scale_negative);
        self.code.b(scale_done);

        self.code.bind(scale_positive);
        self.emit_f64_const(7, 2.0);
        self.code.bind(pos_loop);
        self.code.cbz(11, scale_done);
        self.code.fmul_d(0, 0, 7);
        self.code.sub_imm(11, 11, 1).expect("literal 1 fits imm12");
        self.code.b(pos_loop);

        self.code.bind(scale_negative);
        self.code.neg(11, 11);
        self.emit_f64_const(7, 2.0);
        self.code.bind(neg_loop);
        self.code.cbz(11, scale_done);
        self.code.fdiv_d(0, 0, 7);
        self.code.sub_imm(11, 11, 1).expect("literal 1 fits imm12");
        self.code.b(neg_loop);

        self.code.bind(compute);
        self.code.bind(scale_done);
    }

    fn emit_exp_float_d(&mut self, dn: u8, epilogue: Label) {
        let not_overflow = self.code.new_label();
        let not_underflow = self.code.new_label();
        let scale_positive = self.code.new_label();
        let scale_negative = self.code.new_label();
        let scale_done = self.code.new_label();
        let pos_loop = self.code.new_label();
        let neg_loop = self.code.new_label();

        self.emit_f64_const(7, 709.0);
        self.code.fcmp_d(dn, 7);
        self.code.b_le(not_overflow);
        self.fail_current_function(ERR_OVERFLOW, epilogue);
        self.code.bind(not_overflow);
        self.emit_f64_const(7, -745.0);
        self.code.fcmp_d(dn, 7);
        self.code.b_ge(not_underflow);
        self.emit_f64_const(dn, 0.0);
        self.code.b(scale_done);
        self.code.bind(not_underflow);

        self.emit_f64_const(7, std::f64::consts::LOG2_E);
        self.code.fmul_d(1, dn, 7);
        self.code.fcmp_zero_d(1);
        let k_negative = self.code.new_label();
        let k_ready = self.code.new_label();
        self.code.b_lt(k_negative);
        self.emit_f64_const(7, 0.5);
        self.code.fadd_d(1, 1, 7);
        self.code.b(k_ready);
        self.code.bind(k_negative);
        self.emit_f64_const(7, 0.5);
        self.code.fsub_d(1, 1, 7);
        self.code.bind(k_ready);
        self.code.fcvtzs_x_from_d(11, 1);
        self.code.scvtf_d_from_x(1, 11);
        self.emit_f64_const(7, std::f64::consts::LN_2);
        self.code.fmul_d(1, 1, 7);
        self.code.fsub_d(2, dn, 1);

        self.emit_f64_const(0, 1.0);
        self.emit_f64_const(1, 1.0);
        for i in 1..=18 {
            self.code.fmul_d(1, 1, 2);
            self.emit_f64_const(7, i as f64);
            self.code.fdiv_d(1, 1, 7);
            self.code.fadd_d(0, 0, 1);
        }

        self.code.cmp_zero(11);
        self.code.b_gt(scale_positive);
        self.code.b_lt(scale_negative);
        self.code.b(scale_done);

        self.code.bind(scale_positive);
        self.emit_f64_const(7, 2.0);
        self.code.bind(pos_loop);
        self.code.cbz(11, scale_done);
        self.code.fmul_d(0, 0, 7);
        self.code.sub_imm(11, 11, 1).expect("literal 1 fits imm12");
        self.code.b(pos_loop);

        self.code.bind(scale_negative);
        self.code.neg(11, 11);
        self.emit_f64_const(7, 2.0);
        self.code.bind(neg_loop);
        self.code.cbz(11, scale_done);
        self.code.fdiv_d(0, 0, 7);
        self.code.sub_imm(11, 11, 1).expect("literal 1 fits imm12");
        self.code.b(neg_loop);

        self.code.bind(scale_done);
    }

    fn emit_log_d(&mut self, dn: u8, epilogue: Label) {
        self.emit_domain_positive_d(dn, epilogue);
        self.code.mov_imm(11, 0);
        self.emit_f64_const(7, std::f64::consts::SQRT_2);
        let high_loop = self.code.new_label();
        let low_check = self.code.new_label();
        let low_loop = self.code.new_label();
        let reduced = self.code.new_label();

        self.code.bind(high_loop);
        self.code.fcmp_d(dn, 7);
        self.code.b_le(low_check);
        self.emit_f64_const(6, 2.0);
        self.code.fdiv_d(dn, dn, 6);
        self.code.add_imm(11, 11, 1).expect("literal 1 fits imm12");
        self.code.b(high_loop);

        self.code.bind(low_check);
        self.emit_f64_const(7, std::f64::consts::FRAC_1_SQRT_2);
        self.code.bind(low_loop);
        self.code.fcmp_d(dn, 7);
        self.code.b_ge(reduced);
        self.emit_f64_const(6, 2.0);
        self.code.fmul_d(dn, dn, 6);
        self.code.sub_imm(11, 11, 1).expect("literal 1 fits imm12");
        self.code.b(low_loop);

        self.code.bind(reduced);
        self.emit_f64_const(6, 1.0);
        self.code.fsub_d(1, dn, 6);
        self.code.fadd_d(2, dn, 6);
        self.code.fdiv_d(1, 1, 2);
        self.code.fmul_d(2, 1, 1);
        self.code.fmov_d(3, 1);
        self.code.fmov_d(0, 1);
        for divisor in (3..=31).step_by(2) {
            self.code.fmul_d(3, 3, 2);
            self.emit_f64_const(7, divisor as f64);
            self.code.fdiv_d(4, 3, 7);
            self.code.fadd_d(0, 0, 4);
        }
        self.emit_f64_const(7, 2.0);
        self.code.fmul_d(0, 0, 7);
        self.code.scvtf_d_from_x(1, 11);
        self.emit_f64_const(7, std::f64::consts::LN_2);
        self.code.fmul_d(1, 1, 7);
        self.code.fadd_d(0, 0, 1);
    }

    fn emit_reduce_angle_d(&mut self, dn: u8) {
        self.emit_f64_const(7, 1.0 / std::f64::consts::TAU);
        self.code.fmul_d(1, dn, 7);
        self.code.fcmp_zero_d(1);
        let k_negative = self.code.new_label();
        let k_ready = self.code.new_label();
        self.code.b_lt(k_negative);
        self.emit_f64_const(7, 0.5);
        self.code.fadd_d(1, 1, 7);
        self.code.b(k_ready);
        self.code.bind(k_negative);
        self.emit_f64_const(7, 0.5);
        self.code.fsub_d(1, 1, 7);
        self.code.bind(k_ready);
        self.code.fcvtzs_x_from_d(11, 1);
        self.code.scvtf_d_from_x(1, 11);
        self.emit_f64_const(7, std::f64::consts::TAU);
        self.code.fmul_d(1, 1, 7);
        self.code.fsub_d(dn, dn, 1);
    }

    fn emit_sin_d(&mut self, dn: u8) {
        self.emit_reduce_angle_d(dn);
        self.code.fmul_d(1, dn, dn);
        self.code.fmov_d(2, dn);
        self.code.fmov_d(0, dn);
        let mut subtract = true;
        for divisor in [6.0, 20.0, 42.0, 72.0, 110.0, 156.0] {
            self.code.fmul_d(2, 2, 1);
            self.emit_f64_const(7, divisor);
            self.code.fdiv_d(2, 2, 7);
            if subtract {
                self.code.fsub_d(0, 0, 2);
            } else {
                self.code.fadd_d(0, 0, 2);
            }
            subtract = !subtract;
        }
    }

    fn emit_cos_d(&mut self, dn: u8) {
        self.emit_reduce_angle_d(dn);
        self.code.fmul_d(1, dn, dn);
        self.emit_f64_const(2, 1.0);
        self.emit_f64_const(0, 1.0);
        let mut subtract = true;
        for divisor in [2.0, 12.0, 30.0, 56.0, 90.0, 132.0] {
            self.code.fmul_d(2, 2, 1);
            self.emit_f64_const(7, divisor);
            self.code.fdiv_d(2, 2, 7);
            if subtract {
                self.code.fsub_d(0, 0, 2);
            } else {
                self.code.fadd_d(0, 0, 2);
            }
            subtract = !subtract;
        }
    }

    fn emit_tan_d(&mut self, dn: u8, epilogue: Label) {
        self.code.fmov_d(5, dn);
        self.emit_sin_d(dn);
        self.code.fmov_d(4, 0);
        self.code.fmov_d(dn, 5);
        self.emit_cos_d(dn);
        self.code.fabs_d(6, 0);
        self.emit_f64_const(7, 1.0e-12);
        self.code.fcmp_d(6, 7);
        let ok = self.code.new_label();
        self.code.b_gt(ok);
        self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(ok);
        self.code.fdiv_d(0, 4, 0);
    }

    fn emit_atan_poly_d(&mut self, dn: u8) {
        self.code.fmul_d(1, dn, dn);
        self.code.fmov_d(2, dn);
        self.code.fmov_d(0, dn);
        let mut subtract = true;
        for divisor in (3..=31).step_by(2) {
            self.code.fmul_d(2, 2, 1);
            self.emit_f64_const(7, divisor as f64);
            self.code.fdiv_d(3, 2, 7);
            if subtract {
                self.code.fsub_d(0, 0, 3);
            } else {
                self.code.fadd_d(0, 0, 3);
            }
            subtract = !subtract;
        }
    }

    fn emit_atan_d(&mut self, dn: u8) {
        let negative = self.code.new_label();
        let abs_ready = self.code.new_label();
        let small = self.code.new_label();
        let medium = self.code.new_label();
        let restore = self.code.new_label();
        self.code.mov_imm(12, 0);
        self.code.fcmp_zero_d(dn);
        self.code.b_lt(negative);
        self.code.b(abs_ready);
        self.code.bind(negative);
        self.code.mov_imm(12, 1);
        self.code.fneg_d(dn, dn);
        self.code.bind(abs_ready);

        self.emit_f64_const(7, std::f64::consts::SQRT_2 - 1.0);
        self.code.fcmp_d(dn, 7);
        self.code.b_le(small);
        self.emit_f64_const(7, 1.0);
        self.code.fcmp_d(dn, 7);
        self.code.b_le(medium);

        self.emit_f64_const(6, 1.0);
        self.code.fdiv_d(dn, 6, dn);
        self.emit_atan_poly_d(dn);
        self.emit_f64_const(7, std::f64::consts::FRAC_PI_2);
        self.code.fsub_d(0, 7, 0);
        self.code.b(restore);

        self.code.bind(medium);
        self.emit_f64_const(6, 1.0);
        self.code.fsub_d(1, dn, 6);
        self.code.fadd_d(2, dn, 6);
        self.code.fdiv_d(dn, 1, 2);
        self.emit_atan_poly_d(dn);
        self.emit_f64_const(7, std::f64::consts::FRAC_PI_4);
        self.code.fadd_d(0, 0, 7);
        self.code.b(restore);

        self.code.bind(small);
        self.emit_atan_poly_d(dn);

        self.code.bind(restore);
        let done = self.code.new_label();
        self.code.cbz(12, done);
        self.code.fneg_d(0, 0);
        self.code.bind(done);
    }

    fn emit_atan2_d(&mut self, y: u8, x: u8, epilogue: Label) {
        let x_negative = self.code.new_label();
        let x_zero = self.code.new_label();
        let y_negative_for_x_neg = self.code.new_label();
        let y_positive_axis = self.code.new_label();
        let y_negative_axis = self.code.new_label();
        let done = self.code.new_label();

        self.code.fcmp_zero_d(x);
        self.code.b_lt(x_negative);
        self.code.b_eq(x_zero);
        self.code.fdiv_d(0, y, x);
        self.emit_atan_d(0);
        self.code.b(done);

        self.code.bind(x_negative);
        self.code.fdiv_d(0, y, x);
        self.emit_atan_d(0);
        self.code.fcmp_zero_d(y);
        self.code.b_lt(y_negative_for_x_neg);
        self.emit_f64_const(7, std::f64::consts::PI);
        self.code.fadd_d(0, 0, 7);
        self.code.b(done);
        self.code.bind(y_negative_for_x_neg);
        self.emit_f64_const(7, std::f64::consts::PI);
        self.code.fsub_d(0, 0, 7);
        self.code.b(done);

        self.code.bind(x_zero);
        self.code.fcmp_zero_d(y);
        self.code.b_gt(y_positive_axis);
        self.code.b_lt(y_negative_axis);
        self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(y_positive_axis);
        self.emit_f64_const(0, std::f64::consts::FRAC_PI_2);
        self.code.b(done);
        self.code.bind(y_negative_axis);
        self.emit_f64_const(0, -std::f64::consts::FRAC_PI_2);
        self.code.bind(done);
    }

    fn emit_asin_d(&mut self, dn: u8, epilogue: Label) {
        self.emit_domain_abs_le_one_d(dn, epilogue);
        self.code.fmov_d(5, dn);
        self.emit_f64_const(1, 1.0);
        self.code.fmul_d(2, dn, dn);
        self.code.fsub_d(1, 1, 2);
        self.code.fsqrt_d(1, 1);
        self.code.fmov_d(0, 5);
        self.emit_atan2_d(0, 1, epilogue);
    }

    fn emit_acos_d(&mut self, dn: u8, epilogue: Label) {
        self.emit_asin_d(dn, epilogue);
        self.emit_f64_const(7, std::f64::consts::FRAC_PI_2);
        self.code.fsub_d(0, 7, 0);
    }

    fn emit_pow_d(&mut self, base: u8, exponent: u8, epilogue: Label) {
        self.emit_domain_positive_d(base, epilogue);
        self.code.fmov_d(5, exponent);
        self.emit_log_d(base, epilogue);
        self.code.fmul_d(0, 0, 5);
        self.emit_exp_d(0, epilogue);
    }

    fn emit_pow_float_d(&mut self, base: u8, exponent: u8, epilogue: Label) {
        self.emit_domain_positive_d(base, epilogue);
        self.code.fmov_d(5, exponent);
        self.emit_log_d(base, epilogue);
        self.code.fmul_d(0, 0, 5);
        self.emit_exp_float_d(0, epilogue);
    }

    fn emit_general_numeric_predicate(
        &mut self,
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("numeric predicate references missing register {src}"))?;
        let false_label = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(src))?;
        if matches!(src_type, NativeType::Float | NativeType::Fixed) {
            self.code.mov_imm(10, 0x8000_0000_0000_0000);
            self.code.and_reg(11, 9, 10);
            self.code.mov_imm(12, 0x7fff_ffff_ffff_ffff);
            self.code.and_reg(13, 9, 12);
            match opcode {
                NATIVE_OPCODE_GENERAL_IS_POSITIVE => {
                    self.code.cbnz(11, false_label);
                    self.code.cbz(13, false_label);
                }
                NATIVE_OPCODE_GENERAL_IS_NEGATIVE => {
                    self.code.cbz(11, false_label);
                    self.code.cbz(13, false_label);
                }
                NATIVE_OPCODE_GENERAL_IS_ZERO => {
                    self.code.cbnz(13, false_label);
                }
                _ => unreachable!(),
            }
        } else {
            self.code.cmp_zero(9);
            match opcode {
                NATIVE_OPCODE_GENERAL_IS_POSITIVE => self.code.b_le(false_label),
                NATIVE_OPCODE_GENERAL_IS_NEGATIVE => self.code.b_ge(false_label),
                NATIVE_OPCODE_GENERAL_IS_ZERO => self.code.b_ne(false_label),
                _ => unreachable!(),
            }
        }
        self.code.mov_imm(10, 1);
        self.code.b(done);
        self.code.bind(false_label);
        self.code.mov_imm(10, 0);
        self.code.bind(done);
        self.code.str_imm(10, 31, slot_offset(dst))
    }

    fn emit_general_length_predicate(
        &mut self,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let false_label = self.code.new_label();
        let done = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(src) + 8)?;
        self.code.cmp_zero(9);
        if opcode == NATIVE_OPCODE_GENERAL_IS_EMPTY {
            self.code.b_ne(false_label);
        } else {
            self.code.b_eq(false_label);
        }
        self.code.mov_imm(10, 1);
        self.code.b(done);
        self.code.bind(false_label);
        self.code.mov_imm(10, 0);
        self.code.bind(done);
        self.code.str_imm(10, 31, slot_offset(dst))
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

    fn emit_allocate_string_from_len_reg(
        &mut self,
        len_reg: u8,
        epilogue: Label,
    ) -> Result<(), String> {
        let scratch = self.current_scratch + 56;
        let alloc_size_ok = self.code.new_label();
        let alloc_ok = self.code.new_label();
        self.code.str_imm(len_reg, 31, scratch)?;
        self.code.add_imm(0, len_reg, HEAP_HEADER_SIZE)?;
        self.code.cmp_reg(0, len_reg);
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
        self.code.ldr_imm(10, 31, scratch)?;
        self.write_heap_header_from_regs(1, HEAP_KIND_STRING, 10, 10, 0)
    }

    fn write_heap_header_from_regs(
        &mut self,
        object_reg: u8,
        kind: u64,
        length_reg: u8,
        capacity_reg: u8,
        aux: u64,
    ) -> Result<(), String> {
        self.code.mov_imm(16, kind);
        self.code.str_imm(16, object_reg, 0)?;
        self.code.str_imm(length_reg, object_reg, 8)?;
        self.code.str_imm(capacity_reg, object_reg, 16)?;
        self.code.mov_imm(16, aux);
        self.code.str_imm(16, object_reg, 24)
    }

    fn store_string_slot(
        &mut self,
        dst: u32,
        object_reg: u8,
        len_scratch_offset: usize,
    ) -> Result<(), String> {
        self.code.ldr_imm(9, 31, len_scratch_offset)?;
        self.code.str_imm(object_reg, 31, slot_offset(dst))?;
        self.code.str_imm(9, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn emit_allocate_string_buffer(
        &mut self,
        capacity: u64,
        epilogue: Label,
    ) -> Result<(), String> {
        self.emit_allocate_heap_object(HEAP_KIND_STRING, 0, 0, capacity as usize, epilogue)
    }

    fn store_string_slot_from_heap(
        &mut self,
        dst: u32,
        object_reg: u8,
        len_reg: u8,
        capacity: u64,
    ) -> Result<(), String> {
        self.code.mov_imm(9, HEAP_KIND_STRING);
        self.code.str_imm(9, object_reg, 0)?;
        self.code.str_imm(len_reg, object_reg, 8)?;
        self.code.mov_imm(9, capacity);
        self.code.str_imm(9, object_reg, 16)?;
        self.code.str_imm(31, object_reg, 24)?;
        self.code.str_imm(object_reg, 31, slot_offset(dst))?;
        self.code.str_imm(len_reg, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn fail_current_function(&mut self, code: u64, epilogue: Label) {
        self.code.mov_imm(0, code);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);
    }

    fn expect_register_type(
        &self,
        function: &bytecode::NativeFunction,
        register: u32,
        expected: NativeType,
        opcode: &str,
    ) -> Result<(), String> {
        let actual = function
            .registers
            .get(register as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("{opcode} references missing register {register}"))?;
        if actual != expected {
            return Err(format!(
                "{opcode} requires a {} register, got {}",
                native_type_name(expected),
                native_type_name(actual)
            ));
        }
        Ok(())
    }

    fn emit_bool_to_string(&mut self, dst: u32, src: u32, epilogue: Label) -> Result<(), String> {
        let false_label = self.code.new_label();
        let done = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.cbz(9, false_label);
        self.emit_ascii_string(dst, b"TRUE", epilogue)?;
        self.code.b(done);
        self.code.bind(false_label);
        self.emit_ascii_string(dst, b"FALSE", epilogue)?;
        self.code.bind(done);
        Ok(())
    }

    fn emit_ascii_string(&mut self, dst: u32, bytes: &[u8], epilogue: Label) -> Result<(), String> {
        let scratch = self.current_scratch + 48;
        self.code.mov_imm(12, bytes.len() as u64);
        self.emit_allocate_string_from_len_reg(12, epilogue)?;
        self.code.mov_imm(12, bytes.len() as u64);
        self.code.str_imm(12, 31, scratch)?;
        self.code.add_imm(13, 1, HEAP_HEADER_SIZE)?;
        for byte in bytes {
            self.code.mov_imm(14, *byte as u64);
            self.code.strb_imm(14, 13, 0);
            self.code.add_imm(13, 13, 1)?;
        }
        self.store_string_slot(dst, 1, scratch)
    }

    fn emit_integer_to_string(
        &mut self,
        dst: u32,
        src: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let zero = self.code.new_label();
        let nonnegative = self.code.new_label();
        let loop_start = self.code.new_label();
        let digits_done = self.code.new_label();
        let copy_loop = self.code.new_label();
        let sign_done = self.code.new_label();
        let copy_done = self.code.new_label();
        let scratch = self.current_scratch;
        let buffer = scratch + 80;

        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.mov_imm(20, 0);
        self.code.mov_imm(10, 0);
        self.code.cbz(9, zero);
        self.code.cmp_zero(9);
        self.code.b_ge(nonnegative);
        self.code.neg(9, 9);
        self.code.mov_imm(20, 1);
        self.code.bind(nonnegative);
        self.code.add_imm(11, 31, buffer + 31)?;
        self.code.mov_imm(14, 10);
        self.code.bind(loop_start);
        self.code.cbz(9, digits_done);
        self.code.sdiv(12, 9, 14);
        self.code.msub(13, 12, 14, 9);
        self.code.add_imm(13, 13, b'0' as usize)?;
        self.code.strb_imm(13, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.add_imm(10, 10, 1)?;
        self.code.mov_reg(9, 12);
        self.code.b(loop_start);

        self.code.bind(zero);
        self.code.add_imm(11, 31, buffer + 31)?;
        self.code.mov_imm(13, b'0' as u64);
        self.code.strb_imm(13, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.mov_imm(10, 1);

        self.code.bind(digits_done);
        self.code.cbz(20, sign_done);
        self.code.mov_imm(13, b'-' as u64);
        self.code.strb_imm(13, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.add_imm(10, 10, 1)?;
        self.code.bind(sign_done);
        self.code.add_imm(11, 11, 1)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch)?;

        self.emit_allocate_string_from_len_reg(10, epilogue)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch)?;
        self.code.add_imm(12, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(copy_loop);
        self.code.cbz(10, copy_done);
        self.code.ldrb_imm(13, 11, 0);
        self.code.strb_imm(13, 12, 0);
        self.code.add_imm(11, 11, 1)?;
        self.code.add_imm(12, 12, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_done);
        self.store_string_slot(dst, 1, scratch + 8)
    }

    fn emit_fixed_to_string(&mut self, dst: u32, src: u32, epilogue: Label) -> Result<(), String> {
        let min_fixed = self.code.new_label();
        let nonnegative = self.code.new_label();
        let abs_done = self.code.new_label();
        let fraction_loop = self.code.new_label();
        let integer_zero = self.code.new_label();
        let integer_loop = self.code.new_label();
        let integer_done = self.code.new_label();
        let sign_done = self.code.new_label();
        let copy_loop = self.code.new_label();
        let copy_done = self.code.new_label();
        let scratch = self.current_scratch;
        let buffer = scratch + 80;

        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.mov_imm(10, i64::MIN as u64);
        self.code.cmp_reg(9, 10);
        self.code.b_eq(min_fixed);

        self.code.mov_imm(20, 0);
        self.code.cmp_zero(9);
        self.code.b_ge(nonnegative);
        self.code.neg(9, 9);
        self.code.mov_imm(20, 1);
        self.code.bind(nonnegative);
        self.code.bind(abs_done);

        self.code.lsr_imm(12, 9, 32);
        self.code.lsl_imm(13, 9, 32);
        self.code.lsr_imm(13, 13, 32);
        self.code.mov_imm(14, 1_000_000);
        self.code.mul(13, 13, 14);
        self.code.mov_imm(14, 0x8000_0000);
        self.code.add_reg(13, 13, 14);
        self.code.mov_imm(14, 0x1_0000_0000);
        self.code.sdiv(13, 13, 14);
        self.code.mov_imm(14, 1_000_000);
        self.code.cmp_reg(13, 14);
        let fraction_ok = self.code.new_label();
        self.code.b_ne(fraction_ok);
        self.code.mov_imm(13, 0);
        self.code.add_imm(12, 12, 1)?;
        self.code.bind(fraction_ok);

        self.code.add_imm(11, 31, buffer + 63)?;
        self.code.mov_imm(10, 0);
        self.code.mov_imm(15, 6);
        self.code.mov_imm(14, 10);
        self.code.bind(fraction_loop);
        self.code.sdiv(16, 13, 14);
        self.code.msub(17, 16, 14, 13);
        self.code.add_imm(17, 17, b'0' as usize)?;
        self.code.strb_imm(17, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.add_imm(10, 10, 1)?;
        self.code.mov_reg(13, 16);
        self.code.sub_imm(15, 15, 1)?;
        self.code.cbnz(15, fraction_loop);

        self.code.mov_imm(17, b'.' as u64);
        self.code.strb_imm(17, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.add_imm(10, 10, 1)?;

        self.code.cbz(12, integer_zero);
        self.code.mov_imm(14, 10);
        self.code.bind(integer_loop);
        self.code.cbz(12, integer_done);
        self.code.sdiv(13, 12, 14);
        self.code.msub(16, 13, 14, 12);
        self.code.add_imm(16, 16, b'0' as usize)?;
        self.code.strb_imm(16, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.add_imm(10, 10, 1)?;
        self.code.mov_reg(12, 13);
        self.code.b(integer_loop);

        self.code.bind(integer_zero);
        self.code.mov_imm(16, b'0' as u64);
        self.code.strb_imm(16, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.add_imm(10, 10, 1)?;

        self.code.bind(integer_done);
        self.code.cbz(20, sign_done);
        self.code.mov_imm(16, b'-' as u64);
        self.code.strb_imm(16, 11, 0);
        self.code.sub_imm(11, 11, 1)?;
        self.code.add_imm(10, 10, 1)?;
        self.code.bind(sign_done);
        self.code.add_imm(11, 11, 1)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.code.str_imm(11, 31, scratch)?;

        self.emit_allocate_string_from_len_reg(10, epilogue)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.ldr_imm(11, 31, scratch)?;
        self.code.add_imm(12, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(copy_loop);
        self.code.cbz(10, copy_done);
        self.code.ldrb_imm(13, 11, 0);
        self.code.strb_imm(13, 12, 0);
        self.code.add_imm(11, 11, 1)?;
        self.code.add_imm(12, 12, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_done);
        self.store_string_slot(dst, 1, scratch + 8)?;

        let done = self.code.new_label();
        self.code.b(done);
        self.code.bind(min_fixed);
        self.emit_ascii_string(dst, b"-2147483648.000000", epilogue)?;
        self.code.bind(done);
        Ok(())
    }

    fn emit_float_to_string(&mut self, dst: u32, src: u32, epilogue: Label) -> Result<(), String> {
        self.emit_float_to_fixed(dst, src)?;
        self.emit_fixed_to_string(dst, dst, epilogue)
    }

    fn emit_byte_list_to_string(
        &mut self,
        dst: u32,
        src: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let copy_loop = self.code.new_label();
        let copy_done = self.code.new_label();
        let scratch = self.current_scratch;
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.ldr_imm(10, 31, slot_offset(src) + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.str_imm(9, 31, scratch)?;
        self.code.str_imm(10, 31, scratch + 8)?;
        self.emit_allocate_string_from_len_reg(10, epilogue)?;
        self.code.ldr_imm(9, 31, scratch)?;
        self.code.ldr_imm(10, 31, scratch + 8)?;
        self.code.add_imm(11, 1, HEAP_HEADER_SIZE)?;
        self.code.bind(copy_loop);
        self.code.cbz(10, copy_done);
        self.code.ldr_imm(12, 9, 0)?;
        self.code.strb_imm(12, 11, 0);
        self.code.add_imm(9, 9, SLOT_SIZE)?;
        self.code.add_imm(11, 11, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_done);
        self.store_string_slot(dst, 1, scratch + 8)
    }

    fn emit_parse_integer_to_slot(
        &mut self,
        dst: u32,
        src: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let loop_start = self.code.new_label();
        let digit = self.code.new_label();
        let done = self.code.new_label();
        let parse_error = self.code.new_label();
        let after_sign = self.code.new_label();
        let end = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.ldr_imm(10, 31, slot_offset(src) + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.cbz(10, parse_error);
        self.code.mov_imm(11, 0);
        self.code.mov_imm(12, 0);
        self.code.ldrb_imm(13, 9, 0);
        self.code.cmp_reg_imm(13, b'-' as u64);
        self.code.b_ne(after_sign);
        self.code.mov_imm(12, 1);
        self.code.add_imm(9, 9, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.cbz(10, parse_error);
        self.code.bind(after_sign);

        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.code.ldrb_imm(13, 9, 0);
        self.code.cmp_reg_imm(13, b'0' as u64);
        self.code.b_hs(digit);
        self.code.b(parse_error);
        self.code.bind(digit);
        self.code.cmp_reg_imm(13, b'9' as u64);
        self.code.b_hi(parse_error);
        self.code.sub_imm(13, 13, b'0' as usize)?;
        self.code.mov_imm(14, 10);
        self.code.mul(11, 11, 14);
        self.code.add_reg(11, 11, 13);
        self.code.add_imm(9, 9, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);

        self.code.bind(done);
        let positive = self.code.new_label();
        self.code.cbz(12, positive);
        self.code.neg(11, 11);
        self.code.bind(positive);
        self.code.str_imm(11, 31, slot_offset(dst))?;
        self.code.b(end);

        self.code.bind(parse_error);
        self.emit_error(ERR_PARSE, epilogue);
        self.code.bind(end);
        Ok(())
    }

    fn emit_integer_to_fixed(&mut self, dst: u32, src: u32, epilogue: Label) -> Result<(), String> {
        let valid_low = self.code.new_label();
        let valid_high = self.code.new_label();
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.mov_imm(10, i32::MIN as i64 as u64);
        self.code.cmp_reg(9, 10);
        self.code.b_ge(valid_low);
        self.emit_error(ERR_OVERFLOW, epilogue);
        self.code.bind(valid_low);
        self.code.mov_imm(10, i32::MAX as u64);
        self.code.cmp_reg(9, 10);
        self.code.b_le(valid_high);
        self.emit_error(ERR_OVERFLOW, epilogue);
        self.code.bind(valid_high);
        self.code.lsl_imm(9, 9, 32);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_integer_slot_to_float(&mut self, dst: u32, src: u32) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.scvtf_d_from_x(0, 9);
        self.code.fmov_x_from_d(9, 0);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_fixed_to_float(&mut self, dst: u32, src: u32) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.scvtf_d_from_x(0, 9);
        self.code.mov_imm(10, 4_294_967_296.0_f64.to_bits());
        self.code.fmov_d_from_x(1, 10);
        self.code.fdiv_d(0, 0, 1);
        self.code.fmov_x_from_d(9, 0);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_float_to_integer(&mut self, dst: u32, src: u32) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.fmov_d_from_x(0, 9);
        self.code.fcvtzs_x_from_d(9, 0);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn emit_float_to_fixed(&mut self, dst: u32, src: u32) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.fmov_d_from_x(0, 9);
        self.code.mov_imm(10, 4_294_967_296.0_f64.to_bits());
        self.code.fmov_d_from_x(1, 10);
        self.code.fmul_d(0, 0, 1);
        self.code.fcvtzs_x_from_d(9, 0);
        self.code.str_imm(9, 31, slot_offset(dst))
    }

    fn load_float_as_double(&mut self, dd: u8, src: u32) -> Result<(), String> {
        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.fmov_d_from_x(dd, 9);
        Ok(())
    }

    fn emit_parse_fixed_to_slot(
        &mut self,
        dst: u32,
        src: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        let loop_start = self.code.new_label();
        let digit = self.code.new_label();
        let decimal_point = self.code.new_label();
        let after_sign = self.code.new_label();
        let parse_error = self.code.new_label();
        let done = self.code.new_label();
        let positive = self.code.new_label();
        let end = self.code.new_label();
        let decimal_digit = self.code.new_label();

        self.code.ldr_imm(9, 31, slot_offset(src))?;
        self.code.ldr_imm(10, 31, slot_offset(src) + 8)?;
        self.code.add_imm(9, 9, HEAP_HEADER_SIZE)?;
        self.code.cbz(10, parse_error);
        self.code.mov_imm(11, 0);
        self.code.mov_imm(12, 0);
        self.code.mov_imm(13, 0);
        self.code.mov_imm(14, 1);
        self.code.mov_imm(15, 0);
        self.code.mov_imm(20, 0);
        self.code.ldrb_imm(16, 9, 0);
        self.code.cmp_reg_imm(16, b'-' as u64);
        self.code.b_ne(after_sign);
        self.code.mov_imm(12, 1);
        self.code.add_imm(9, 9, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.cbz(10, parse_error);
        self.code.bind(after_sign);

        self.code.bind(loop_start);
        self.code.cbz(10, done);
        self.code.ldrb_imm(16, 9, 0);
        self.code.cmp_reg_imm(16, b'.' as u64);
        self.code.b_eq(decimal_point);
        self.code.cmp_reg_imm(16, b'0' as u64);
        self.code.b_hs(digit);
        self.code.b(parse_error);

        self.code.bind(decimal_point);
        self.code.cbnz(15, parse_error);
        self.code.mov_imm(15, 1);
        self.code.add_imm(9, 9, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);

        self.code.bind(digit);
        self.code.cmp_reg_imm(16, b'9' as u64);
        self.code.b_hi(parse_error);
        self.code.sub_imm(16, 16, b'0' as usize)?;
        self.code.mov_imm(20, 1);
        self.code.cbnz(15, decimal_digit);
        let after_digit = self.code.new_label();
        self.code.mov_imm(17, 10);
        self.code.mul(11, 11, 17);
        self.code.add_reg(11, 11, 16);
        self.code.b(after_digit);
        self.code.bind(decimal_digit);
        self.code.mov_imm(17, 1_000_000);
        self.code.cmp_reg(14, 17);
        let skip_fraction = self.code.new_label();
        self.code.b_ge(skip_fraction);
        self.code.mov_imm(17, 10);
        self.code.mul(13, 13, 17);
        self.code.add_reg(13, 13, 16);
        self.code.mul(14, 14, 17);
        self.code.bind(skip_fraction);
        self.code.bind(after_digit);
        self.code.add_imm(9, 9, 1)?;
        self.code.sub_imm(10, 10, 1)?;
        self.code.b(loop_start);

        self.code.bind(done);
        self.code.cbz(20, parse_error);
        self.code.lsl_imm(11, 11, 32);
        self.code.cbz(13, positive);
        self.code.mov_imm(16, 0x1_0000_0000);
        self.code.mul(13, 13, 16);
        self.code.sdiv(13, 13, 14);
        self.code.add_reg(11, 11, 13);
        self.code.bind(positive);
        self.code.cbz(12, end);
        self.code.neg(11, 11);
        self.code.bind(end);
        self.code.str_imm(11, 31, slot_offset(dst))?;
        let after_success = self.code.new_label();
        self.code.b(after_success);

        self.code.bind(parse_error);
        self.emit_error(ERR_PARSE, epilogue);
        self.code.bind(after_success);
        Ok(())
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

    fn emit_numeric_arithmetic(
        &mut self,
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let left = operand(instruction, 1)?;
        let right = operand(instruction, 2)?;
        let dst_type = function
            .registers
            .get(dst as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("arithmetic references missing register {dst}"))?;
        let left_type = function
            .registers
            .get(left as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("arithmetic references missing register {left}"))?;
        let right_type = function
            .registers
            .get(right as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("arithmetic references missing register {right}"))?;
        if dst_type == NativeType::Fixed
            && !matches!(
                opcode,
                NATIVE_OPCODE_ADD | NATIVE_OPCODE_SUB | NATIVE_OPCODE_MUL | NATIVE_OPCODE_DIV
            )
        {
            return Err(format!(
                "native fixed-point lowering does not support opcode {opcode} yet"
            ));
        }
        if dst_type == NativeType::Fixed {
            self.code.ldr_imm(9, 31, slot_offset(left))?;
            self.code.ldr_imm(10, 31, slot_offset(right))?;
            return self.emit_fixed_arithmetic(opcode, dst, epilogue);
        }
        if dst_type == NativeType::Float {
            return self
                .emit_float_arithmetic(opcode, dst, left, left_type, right, right_type, epilogue);
        }
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

    fn emit_float_arithmetic(
        &mut self,
        opcode: u16,
        dst: u32,
        left: u32,
        left_type: NativeType,
        right: u32,
        right_type: NativeType,
        epilogue: Label,
    ) -> Result<(), String> {
        self.load_numeric_as_double(0, left, left_type)?;
        self.load_numeric_as_double(1, right, right_type)?;
        match opcode {
            NATIVE_OPCODE_ADD => self.code.fadd_d(0, 0, 1),
            NATIVE_OPCODE_SUB => self.code.fsub_d(0, 0, 1),
            NATIVE_OPCODE_MUL => self.code.fmul_d(0, 0, 1),
            NATIVE_OPCODE_DIV => {
                let nonzero = self.code.new_label();
                self.code.fcmp_zero_d(1);
                self.code.b_ne(nonzero);
                self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
                self.code.bind(nonzero);
                self.code.fdiv_d(0, 0, 1);
            }
            NATIVE_OPCODE_POW => self.emit_pow_float_d(0, 1, epilogue),
            _ => {
                return Err(format!(
                    "native float arithmetic does not support opcode {opcode}"
                ));
            }
        }
        self.store_double(dst, 0)
    }

    fn emit_fixed_arithmetic(
        &mut self,
        opcode: u16,
        dst: u32,
        epilogue: Label,
    ) -> Result<(), String> {
        match opcode {
            NATIVE_OPCODE_ADD => {
                let ok = self.code.new_label();
                self.code.adds_reg(11, 9, 10);
                self.code.b_vc(ok);
                self.fail_current_function(ERR_OVERFLOW, epilogue);
                self.code.bind(ok);
            }
            NATIVE_OPCODE_SUB => {
                let ok = self.code.new_label();
                self.code.subs_reg(11, 9, 10);
                self.code.b_vc(ok);
                self.fail_current_function(ERR_OVERFLOW, epilogue);
                self.code.bind(ok);
            }
            NATIVE_OPCODE_MUL => {
                self.code.mul(11, 9, 10);
                self.code.smulh(12, 9, 10);
                self.code.lsr_imm(11, 11, 32);
                self.code.lsl_imm(12, 12, 32);
                self.code.orr_reg(11, 12, 11);
                self.code.asr_imm(13, 12, 31);
                let ok_zero = self.code.new_label();
                let ok = self.code.new_label();
                self.code.cbz(13, ok_zero);
                self.code.mov_imm(14, (-1_i64) as u64);
                self.code.cmp_reg(13, 14);
                self.code.b_eq(ok);
                self.fail_current_function(ERR_OVERFLOW, epilogue);
                self.code.bind(ok_zero);
                self.code.bind(ok);
            }
            NATIVE_OPCODE_DIV => self.emit_fixed_div(epilogue),
            _ => unreachable!(),
        }
        self.code.str_imm(11, 31, slot_offset(dst))
    }

    fn emit_fixed_div(&mut self, epilogue: Label) {
        self.emit_nonzero_or_error(10, epilogue);
        self.code.eor(15, 9, 10);
        self.code.cmp_zero(9);
        let left_positive = self.code.new_label();
        self.code.b_ge(left_positive);
        self.code.neg(9, 9);
        self.code.bind(left_positive);
        self.code.cmp_zero(10);
        let right_positive = self.code.new_label();
        self.code.b_ge(right_positive);
        self.code.neg(10, 10);
        self.code.bind(right_positive);

        self.code.lsr_imm(12, 9, 32);
        self.code.lsl_imm(13, 9, 32);
        self.code.mov_imm(11, 0);
        self.code.mov_imm(14, 0);
        self.code.mov_imm(16, 64);
        let loop_start = self.code.new_label();
        let no_bit = self.code.new_label();
        let skip_subtract = self.code.new_label();
        let done = self.code.new_label();

        self.code.bind(loop_start);
        self.code.cbz(16, done);
        self.code.lsl_imm(14, 14, 1);
        self.code.lsr_imm(17, 12, 63);
        self.code.cbz(17, no_bit);
        self.code.mov_imm(17, 1);
        self.code.orr_reg(14, 14, 17);
        self.code.bind(no_bit);
        self.code.lsl_imm(12, 12, 1);
        self.code.lsr_imm(17, 13, 63);
        self.code.orr_reg(12, 12, 17);
        self.code.lsl_imm(13, 13, 1);
        self.code.lsl_imm(11, 11, 1);
        self.code.cmp_reg(14, 10);
        self.code.b_lo(skip_subtract);
        self.code.sub_reg(14, 14, 10);
        self.code.mov_imm(17, 1);
        self.code.orr_reg(11, 11, 17);
        self.code.bind(skip_subtract);
        self.code.sub_imm(16, 16, 1).expect("literal 1 fits imm12");
        self.code.b(loop_start);

        self.code.bind(done);
        self.code.cmp_zero(15);
        let quotient_positive = self.code.new_label();
        let quotient_done = self.code.new_label();
        self.code.b_lt(quotient_positive);
        self.code.cmp_zero(11);
        self.code.b_ge(quotient_done);
        self.fail_current_function(ERR_OVERFLOW, epilogue);
        self.code.bind(quotient_positive);
        self.code.neg(11, 11);
        self.code.bind(quotient_done);
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
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let left = operand(instruction, 1)?;
        let right = operand(instruction, 2)?;
        let left_type = function
            .registers
            .get(left as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("comparison references missing register {left}"))?;
        let right_type = function
            .registers
            .get(right as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("comparison references missing register {right}"))?;
        let false_label = self.code.new_label();
        let done = self.code.new_label();
        if promoted_numeric_comparison_uses_double(left_type, right_type) {
            self.load_numeric_as_double(0, left, left_type)?;
            self.load_numeric_as_double(1, right, right_type)?;
            self.code.fcmp_d(0, 1);
        } else {
            self.code.ldr_imm(9, 31, slot_offset(left))?;
            self.code.ldr_imm(10, 31, slot_offset(right))?;
            self.code.cmp_reg(9, 10);
        }
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
        function: &bytecode::NativeFunction,
        opcode: u16,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let operand = operand(instruction, 1)?;
        let dst_type = function
            .registers
            .get(dst as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("unary references missing register {dst}"))?;
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
            NATIVE_OPCODE_NEG if dst_type == NativeType::Float => {
                self.code.fmov_d_from_x(0, 9);
                self.code.fneg_d(0, 0);
                self.code.fmov_x_from_d(10, 0);
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

    fn emit_ascii_whitespace_flag(&mut self, byte: u8, dst: u8) -> Result<(), String> {
        let true_label = self.code.new_label();
        let done = self.code.new_label();
        self.code.cmp_reg_imm(byte, b' ' as u64);
        self.code.b_eq(true_label);
        self.code.cmp_reg_imm(byte, b'\t' as u64);
        self.code.b_eq(true_label);
        self.code.cmp_reg_imm(byte, b'\n' as u64);
        self.code.b_eq(true_label);
        self.code.cmp_reg_imm(byte, b'\r' as u64);
        self.code.b_eq(true_label);
        self.code.cmp_reg_imm(byte, 0x0b);
        self.code.b_eq(true_label);
        self.code.cmp_reg_imm(byte, 0x0c);
        self.code.b_eq(true_label);
        self.code.mov_imm(dst, 0);
        self.code.b(done);
        self.code.bind(true_label);
        self.code.mov_imm(dst, 1);
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

    fn emit_call_value_result(
        &mut self,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let function_value = operand(instruction, 1)?;
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

        self.code.ldr_imm(20, 31, slot_offset(function_value))?;
        let done = self.code.new_label();
        let mut labels = Vec::new();
        for index in 0..self.program.functions.len() {
            labels.push(self.code.new_label());
            self.code.mov_imm(21, index as u64);
            self.code.cmp_reg(20, 21);
            self.code.b_eq(labels[index]);
        }
        self.code.mov_imm(0, ERR_INVALID_ARGUMENT);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(done);
        for (index, label) in labels.into_iter().enumerate() {
            self.code.bind(label);
            self.code.bl(self.function_label(index as u32)?);
            self.code.b(done);
        }
        self.code.bind(done);
        self.store_result(dst)
    }

    fn emit_io_write(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let src = operand(instruction, 1)?;
        let fd = operand(instruction, 2)? as u64;
        let append_newline = operand(instruction, 3)? != 0;
        let src_type = function
            .registers
            .get(src as usize)
            .map(|register| register.type_)
            .ok_or_else(|| format!("IO_WRITE references missing register {src}"))?;
        if src_type != NativeType::String {
            return Err(format!(
                "IO_WRITE requires a String register, got {}",
                native_type_name(src_type)
            ));
        }

        self.code.ldr_imm(1, 31, slot_offset(src))?;
        self.code.ldr_imm(2, 31, slot_offset(src) + 8)?;
        self.code.add_imm(1, 1, HEAP_HEADER_SIZE)?;
        self.emit_write_buffer(fd, epilogue)?;
        if append_newline {
            self.emit_newline_write(fd, epilogue)?;
        }
        self.store_zero_slot(dst)
    }

    fn emit_newline_write(&mut self, fd: u64, epilogue: Label) -> Result<(), String> {
        self.emit_data_addr(1, self.data.newline);
        self.code.mov_imm(2, 1);
        self.emit_write_buffer(fd, epilogue)
    }

    fn emit_write_buffer(&mut self, fd: u64, epilogue: Label) -> Result<(), String> {
        let ok = self.code.new_label();
        self.code.mov_imm(0, fd);
        self.code.mov_imm(16, DARWIN_SYSCALL_WRITE);
        self.code.svc();
        self.code.cmp_zero(0);
        self.code.b_ge(ok);
        self.fail_current_function(ERR_OUTPUT_FAILURE, epilogue);
        self.code.bind(ok);
        Ok(())
    }

    fn emit_io_flush(&mut self, instruction: &bytecode::NativeInstruction) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        self.store_zero_slot(dst)
    }

    fn emit_io_read_line(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let prompt = operand(instruction, 1)?;
        if prompt != u32::MAX {
            let prompt_type = function
                .registers
                .get(prompt as usize)
                .map(|register| register.type_)
                .ok_or_else(|| format!("IO_READ_LINE references missing prompt {prompt}"))?;
            if prompt_type != NativeType::String {
                return Err(format!(
                    "IO_READ_LINE prompt requires a String register, got {}",
                    native_type_name(prompt_type)
                ));
            }
            let no_prompt = self.code.new_label();
            self.code.ldr_imm(2, 31, slot_offset(prompt) + 8)?;
            self.code.cbz(2, no_prompt);
            self.code.ldr_imm(1, 31, slot_offset(prompt))?;
            self.code.add_imm(1, 1, HEAP_HEADER_SIZE)?;
            self.emit_write_buffer(1, epilogue)?;
            self.code.bind(no_prompt);
        }

        self.emit_allocate_string_buffer(4096, epilogue)?;
        self.code.mov_reg(20, 1);
        self.code.mov_imm(21, 0);
        let loop_start = self.code.new_label();
        let done = self.code.new_label();
        let eof = self.code.new_label();
        let input_error = self.code.new_label();
        let too_long = self.code.new_label();

        self.code.bind(loop_start);
        self.code.cmp_reg_imm(21, 4096);
        self.code.b_hs(too_long);
        self.code.mov_imm(0, 0);
        self.code.add_imm(1, 31, self.current_scratch)?;
        self.code.mov_imm(2, 1);
        self.code.mov_imm(16, DARWIN_SYSCALL_READ);
        self.code.svc();
        self.code.cmp_zero(0);
        self.code.b_eq(eof);
        self.code.b_lt(input_error);
        self.code.ldrb_imm(9, 31, self.current_scratch);
        self.code.cmp_reg_imm(9, b'\n' as u64);
        self.code.b_eq(done);
        self.code.cmp_reg_imm(9, b'\r' as u64);
        self.code.b_eq(done);
        self.code.add_reg(10, 20, 21);
        self.code.strb_imm(9, 10, HEAP_HEADER_SIZE);
        self.code.add_imm(21, 21, 1)?;
        self.code.b(loop_start);

        self.code.bind(done);
        self.store_string_slot_from_heap(dst, 20, 21, 4096)?;
        let success_end = self.code.new_label();
        self.code.b(success_end);

        self.code.bind(eof);
        self.fail_current_function(ERR_EOF, epilogue);
        self.code.bind(input_error);
        self.fail_current_function(ERR_INPUT_FAILURE, epilogue);
        self.code.bind(too_long);
        self.fail_current_function(ERR_INPUT_FAILURE, epilogue);
        self.code.bind(success_end);
        Ok(())
    }

    fn emit_io_read_char(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        self.expect_register_type(function, dst, NativeType::String, "IO_READ_CHAR")?;
        self.emit_allocate_string_buffer(4, epilogue)?;
        self.code.mov_reg(20, 1);
        self.code.mov_imm(0, 0);
        self.code.add_imm(1, 20, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(2, 1);
        self.code.mov_imm(16, DARWIN_SYSCALL_READ);
        self.code.svc();
        let have_first = self.code.new_label();
        let eof = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_gt(have_first);
        self.code.b_eq(eof);
        self.fail_current_function(ERR_INPUT_FAILURE, epilogue);
        self.code.bind(eof);
        self.fail_current_function(ERR_EOF, epilogue);
        self.code.bind(have_first);
        self.code.ldrb_imm(9, 20, HEAP_HEADER_SIZE);
        self.code.mov_imm(21, 1);
        self.code.cmp_reg_imm(9, 0x80);
        let finish = self.code.new_label();
        self.code.b_lo(finish);
        self.code.cmp_reg_imm(9, 0xc2);
        let invalid = self.code.new_label();
        self.code.b_lo(invalid);
        self.code.cmp_reg_imm(9, 0xe0);
        let read_two = self.code.new_label();
        self.code.b_lo(read_two);
        self.code.cmp_reg_imm(9, 0xf0);
        let read_three = self.code.new_label();
        self.code.b_lo(read_three);
        self.code.cmp_reg_imm(9, 0xf5);
        self.code.b_hs(invalid);
        self.code.mov_imm(21, 4);
        self.emit_read_remaining_char_bytes(epilogue)?;
        self.code.b(finish);
        self.code.bind(read_two);
        self.code.mov_imm(21, 2);
        self.emit_read_remaining_char_bytes(epilogue)?;
        self.code.b(finish);
        self.code.bind(read_three);
        self.code.mov_imm(21, 3);
        self.emit_read_remaining_char_bytes(epilogue)?;
        self.code.bind(finish);
        self.store_string_slot_from_heap(dst, 20, 21, 4)?;
        let done = self.code.new_label();
        self.code.b(done);
        self.code.bind(invalid);
        self.fail_current_function(ERR_INVALID_UTF8, epilogue);
        self.code.bind(done);
        Ok(())
    }

    fn emit_read_remaining_char_bytes(&mut self, epilogue: Label) -> Result<(), String> {
        let loop_start = self.code.new_label();
        let done = self.code.new_label();
        self.code.mov_imm(22, 1);
        self.code.bind(loop_start);
        self.code.cmp_reg(22, 21);
        self.code.b_hs(done);
        self.code.mov_imm(0, 0);
        self.code.add_reg(1, 20, 22);
        self.code.add_imm(1, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(2, 1);
        self.code.mov_imm(16, DARWIN_SYSCALL_READ);
        self.code.svc();
        self.code.cmp_zero(0);
        let eof = self.code.new_label();
        let input_error = self.code.new_label();
        self.code.b_eq(eof);
        self.code.b_lt(input_error);
        self.code.cmp_reg_imm(0, 1);
        let ok_read = self.code.new_label();
        self.code.b_eq(ok_read);
        self.code.bind(eof);
        self.fail_current_function(ERR_EOF, epilogue);
        self.code.bind(input_error);
        self.fail_current_function(ERR_INPUT_FAILURE, epilogue);
        self.code.bind(ok_read);
        self.code.ldrb_imm(9, 1, 0);
        self.code.cmp_reg_imm(9, 0x80);
        let invalid = self.code.new_label();
        self.code.b_lo(invalid);
        self.code.cmp_reg_imm(9, 0xc0);
        self.code.b_hs(invalid);
        self.code.add_imm(22, 22, 1)?;
        self.code.b(loop_start);
        self.code.bind(invalid);
        self.fail_current_function(ERR_INVALID_UTF8, epilogue);
        self.code.bind(done);
        Ok(())
    }

    fn emit_io_read_byte(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        self.expect_register_type(function, dst, NativeType::Byte, "IO_READ_BYTE")?;
        self.code.mov_imm(0, 0);
        self.code.add_imm(1, 31, self.current_scratch)?;
        self.code.mov_imm(2, 1);
        self.code.mov_imm(16, DARWIN_SYSCALL_READ);
        self.code.svc();
        let ok = self.code.new_label();
        let eof = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_gt(ok);
        self.code.b_eq(eof);
        self.fail_current_function(ERR_INPUT_FAILURE, epilogue);
        self.code.bind(eof);
        self.fail_current_function(ERR_EOF, epilogue);
        self.code.bind(ok);
        self.code.ldrb_imm(9, 31, self.current_scratch);
        self.code.str_imm(9, 31, slot_offset(dst))?;
        self.code.str_imm(31, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn emit_io_is_terminal(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let fd = operand(instruction, 1)? as u64;
        self.expect_register_type(function, dst, NativeType::Boolean, "IO_IS_TERMINAL")?;
        self.code.mov_imm(0, fd);
        self.code.mov_imm(1, DARWIN_TIOCGETA);
        self.code.add_imm(2, 31, self.current_scratch)?;
        self.code.mov_imm(16, DARWIN_SYSCALL_IOCTL);
        self.code.svc();
        let yes = self.code.new_label();
        let done = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_eq(yes);
        self.code.str_imm(31, 31, slot_offset(dst))?;
        self.code.b(done);
        self.code.bind(yes);
        self.code.mov_imm(9, 1);
        self.code.str_imm(9, 31, slot_offset(dst))?;
        self.code.bind(done);
        self.code.str_imm(31, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn emit_io_terminal_size(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        self.expect_register_type(function, dst, NativeType::Other, "IO_TERMINAL_SIZE")?;
        self.code.mov_imm(0, 1);
        self.code.mov_imm(1, DARWIN_TIOCGWINSZ);
        self.code.add_imm(2, 31, self.current_scratch)?;
        self.code.mov_imm(16, DARWIN_SYSCALL_IOCTL);
        self.code.svc();
        let ok = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_eq(ok);
        self.fail_current_function(ERR_NOT_TERMINAL, epilogue);
        self.code.bind(ok);
        self.emit_allocate_heap_object(HEAP_KIND_RECORD, 2, 0, 2 * SLOT_SIZE, epilogue)?;
        self.code.mov_reg(20, 1);
        self.code.ldrb_imm(9, 31, self.current_scratch + 2);
        self.code.ldrb_imm(10, 31, self.current_scratch + 3);
        self.code.mov_imm(11, 256);
        self.code.mul(10, 10, 11);
        self.code.add_reg(9, 9, 10);
        self.code.str_imm(9, 20, HEAP_HEADER_SIZE)?;
        self.code.ldrb_imm(9, 31, self.current_scratch);
        self.code.ldrb_imm(10, 31, self.current_scratch + 1);
        self.code.mul(10, 10, 11);
        self.code.add_reg(9, 9, 10);
        self.code.str_imm(9, 20, HEAP_HEADER_SIZE + SLOT_SIZE)?;
        self.code.str_imm(20, 31, slot_offset(dst))?;
        self.code.mov_imm(9, 2);
        self.code.str_imm(9, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)
    }

    fn emit_io_open(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let path = operand(instruction, 1)?;
        let mode = operand(instruction, 2)?;
        self.expect_register_type(function, dst, NativeType::FileHandle, "IO_OPEN")?;
        self.expect_register_type(function, path, NativeType::String, "IO_OPEN path")?;
        self.expect_register_type(function, mode, NativeType::String, "IO_OPEN mode")?;

        self.emit_open_mode_flags(mode, epilogue)?;
        self.code.str_imm(22, 31, self.current_scratch + 56)?;

        self.code.ldr_imm(20, 31, slot_offset(path))?;
        self.code.ldr_imm(21, 31, slot_offset(path) + 8)?;
        self.code.str_imm(20, 31, self.current_scratch + 64)?;
        self.code.str_imm(21, 31, self.current_scratch + 72)?;
        let invalid_path = self.code.new_label();
        self.code.cbz(21, invalid_path);
        self.code.add_imm(0, 21, 1)?;
        self.code.mov_imm(1, 1);
        self.code.bl(self.arena_alloc);
        let allocated = self.code.new_label();
        self.code.cbz(0, allocated);
        self.code.mov_imm(1, 0);
        self.code.mov_imm(2, 0);
        self.code.b(epilogue);

        self.code.bind(allocated);
        self.code.ldr_imm(20, 31, self.current_scratch + 64)?;
        self.code.ldr_imm(21, 31, self.current_scratch + 72)?;
        self.code.mov_reg(22, 1);
        self.code.add_imm(23, 20, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(24, 0);
        let copy_loop = self.code.new_label();
        let copy_done = self.code.new_label();
        self.code.bind(copy_loop);
        self.code.cmp_reg(24, 21);
        self.code.b_hs(copy_done);
        self.code.add_reg(25, 23, 24);
        self.code.ldrb_imm(26, 25, 0);
        self.code.cbz(26, invalid_path);
        self.code.add_reg(27, 22, 24);
        self.code.strb_imm(26, 27, 0);
        self.code.add_imm(24, 24, 1)?;
        self.code.b(copy_loop);
        self.code.bind(copy_done);
        self.code.add_reg(27, 22, 21);
        self.code.strb_imm(31, 27, 0);

        self.code.mov_reg(0, 22);
        self.code.ldr_imm(1, 31, self.current_scratch + 56)?;
        self.code.mov_imm(2, DARWIN_OPEN_PERMISSIONS);
        self.code.mov_imm(16, DARWIN_SYSCALL_OPEN);
        self.code.svc();
        let opened = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_ge(opened);
        self.fail_current_function(ERR_NOT_FOUND, epilogue);
        self.code.bind(opened);
        self.code.str_imm(0, 31, slot_offset(dst))?;
        self.code.str_imm(31, 31, slot_offset(dst) + 8)?;
        self.code.str_imm(31, 31, slot_offset(dst) + 16)?;
        let done = self.code.new_label();
        self.code.b(done);
        self.code.bind(invalid_path);
        self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(done);
        Ok(())
    }

    fn emit_open_mode_flags(&mut self, mode: u32, epilogue: Label) -> Result<(), String> {
        self.code.ldr_imm(20, 31, slot_offset(mode))?;
        self.code.ldr_imm(21, 31, slot_offset(mode) + 8)?;

        let read = self.code.new_label();
        let write = self.code.new_label();
        let read_write = self.code.new_label();
        let append = self.code.new_label();
        let invalid = self.code.new_label();
        self.branch_if_ascii_literal(20, 21, b"r", read);
        self.branch_if_ascii_literal(20, 21, b"read", read);
        self.branch_if_ascii_literal(20, 21, b"w", write);
        self.branch_if_ascii_literal(20, 21, b"write", write);
        self.branch_if_ascii_literal(20, 21, b"rw", read_write);
        self.branch_if_ascii_literal(20, 21, b"readWrite", read_write);
        self.branch_if_ascii_literal(20, 21, b"a", append);
        self.branch_if_ascii_literal(20, 21, b"append", append);
        self.code.b(invalid);

        let done = self.code.new_label();
        self.code.bind(read);
        self.code.mov_imm(22, 0);
        self.code.b(done);
        self.code.bind(write);
        self.code
            .mov_imm(22, DARWIN_O_WRONLY | DARWIN_O_CREAT | DARWIN_O_TRUNC);
        self.code.b(done);
        self.code.bind(read_write);
        self.code.mov_imm(22, DARWIN_O_RDWR | DARWIN_O_CREAT);
        self.code.b(done);
        self.code.bind(append);
        self.code
            .mov_imm(22, DARWIN_O_WRONLY | DARWIN_O_CREAT | DARWIN_O_APPEND);
        self.code.b(done);
        self.code.bind(invalid);
        self.fail_current_function(ERR_INVALID_ARGUMENT, epilogue);
        self.code.bind(done);
        Ok(())
    }

    fn branch_if_ascii_literal(&mut self, ptr: u8, len: u8, literal: &[u8], target: Label) {
        let next = self.code.new_label();
        self.code.cmp_reg_imm(len, literal.len() as u64);
        self.code.b_ne(next);
        for (index, byte) in literal.iter().enumerate() {
            self.code.ldrb_imm(18, ptr, HEAP_HEADER_SIZE + index);
            self.code.cmp_reg_imm(18, u64::from(*byte));
            self.code.b_ne(next);
        }
        self.code.b(target);
        self.code.bind(next);
    }

    fn emit_io_close(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let handle = operand(instruction, 1)?;
        self.expect_register_type(function, dst, NativeType::Nothing, "IO_CLOSE")?;
        self.expect_register_type(function, handle, NativeType::FileHandle, "IO_CLOSE")?;
        self.code.ldr_imm(0, 31, slot_offset(handle))?;
        self.code.mov_imm(16, DARWIN_SYSCALL_CLOSE);
        self.code.svc();
        let closed = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_ge(closed);
        self.fail_current_function(ERR_RESOURCE_CLOSED, epilogue);
        self.code.bind(closed);
        self.store_zero_slot(dst)
    }

    fn emit_fs_write_all(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let dst = operand(instruction, 0)?;
        let handle = operand(instruction, 1)?;
        let value = operand(instruction, 2)?;
        self.expect_register_type(function, dst, NativeType::Nothing, "FS_WRITE_ALL")?;
        self.expect_register_type(
            function,
            handle,
            NativeType::FileHandle,
            "FS_WRITE_ALL file",
        )?;
        self.expect_register_type(function, value, NativeType::String, "FS_WRITE_ALL value")?;
        self.code.ldr_imm(0, 31, slot_offset(handle))?;
        self.code.ldr_imm(1, 31, slot_offset(value))?;
        self.code.ldr_imm(2, 31, slot_offset(value) + 8)?;
        self.code.add_imm(1, 1, HEAP_HEADER_SIZE)?;
        self.code.mov_imm(16, DARWIN_SYSCALL_WRITE);
        self.code.svc();
        let ok = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_ge(ok);
        self.fail_current_function(ERR_OUTPUT_FAILURE, epilogue);
        self.code.bind(ok);
        self.store_zero_slot(dst)
    }

    fn emit_fs_default_result(
        &mut self,
        _function: &bytecode::NativeFunction,
        _instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        eprintln!("AI FUCKING LIED");
        std::process::exit(1);
    }

    fn emit_thread_default_result(
        &mut self,
        _function: &bytecode::NativeFunction,
        _instruction: &bytecode::NativeInstruction,
    ) -> Result<(), String> {
        eprintln!("AI FUCKING LIED");
        std::process::exit(1);
    }

    fn emit_close_resource(
        &mut self,
        function: &bytecode::NativeFunction,
        instruction: &bytecode::NativeInstruction,
        epilogue: Label,
    ) -> Result<(), String> {
        let handle = operand(instruction, 0)?;
        let close_function_id = operand(instruction, 1)?;
        if close_function_id != 0xffff_ff00 {
            return Err(format!(
                "unsupported resource close function id {close_function_id}"
            ));
        }
        self.expect_register_type(function, handle, NativeType::FileHandle, "CLOSE_RESOURCE")?;
        self.code.ldr_imm(0, 31, slot_offset(handle))?;
        self.code.mov_imm(16, DARWIN_SYSCALL_CLOSE);
        self.code.svc();
        let closed = self.code.new_label();
        self.code.cmp_zero(0);
        self.code.b_ge(closed);
        self.fail_current_function(ERR_RESOURCE_CLOSED, epilogue);
        self.code.bind(closed);
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
        self.code.mov_imm(16, DARWIN_SYSCALL_MMAP);
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
        self.code.mov_imm(16, DARWIN_SYSCALL_MUNMAP);
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
        self.code.mov_imm(16, DARWIN_SYSCALL_EXIT);
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
        NativeType::Byte => "Byte",
        NativeType::Integer => "Integer",
        NativeType::Float => "Float",
        NativeType::Fixed => "Fixed",
        NativeType::String => "String",
        NativeType::FileHandle => "FileHandle",
        NativeType::Result => "Result",
        NativeType::Other => "Other",
    }
}

fn promoted_numeric_comparison_uses_double(left: NativeType, right: NativeType) -> bool {
    matches!(left, NativeType::Float)
        || matches!(right, NativeType::Float)
        || (matches!(left, NativeType::Fixed) && !matches!(right, NativeType::Fixed))
        || (!matches!(left, NativeType::Fixed) && matches!(right, NativeType::Fixed))
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
            let target = self.labels[patch.label.0].expect("unbound AArch64 label");
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

    fn b_vc(&mut self, label: Label) {
        self.b_cond(label, 7);
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
        if let Ok(imm) = checked_imm12(value) {
            self.emit(0x9100_0000 | (imm << 10) | ((rn as u32) << 5) | rd as u32);
        } else {
            let scratch = scratch_excluding(rd, rn);
            self.mov_imm(scratch, value as u64);
            self.add_reg(rd, rn, scratch);
        }
        Ok(())
    }

    fn sub_imm(&mut self, rd: u8, rn: u8, value: usize) -> Result<(), String> {
        if let Ok(imm) = checked_imm12(value) {
            self.emit(0xd100_0000 | (imm << 10) | ((rn as u32) << 5) | rd as u32);
        } else {
            let scratch = scratch_excluding(rd, rn);
            self.mov_imm(scratch, value as u64);
            self.sub_reg(rd, rn, scratch);
        }
        Ok(())
    }

    fn add_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x8b00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn sub_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0xcb00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn adds_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0xab00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn subs_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0xeb00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
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

    fn orr_reg(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0xaa00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn mvn(&mut self, rd: u8, rm: u8) {
        self.emit(0xaa20_03e0 | ((rm as u32) << 16) | rd as u32);
    }

    fn mul(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x9b00_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn smulh(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0x9b40_7c00 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
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

    fn scvtf_d_from_x(&mut self, dd: u8, rn: u8) {
        self.emit(0x9e62_0000 | ((rn as u32) << 5) | dd as u32);
    }

    fn fcvtzs_x_from_d(&mut self, rd: u8, dn: u8) {
        self.emit(0x9e78_0000 | ((dn as u32) << 5) | rd as u32);
    }

    fn fcvtms_x_from_d(&mut self, rd: u8, dn: u8) {
        self.emit(0x9e70_0000 | ((dn as u32) << 5) | rd as u32);
    }

    fn fcvtps_x_from_d(&mut self, rd: u8, dn: u8) {
        self.emit(0x9e68_0000 | ((dn as u32) << 5) | rd as u32);
    }

    fn fcvtas_x_from_d(&mut self, rd: u8, dn: u8) {
        self.emit(0x9e64_0000 | ((dn as u32) << 5) | rd as u32);
    }

    fn fmov_x_from_d(&mut self, rd: u8, dn: u8) {
        self.emit(0x9e66_0000 | ((dn as u32) << 5) | rd as u32);
    }

    fn fmov_d_from_x(&mut self, dd: u8, rn: u8) {
        self.emit(0x9e67_0000 | ((rn as u32) << 5) | dd as u32);
    }

    fn fmov_d(&mut self, dd: u8, dn: u8) {
        self.emit(0x1e60_4000 | ((dn as u32) << 5) | dd as u32);
    }

    fn fadd_d(&mut self, dd: u8, dn: u8, dm: u8) {
        self.emit(0x1e60_2800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32);
    }

    fn fsub_d(&mut self, dd: u8, dn: u8, dm: u8) {
        self.emit(0x1e60_3800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32);
    }

    fn fmul_d(&mut self, dd: u8, dn: u8, dm: u8) {
        self.emit(0x1e60_0800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32);
    }

    fn fdiv_d(&mut self, dd: u8, dn: u8, dm: u8) {
        self.emit(0x1e60_1800 | ((dm as u32) << 16) | ((dn as u32) << 5) | dd as u32);
    }

    fn fneg_d(&mut self, dd: u8, dn: u8) {
        self.emit(0x1e61_4000 | ((dn as u32) << 5) | dd as u32);
    }

    fn fabs_d(&mut self, dd: u8, dn: u8) {
        self.emit(0x1e60_c000 | ((dn as u32) << 5) | dd as u32);
    }

    fn fsqrt_d(&mut self, dd: u8, dn: u8) {
        self.emit(0x1e61_c000 | ((dn as u32) << 5) | dd as u32);
    }

    fn fcmp_zero_d(&mut self, dn: u8) {
        self.emit(0x1e60_2000 | ((dn as u32) << 5) | 0x8);
    }

    fn fcmp_d(&mut self, dn: u8, dm: u8) {
        self.emit(0x1e60_2000 | ((dm as u32) << 16) | ((dn as u32) << 5));
    }

    fn neg(&mut self, rd: u8, rm: u8) {
        self.emit(0xcb00_03e0 | ((rm as u32) << 16) | rd as u32);
    }

    fn eor(&mut self, rd: u8, rn: u8, rm: u8) {
        self.emit(0xca00_0000 | ((rm as u32) << 16) | ((rn as u32) << 5) | rd as u32);
    }

    fn lsl_imm(&mut self, rd: u8, rn: u8, shift: u8) {
        debug_assert!(shift < 64);
        let immr = (64 - shift as u32) & 63;
        let imms = 63 - shift as u32;
        self.emit(0xd340_0000 | (immr << 16) | (imms << 10) | ((rn as u32) << 5) | rd as u32);
    }

    fn lsr_imm(&mut self, rd: u8, rn: u8, shift: u8) {
        debug_assert!(shift < 64);
        self.emit(
            0xd340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        );
    }

    fn asr_imm(&mut self, rd: u8, rn: u8, shift: u8) {
        debug_assert!(shift < 64);
        self.emit(
            0x9340_0000 | ((shift as u32) << 16) | (63 << 10) | ((rn as u32) << 5) | rd as u32,
        );
    }

    fn ldr_imm(&mut self, rt: u8, rn: u8, offset: usize) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned AArch64 load offset {offset}"));
        }
        let imm = checked_imm12(offset / 8)?;
        self.emit(0xf940_0000 | (imm << 10) | ((rn as u32) << 5) | rt as u32);
        Ok(())
    }

    fn str_imm(&mut self, rt: u8, rn: u8, offset: usize) -> Result<(), String> {
        if offset % 8 != 0 {
            return Err(format!("unaligned AArch64 store offset {offset}"));
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
            "AArch64 ADR target out of range"
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
        return Err(format!("AArch64 immediate {value} exceeds 12-bit encoding"));
    }
    Ok(value as u32)
}

fn scratch_excluding(a: u8, b: u8) -> u8 {
    [17, 16, 15]
        .into_iter()
        .find(|candidate| *candidate != a && *candidate != b)
        .expect("scratch register candidate list is non-empty")
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
