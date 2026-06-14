#![allow(dead_code)]

use crate::builtins;
use crate::ir::{IrFunction, IrMatchPattern, IrOp, IrProject, IrType, IrValue};
use crate::numeric;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

const SECTION_MANIFEST: u16 = 1;
const SECTION_STRING_POOL: u16 = 2;
const SECTION_TYPE_TABLE: u16 = 3;
const SECTION_CONST_POOL: u16 = 4;
const SECTION_IMPORT_TABLE: u16 = 5;
const SECTION_EXPORT_TABLE: u16 = 6;
const SECTION_GLOBAL_TABLE: u16 = 7;
const SECTION_FUNCTION_TABLE: u16 = 8;
const SECTION_CODE: u16 = 9;
const SECTION_RESOURCE_TABLE: u16 = 11;
const SECTION_ABI_INDEX: u16 = 15;

const ABI_FORMAT_VERSION: u16 = 1;
const ABI_HASH_LEN: usize = 32;

pub(crate) const TYPE_NOTHING: u32 = 1;
pub(crate) const TYPE_BOOLEAN: u32 = 2;
pub(crate) const TYPE_INTEGER: u32 = 3;
pub(crate) const TYPE_FLOAT: u32 = 4;
pub(crate) const TYPE_FIXED: u32 = 5;
pub(crate) const TYPE_STRING: u32 = 6;
pub(crate) const TYPE_BYTE: u32 = 7;
pub(crate) const TYPE_ERROR: u32 = 8;
pub(crate) const TYPE_TERMINAL_SIZE: u32 = 9;
pub(crate) const TYPE_FILE_HANDLE: u32 = 0xffff_ff00;
const FIRST_TABLE_TYPE_ID: u32 = 10;

const FUNCTION_BYTECODE: u16 = 1;

const FUNCTION_FLAG_ISOLATED: u16 = 1 << 2;
const FUNCTION_FLAG_PRIVATE: u16 = 1 << 1;
const FUNCTION_FLAG_SUB: u16 = 1 << 3;
const FUNCTION_FLAG_RETURNS_NOTHING: u16 = 1 << 5;

const REGISTER_FLAG_PARAMETER: u32 = 1 << 0;
const REGISTER_FLAG_MUTABLE_LOCAL_CELL: u32 = 1 << 1;
const REGISTER_FLAG_RESOURCE: u32 = 1 << 2;
const REGISTER_FLAG_INITIALIZED_AT_ENTRY: u32 = 1 << 3;

const OPCODE_LOAD_CONST: u16 = 1;
const OPCODE_LOAD_DEFAULT: u16 = 2;
const OPCODE_ADD: u16 = 20;
const OPCODE_SUB: u16 = 21;
const OPCODE_MUL: u16 = 22;
const OPCODE_DIV: u16 = 23;
const OPCODE_EQUAL: u16 = 24;
const OPCODE_NOT_EQUAL: u16 = 25;
const OPCODE_LESS: u16 = 26;
const OPCODE_LESS_EQUAL: u16 = 27;
const OPCODE_GREATER: u16 = 28;
const OPCODE_GREATER_EQUAL: u16 = 29;
const OPCODE_MOD: u16 = 30;
const OPCODE_POW: u16 = 31;
const OPCODE_NOT: u16 = 32;
const OPCODE_XOR: u16 = 33;
const OPCODE_NEG: u16 = 34;
const OPCODE_CONCAT: u16 = 40;
pub(crate) const OPCODE_IO_WRITE: u16 = 50;
pub(crate) const OPCODE_IO_FLUSH: u16 = 51;
pub(crate) const OPCODE_IO_READ_LINE: u16 = 52;
pub(crate) const OPCODE_IO_READ_CHAR: u16 = 53;
pub(crate) const OPCODE_IO_READ_BYTE: u16 = 54;
pub(crate) const OPCODE_IO_IS_TERMINAL: u16 = 55;
pub(crate) const OPCODE_IO_TERMINAL_SIZE: u16 = 56;
pub(crate) const OPCODE_IO_POLL_INPUT: u16 = 57;
pub(crate) const OPCODE_IO_OPEN: u16 = 58;
pub(crate) const OPCODE_IO_CLOSE: u16 = 59;
pub(crate) const OPCODE_FS_FILE_EXISTS: u16 = 180;
pub(crate) const OPCODE_FS_DIRECTORY_EXISTS: u16 = 181;
pub(crate) const OPCODE_FS_EXISTS: u16 = 182;
pub(crate) const OPCODE_FS_READ_TEXT: u16 = 183;
pub(crate) const OPCODE_FS_WRITE_TEXT: u16 = 184;
pub(crate) const OPCODE_FS_WRITE_TEXT_ATOMIC: u16 = 185;
pub(crate) const OPCODE_FS_APPEND_TEXT: u16 = 186;
pub(crate) const OPCODE_FS_OPEN: u16 = 187;
pub(crate) const OPCODE_FS_OPEN_NO_FOLLOW: u16 = 188;
pub(crate) const OPCODE_FS_CREATE_TEMP_FILE: u16 = 189;
pub(crate) const OPCODE_FS_READ_LINE: u16 = 190;
pub(crate) const OPCODE_FS_READ_ALL: u16 = 191;
pub(crate) const OPCODE_FS_WRITE_ALL: u16 = 192;
pub(crate) const OPCODE_FS_CLOSE: u16 = 193;
pub(crate) const OPCODE_FS_EOF: u16 = 194;
pub(crate) const OPCODE_FS_CANONICAL_PATH: u16 = 195;
pub(crate) const OPCODE_FS_IS_WITHIN: u16 = 196;
pub(crate) const OPCODE_FS_PATH_JOIN: u16 = 197;
pub(crate) const OPCODE_FS_PATH_DIR_NAME: u16 = 198;
pub(crate) const OPCODE_FS_PATH_BASE_NAME: u16 = 199;
pub(crate) const OPCODE_FS_PATH_EXTENSION: u16 = 200;
pub(crate) const OPCODE_FS_PATH_NORMALIZE: u16 = 201;
pub(crate) const OPCODE_FS_DELETE_FILE: u16 = 202;
pub(crate) const OPCODE_FS_CREATE_DIRECTORY: u16 = 203;
pub(crate) const OPCODE_FS_CREATE_DIRECTORIES: u16 = 204;
pub(crate) const OPCODE_FS_DELETE_DIRECTORY: u16 = 205;
pub(crate) const OPCODE_FS_LIST_DIRECTORY: u16 = 206;
pub(crate) const OPCODE_FS_CURRENT_DIRECTORY: u16 = 207;
pub(crate) const OPCODE_FS_SET_CURRENT_DIRECTORY: u16 = 208;
pub(crate) const OPCODE_FS_READ_BYTES: u16 = 209;
pub(crate) const OPCODE_FS_WRITE_BYTES: u16 = 210;
pub(crate) const OPCODE_FS_WRITE_BYTES_ATOMIC: u16 = 211;
pub(crate) const OPCODE_FS_APPEND_BYTES: u16 = 212;
pub(crate) const OPCODE_FS_READ_ALL_BYTES: u16 = 213;
pub(crate) const OPCODE_FS_WRITE_ALL_BYTES: u16 = 214;
pub(crate) const OPCODE_FS_TEMP_DIRECTORY: u16 = 215;
pub(crate) const OPCODE_THREAD_START: u16 = 220;
pub(crate) const OPCODE_THREAD_IS_RUNNING: u16 = 221;
pub(crate) const OPCODE_THREAD_WAIT_FOR: u16 = 222;
pub(crate) const OPCODE_THREAD_CANCEL: u16 = 223;
pub(crate) const OPCODE_THREAD_SEND: u16 = 224;
pub(crate) const OPCODE_THREAD_POLL: u16 = 225;
pub(crate) const OPCODE_THREAD_READ: u16 = 226;
pub(crate) const OPCODE_THREAD_RECEIVE: u16 = 227;
pub(crate) const OPCODE_THREAD_EMIT: u16 = 228;
pub(crate) const OPCODE_THREAD_IS_CANCELLED: u16 = 229;
const OPCODE_CALL_RESULT: u16 = 60;
const OPCODE_UNWRAP_RESULT: u16 = 61;
const OPCODE_LOAD_FUNCTION: u16 = 62;
const OPCODE_CALL_VALUE_RESULT: u16 = 63;
const OPCODE_RETURN_OK: u16 = 70;
const OPCODE_RETURN_ERR: u16 = 71;
const OPCODE_CONSTRUCT_RECORD: u16 = 80;
const OPCODE_CONSTRUCT_VARIANT: u16 = 81;
const OPCODE_LOAD_FIELD: u16 = 82;
const OPCODE_LOAD_ENUM_MEMBER: u16 = 83;
const OPCODE_CONSTRUCT_LIST: u16 = 84;
const OPCODE_CONSTRUCT_MAP: u16 = 85;
const OPCODE_COLLECTION_ITER_BEGIN: u16 = 86;
const OPCODE_COLLECTION_ITER_NEXT: u16 = 87;
const OPCODE_LOAD_MAP_ENTRY_FIELD: u16 = 88;
const OPCODE_BRANCH: u16 = 90;
const OPCODE_BRANCH_IF_FALSE: u16 = 91;
const OPCODE_VARIANT_MATCH: u16 = 92;
const OPCODE_BRANCH_IF_TRUE: u16 = 93;
pub(crate) const OPCODE_GENERAL_LEN: u16 = 100;
pub(crate) const OPCODE_GENERAL_FIND: u16 = 101;
pub(crate) const OPCODE_GENERAL_MID: u16 = 102;
pub(crate) const OPCODE_GENERAL_REPLACE: u16 = 103;
pub(crate) const OPCODE_GENERAL_TO_STRING: u16 = 104;
pub(crate) const OPCODE_GENERAL_TO_INT: u16 = 105;
pub(crate) const OPCODE_GENERAL_TO_FLOAT: u16 = 106;
pub(crate) const OPCODE_GENERAL_TO_FIXED: u16 = 107;
pub(crate) const OPCODE_GENERAL_TO_BYTE: u16 = 108;
pub(crate) const OPCODE_GENERAL_IS_NUMERIC: u16 = 109;
pub(crate) const OPCODE_GENERAL_IS_EVEN: u16 = 110;
pub(crate) const OPCODE_GENERAL_IS_ODD: u16 = 111;
pub(crate) const OPCODE_GENERAL_IS_POSITIVE: u16 = 112;
pub(crate) const OPCODE_GENERAL_IS_NEGATIVE: u16 = 113;
pub(crate) const OPCODE_GENERAL_IS_ZERO: u16 = 114;
pub(crate) const OPCODE_GENERAL_IS_EMPTY: u16 = 115;
pub(crate) const OPCODE_GENERAL_IS_NOT_EMPTY: u16 = 116;
pub(crate) const OPCODE_COLLECTION_GET: u16 = 120;
pub(crate) const OPCODE_COLLECTION_GET_OR: u16 = 121;
pub(crate) const OPCODE_COLLECTION_FIND: u16 = 122;
pub(crate) const OPCODE_COLLECTION_MID: u16 = 123;
pub(crate) const OPCODE_COLLECTION_REPLACE: u16 = 124;
pub(crate) const OPCODE_COLLECTION_SET: u16 = 125;
pub(crate) const OPCODE_COLLECTION_APPEND: u16 = 126;
pub(crate) const OPCODE_COLLECTION_PREPEND: u16 = 127;
pub(crate) const OPCODE_COLLECTION_INSERT: u16 = 128;
pub(crate) const OPCODE_COLLECTION_REMOVE_AT: u16 = 129;
pub(crate) const OPCODE_COLLECTION_REMOVE_KEY: u16 = 130;
pub(crate) const OPCODE_COLLECTION_KEYS: u16 = 131;
pub(crate) const OPCODE_COLLECTION_VALUES: u16 = 132;
pub(crate) const OPCODE_COLLECTION_HAS_KEY: u16 = 133;
pub(crate) const OPCODE_COLLECTION_CONTAINS: u16 = 134;
pub(crate) const OPCODE_COLLECTION_SUM: u16 = 135;
pub(crate) const OPCODE_COLLECTION_FOR_EACH: u16 = 136;
pub(crate) const OPCODE_COLLECTION_TRANSFORM: u16 = 137;
pub(crate) const OPCODE_COLLECTION_FILTER: u16 = 138;
pub(crate) const OPCODE_COLLECTION_REDUCE: u16 = 139;
pub(crate) const OPCODE_STRING_TRIM: u16 = 140;
pub(crate) const OPCODE_STRING_TRIM_START: u16 = 141;
pub(crate) const OPCODE_STRING_TRIM_END: u16 = 142;
pub(crate) const OPCODE_STRING_UPPER: u16 = 143;
pub(crate) const OPCODE_STRING_LOWER: u16 = 144;
pub(crate) const OPCODE_STRING_CASE_FOLD: u16 = 145;
pub(crate) const OPCODE_STRING_NORMALIZE_NFC: u16 = 146;
pub(crate) const OPCODE_STRING_GRAPHEMES: u16 = 147;
pub(crate) const OPCODE_STRING_STARTS_WITH: u16 = 148;
pub(crate) const OPCODE_STRING_ENDS_WITH: u16 = 149;
pub(crate) const OPCODE_STRING_CONTAINS: u16 = 150;
pub(crate) const OPCODE_STRING_SPLIT: u16 = 151;
pub(crate) const OPCODE_STRING_JOIN: u16 = 152;
pub(crate) const OPCODE_STRING_BYTE_LEN: u16 = 153;
pub(crate) const OPCODE_STRING_REGEX_MATCH: u16 = 154;
pub(crate) const OPCODE_STRING_REGEX_FIND: u16 = 155;
pub(crate) const OPCODE_STRING_REGEX_REPLACE: u16 = 156;
pub(crate) const OPCODE_MATH_PI: u16 = 230;
pub(crate) const OPCODE_MATH_E: u16 = 231;
pub(crate) const OPCODE_MATH_ABS: u16 = 232;
pub(crate) const OPCODE_MATH_SIGN: u16 = 233;
pub(crate) const OPCODE_MATH_MIN: u16 = 234;
pub(crate) const OPCODE_MATH_MAX: u16 = 235;
pub(crate) const OPCODE_MATH_CLAMP: u16 = 236;
pub(crate) const OPCODE_MATH_FLOOR: u16 = 237;
pub(crate) const OPCODE_MATH_CEIL: u16 = 238;
pub(crate) const OPCODE_MATH_ROUND: u16 = 239;
pub(crate) const OPCODE_MATH_TRUNC: u16 = 240;
pub(crate) const OPCODE_MATH_SQRT: u16 = 241;
pub(crate) const OPCODE_MATH_POW: u16 = 242;
pub(crate) const OPCODE_MATH_EXP: u16 = 243;
pub(crate) const OPCODE_MATH_LOG: u16 = 244;
pub(crate) const OPCODE_MATH_LOG10: u16 = 245;
pub(crate) const OPCODE_MATH_SIN: u16 = 246;
pub(crate) const OPCODE_MATH_COS: u16 = 247;
pub(crate) const OPCODE_MATH_TAN: u16 = 248;
pub(crate) const OPCODE_MATH_ASIN: u16 = 249;
pub(crate) const OPCODE_MATH_ACOS: u16 = 250;
pub(crate) const OPCODE_MATH_ATAN: u16 = 251;
pub(crate) const OPCODE_MATH_ATAN2: u16 = 252;
pub(crate) const OPCODE_MATH_RADIANS: u16 = 253;
pub(crate) const OPCODE_MATH_DEGREES: u16 = 254;
pub(crate) const OPCODE_MATH_IS_FINITE: u16 = 255;
pub(crate) const OPCODE_USING_ENTER: u16 = 170;
pub(crate) const OPCODE_USING_LEAVE: u16 = 171;
pub(crate) const OPCODE_CLOSE_RESOURCE: u16 = 172;

pub fn write_bytecode_hex(
    project_dir: &Path,
    ir: &IrProject,
    version: &str,
) -> Result<PathBuf, String> {
    let metadata = BytecodeMetadata::new(ir.name.clone(), version.to_string());
    let bytes = build_bytecode_bytes(ir, &metadata)?;
    let hex_path = project_dir.join(format!("{}.hex", ir.name));
    fs::write(&hex_path, hex_dump(&bytes))
        .map_err(|err| format!("failed to write '{}': {err}", hex_path.display()))?;
    Ok(hex_path)
}

pub fn write_merged_bytecode_hex(
    project_dir: &Path,
    ir: &IrProject,
    version: &str,
    packages: &[PathBuf],
) -> Result<PathBuf, String> {
    let metadata = BytecodeMetadata::new(ir.name.clone(), version.to_string());
    let bytes = build_merged_bytecode_bytes(ir, &metadata, packages)?;
    let hex_path = project_dir.join(format!("{}.hex", ir.name));
    fs::write(&hex_path, hex_dump(&bytes))
        .map_err(|err| format!("failed to write '{}': {err}", hex_path.display()))?;
    Ok(hex_path)
}

pub fn build_bytecode_bytes(
    ir: &IrProject,
    metadata: &BytecodeMetadata,
) -> Result<Vec<u8>, String> {
    Ok(lower_project(ir, metadata)?.encode())
}

pub fn build_merged_bytecode_bytes(
    ir: &IrProject,
    metadata: &BytecodeMetadata,
    packages: &[PathBuf],
) -> Result<Vec<u8>, String> {
    Ok(lower_merged_project(ir, metadata, packages)?.encode())
}

#[derive(Clone)]
pub struct BytecodeMetadata {
    pub name: String,
    pub ident: String,
    pub version: String,
    pub ident_key: String,
    pub author: String,
    pub url: String,
    pub dependencies: Vec<BytecodeDependency>,
}

impl BytecodeMetadata {
    pub fn new(name: String, version: String) -> Self {
        Self {
            name,
            ident: String::new(),
            version,
            ident_key: String::new(),
            author: String::new(),
            url: String::new(),
            dependencies: Vec::new(),
        }
    }
}

#[derive(Clone)]
pub struct BytecodeDependency {
    pub name: String,
    pub ident: String,
    pub version: String,
    pub pin: bool,
    pub flags: u32,
}

#[derive(Clone)]
pub struct BytecodeExport {
    pub name: String,
    pub kind: BytecodeExportKind,
    pub isolated: bool,
    pub params: Vec<BytecodeExportParam>,
    pub return_type: String,
}

#[derive(Clone, Copy, Eq, PartialEq, Hash)]
pub enum BytecodeExportKind {
    Func,
    Sub,
}

#[derive(Clone)]
pub struct BytecodeExportParam {
    pub type_: String,
    pub has_default: bool,
}

pub struct BytecodePackageInfo {
    pub manifest_name: String,
    pub manifest_ident: String,
    pub manifest_version: String,
    pub manifest_ident_key: String,
    pub author: String,
    pub url: String,
    pub type_count: usize,
    pub const_count: usize,
    pub resource_count: usize,
    pub function_count: usize,
    pub export_count: usize,
    pub import_count: usize,
    pub abi_format_version: u16,
    pub exports: Vec<BytecodePackageInfoExport>,
    pub imports: Vec<BytecodePackageInfoImport>,
}

pub struct BytecodePackageInfoExport {
    pub name: String,
    pub kind: BytecodeExportKind,
    pub sig_hash: String,
}

pub struct BytecodePackageInfoImport {
    pub package_name: String,
    pub package_ident: String,
    pub version: String,
    pub pin: bool,
    pub flags: u32,
    pub used_symbols: Vec<BytecodePackageInfoUsedSymbol>,
}

pub struct BytecodePackageInfoUsedSymbol {
    pub name: String,
    pub sig_hash: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeType {
    Nothing,
    Boolean,
    Byte,
    Integer,
    Float,
    Fixed,
    String,
    FileHandle,
    Result,
    Other,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum NativeImportKind {
    RuntimeHelper,
    LibSystem,
}

pub struct NativeImport {
    pub symbol: &'static str,
    pub kind: NativeImportKind,
}

pub struct NativeProgram {
    pub entry_function: u32,
    pub entry_returns_integer: bool,
    pub types: NativeTypeLayouts,
    pub functions: Vec<NativeFunction>,
    pub constants: Vec<NativeConst>,
    pub imports: Vec<NativeImport>,
}

pub struct NativeFunction {
    pub param_count: usize,
    pub registers: Vec<NativeRegister>,
    pub code: Vec<NativeInstruction>,
}

pub struct NativeRegister {
    pub type_id: u32,
    pub type_: NativeType,
}

pub struct NativeTypeLayouts {
    pub records: HashMap<u32, NativeRecordLayout>,
    pub unions: HashMap<u32, NativeUnionLayout>,
}

pub struct NativeRecordLayout {
    pub fields: HashMap<u32, NativeFieldLayout>,
    pub ordered_fields: Vec<NativeFieldLayout>,
    pub size_slots: usize,
}

pub struct NativeUnionLayout {
    pub variants: HashMap<u32, NativeVariantLayout>,
    pub size_slots: usize,
}

pub struct NativeVariantLayout {
    pub fields: Vec<NativeFieldLayout>,
}

#[derive(Clone)]
pub struct NativeFieldLayout {
    pub name: u32,
    pub offset_slots: usize,
}

pub struct NativeInstruction {
    pub opcode: u16,
    pub operands: Vec<u32>,
}

pub enum NativeConst {
    Nothing,
    Boolean(bool),
    Byte(u8),
    Integer(i64),
    Float(f64),
    Fixed(i64),
    String(String),
    Other,
}

pub const NATIVE_OPCODE_LOAD_CONST: u16 = OPCODE_LOAD_CONST;
pub const NATIVE_OPCODE_LOAD_DEFAULT: u16 = OPCODE_LOAD_DEFAULT;
pub const NATIVE_OPCODE_MOVE: u16 = OPCODE_MOVE;
pub const NATIVE_OPCODE_COPY: u16 = OPCODE_COPY;
pub const NATIVE_OPCODE_ADD: u16 = OPCODE_ADD;
pub const NATIVE_OPCODE_SUB: u16 = OPCODE_SUB;
pub const NATIVE_OPCODE_MUL: u16 = OPCODE_MUL;
pub const NATIVE_OPCODE_DIV: u16 = OPCODE_DIV;
pub const NATIVE_OPCODE_EQUAL: u16 = OPCODE_EQUAL;
pub const NATIVE_OPCODE_NOT_EQUAL: u16 = OPCODE_NOT_EQUAL;
pub const NATIVE_OPCODE_LESS: u16 = OPCODE_LESS;
pub const NATIVE_OPCODE_LESS_EQUAL: u16 = OPCODE_LESS_EQUAL;
pub const NATIVE_OPCODE_GREATER: u16 = OPCODE_GREATER;
pub const NATIVE_OPCODE_GREATER_EQUAL: u16 = OPCODE_GREATER_EQUAL;
pub const NATIVE_OPCODE_MOD: u16 = OPCODE_MOD;
pub const NATIVE_OPCODE_POW: u16 = OPCODE_POW;
pub const NATIVE_OPCODE_NOT: u16 = OPCODE_NOT;
pub const NATIVE_OPCODE_XOR: u16 = OPCODE_XOR;
pub const NATIVE_OPCODE_NEG: u16 = OPCODE_NEG;
pub const NATIVE_OPCODE_CONCAT: u16 = OPCODE_CONCAT;
pub const NATIVE_OPCODE_IO_WRITE: u16 = OPCODE_IO_WRITE;
pub const NATIVE_OPCODE_IO_FLUSH: u16 = OPCODE_IO_FLUSH;
pub const NATIVE_OPCODE_IO_READ_LINE: u16 = OPCODE_IO_READ_LINE;
pub const NATIVE_OPCODE_IO_READ_CHAR: u16 = OPCODE_IO_READ_CHAR;
pub const NATIVE_OPCODE_IO_READ_BYTE: u16 = OPCODE_IO_READ_BYTE;
pub const NATIVE_OPCODE_IO_IS_TERMINAL: u16 = OPCODE_IO_IS_TERMINAL;
pub const NATIVE_OPCODE_IO_TERMINAL_SIZE: u16 = OPCODE_IO_TERMINAL_SIZE;
pub const NATIVE_OPCODE_IO_POLL_INPUT: u16 = OPCODE_IO_POLL_INPUT;
pub const NATIVE_OPCODE_IO_OPEN: u16 = OPCODE_IO_OPEN;
pub const NATIVE_OPCODE_IO_CLOSE: u16 = OPCODE_IO_CLOSE;
pub const NATIVE_OPCODE_FS_FILE_EXISTS: u16 = OPCODE_FS_FILE_EXISTS;
pub const NATIVE_OPCODE_FS_DIRECTORY_EXISTS: u16 = OPCODE_FS_DIRECTORY_EXISTS;
pub const NATIVE_OPCODE_FS_EXISTS: u16 = OPCODE_FS_EXISTS;
pub const NATIVE_OPCODE_FS_READ_TEXT: u16 = OPCODE_FS_READ_TEXT;
pub const NATIVE_OPCODE_FS_WRITE_TEXT: u16 = OPCODE_FS_WRITE_TEXT;
pub const NATIVE_OPCODE_FS_WRITE_TEXT_ATOMIC: u16 = OPCODE_FS_WRITE_TEXT_ATOMIC;
pub const NATIVE_OPCODE_FS_APPEND_TEXT: u16 = OPCODE_FS_APPEND_TEXT;
pub const NATIVE_OPCODE_FS_OPEN: u16 = OPCODE_FS_OPEN;
pub const NATIVE_OPCODE_FS_OPEN_NO_FOLLOW: u16 = OPCODE_FS_OPEN_NO_FOLLOW;
pub const NATIVE_OPCODE_FS_CREATE_TEMP_FILE: u16 = OPCODE_FS_CREATE_TEMP_FILE;
pub const NATIVE_OPCODE_FS_READ_LINE: u16 = OPCODE_FS_READ_LINE;
pub const NATIVE_OPCODE_FS_READ_ALL: u16 = OPCODE_FS_READ_ALL;
pub const NATIVE_OPCODE_FS_WRITE_ALL: u16 = OPCODE_FS_WRITE_ALL;
pub const NATIVE_OPCODE_FS_CLOSE: u16 = OPCODE_FS_CLOSE;
pub const NATIVE_OPCODE_FS_EOF: u16 = OPCODE_FS_EOF;
pub const NATIVE_OPCODE_FS_CANONICAL_PATH: u16 = OPCODE_FS_CANONICAL_PATH;
pub const NATIVE_OPCODE_FS_IS_WITHIN: u16 = OPCODE_FS_IS_WITHIN;
pub const NATIVE_OPCODE_FS_PATH_JOIN: u16 = OPCODE_FS_PATH_JOIN;
pub const NATIVE_OPCODE_FS_PATH_DIR_NAME: u16 = OPCODE_FS_PATH_DIR_NAME;
pub const NATIVE_OPCODE_FS_PATH_BASE_NAME: u16 = OPCODE_FS_PATH_BASE_NAME;
pub const NATIVE_OPCODE_FS_PATH_EXTENSION: u16 = OPCODE_FS_PATH_EXTENSION;
pub const NATIVE_OPCODE_FS_PATH_NORMALIZE: u16 = OPCODE_FS_PATH_NORMALIZE;
pub const NATIVE_OPCODE_FS_DELETE_FILE: u16 = OPCODE_FS_DELETE_FILE;
pub const NATIVE_OPCODE_FS_CREATE_DIRECTORY: u16 = OPCODE_FS_CREATE_DIRECTORY;
pub const NATIVE_OPCODE_FS_CREATE_DIRECTORIES: u16 = OPCODE_FS_CREATE_DIRECTORIES;
pub const NATIVE_OPCODE_FS_DELETE_DIRECTORY: u16 = OPCODE_FS_DELETE_DIRECTORY;
pub const NATIVE_OPCODE_FS_LIST_DIRECTORY: u16 = OPCODE_FS_LIST_DIRECTORY;
pub const NATIVE_OPCODE_FS_CURRENT_DIRECTORY: u16 = OPCODE_FS_CURRENT_DIRECTORY;
pub const NATIVE_OPCODE_FS_TEMP_DIRECTORY: u16 = OPCODE_FS_TEMP_DIRECTORY;
pub const NATIVE_OPCODE_FS_SET_CURRENT_DIRECTORY: u16 = OPCODE_FS_SET_CURRENT_DIRECTORY;
pub const NATIVE_OPCODE_THREAD_START: u16 = OPCODE_THREAD_START;
pub const NATIVE_OPCODE_THREAD_IS_RUNNING: u16 = OPCODE_THREAD_IS_RUNNING;
pub const NATIVE_OPCODE_THREAD_WAIT_FOR: u16 = OPCODE_THREAD_WAIT_FOR;
pub const NATIVE_OPCODE_THREAD_CANCEL: u16 = OPCODE_THREAD_CANCEL;
pub const NATIVE_OPCODE_THREAD_SEND: u16 = OPCODE_THREAD_SEND;
pub const NATIVE_OPCODE_THREAD_POLL: u16 = OPCODE_THREAD_POLL;
pub const NATIVE_OPCODE_THREAD_READ: u16 = OPCODE_THREAD_READ;
pub const NATIVE_OPCODE_THREAD_RECEIVE: u16 = OPCODE_THREAD_RECEIVE;
pub const NATIVE_OPCODE_THREAD_EMIT: u16 = OPCODE_THREAD_EMIT;
pub const NATIVE_OPCODE_THREAD_IS_CANCELLED: u16 = OPCODE_THREAD_IS_CANCELLED;
pub const NATIVE_OPCODE_CALL_RESULT: u16 = OPCODE_CALL_RESULT;
pub const NATIVE_OPCODE_UNWRAP_RESULT: u16 = OPCODE_UNWRAP_RESULT;
pub const NATIVE_OPCODE_LOAD_FUNCTION: u16 = OPCODE_LOAD_FUNCTION;
pub const NATIVE_OPCODE_CALL_VALUE_RESULT: u16 = OPCODE_CALL_VALUE_RESULT;
pub const NATIVE_OPCODE_RETURN_OK: u16 = OPCODE_RETURN_OK;
pub const NATIVE_OPCODE_CONSTRUCT_RECORD: u16 = OPCODE_CONSTRUCT_RECORD;
pub const NATIVE_OPCODE_CONSTRUCT_VARIANT: u16 = OPCODE_CONSTRUCT_VARIANT;
pub const NATIVE_OPCODE_LOAD_FIELD: u16 = OPCODE_LOAD_FIELD;
pub const NATIVE_OPCODE_LOAD_ENUM_MEMBER: u16 = OPCODE_LOAD_ENUM_MEMBER;
pub const NATIVE_OPCODE_CONSTRUCT_LIST: u16 = OPCODE_CONSTRUCT_LIST;
pub const NATIVE_OPCODE_CONSTRUCT_MAP: u16 = OPCODE_CONSTRUCT_MAP;
pub const NATIVE_OPCODE_COLLECTION_ITER_BEGIN: u16 = OPCODE_COLLECTION_ITER_BEGIN;
pub const NATIVE_OPCODE_COLLECTION_ITER_NEXT: u16 = OPCODE_COLLECTION_ITER_NEXT;
pub const NATIVE_OPCODE_LOAD_MAP_ENTRY_FIELD: u16 = OPCODE_LOAD_MAP_ENTRY_FIELD;
pub const NATIVE_OPCODE_BRANCH: u16 = OPCODE_BRANCH;
pub const NATIVE_OPCODE_BRANCH_IF_FALSE: u16 = OPCODE_BRANCH_IF_FALSE;
pub const NATIVE_OPCODE_VARIANT_MATCH: u16 = OPCODE_VARIANT_MATCH;
pub const NATIVE_OPCODE_BRANCH_IF_TRUE: u16 = OPCODE_BRANCH_IF_TRUE;
pub const NATIVE_OPCODE_GENERAL_LEN: u16 = OPCODE_GENERAL_LEN;
pub const NATIVE_OPCODE_GENERAL_FIND: u16 = OPCODE_GENERAL_FIND;
pub const NATIVE_OPCODE_GENERAL_MID: u16 = OPCODE_GENERAL_MID;
pub const NATIVE_OPCODE_GENERAL_REPLACE: u16 = OPCODE_GENERAL_REPLACE;
pub const NATIVE_OPCODE_GENERAL_TO_STRING: u16 = OPCODE_GENERAL_TO_STRING;
pub const NATIVE_OPCODE_GENERAL_TO_INT: u16 = OPCODE_GENERAL_TO_INT;
pub const NATIVE_OPCODE_GENERAL_TO_FLOAT: u16 = OPCODE_GENERAL_TO_FLOAT;
pub const NATIVE_OPCODE_GENERAL_TO_FIXED: u16 = OPCODE_GENERAL_TO_FIXED;
pub const NATIVE_OPCODE_GENERAL_TO_BYTE: u16 = OPCODE_GENERAL_TO_BYTE;
pub const NATIVE_OPCODE_GENERAL_IS_NUMERIC: u16 = OPCODE_GENERAL_IS_NUMERIC;
pub const NATIVE_OPCODE_GENERAL_IS_EVEN: u16 = OPCODE_GENERAL_IS_EVEN;
pub const NATIVE_OPCODE_GENERAL_IS_ODD: u16 = OPCODE_GENERAL_IS_ODD;
pub const NATIVE_OPCODE_GENERAL_IS_POSITIVE: u16 = OPCODE_GENERAL_IS_POSITIVE;
pub const NATIVE_OPCODE_GENERAL_IS_NEGATIVE: u16 = OPCODE_GENERAL_IS_NEGATIVE;
pub const NATIVE_OPCODE_GENERAL_IS_ZERO: u16 = OPCODE_GENERAL_IS_ZERO;
pub const NATIVE_OPCODE_GENERAL_IS_EMPTY: u16 = OPCODE_GENERAL_IS_EMPTY;
pub const NATIVE_OPCODE_GENERAL_IS_NOT_EMPTY: u16 = OPCODE_GENERAL_IS_NOT_EMPTY;
pub const NATIVE_OPCODE_COLLECTION_GET: u16 = OPCODE_COLLECTION_GET;
pub const NATIVE_OPCODE_COLLECTION_GET_OR: u16 = OPCODE_COLLECTION_GET_OR;
pub const NATIVE_OPCODE_COLLECTION_FIND: u16 = OPCODE_COLLECTION_FIND;
pub const NATIVE_OPCODE_COLLECTION_MID: u16 = OPCODE_COLLECTION_MID;
pub const NATIVE_OPCODE_COLLECTION_REPLACE: u16 = OPCODE_COLLECTION_REPLACE;
pub const NATIVE_OPCODE_COLLECTION_SET: u16 = OPCODE_COLLECTION_SET;
pub const NATIVE_OPCODE_COLLECTION_APPEND: u16 = OPCODE_COLLECTION_APPEND;
pub const NATIVE_OPCODE_COLLECTION_PREPEND: u16 = OPCODE_COLLECTION_PREPEND;
pub const NATIVE_OPCODE_COLLECTION_INSERT: u16 = OPCODE_COLLECTION_INSERT;
pub const NATIVE_OPCODE_COLLECTION_REMOVE_AT: u16 = OPCODE_COLLECTION_REMOVE_AT;
pub const NATIVE_OPCODE_COLLECTION_REMOVE_KEY: u16 = OPCODE_COLLECTION_REMOVE_KEY;
pub const NATIVE_OPCODE_COLLECTION_KEYS: u16 = OPCODE_COLLECTION_KEYS;
pub const NATIVE_OPCODE_COLLECTION_VALUES: u16 = OPCODE_COLLECTION_VALUES;
pub const NATIVE_OPCODE_COLLECTION_HAS_KEY: u16 = OPCODE_COLLECTION_HAS_KEY;
pub const NATIVE_OPCODE_COLLECTION_CONTAINS: u16 = OPCODE_COLLECTION_CONTAINS;
pub const NATIVE_OPCODE_COLLECTION_SUM: u16 = OPCODE_COLLECTION_SUM;
pub const NATIVE_OPCODE_COLLECTION_FOR_EACH: u16 = OPCODE_COLLECTION_FOR_EACH;
pub const NATIVE_OPCODE_COLLECTION_TRANSFORM: u16 = OPCODE_COLLECTION_TRANSFORM;
pub const NATIVE_OPCODE_COLLECTION_FILTER: u16 = OPCODE_COLLECTION_FILTER;
pub const NATIVE_OPCODE_COLLECTION_REDUCE: u16 = OPCODE_COLLECTION_REDUCE;
pub const NATIVE_OPCODE_STRING_TRIM: u16 = OPCODE_STRING_TRIM;
pub const NATIVE_OPCODE_STRING_TRIM_START: u16 = OPCODE_STRING_TRIM_START;
pub const NATIVE_OPCODE_STRING_TRIM_END: u16 = OPCODE_STRING_TRIM_END;
pub const NATIVE_OPCODE_STRING_UPPER: u16 = OPCODE_STRING_UPPER;
pub const NATIVE_OPCODE_STRING_LOWER: u16 = OPCODE_STRING_LOWER;
pub const NATIVE_OPCODE_STRING_CASE_FOLD: u16 = OPCODE_STRING_CASE_FOLD;
pub const NATIVE_OPCODE_STRING_NORMALIZE_NFC: u16 = OPCODE_STRING_NORMALIZE_NFC;
pub const NATIVE_OPCODE_STRING_GRAPHEMES: u16 = OPCODE_STRING_GRAPHEMES;
pub const NATIVE_OPCODE_STRING_STARTS_WITH: u16 = OPCODE_STRING_STARTS_WITH;
pub const NATIVE_OPCODE_STRING_ENDS_WITH: u16 = OPCODE_STRING_ENDS_WITH;
pub const NATIVE_OPCODE_STRING_CONTAINS: u16 = OPCODE_STRING_CONTAINS;
pub const NATIVE_OPCODE_STRING_SPLIT: u16 = OPCODE_STRING_SPLIT;
pub const NATIVE_OPCODE_STRING_JOIN: u16 = OPCODE_STRING_JOIN;
pub const NATIVE_OPCODE_STRING_BYTE_LEN: u16 = OPCODE_STRING_BYTE_LEN;
pub const NATIVE_OPCODE_STRING_REGEX_MATCH: u16 = OPCODE_STRING_REGEX_MATCH;
pub const NATIVE_OPCODE_STRING_REGEX_FIND: u16 = OPCODE_STRING_REGEX_FIND;
pub const NATIVE_OPCODE_STRING_REGEX_REPLACE: u16 = OPCODE_STRING_REGEX_REPLACE;
pub const NATIVE_OPCODE_MATH_PI: u16 = OPCODE_MATH_PI;
pub const NATIVE_OPCODE_MATH_E: u16 = OPCODE_MATH_E;
pub const NATIVE_OPCODE_MATH_ABS: u16 = OPCODE_MATH_ABS;
pub const NATIVE_OPCODE_MATH_SIGN: u16 = OPCODE_MATH_SIGN;
pub const NATIVE_OPCODE_MATH_MIN: u16 = OPCODE_MATH_MIN;
pub const NATIVE_OPCODE_MATH_MAX: u16 = OPCODE_MATH_MAX;
pub const NATIVE_OPCODE_MATH_CLAMP: u16 = OPCODE_MATH_CLAMP;
pub const NATIVE_OPCODE_MATH_FLOOR: u16 = OPCODE_MATH_FLOOR;
pub const NATIVE_OPCODE_MATH_CEIL: u16 = OPCODE_MATH_CEIL;
pub const NATIVE_OPCODE_MATH_ROUND: u16 = OPCODE_MATH_ROUND;
pub const NATIVE_OPCODE_MATH_TRUNC: u16 = OPCODE_MATH_TRUNC;
pub const NATIVE_OPCODE_MATH_SQRT: u16 = OPCODE_MATH_SQRT;
pub const NATIVE_OPCODE_MATH_POW: u16 = OPCODE_MATH_POW;
pub const NATIVE_OPCODE_MATH_EXP: u16 = OPCODE_MATH_EXP;
pub const NATIVE_OPCODE_MATH_LOG: u16 = OPCODE_MATH_LOG;
pub const NATIVE_OPCODE_MATH_LOG10: u16 = OPCODE_MATH_LOG10;
pub const NATIVE_OPCODE_MATH_SIN: u16 = OPCODE_MATH_SIN;
pub const NATIVE_OPCODE_MATH_COS: u16 = OPCODE_MATH_COS;
pub const NATIVE_OPCODE_MATH_TAN: u16 = OPCODE_MATH_TAN;
pub const NATIVE_OPCODE_MATH_ASIN: u16 = OPCODE_MATH_ASIN;
pub const NATIVE_OPCODE_MATH_ACOS: u16 = OPCODE_MATH_ACOS;
pub const NATIVE_OPCODE_MATH_ATAN: u16 = OPCODE_MATH_ATAN;
pub const NATIVE_OPCODE_MATH_ATAN2: u16 = OPCODE_MATH_ATAN2;
pub const NATIVE_OPCODE_MATH_RADIANS: u16 = OPCODE_MATH_RADIANS;
pub const NATIVE_OPCODE_MATH_DEGREES: u16 = OPCODE_MATH_DEGREES;
pub const NATIVE_OPCODE_MATH_IS_FINITE: u16 = OPCODE_MATH_IS_FINITE;
pub const NATIVE_OPCODE_USING_ENTER: u16 = OPCODE_USING_ENTER;
pub const NATIVE_OPCODE_USING_LEAVE: u16 = OPCODE_USING_LEAVE;
pub const NATIVE_OPCODE_CLOSE_RESOURCE: u16 = OPCODE_CLOSE_RESOURCE;

const RESOURCE_FLAG_NATIVE: u32 = 1 << 0;
const RESOURCE_FLAG_STANDARD: u32 = 1 << 1;
const RESOURCE_FLAG_CLOSE_MAY_FAIL: u32 = 1 << 3;
const BUILTIN_FS_CLOSE_FUNCTION_ID: u32 = 0xffff_ff00;

pub fn read_package_exports(path: &Path) -> Result<Vec<BytecodeExport>, String> {
    let package = read_package_bytecode(path)?;
    package_exports(&package).map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

pub fn read_package_info(path: &Path) -> Result<BytecodePackageInfo, String> {
    let package = read_package_bytecode(path)?;
    package_info(&package).map_err(|err| format!("failed to read '{}': {err}", path.display()))
}

fn read_package_bytecode(path: &Path) -> Result<PackageBytecode, String> {
    let package =
        fs::read(path).map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let container = mfp_bytecode_payload(&package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    let package = read_bytecode_package(container.bytecode)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    validate_container_manifest_identity(&container.identity, &package)
        .map_err(|err| format!("failed to read '{}': {err}", path.display()))?;
    Ok(package)
}

struct MfpContainer<'a> {
    identity: MfpIdentity,
    bytecode: &'a [u8],
}

struct MfpIdentity {
    name: String,
    ident: String,
    version: String,
    ident_key: String,
}

fn mfp_bytecode_payload(bytes: &[u8]) -> Result<MfpContainer<'_>, String> {
    const MFP_MAGIC: [u8; 8] = [0x4d, 0x46, 0x50, 0x0d, 0x0a, 0x1a, 0x0a, 0x00];
    if bytes.len() < 26 {
        return Err("package is too small to be a valid .mfp package".to_string());
    }
    if bytes[0..8] != MFP_MAGIC {
        return Err("package does not have the MFP package magic".to_string());
    }
    let container_major = checked_u16_at(bytes, 8)?;
    if container_major != 1 {
        return Err(format!(
            "unsupported MFP container major version {container_major}"
        ));
    }
    let signature_length = checked_u32_at(bytes, 22)? as usize;
    let mut offset = 26usize
        .checked_add(signature_length)
        .ok_or_else(|| "invalid .mfp signature length".to_string())?;
    if offset > bytes.len() {
        return Err("truncated .mfp signature".to_string());
    }

    let name = read_length_prefixed(bytes, &mut offset, "name")?;
    let ident = read_length_prefixed(bytes, &mut offset, "ident")?;
    let version = read_length_prefixed(bytes, &mut offset, "version")?;
    let ident_key = read_length_prefixed(bytes, &mut offset, "identKey")?;
    skip_length_prefixed(bytes, &mut offset, "author")?;
    skip_length_prefixed(bytes, &mut offset, "url")?;
    let bytecode_length = checked_u64_at(bytes, offset)? as usize;
    offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid .mfp bytecode length".to_string())?;
    let end = offset
        .checked_add(bytecode_length)
        .ok_or_else(|| "invalid .mfp bytecode length".to_string())?;
    if end != bytes.len() {
        return Err("invalid .mfp bytecode length".to_string());
    }
    Ok(MfpContainer {
        identity: MfpIdentity {
            name,
            ident,
            version,
            ident_key,
        },
        bytecode: &bytes[offset..end],
    })
}

fn validate_container_manifest_identity(
    identity: &MfpIdentity,
    package: &PackageBytecode,
) -> Result<(), String> {
    let strings = &package.project.strings.values;
    let manifest = &package.project.manifest;
    let manifest_name = string_at(strings, manifest.package_name)?;
    let manifest_ident = string_at(strings, manifest.package_ident)?;
    let manifest_version = string_at(strings, manifest.package_version)?;
    let manifest_ident_key = string_at(strings, manifest.ident_key)?;
    if identity.name != manifest_name
        || identity.ident != manifest_ident
        || identity.version != manifest_version
        || identity.ident_key != manifest_ident_key
    {
        return Err("MFP header identity does not match bytecode manifest identity".to_string());
    }
    Ok(())
}

fn read_bytecode_package(bytes: &[u8]) -> Result<PackageBytecode, String> {
    if bytes.len() < 16 || &bytes[0..4] != b"MFBC" {
        return Err("package payload does not have the MFBC bytecode magic".to_string());
    }
    let major = checked_u16_at(bytes, 4)?;
    if major != 1 {
        return Err(format!("unsupported MFBC major version {major}"));
    }
    let section_count = checked_u32_at(bytes, 12)? as usize;
    let table_end = 16usize
        .checked_add(
            section_count
                .checked_mul(24)
                .ok_or_else(|| "invalid MFBC section table length".to_string())?,
        )
        .ok_or_else(|| "invalid MFBC section table length".to_string())?;
    if table_end > bytes.len() {
        return Err("truncated MFBC section table".to_string());
    }

    let mut sections = HashMap::new();
    for index in 0..section_count {
        let entry = 16 + index * 24;
        let id = checked_u16_at(bytes, entry)?;
        let offset = checked_u64_at(bytes, entry + 8)? as usize;
        let length = checked_u64_at(bytes, entry + 16)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid MFBC section length".to_string())?;
        if end > bytes.len() {
            return Err("truncated MFBC section".to_string());
        }
        sections.insert(id, &bytes[offset..end]);
    }

    let string_values = read_string_pool(
        sections
            .get(&SECTION_STRING_POOL)
            .copied()
            .ok_or_else(|| "MFBC is missing the string pool section".to_string())?,
    )?;
    let strings = StringPool {
        values: string_values,
    };
    let types = read_type_entries(
        sections
            .get(&SECTION_TYPE_TABLE)
            .copied()
            .ok_or_else(|| "MFBC is missing the type table section".to_string())?,
        &strings.values,
    )?;
    let type_names = type_entry_names(&types, &strings.values)?;
    let constants = read_const_pool(
        sections
            .get(&SECTION_CONST_POOL)
            .copied()
            .ok_or_else(|| "MFBC is missing the const pool section".to_string())?,
    )?;
    let functions = read_function_table(
        sections
            .get(&SECTION_FUNCTION_TABLE)
            .copied()
            .ok_or_else(|| "MFBC is missing the function table section".to_string())?,
        sections
            .get(&SECTION_CODE)
            .copied()
            .ok_or_else(|| "MFBC is missing the code section".to_string())?,
        &strings.values,
        &type_names,
    )?;
    let exports = read_export_table(
        sections
            .get(&SECTION_EXPORT_TABLE)
            .copied()
            .ok_or_else(|| "MFBC is missing the export table section".to_string())?,
    )?;
    let resources = match sections.get(&SECTION_RESOURCE_TABLE).copied() {
        Some(section) => read_resource_table(section)?,
        None => ResourceTable::new(),
    };
    let manifest = read_manifest(
        sections
            .get(&SECTION_MANIFEST)
            .copied()
            .ok_or_else(|| "MFBC is missing the manifest section".to_string())?,
    )?;
    let imports = read_import_table(
        sections
            .get(&SECTION_IMPORT_TABLE)
            .copied()
            .ok_or_else(|| "MFBC is missing the import table section".to_string())?,
    )?;
    let abi = read_abi_index(
        sections
            .get(&SECTION_ABI_INDEX)
            .copied()
            .ok_or_else(|| "MFBC is missing the ABI_INDEX section".to_string())?,
    )?;
    validate_abi_index(
        &abi,
        &exports,
        &imports,
        &strings.values,
        &types,
        &constants,
        &functions,
    )?;

    Ok(PackageBytecode {
        project: BytecodeProject {
            strings,
            types,
            constants,
            resources,
            manifest,
            imports,
            abi,
            entry_function: u32::MAX,
            entry_flags: 0,
            functions,
        },
        exports,
    })
}

fn package_exports(package: &PackageBytecode) -> Result<Vec<BytecodeExport>, String> {
    let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
    package
        .exports
        .iter()
        .map(|export| {
            let function = package
                .project
                .functions
                .get(export.function_id as usize)
                .ok_or_else(|| {
                    format!("export references missing function {}", export.function_id)
                })?;
            Ok(BytecodeExport {
                name: string_at(&package.project.strings.values, export.name)?.to_string(),
                kind: export.kind,
                isolated: function.flags & FUNCTION_FLAG_ISOLATED != 0,
                params: function
                    .params
                    .iter()
                    .map(|param| {
                        Ok::<BytecodeExportParam, String>(BytecodeExportParam {
                            type_: type_name(&type_names, param.type_id)?.to_string(),
                            has_default: param.flags & 1 != 0,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                return_type: type_name(&type_names, function.return_type)?.to_string(),
            })
        })
        .collect()
}

fn package_info(package: &PackageBytecode) -> Result<BytecodePackageInfo, String> {
    let strings = &package.project.strings.values;
    if package.project.abi.exports.len() != package.exports.len() {
        return Err("ABI_INDEX export count disagrees with EXPORT_TABLE".to_string());
    }

    let exports = package
        .exports
        .iter()
        .zip(package.project.abi.exports.iter())
        .map(|(export, abi_export)| {
            let name = string_at(strings, export.name)?.to_string();
            if export.name != abi_export.name || export.kind != abi_export.kind {
                return Err(format!(
                    "ABI_INDEX export `{name}` disagrees with EXPORT_TABLE order"
                ));
            }
            Ok(BytecodePackageInfoExport {
                name,
                kind: export.kind,
                sig_hash: hex_hash(&abi_export.sig_hash),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut abi_edges = package
        .project
        .abi
        .dep_edges
        .iter()
        .map(|edge| {
            Ok((
                (
                    string_at(strings, edge.package_name)?.to_string(),
                    string_at(strings, edge.package_ident)?.to_string(),
                ),
                edge,
            ))
        })
        .collect::<Result<HashMap<_, _>, String>>()?;

    let imports = package
        .project
        .imports
        .entries
        .iter()
        .map(|entry| {
            let package_name = string_at(strings, entry.package_name)?.to_string();
            let package_ident = string_at(strings, entry.package_ident)?.to_string();
            let edge = abi_edges.remove(&(package_name.clone(), package_ident.clone()));
            let used_symbols = edge
                .map(|edge| {
                    edge.used_symbols
                        .iter()
                        .map(|symbol| {
                            Ok(BytecodePackageInfoUsedSymbol {
                                name: string_at(strings, symbol.name)?.to_string(),
                                sig_hash: hex_hash(&symbol.sig_hash),
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()
                })
                .transpose()?
                .unwrap_or_default();
            Ok(BytecodePackageInfoImport {
                package_name,
                package_ident,
                version: string_at(strings, entry.version)?.to_string(),
                pin: entry.pin,
                flags: entry.flags,
                used_symbols,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    Ok(BytecodePackageInfo {
        manifest_name: string_at(strings, package.project.manifest.package_name)?.to_string(),
        manifest_ident: string_at(strings, package.project.manifest.package_ident)?.to_string(),
        manifest_version: string_at(strings, package.project.manifest.package_version)?.to_string(),
        manifest_ident_key: string_at(strings, package.project.manifest.ident_key)?.to_string(),
        author: string_at(strings, package.project.manifest.author)?.to_string(),
        url: string_at(strings, package.project.manifest.url)?.to_string(),
        type_count: package.project.types.entries.len(),
        const_count: package.project.constants.entries.len(),
        resource_count: package.project.resources.entries.len(),
        function_count: package.project.functions.len(),
        export_count: package.exports.len(),
        import_count: package.project.imports.entries.len(),
        abi_format_version: ABI_FORMAT_VERSION,
        exports,
        imports,
    })
}

struct DecodedExport {
    name: u32,
    kind: BytecodeExportKind,
    function_id: u32,
}

fn read_string_pool(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut strings = Vec::with_capacity(count);
    for _ in 0..count {
        let length = cursor_u32(bytes, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid string pool entry length".to_string())?;
        if end > bytes.len() {
            return Err("truncated string pool entry".to_string());
        }
        strings.push(
            std::str::from_utf8(&bytes[offset..end])
                .map_err(|_| "string pool entry is not valid UTF-8".to_string())?
                .to_string(),
        );
        offset = end;
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in string pool".to_string());
    }
    Ok(strings)
}

fn read_type_entries(bytes: &[u8], strings: &[String]) -> Result<TypeTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let entries_end = 4usize
        .checked_add(
            count
                .checked_mul(20)
                .ok_or_else(|| "invalid type table length".to_string())?,
        )
        .ok_or_else(|| "invalid type table length".to_string())?;
    if entries_end > bytes.len() {
        return Err("truncated type table".to_string());
    }

    let mut entries = Vec::with_capacity(count);
    let mut ids = HashMap::new();
    for index in 0..count {
        let kind = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let name = cursor_u32(bytes, &mut offset)?;
        let owner_package = cursor_u32(bytes, &mut offset)?;
        let payload_offset = cursor_u32(bytes, &mut offset)? as usize;
        let payload_length = cursor_u32(bytes, &mut offset)? as usize;
        let payload_end = payload_offset
            .checked_add(payload_length)
            .ok_or_else(|| "invalid type payload length".to_string())?;
        if payload_offset < entries_end || payload_end > bytes.len() {
            return Err("invalid type payload bounds".to_string());
        }
        let id = FIRST_TABLE_TYPE_ID + index as u32;
        ids.insert(string_at(strings, name)?.to_string(), id);
        entries.push(TypeEntry {
            kind,
            name,
            owner_package,
            payload: bytes[payload_offset..payload_end].to_vec(),
        });
    }

    Ok(TypeTable { entries, ids })
}

fn type_entry_names(types: &TypeTable, strings: &[String]) -> Result<HashMap<u32, String>, String> {
    let raw = types
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            (
                FIRST_TABLE_TYPE_ID + index as u32,
                (entry.kind, entry.name, entry.payload.clone()),
            )
        })
        .collect::<HashMap<_, _>>();
    let mut decoded = HashMap::new();
    for id in raw.keys().copied().collect::<Vec<_>>() {
        let name = decode_type_name(id, &raw, strings, &mut decoded)?;
        decoded.insert(id, name);
    }
    Ok(decoded)
}

fn decode_type_name(
    id: u32,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
) -> Result<String, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name.to_string());
    }
    if let Some(name) = decoded.get(&id) {
        return Ok(name.clone());
    }
    let Some((kind, name, payload)) = raw.get(&id) else {
        return Err(format!("unknown type id {id}"));
    };
    let decoded_name = match *kind {
        4 => {
            let element = read_payload_type(payload, 0, raw, strings, decoded)?;
            format!("List OF {element}")
        }
        5 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded)?;
            format!("Map OF {key} TO {value}")
        }
        6 => {
            let success = read_payload_type(payload, 0, raw, strings, decoded)?;
            format!("Result OF {success}")
        }
        7 => {
            let message = read_payload_type(payload, 0, raw, strings, decoded)?;
            let output = read_payload_type(payload, 4, raw, strings, decoded)?;
            format!("Thread OF {message} TO {output}")
        }
        8 => decode_function_type(payload, raw, strings, decoded)?,
        9 => {
            let key = read_payload_type(payload, 0, raw, strings, decoded)?;
            let value = read_payload_type(payload, 4, raw, strings, decoded)?;
            format!("MapEntry OF {key} TO {value}")
        }
        _ => string_at(strings, *name)?.to_string(),
    };
    decoded.insert(id, decoded_name.clone());
    Ok(decoded_name)
}

fn decode_function_type(
    payload: &[u8],
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
) -> Result<String, String> {
    let mut offset = 0;
    let isolated = cursor_u32(payload, &mut offset)? != 0;
    let param_count = cursor_u32(payload, &mut offset)? as usize;
    let return_type = cursor_u32(payload, &mut offset)?;
    let returns = decode_type_name(return_type, raw, strings, decoded)?;
    let mut params = Vec::with_capacity(param_count);
    for _ in 0..param_count {
        let param = cursor_u32(payload, &mut offset)?;
        params.push(decode_type_name(param, raw, strings, decoded)?);
    }
    let prefix = if isolated { "ISOLATED FUNC" } else { "FUNC" };
    Ok(format!("{prefix}({}) AS {returns}", params.join(", ")))
}

fn read_payload_type(
    payload: &[u8],
    offset: usize,
    raw: &HashMap<u32, (u16, u32, Vec<u8>)>,
    strings: &[String],
    decoded: &mut HashMap<u32, String>,
) -> Result<String, String> {
    let id = checked_u32_at(payload, offset)?;
    decode_type_name(id, raw, strings, decoded)
}

fn primitive_type_name(id: u32) -> Option<&'static str> {
    match id {
        TYPE_NOTHING => Some("Nothing"),
        TYPE_BOOLEAN => Some("Boolean"),
        TYPE_INTEGER => Some("Integer"),
        TYPE_FLOAT => Some("Float"),
        TYPE_FIXED => Some("Fixed"),
        TYPE_STRING => Some("String"),
        TYPE_BYTE => Some("Byte"),
        TYPE_ERROR => Some("Error"),
        TYPE_TERMINAL_SIZE => Some("TerminalSize"),
        TYPE_FILE_HANDLE => Some("File"),
        _ => None,
    }
}

fn read_function_table(
    bytes: &[u8],
    code: &[u8],
    strings: &[String],
    _types: &HashMap<u32, String>,
) -> Result<Vec<Function>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut functions = Vec::with_capacity(count);
    for _ in 0..count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = cursor_u16(bytes, &mut offset)?;
        let flags = cursor_u16(bytes, &mut offset)?;
        let param_count = cursor_u32(bytes, &mut offset)? as usize;
        let return_type = cursor_u32(bytes, &mut offset)?;
        let register_count = cursor_u32(bytes, &mut offset)? as usize;
        let code_offset = cursor_u64(bytes, &mut offset)? as usize;
        let code_length = cursor_u64(bytes, &mut offset)? as usize;
        let _source_map = cursor_u32(bytes, &mut offset)?;
        let cleanup_count = cursor_u32(bytes, &mut offset)? as usize;
        let _cleanup_offset = cursor_u64(bytes, &mut offset)?;

        let mut params = Vec::with_capacity(param_count);
        for _ in 0..param_count {
            let param_name = cursor_u32(bytes, &mut offset)?;
            let _ = string_at(strings, param_name)?;
            let param_type = cursor_u32(bytes, &mut offset)?;
            let param_flags = cursor_u32(bytes, &mut offset)?;
            let default_const = cursor_u32(bytes, &mut offset)?;
            params.push(Param {
                name: param_name,
                type_id: param_type,
                flags: param_flags,
                default_const,
            });
        }
        let mut registers = Vec::with_capacity(register_count);
        for _ in 0..register_count {
            registers.push(Register {
                type_id: cursor_u32(bytes, &mut offset)?,
                flags: cursor_u32(bytes, &mut offset)?,
            });
        }
        let mut cleanups = Vec::with_capacity(cleanup_count);
        for _ in 0..cleanup_count {
            cleanups.push(Cleanup {
                id: cursor_u32(bytes, &mut offset)?,
                start_pc: cursor_u32(bytes, &mut offset)?,
                end_pc: cursor_u32(bytes, &mut offset)?,
                resource_register: cursor_u32(bytes, &mut offset)?,
                close_function_id: cursor_u32(bytes, &mut offset)?,
            });
        }

        let code_end = code_offset
            .checked_add(code_length)
            .ok_or_else(|| "invalid function code length".to_string())?;
        if code_end > code.len() {
            return Err("truncated function code".to_string());
        }
        functions.push(Function {
            name,
            kind,
            flags,
            return_type,
            params,
            registers,
            code: read_function_code(&code[code_offset..code_end])?,
            cleanups,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in function table".to_string());
    }
    Ok(functions)
}

fn read_function_code(bytes: &[u8]) -> Result<Vec<Instruction>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut instructions = Vec::with_capacity(count);
    for _ in 0..count {
        let opcode = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let operand_count = cursor_u16(bytes, &mut offset)? as usize;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let mut operands = Vec::with_capacity(operand_count);
        for _ in 0..operand_count {
            operands.push(cursor_u32(bytes, &mut offset)?);
        }
        instructions.push(Instruction { opcode, operands });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in function code".to_string());
    }
    Ok(instructions)
}

fn read_const_pool(bytes: &[u8]) -> Result<ConstPool, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let kind = cursor_u16(bytes, &mut offset)?;
        let _reserved = cursor_u16(bytes, &mut offset)?;
        let length = cursor_u32(bytes, &mut offset)? as usize;
        let end = offset
            .checked_add(length)
            .ok_or_else(|| "invalid const payload length".to_string())?;
        if end > bytes.len() {
            return Err("truncated const payload".to_string());
        }
        entries.push(ConstEntry {
            kind,
            payload: bytes[offset..end].to_vec(),
        });
        offset = end;
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in const pool".to_string());
    }
    Ok(ConstPool { entries })
}

fn read_manifest(bytes: &[u8]) -> Result<BytecodeManifest, String> {
    let mut offset = 0;
    let manifest = BytecodeManifest {
        package_name: cursor_u32(bytes, &mut offset)?,
        package_ident: cursor_u32(bytes, &mut offset)?,
        package_version: cursor_u32(bytes, &mut offset)?,
        ident_key: cursor_u32(bytes, &mut offset)?,
        author: cursor_u32(bytes, &mut offset)?,
        url: cursor_u32(bytes, &mut offset)?,
    };
    if offset != bytes.len() {
        return Err("invalid trailing bytes in manifest".to_string());
    }
    Ok(manifest)
}

fn read_import_table(bytes: &[u8]) -> Result<ImportTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(ImportEntry {
            package_name: cursor_u32(bytes, &mut offset)?,
            package_ident: cursor_u32(bytes, &mut offset)?,
            version: cursor_u32(bytes, &mut offset)?,
            pin: match cursor_u8(bytes, &mut offset)? {
                0 => false,
                1 => true,
                other => return Err(format!("unsupported import pin value {other}")),
            },
            flags: cursor_u32(bytes, &mut offset)?,
            used_symbols: read_used_symbols(bytes, &mut offset)?,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in import table".to_string());
    }
    Ok(ImportTable { entries })
}

fn read_used_symbols(bytes: &[u8], offset: &mut usize) -> Result<Vec<AbiUsedSymbol>, String> {
    let count = cursor_u32(bytes, offset)? as usize;
    let mut symbols = Vec::with_capacity(count);
    for _ in 0..count {
        symbols.push(AbiUsedSymbol {
            name: cursor_u32(bytes, offset)?,
            sig_hash: cursor_hash(bytes, offset)?,
        });
    }
    Ok(symbols)
}

fn read_resource_table(bytes: &[u8]) -> Result<ResourceTable, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        entries.push(ResourceEntry {
            type_id: cursor_u32(bytes, &mut offset)?,
            close_function_id: cursor_u32(bytes, &mut offset)?,
            flags: cursor_u32(bytes, &mut offset)?,
        });
    }
    Ok(ResourceTable { entries })
}

fn read_export_table(bytes: &[u8]) -> Result<Vec<DecodedExport>, String> {
    let mut offset = 0;
    let count = cursor_u32(bytes, &mut offset)? as usize;
    let mut exports = Vec::with_capacity(count);
    for _ in 0..count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = match cursor_u16(bytes, &mut offset)? {
            1 => BytecodeExportKind::Func,
            2 => BytecodeExportKind::Sub,
            other => return Err(format!("unsupported export kind {other}")),
        };
        let _flags = cursor_u16(bytes, &mut offset)?;
        let function_id = cursor_u32(bytes, &mut offset)?;
        exports.push(DecodedExport {
            name,
            kind,
            function_id,
        });
    }
    if offset != bytes.len() {
        return Err("invalid trailing bytes in export table".to_string());
    }
    Ok(exports)
}

fn read_abi_index(bytes: &[u8]) -> Result<AbiIndex, String> {
    let mut offset = 0;
    let version = cursor_u16(bytes, &mut offset)?;
    if version != ABI_FORMAT_VERSION {
        return Err(format!("unsupported ABI_INDEX format version {version}"));
    }
    let _reserved = cursor_u16(bytes, &mut offset)?;

    let export_count = cursor_u32(bytes, &mut offset)? as usize;
    let mut exports = Vec::with_capacity(export_count);
    for _ in 0..export_count {
        let name = cursor_u32(bytes, &mut offset)?;
        let kind = match cursor_u16(bytes, &mut offset)? {
            1 => BytecodeExportKind::Func,
            2 => BytecodeExportKind::Sub,
            other => return Err(format!("unsupported ABI_INDEX export kind {other}")),
        };
        let sig_hash = cursor_hash(bytes, &mut offset)?;
        exports.push(AbiExport {
            name,
            kind,
            sig_hash,
        });
    }

    let edge_count = cursor_u32(bytes, &mut offset)? as usize;
    let mut dep_edges = Vec::with_capacity(edge_count);
    for _ in 0..edge_count {
        let package_name = cursor_u32(bytes, &mut offset)?;
        let package_ident = cursor_u32(bytes, &mut offset)?;
        let version_request = cursor_u32(bytes, &mut offset)?;
        let pin = match cursor_u8(bytes, &mut offset)? {
            0 => false,
            1 => true,
            other => return Err(format!("unsupported ABI_INDEX dep pin value {other}")),
        };
        let used_count = cursor_u32(bytes, &mut offset)? as usize;
        let mut used_symbols = Vec::with_capacity(used_count);
        for _ in 0..used_count {
            used_symbols.push(AbiUsedSymbol {
                name: cursor_u32(bytes, &mut offset)?,
                sig_hash: cursor_hash(bytes, &mut offset)?,
            });
        }
        dep_edges.push(AbiDepEdge {
            package_name,
            package_ident,
            version_request,
            pin,
            used_symbols,
        });
    }

    if offset != bytes.len() {
        return Err("invalid trailing bytes in ABI_INDEX".to_string());
    }

    Ok(AbiIndex { exports, dep_edges })
}

fn validate_abi_index(
    abi: &AbiIndex,
    exports: &[DecodedExport],
    imports: &ImportTable,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
    functions: &[Function],
) -> Result<(), String> {
    if abi.exports.len() != exports.len() {
        return Err("ABI_INDEX export count disagrees with EXPORT_TABLE".to_string());
    }

    for (export, abi_export) in exports.iter().zip(abi.exports.iter()) {
        if export.name != abi_export.name || export.kind != abi_export.kind {
            let name = string_at(strings, export.name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX export `{name}` disagrees with EXPORT_TABLE order"
            ));
        }
        let Some(function) = functions.get(export.function_id as usize) else {
            return Err(format!(
                "export references missing function {}",
                export.function_id
            ));
        };
        let expected = function_sig_hash(function, export.kind, strings, types, constants)?;
        if abi_export.sig_hash != expected {
            let name = string_at(strings, export.name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX export `{name}` sigHash disagrees with bytecode (required {}, provided {})",
                hex_hash(&expected),
                hex_hash(&abi_export.sig_hash)
            ));
        }
    }

    let import_names = imports
        .entries
        .iter()
        .map(|entry| {
            Ok::<(String, String), String>((
                string_at(strings, entry.package_name)?.to_string(),
                string_at(strings, entry.package_ident)?.to_string(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let edge_names = abi
        .dep_edges
        .iter()
        .map(|edge| {
            Ok::<(String, String), String>((
                string_at(strings, edge.package_name)?.to_string(),
                string_at(strings, edge.package_ident)?.to_string(),
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    if sorted_pairs(import_names) != sorted_pairs(edge_names) {
        return Err("ABI_INDEX dependency edges disagree with IMPORT_TABLE entries".to_string());
    }

    for import in &imports.entries {
        let Some(edge) = abi.dep_edges.iter().find(|edge| {
            edge.package_name == import.package_name && edge.package_ident == import.package_ident
        }) else {
            continue;
        };
        if edge.version_request != import.version || edge.pin != import.pin {
            let name = string_at(strings, import.package_name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX dependency edge `{name}` disagrees with IMPORT_TABLE request"
            ));
        }
        if edge.used_symbols.len() != import.used_symbols.len()
            || edge
                .used_symbols
                .iter()
                .zip(import.used_symbols.iter())
                .any(|(a, b)| a.name != b.name || a.sig_hash != b.sig_hash)
        {
            let name = string_at(strings, import.package_name).unwrap_or("<invalid>");
            return Err(format!(
                "ABI_INDEX dependency edge `{name}` disagrees with IMPORT_TABLE used symbols"
            ));
        }
    }

    Ok(())
}

fn type_name(types: &HashMap<u32, String>, id: u32) -> Result<&str, String> {
    if let Some(name) = primitive_type_name(id) {
        return Ok(name);
    }
    types
        .get(&id)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown type id {id}"))
}

fn string_at(strings: &[String], id: u32) -> Result<&str, String> {
    strings
        .get(id as usize)
        .map(String::as_str)
        .ok_or_else(|| format!("unknown string id {id}"))
}

fn function_sig_hash(
    function: &Function,
    export_kind: BytecodeExportKind,
    strings: &[String],
    types: &TypeTable,
    constants: &ConstPool,
) -> Result<[u8; ABI_HASH_LEN], String> {
    let mut serializer = AbiSerializer::new(strings, types, constants);
    serializer.bytes.extend_from_slice(b"MFBABI\0");
    serializer.put_u16(ABI_FORMAT_VERSION);
    serializer.put_str("function");
    serializer.put_u16(match export_kind {
        BytecodeExportKind::Func => 1,
        BytecodeExportKind::Sub => 2,
    });
    serializer.put_u16(function.flags & (FUNCTION_FLAG_ISOLATED | FUNCTION_FLAG_SUB));
    serializer.put_u32(function.params.len() as u32);
    for param in &function.params {
        serializer.serialize_type(param.type_id)?;
        if param.default_const == u32::MAX {
            serializer.put_u8(0);
        } else {
            serializer.put_u8(1);
            serializer.serialize_const(param.default_const)?;
        }
    }
    serializer.serialize_type(function.return_type)?;
    Ok(hash_bytes(&serializer.bytes))
}

struct AbiSerializer<'a> {
    strings: &'a [String],
    types: &'a TypeTable,
    constants: &'a ConstPool,
    bytes: Vec<u8>,
    type_refs: HashMap<u32, u32>,
    next_ref: u32,
}

impl<'a> AbiSerializer<'a> {
    fn new(strings: &'a [String], types: &'a TypeTable, constants: &'a ConstPool) -> Self {
        Self {
            strings,
            types,
            constants,
            bytes: Vec::new(),
            type_refs: HashMap::new(),
            next_ref: 0,
        }
    }

    fn serialize_type(&mut self, id: u32) -> Result<(), String> {
        if let Some(primitive) = primitive_type_name(id) {
            self.put_u8(1);
            self.put_u32(id);
            self.put_str(primitive);
            return Ok(());
        }

        if let Some(ref_id) = self.type_refs.get(&id).copied() {
            self.put_u8(2);
            self.put_u32(ref_id);
            return Ok(());
        }

        let entry = self
            .types
            .entries
            .get((id - FIRST_TABLE_TYPE_ID) as usize)
            .ok_or_else(|| format!("unknown type id {id}"))?;
        let ref_id = self.next_ref;
        self.next_ref = self
            .next_ref
            .checked_add(1)
            .ok_or_else(|| "ABI type graph has too many nodes".to_string())?;
        self.type_refs.insert(id, ref_id);

        self.put_u8(3);
        self.put_u32(ref_id);
        self.put_u16(entry.kind);
        match entry.kind {
            1 => self.serialize_record_type(entry),
            2 => self.serialize_union_type(entry),
            3 => self.serialize_enum_type(entry),
            4 => {
                self.put_str("list");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            5 => {
                self.put_str("map");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)
            }
            6 => {
                self.put_str("result");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)
            }
            7 => {
                self.put_str("thread");
                self.serialize_type(checked_u32_at(&entry.payload, 0)?)?;
                self.serialize_type(checked_u32_at(&entry.payload, 4)?)
            }
            8 => self.serialize_function_type(entry),
            _ => {
                self.put_str("opaque");
                self.put_str(string_at(self.strings, entry.name)?);
                Ok(())
            }
        }
    }

    fn serialize_record_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("record");
        let mut offset = 0;
        let field_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(field_count);
        for _ in 0..field_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let type_id = cursor_u32(&entry.payload, &mut offset)?;
            let _visibility = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.serialize_type(type_id)?;
        }
        Ok(())
    }

    fn serialize_union_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("union");
        let mut offset = 0;
        let variant_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(variant_count);
        for _ in 0..variant_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            let field_count = cursor_u32(&entry.payload, &mut offset)?;
            self.put_u32(field_count);
            for _ in 0..field_count {
                let field_name = cursor_u32(&entry.payload, &mut offset)?;
                let field_type = cursor_u32(&entry.payload, &mut offset)?;
                self.put_str(string_at(self.strings, field_name)?);
                self.serialize_type(field_type)?;
            }
        }
        Ok(())
    }

    fn serialize_enum_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("enum");
        let mut offset = 0;
        let member_count = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(member_count);
        for _ in 0..member_count {
            let name = cursor_u32(&entry.payload, &mut offset)?;
            let ordinal = cursor_u32(&entry.payload, &mut offset)?;
            self.put_str(string_at(self.strings, name)?);
            self.put_u32(ordinal);
        }
        Ok(())
    }

    fn serialize_function_type(&mut self, entry: &TypeEntry) -> Result<(), String> {
        self.put_str("function-type");
        let mut offset = 0;
        let isolated = cursor_u32(&entry.payload, &mut offset)?;
        let param_count = cursor_u32(&entry.payload, &mut offset)?;
        let return_type = cursor_u32(&entry.payload, &mut offset)?;
        self.put_u32(isolated);
        self.put_u32(param_count);
        self.serialize_type(return_type)?;
        for _ in 0..param_count {
            self.serialize_type(cursor_u32(&entry.payload, &mut offset)?)?;
        }
        Ok(())
    }

    fn serialize_const(&mut self, id: u32) -> Result<(), String> {
        let constant = self
            .constants
            .entries
            .get(id as usize)
            .ok_or_else(|| format!("unknown const id {id}"))?;
        self.put_u16(constant.kind);
        match constant.kind {
            6 => {
                let string_id = checked_u32_at(&constant.payload, 0)?;
                self.put_str(string_at(self.strings, string_id)?);
            }
            _ => {
                self.put_u32(constant.payload.len() as u32);
                self.bytes.extend_from_slice(&constant.payload);
            }
        }
        Ok(())
    }

    fn put_u8(&mut self, value: u8) {
        self.bytes.push(value);
    }

    fn put_u16(&mut self, value: u16) {
        put_u16(&mut self.bytes, value);
    }

    fn put_u32(&mut self, value: u32) {
        put_u32(&mut self.bytes, value);
    }

    fn put_str(&mut self, value: &str) {
        put_bytes(&mut self.bytes, value.as_bytes());
    }
}

fn hash_bytes(bytes: &[u8]) -> [u8; ABI_HASH_LEN] {
    let digest = Sha256::digest(bytes);
    let mut hash = [0; ABI_HASH_LEN];
    hash.copy_from_slice(&digest);
    hash
}

fn sorted_strings(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}

fn sorted_pairs(mut values: Vec<(String, String)>) -> Vec<(String, String)> {
    values.sort();
    values
}

fn hex_hash(hash: &[u8; ABI_HASH_LEN]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn skip_length_prefixed(bytes: &[u8], offset: &mut usize, field: &str) -> Result<(), String> {
    let length = cursor_u32(bytes, offset)? as usize;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    if end > bytes.len() {
        return Err(format!("truncated .mfp {field}"));
    }
    *offset = end;
    Ok(())
}

fn read_length_prefixed(
    bytes: &[u8],
    offset: &mut usize,
    field: &str,
) -> Result<String, String> {
    let length = cursor_u32(bytes, offset)? as usize;
    let end = offset
        .checked_add(length)
        .ok_or_else(|| format!("invalid .mfp {field} length"))?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| format!("truncated .mfp {field}"))?;
    *offset = end;
    String::from_utf8(value.to_vec()).map_err(|_| format!(".mfp {field} is not valid UTF-8"))
}

fn cursor_u8(bytes: &[u8], offset: &mut usize) -> Result<u8, String> {
    let value = *bytes
        .get(*offset)
        .ok_or_else(|| "truncated bytecode".to_string())?;
    *offset = offset
        .checked_add(1)
        .ok_or_else(|| "invalid u8 offset".to_string())?;
    Ok(value)
}

fn cursor_u16(bytes: &[u8], offset: &mut usize) -> Result<u16, String> {
    let value = checked_u16_at(bytes, *offset)?;
    *offset = offset
        .checked_add(2)
        .ok_or_else(|| "invalid u16 offset".to_string())?;
    Ok(value)
}

fn cursor_hash(bytes: &[u8], offset: &mut usize) -> Result<[u8; ABI_HASH_LEN], String> {
    let end = offset
        .checked_add(ABI_HASH_LEN)
        .ok_or_else(|| "invalid hash offset".to_string())?;
    let value = bytes
        .get(*offset..end)
        .ok_or_else(|| "truncated ABI hash".to_string())?;
    let mut hash = [0; ABI_HASH_LEN];
    hash.copy_from_slice(value);
    *offset = end;
    Ok(hash)
}

fn cursor_u32(bytes: &[u8], offset: &mut usize) -> Result<u32, String> {
    let value = checked_u32_at(bytes, *offset)?;
    *offset = offset
        .checked_add(4)
        .ok_or_else(|| "invalid u32 offset".to_string())?;
    Ok(value)
}

fn cursor_u64(bytes: &[u8], offset: &mut usize) -> Result<u64, String> {
    let value = checked_u64_at(bytes, *offset)?;
    *offset = offset
        .checked_add(8)
        .ok_or_else(|| "invalid u64 offset".to_string())?;
    Ok(value)
}

fn checked_u16_at(bytes: &[u8], offset: usize) -> Result<u16, String> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| "truncated bytecode".to_string())?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn checked_u32_at(bytes: &[u8], offset: usize) -> Result<u32, String> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| "truncated bytecode".to_string())?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn checked_u64_at(bytes: &[u8], offset: usize) -> Result<u64, String> {
    let value = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| "truncated bytecode".to_string())?;
    Ok(u64::from_le_bytes([
        value[0], value[1], value[2], value[3], value[4], value[5], value[6], value[7],
    ]))
}

pub fn native_program(ir: &IrProject) -> Result<NativeProgram, String> {
    let metadata = BytecodeMetadata::new(ir.name.clone(), "native".to_string());
    let project = lower_project(ir, &metadata)?;
    native_program_from_project(project)
}

pub fn native_program_with_packages(
    ir: &IrProject,
    packages: &[PathBuf],
) -> Result<NativeProgram, String> {
    let metadata = BytecodeMetadata::new(ir.name.clone(), "native".to_string());
    let project = lower_merged_project(ir, &metadata, packages)?;
    native_program_from_project(project)
}

fn native_program_from_project(project: BytecodeProject) -> Result<NativeProgram, String> {
    if project.entry_function == u32::MAX {
        return Err("native executable output requires an executable entry point".to_string());
    }

    let mut strings = HashMap::new();
    for (index, value) in project.strings.values.iter().enumerate() {
        strings.insert(index as u32, value.clone());
    }

    let mut result_success_types = HashMap::new();
    for (index, entry) in project.types.entries.iter().enumerate() {
        if entry.kind == 6 && entry.payload.len() >= 4 {
            result_success_types.insert(index as u32, read_u32(&entry.payload, 0));
        }
    }

    let constants = project
        .constants
        .entries
        .iter()
        .map(|constant| native_const(constant, &strings))
        .collect::<Result<Vec<_>, _>>()?;

    let functions = project
        .functions
        .iter()
        .map(|function| native_function(function, &result_success_types))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(NativeProgram {
        entry_function: project.entry_function,
        entry_returns_integer: project.entry_flags & (1 << 2) != 0,
        types: native_type_layouts_from_bytecode(&project.types, &project.strings)?,
        functions,
        constants,
        imports: native_imports(&project.functions),
    })
}

fn native_imports(functions: &[Function]) -> Vec<NativeImport> {
    let mut needs_open = false;
    let mut needs_close = false;
    for function in functions {
        for instruction in &function.code {
            needs_open |= matches!(instruction.opcode, OPCODE_IO_OPEN | OPCODE_FS_OPEN);
            needs_close |= matches!(
                instruction.opcode,
                OPCODE_IO_CLOSE | OPCODE_FS_CLOSE | OPCODE_CLOSE_RESOURCE
            );
        }
    }

    let mut imports = Vec::new();
    if needs_open {
        imports.push(NativeImport {
            symbol: "mfb_io_open",
            kind: NativeImportKind::RuntimeHelper,
        });
        imports.push(NativeImport {
            symbol: "_open",
            kind: NativeImportKind::LibSystem,
        });
    }
    if needs_close {
        imports.push(NativeImport {
            symbol: "mfb_io_close",
            kind: NativeImportKind::RuntimeHelper,
        });
        imports.push(NativeImport {
            symbol: "_close",
            kind: NativeImportKind::LibSystem,
        });
    }
    if needs_open || needs_close {
        imports.push(NativeImport {
            symbol: "___error",
            kind: NativeImportKind::LibSystem,
        });
    }
    imports
}

fn native_function(
    function: &Function,
    result_success_types: &HashMap<u32, u32>,
) -> Result<NativeFunction, String> {
    if function.kind != FUNCTION_BYTECODE {
        return Err(format!(
            "native output does not support function kind {}",
            function.kind
        ));
    }

    Ok(NativeFunction {
        param_count: function.params.len(),
        registers: function
            .registers
            .iter()
            .map(|register| NativeRegister {
                type_id: register.type_id,
                type_: native_type(register.type_id, result_success_types),
            })
            .collect(),
        code: function
            .code
            .iter()
            .map(|instruction| NativeInstruction {
                opcode: instruction.opcode,
                operands: instruction.operands.clone(),
            })
            .collect(),
    })
}

fn native_type(type_id: u32, result_success_types: &HashMap<u32, u32>) -> NativeType {
    match type_id {
        TYPE_NOTHING => NativeType::Nothing,
        TYPE_BOOLEAN => NativeType::Boolean,
        TYPE_BYTE => NativeType::Byte,
        TYPE_INTEGER => NativeType::Integer,
        TYPE_FLOAT => NativeType::Float,
        TYPE_FIXED => NativeType::Fixed,
        TYPE_STRING => NativeType::String,
        TYPE_FILE_HANDLE => NativeType::FileHandle,
        id if result_success_types.contains_key(&id) => NativeType::Result,
        _ => NativeType::Other,
    }
}

fn native_const(
    constant: &ConstEntry,
    strings: &HashMap<u32, String>,
) -> Result<NativeConst, String> {
    match constant.kind {
        1 => Ok(NativeConst::Nothing),
        2 => Ok(NativeConst::Boolean(
            constant.payload.first().copied().unwrap_or(0) != 0,
        )),
        3 => Ok(NativeConst::Integer(read_i64(&constant.payload, 0))),
        4 => Ok(NativeConst::Float(f64::from_bits(read_u64(
            &constant.payload,
            0,
        )))),
        5 => Ok(NativeConst::Fixed(read_i64(&constant.payload, 0))),
        6 => {
            let string_id = read_u32(&constant.payload, 0);
            let value = strings.get(&string_id).cloned().ok_or_else(|| {
                format!("String constant references missing string pool entry {string_id}")
            })?;
            Ok(NativeConst::String(value))
        }
        7 => Ok(NativeConst::Byte(
            constant.payload.first().copied().unwrap_or(0),
        )),
        _ => Ok(NativeConst::Other),
    }
}

fn native_type_layouts_from_bytecode(
    types: &TypeTable,
    strings: &StringPool,
) -> Result<NativeTypeLayouts, String> {
    let mut records = HashMap::new();
    let mut unions = HashMap::new();

    if let (Some(code), Some(message)) = (string_id(strings, "code"), string_id(strings, "message"))
    {
        let fields = vec![
            NativeFieldLayout {
                name: code,
                offset_slots: 0,
            },
            NativeFieldLayout {
                name: message,
                offset_slots: 1,
            },
        ];
        records.insert(
            TYPE_ERROR,
            NativeRecordLayout {
                size_slots: 2,
                fields: fields
                    .iter()
                    .cloned()
                    .map(|field| (field.name, field))
                    .collect(),
                ordered_fields: fields,
            },
        );
    }

    if let (Some(columns), Some(rows)) = (string_id(strings, "columns"), string_id(strings, "rows"))
    {
        let fields = vec![
            NativeFieldLayout {
                name: columns,
                offset_slots: 0,
            },
            NativeFieldLayout {
                name: rows,
                offset_slots: 1,
            },
        ];
        records.insert(
            TYPE_TERMINAL_SIZE,
            NativeRecordLayout {
                size_slots: 2,
                fields: fields
                    .iter()
                    .cloned()
                    .map(|field| (field.name, field))
                    .collect(),
                ordered_fields: fields,
            },
        );
    }

    for (index, entry) in types.entries.iter().enumerate() {
        let type_id = FIRST_TABLE_TYPE_ID + index as u32;
        match entry.kind {
            1 if !entry.payload.is_empty() => {
                let fields = native_record_field_layouts(&entry.payload, 0)?;
                records.insert(
                    type_id,
                    NativeRecordLayout {
                        size_slots: fields.len(),
                        fields: fields
                            .iter()
                            .cloned()
                            .map(|field| (field.name, field))
                            .collect(),
                        ordered_fields: fields,
                    },
                );
            }
            2 if !entry.payload.is_empty() => {
                let mut offset = 0;
                let variant_count = cursor_u32(&entry.payload, &mut offset)? as usize;
                let mut variants = HashMap::new();
                let mut max_payload_slots = 0usize;
                for _ in 0..variant_count {
                    let variant_name = cursor_u32(&entry.payload, &mut offset)?;
                    let field_count = cursor_u32(&entry.payload, &mut offset)? as usize;
                    max_payload_slots = max_payload_slots.max(field_count);
                    let mut fields = Vec::with_capacity(field_count);
                    for index in 0..field_count {
                        let field_name = cursor_u32(&entry.payload, &mut offset)?;
                        let _field_type = cursor_u32(&entry.payload, &mut offset)?;
                        fields.push(NativeFieldLayout {
                            name: field_name,
                            offset_slots: 1 + index,
                        });
                    }
                    variants.insert(variant_name, NativeVariantLayout { fields });
                }
                unions.insert(
                    type_id,
                    NativeUnionLayout {
                        variants,
                        size_slots: 1 + max_payload_slots,
                    },
                );
            }
            _ => {}
        }
    }

    Ok(NativeTypeLayouts { records, unions })
}

fn native_record_field_layouts(
    payload: &[u8],
    base_offset_slots: usize,
) -> Result<Vec<NativeFieldLayout>, String> {
    let mut offset = 0;
    let field_count = cursor_u32(payload, &mut offset)? as usize;
    let mut fields = Vec::with_capacity(field_count);
    for index in 0..field_count {
        fields.push(NativeFieldLayout {
            name: cursor_u32(payload, &mut offset)?,
            offset_slots: base_offset_slots + index,
        });
        let _field_type = cursor_u32(payload, &mut offset)?;
        let _visibility = cursor_u32(payload, &mut offset)?;
    }
    Ok(fields)
}

fn string_id(strings: &StringPool, value: &str) -> Option<u32> {
    strings
        .values
        .iter()
        .position(|existing| existing == value)
        .map(|index| index as u32)
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    let mut value = [0; 4];
    value.copy_from_slice(&bytes[offset..offset + 4]);
    u32::from_le_bytes(value)
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}

fn read_i64(bytes: &[u8], offset: usize) -> i64 {
    let mut value = [0; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    i64::from_le_bytes(value)
}

struct BytecodeProject {
    strings: StringPool,
    types: TypeTable,
    constants: ConstPool,
    resources: ResourceTable,
    manifest: BytecodeManifest,
    imports: ImportTable,
    abi: AbiIndex,
    entry_function: u32,
    entry_flags: u32,
    functions: Vec<Function>,
}

struct PackageBytecode {
    project: BytecodeProject,
    exports: Vec<DecodedExport>,
}

struct BytecodeManifest {
    package_name: u32,
    package_ident: u32,
    package_version: u32,
    ident_key: u32,
    author: u32,
    url: u32,
}

struct StringPool {
    values: Vec<String>,
}

struct TypeTable {
    entries: Vec<TypeEntry>,
    ids: HashMap<String, u32>,
}

struct TypeEntry {
    kind: u16,
    name: u32,
    owner_package: u32,
    payload: Vec<u8>,
}

struct ConstPool {
    entries: Vec<ConstEntry>,
}

struct ConstEntry {
    kind: u16,
    payload: Vec<u8>,
}

struct ResourceTable {
    entries: Vec<ResourceEntry>,
}

struct ResourceEntry {
    type_id: u32,
    close_function_id: u32,
    flags: u32,
}

struct ImportTable {
    entries: Vec<ImportEntry>,
}

struct ImportEntry {
    package_name: u32,
    package_ident: u32,
    version: u32,
    pin: bool,
    flags: u32,
    used_symbols: Vec<AbiUsedSymbol>,
}

#[derive(Clone)]
struct AbiIndex {
    exports: Vec<AbiExport>,
    dep_edges: Vec<AbiDepEdge>,
}

#[derive(Clone)]
struct AbiExport {
    name: u32,
    kind: BytecodeExportKind,
    sig_hash: [u8; ABI_HASH_LEN],
}

#[derive(Clone)]
struct AbiDepEdge {
    package_name: u32,
    package_ident: u32,
    version_request: u32,
    pin: bool,
    used_symbols: Vec<AbiUsedSymbol>,
}

#[derive(Clone)]
struct AbiUsedSymbol {
    name: u32,
    sig_hash: [u8; ABI_HASH_LEN],
}

struct TypeModel {
    records: HashMap<String, RecordModel>,
    variants: HashMap<String, VariantModel>,
    enums: HashMap<String, EnumModel>,
}

#[derive(Clone)]
struct RecordModel {
    fields: Vec<FieldModel>,
}

#[derive(Clone)]
struct VariantModel {
    union_name: String,
    fields: Vec<FieldModel>,
}

struct EnumModel {
    members: Vec<String>,
}

#[derive(Clone)]
struct FieldModel {
    name: String,
    type_name: String,
}

impl TypeModel {
    fn new(ir: &IrProject) -> Self {
        let mut records = HashMap::new();
        let mut variants = HashMap::new();
        let mut enums = HashMap::new();
        records.insert(
            "Error".to_string(),
            RecordModel {
                fields: vec![
                    FieldModel {
                        name: "code".to_string(),
                        type_name: "Integer".to_string(),
                    },
                    FieldModel {
                        name: "message".to_string(),
                        type_name: "String".to_string(),
                    },
                ],
            },
        );
        records.insert(
            "TerminalSize".to_string(),
            RecordModel {
                fields: vec![
                    FieldModel {
                        name: "columns".to_string(),
                        type_name: "Integer".to_string(),
                    },
                    FieldModel {
                        name: "rows".to_string(),
                        type_name: "Integer".to_string(),
                    },
                ],
            },
        );

        for ir_type in &ir.types {
            match ir_type.kind.as_str() {
                "type" => {
                    records.insert(
                        ir_type.name.clone(),
                        RecordModel {
                            fields: ir_type.fields.iter().map(FieldModel::from_ir).collect(),
                        },
                    );
                }
                "union" => {
                    for variant in &ir_type.variants {
                        variants.insert(
                            variant.name.clone(),
                            VariantModel {
                                union_name: ir_type.name.clone(),
                                fields: variant.fields.iter().map(FieldModel::from_ir).collect(),
                            },
                        );
                    }
                }
                "enum" => {
                    enums.insert(
                        ir_type.name.clone(),
                        EnumModel {
                            members: ir_type
                                .members
                                .iter()
                                .map(|member| member.name.clone())
                                .collect(),
                        },
                    );
                }
                _ => {}
            }
        }

        Self {
            records,
            variants,
            enums,
        }
    }
}

impl FieldModel {
    fn from_ir(field: &crate::ir::IrField) -> Self {
        Self {
            name: field.name.clone(),
            type_name: field.type_.clone(),
        }
    }
}

struct Function {
    name: u32,
    kind: u16,
    flags: u16,
    return_type: u32,
    params: Vec<Param>,
    registers: Vec<Register>,
    code: Vec<Instruction>,
    cleanups: Vec<Cleanup>,
}

fn is_exported_function(function: &Function) -> bool {
    function.kind == FUNCTION_BYTECODE && function.flags & FUNCTION_FLAG_PRIVATE == 0
}

struct Param {
    name: u32,
    type_id: u32,
    flags: u32,
    default_const: u32,
}

struct Register {
    type_id: u32,
    flags: u32,
}

struct Instruction {
    opcode: u16,
    operands: Vec<u32>,
}

struct Cleanup {
    id: u32,
    start_pc: u32,
    end_pc: u32,
    resource_register: u32,
    close_function_id: u32,
}

fn lower_project(ir: &IrProject, metadata: &BytecodeMetadata) -> Result<BytecodeProject, String> {
    lower_project_with_external_functions(
        ir,
        metadata,
        &HashMap::new(),
        &HashMap::new(),
        &HashMap::new(),
    )
}

fn lower_merged_project(
    ir: &IrProject,
    metadata: &BytecodeMetadata,
    package_paths: &[PathBuf],
) -> Result<BytecodeProject, String> {
    let packages = package_paths
        .iter()
        .map(|path| read_package_bytecode(path))
        .collect::<Result<Vec<_>, _>>()?;
    let app_function_count = ir.functions.len() as u32;
    let mut external_function_ids = HashMap::new();
    let mut external_function_returns = HashMap::new();
    let mut external_function_abi_hashes = HashMap::new();
    let mut next_function_id = app_function_count;
    for package in &packages {
        let package_name = string_at(
            &package.project.strings.values,
            package.project.manifest.package_name,
        )?;
        let type_names = type_entry_names(&package.project.types, &package.project.strings.values)?;
        for (export, abi_export) in package.exports.iter().zip(package.project.abi.exports.iter()) {
            let function = package
                .project
                .functions
                .get(export.function_id as usize)
                .ok_or_else(|| {
                    format!("export references missing function {}", export.function_id)
                })?;
            let export_name = string_at(&package.project.strings.values, export.name)?;
            if export.name != abi_export.name || export.kind != abi_export.kind {
                return Err(format!(
                    "ABI_INDEX export `{export_name}` disagrees with EXPORT_TABLE order"
                ));
            }
            external_function_ids.insert(
                format!("{package_name}.{export_name}"),
                next_function_id + export.function_id,
            );
            external_function_returns.insert(
                format!("{package_name}.{export_name}"),
                type_name(&type_names, function.return_type)?.to_string(),
            );
            external_function_abi_hashes
                .insert(format!("{package_name}.{export_name}"), abi_export.sig_hash);
        }
        next_function_id = next_function_id
            .checked_add(package.project.functions.len() as u32)
            .ok_or_else(|| "merged bytecode has too many functions".to_string())?;
    }

    let mut project = lower_project_with_external_functions(
        ir,
        metadata,
        &external_function_ids,
        &external_function_returns,
        &external_function_abi_hashes,
    )?;
    for package in packages {
        merge_package_bytecode(&mut project, package)?;
    }
    project.abi = AbiIndex::from_project(
        &project.strings,
        &project.types,
        &project.constants,
        &project.imports,
        &project.functions,
    )?;
    Ok(project)
}

fn lower_project_with_external_functions(
    ir: &IrProject,
    metadata: &BytecodeMetadata,
    external_function_ids: &HashMap<String, u32>,
    external_function_returns: &HashMap<String, String>,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
) -> Result<BytecodeProject, String> {
    let mut strings = StringPool::new();
    let ident = if metadata.ident.is_empty() {
        &metadata.name
    } else {
        &metadata.ident
    };
    let manifest = BytecodeManifest {
        package_name: strings.intern(&metadata.name),
        package_ident: strings.intern(ident),
        package_version: strings.intern(&metadata.version),
        ident_key: strings.intern(&metadata.ident_key),
        author: strings.intern(&metadata.author),
        url: strings.intern(&metadata.url),
    };
    let mut imports = ImportTable::from_metadata(&mut strings, metadata);

    let mut types = TypeTable::new();
    for ir_type in &ir.types {
        types.reserve_source_type(&mut strings, &metadata.name, ir_type);
    }
    types.populate_source_payloads(&mut strings, &ir.types)?;
    let mut resources = ResourceTable::new();
    if ir_uses_resource_type(ir) {
        resources.add_standard_file(&mut types, &mut strings);
    }
    let type_model = TypeModel::new(ir);

    let mut constants = ConstPool::new();
    let mut function_ids = HashMap::new();
    let mut function_return_types = HashMap::new();
    let mut function_return_type_names = HashMap::new();
    for (index, function) in ir.functions.iter().enumerate() {
        function_ids.insert(function.name.clone(), index as u32);
        let return_type = types.type_id(&mut strings, &function.returns);
        function_return_types.insert(function.name.clone(), return_type);
        function_return_type_names.insert(function.name.clone(), function.returns.clone());
    }
    for (name, id) in external_function_ids {
        function_ids.insert(name.clone(), *id);
    }
    for (name, return_type_name) in external_function_returns {
        let return_type = types.type_id(&mut strings, return_type_name);
        function_return_types.insert(name.clone(), return_type);
        function_return_type_names.insert(name.clone(), return_type_name.clone());
    }

    let mut functions = Vec::new();
    let mut used_imported_functions = HashSet::new();
    for function in &ir.functions {
        functions.push(lower_function(
            function,
            &mut strings,
            &mut types,
            &mut constants,
            &function_ids,
            &function_return_types,
            &function_return_type_names,
            &type_model,
            external_function_abi_hashes,
            &mut used_imported_functions,
        )?);
    }
    imports.record_used_imports(
        &mut strings,
        &used_imported_functions,
        external_function_abi_hashes,
    );
    let abi = AbiIndex::from_project(&strings, &types, &constants, &imports, &functions)?;

    let (entry_function, entry_flags) = if let Some(entry) = &ir.entry {
        let function_id = *function_ids.get(&entry.name).ok_or_else(|| {
            format!(
                "entry function `{}` was not lowered to bytecode",
                entry.name
            )
        })?;
        let mut flags = 1;
        if entry.accepts_args {
            flags |= 1 << 1;
        }
        if entry.returns == "Integer" {
            flags |= 1 << 2;
        }
        (function_id, flags)
    } else {
        (u32::MAX, 0)
    };

    Ok(BytecodeProject {
        strings,
        types,
        constants,
        resources,
        manifest,
        imports,
        abi,
        entry_function,
        entry_flags,
        functions,
    })
}

struct MergeMap {
    strings: HashMap<u32, u32>,
    types: HashMap<u32, u32>,
    constants: HashMap<u32, u32>,
    functions: HashMap<u32, u32>,
}

fn merge_package_bytecode(
    project: &mut BytecodeProject,
    package: PackageBytecode,
) -> Result<(), String> {
    let package_name = string_at(
        &package.project.strings.values,
        package.project.manifest.package_name,
    )?
    .to_string();
    let mut map = MergeMap {
        strings: HashMap::new(),
        types: HashMap::new(),
        constants: HashMap::new(),
        functions: HashMap::new(),
    };

    for (id, value) in package.project.strings.values.iter().enumerate() {
        let merged = project.strings.intern(value);
        map.strings.insert(id as u32, merged);
    }

    for primitive in [
        TYPE_NOTHING,
        TYPE_BOOLEAN,
        TYPE_INTEGER,
        TYPE_FLOAT,
        TYPE_FIXED,
        TYPE_STRING,
        TYPE_BYTE,
        TYPE_ERROR,
        TYPE_TERMINAL_SIZE,
        TYPE_FILE_HANDLE,
    ] {
        map.types.insert(primitive, primitive);
    }

    for (index, entry) in package.project.types.entries.iter().enumerate() {
        let old_id = FIRST_TABLE_TYPE_ID + index as u32;
        let name = remap_string(&map, entry.name)?;
        let owner_package = remap_string(&map, entry.owner_package)?;
        let new_id = FIRST_TABLE_TYPE_ID + project.types.entries.len() as u32;
        let visible_name = merged_type_key(&project.strings, name, owner_package, &package_name)?;
        project.types.ids.insert(visible_name, new_id);
        project.types.entries.push(TypeEntry {
            kind: entry.kind,
            name,
            owner_package,
            payload: Vec::new(),
        });
        map.types.insert(old_id, new_id);
    }

    let type_start = project.types.entries.len() - package.project.types.entries.len();
    for (offset, entry) in package.project.types.entries.iter().enumerate() {
        project.types.entries[type_start + offset].payload =
            remap_type_payload(entry.kind, &entry.payload, &map)?;
    }

    for (index, constant) in package.project.constants.entries.iter().enumerate() {
        let new_id = project.constants.entries.len() as u32;
        project.constants.entries.push(ConstEntry {
            kind: constant.kind,
            payload: remap_const_payload(constant.kind, &constant.payload, &map)?,
        });
        map.constants.insert(index as u32, new_id);
    }

    let function_start = project.functions.len() as u32;
    for index in 0..package.project.functions.len() {
        map.functions
            .insert(index as u32, function_start + index as u32);
    }

    for function in package.project.functions {
        project.functions.push(remap_function(function, &map)?);
    }

    for resource in package.project.resources.entries {
        project.resources.entries.push(ResourceEntry {
            type_id: remap_type(&map, resource.type_id)?,
            close_function_id: remap_function_id_if_needed(&map, resource.close_function_id)?,
            flags: resource.flags,
        });
    }

    Ok(())
}

fn merged_type_key(
    strings: &StringPool,
    name: u32,
    owner_package: u32,
    package_name: &str,
) -> Result<String, String> {
    let name = string_at(&strings.values, name)?;
    let owner = string_at(&strings.values, owner_package).unwrap_or("");
    if owner.is_empty() {
        Ok(name.to_string())
    } else {
        Ok(format!("{package_name}.{name}"))
    }
}

fn remap_function(function: Function, map: &MergeMap) -> Result<Function, String> {
    Ok(Function {
        name: remap_string(map, function.name)?,
        kind: function.kind,
        flags: function.flags,
        return_type: remap_type(map, function.return_type)?,
        params: function
            .params
            .into_iter()
            .map(|param| {
                Ok(Param {
                    name: remap_string(map, param.name)?,
                    type_id: remap_type(map, param.type_id)?,
                    flags: param.flags,
                    default_const: remap_const_id_if_needed(map, param.default_const)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        registers: function
            .registers
            .into_iter()
            .map(|register| {
                Ok(Register {
                    type_id: remap_type(map, register.type_id)?,
                    flags: register.flags,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
        code: function
            .code
            .into_iter()
            .map(|instruction| remap_instruction(instruction, map))
            .collect::<Result<Vec<_>, _>>()?,
        cleanups: function
            .cleanups
            .into_iter()
            .map(|cleanup| {
                Ok(Cleanup {
                    id: cleanup.id,
                    start_pc: cleanup.start_pc,
                    end_pc: cleanup.end_pc,
                    resource_register: cleanup.resource_register,
                    close_function_id: remap_function_id_if_needed(map, cleanup.close_function_id)?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    })
}

fn remap_instruction(mut instruction: Instruction, map: &MergeMap) -> Result<Instruction, String> {
    match instruction.opcode {
        OPCODE_LOAD_CONST => {
            remap_operand(&mut instruction, 1, |value| remap_const(map, value))?;
        }
        OPCODE_LOAD_DEFAULT => {
            remap_operand(&mut instruction, 1, |value| remap_type(map, value))?;
        }
        OPCODE_LOAD_FUNCTION => {
            remap_operand(&mut instruction, 1, |value| remap_function_id(map, value))?;
        }
        OPCODE_CALL_RESULT => {
            remap_operand(&mut instruction, 1, |value| remap_function_id(map, value))?;
        }
        OPCODE_CONSTRUCT_RECORD | OPCODE_CONSTRUCT_LIST | OPCODE_CONSTRUCT_MAP => {
            remap_operand(&mut instruction, 1, |value| remap_type(map, value))?;
        }
        OPCODE_COLLECTION_ITER_BEGIN => {
            remap_operand(&mut instruction, 2, |value| remap_type(map, value))?;
        }
        OPCODE_LOAD_MAP_ENTRY_FIELD => {
            remap_operand(&mut instruction, 2, |value| remap_type(map, value))?;
        }
        OPCODE_CONSTRUCT_VARIANT => {
            remap_operand(&mut instruction, 1, |value| remap_type(map, value))?;
            remap_operand(&mut instruction, 2, |value| remap_string(map, value))?;
        }
        OPCODE_LOAD_ENUM_MEMBER => {
            remap_operand(&mut instruction, 1, |value| remap_type(map, value))?;
            remap_operand(&mut instruction, 2, |value| remap_string(map, value))?;
        }
        OPCODE_LOAD_FIELD | OPCODE_VARIANT_MATCH => {
            remap_operand(&mut instruction, 2, |value| remap_string(map, value))?;
        }
        OPCODE_USING_ENTER | OPCODE_CLOSE_RESOURCE => {
            remap_operand(&mut instruction, 1, |value| {
                remap_function_id_if_needed(map, value)
            })?;
        }
        _ => {}
    }
    Ok(instruction)
}

fn remap_operand<F>(instruction: &mut Instruction, index: usize, remap: F) -> Result<(), String>
where
    F: FnOnce(u32) -> Result<u32, String>,
{
    let operand = instruction
        .operands
        .get_mut(index)
        .ok_or_else(|| format!("opcode {} is missing operand {index}", instruction.opcode))?;
    *operand = remap(*operand)?;
    Ok(())
}

fn remap_type_payload(kind: u16, payload: &[u8], map: &MergeMap) -> Result<Vec<u8>, String> {
    if payload.is_empty() {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    match kind {
        1 => {
            let mut offset = 0;
            let field_count = cursor_u32(payload, &mut offset)?;
            put_u32(&mut out, field_count);
            for _ in 0..field_count {
                put_u32(
                    &mut out,
                    remap_string(map, cursor_u32(payload, &mut offset)?)?,
                );
                put_u32(
                    &mut out,
                    remap_type(map, cursor_u32(payload, &mut offset)?)?,
                );
                put_u32(&mut out, cursor_u32(payload, &mut offset)?);
            }
        }
        2 => {
            let mut offset = 0;
            let variant_count = cursor_u32(payload, &mut offset)?;
            put_u32(&mut out, variant_count);
            for _ in 0..variant_count {
                put_u32(
                    &mut out,
                    remap_string(map, cursor_u32(payload, &mut offset)?)?,
                );
                let field_count = cursor_u32(payload, &mut offset)?;
                put_u32(&mut out, field_count);
                for _ in 0..field_count {
                    put_u32(
                        &mut out,
                        remap_string(map, cursor_u32(payload, &mut offset)?)?,
                    );
                    put_u32(
                        &mut out,
                        remap_type(map, cursor_u32(payload, &mut offset)?)?,
                    );
                }
            }
        }
        3 => {
            let mut offset = 0;
            let member_count = cursor_u32(payload, &mut offset)?;
            put_u32(&mut out, member_count);
            for _ in 0..member_count {
                put_u32(
                    &mut out,
                    remap_string(map, cursor_u32(payload, &mut offset)?)?,
                );
                put_u32(&mut out, cursor_u32(payload, &mut offset)?);
            }
        }
        4 | 6 => {
            put_u32(&mut out, remap_type(map, checked_u32_at(payload, 0)?)?);
        }
        5 | 9 => {
            put_u32(&mut out, remap_type(map, checked_u32_at(payload, 0)?)?);
            put_u32(&mut out, remap_type(map, checked_u32_at(payload, 4)?)?);
        }
        8 => {
            let mut offset = 0;
            put_u32(&mut out, cursor_u32(payload, &mut offset)?);
            let param_count = cursor_u32(payload, &mut offset)?;
            put_u32(&mut out, param_count);
            put_u32(
                &mut out,
                remap_type(map, cursor_u32(payload, &mut offset)?)?,
            );
            for _ in 0..param_count {
                put_u32(
                    &mut out,
                    remap_type(map, cursor_u32(payload, &mut offset)?)?,
                );
            }
        }
        _ => out.extend_from_slice(payload),
    }
    Ok(out)
}

fn remap_const_payload(kind: u16, payload: &[u8], map: &MergeMap) -> Result<Vec<u8>, String> {
    if kind == 6 {
        let mut out = Vec::new();
        put_u32(&mut out, remap_string(map, checked_u32_at(payload, 0)?)?);
        Ok(out)
    } else {
        Ok(payload.to_vec())
    }
}

fn remap_string(map: &MergeMap, id: u32) -> Result<u32, String> {
    map.strings
        .get(&id)
        .copied()
        .ok_or_else(|| format!("merged bytecode references unknown string id {id}"))
}

fn remap_type(map: &MergeMap, id: u32) -> Result<u32, String> {
    map.types
        .get(&id)
        .copied()
        .ok_or_else(|| format!("merged bytecode references unknown type id {id}"))
}

fn remap_const(map: &MergeMap, id: u32) -> Result<u32, String> {
    map.constants
        .get(&id)
        .copied()
        .ok_or_else(|| format!("merged bytecode references unknown const id {id}"))
}

fn remap_function_id(map: &MergeMap, id: u32) -> Result<u32, String> {
    map.functions
        .get(&id)
        .copied()
        .ok_or_else(|| format!("merged bytecode references unknown function id {id}"))
}

fn remap_const_id_if_needed(map: &MergeMap, id: u32) -> Result<u32, String> {
    if id == u32::MAX {
        Ok(id)
    } else {
        remap_const(map, id)
    }
}

fn remap_function_id_if_needed(map: &MergeMap, id: u32) -> Result<u32, String> {
    if id == u32::MAX || id >= 0xffff_0000 {
        Ok(id)
    } else {
        remap_function_id(map, id)
    }
}

fn ir_uses_resource_type(ir: &IrProject) -> bool {
    ir.functions.iter().any(|function| {
        function
            .params
            .iter()
            .any(|param| is_resource_type_name(&param.type_))
            || is_resource_type_name(&function.returns)
            || ops_use_resource_type(&function.body)
    })
}

fn ops_use_resource_type(ops: &[IrOp]) -> bool {
    ops.iter().any(|op| match op {
        IrOp::Bind { type_, value, .. } => {
            is_resource_type_name(type_) || value.as_ref().is_some_and(value_uses_resource_type)
        }
        IrOp::Assign { value, .. } | IrOp::Eval { value } => value_uses_resource_type(value),
        IrOp::Return { value } => value.as_ref().is_some_and(value_uses_resource_type),
        IrOp::Fail { error } => value_uses_resource_type(error),
        IrOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_resource_type(condition)
                || ops_use_resource_type(then_body)
                || ops_use_resource_type(else_body)
        }
        IrOp::Match { value, cases } => {
            value_uses_resource_type(value)
                || cases.iter().any(|case| ops_use_resource_type(&case.body))
        }
        IrOp::ForEach {
            type_,
            iterable,
            body,
            ..
        } => {
            is_resource_type_name(type_)
                || value_uses_resource_type(iterable)
                || ops_use_resource_type(body)
        }
        IrOp::Using {
            type_, value, body, ..
        } => {
            is_resource_type_name(type_)
                || value_uses_resource_type(value)
                || ops_use_resource_type(body)
        }
    })
}

fn value_uses_resource_type(value: &IrValue) -> bool {
    match value {
        IrValue::Const { type_, .. }
        | IrValue::FunctionRef { type_, .. }
        | IrValue::Constructor { type_, .. }
        | IrValue::ListLiteral { type_, .. }
        | IrValue::MapLiteral { type_, .. } => is_resource_type_name(type_),
        IrValue::Call { target, args } => {
            builtins::call_return_type_name(target).is_some_and(is_resource_type_name)
                || args.iter().any(value_uses_resource_type)
        }
        IrValue::MemberAccess { target, .. } => value_uses_resource_type(target),
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_resource_type(target)
                || updates
                    .iter()
                    .any(|update| value_uses_resource_type(&update.value))
        }
        IrValue::Binary { left, right, .. } => {
            value_uses_resource_type(left) || value_uses_resource_type(right)
        }
        IrValue::Unary { operand, .. } => value_uses_resource_type(operand),
        IrValue::Local(_) => false,
    }
}

fn is_resource_type_name(type_name: &str) -> bool {
    builtins::is_resource_type(type_name)
}

fn lower_function(
    function: &IrFunction,
    strings: &mut StringPool,
    types: &mut TypeTable,
    constants: &mut ConstPool,
    function_ids: &HashMap<String, u32>,
    function_return_types: &HashMap<String, u32>,
    function_return_type_names: &HashMap<String, String>,
    type_model: &TypeModel,
    external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    used_imported_functions: &mut HashSet<String>,
) -> Result<Function, String> {
    let mut builder = FunctionBuilder::new(
        strings,
        types,
        constants,
        function_ids,
        function_return_types,
        function_return_type_names,
        type_model,
        external_function_abi_hashes,
        used_imported_functions,
    );
    let mut params = Vec::new();
    let mut locals = HashMap::new();

    for param in &function.params {
        let type_id = builder.type_id(&param.type_);
        let register = builder.add_register(
            type_id,
            REGISTER_FLAG_PARAMETER | REGISTER_FLAG_INITIALIZED_AT_ENTRY,
        );
        locals.insert(
            param.name.clone(),
            ValueSlot {
                register,
                type_name: param.type_.clone(),
            },
        );
        params.push(Param {
            name: builder.strings.intern(&param.name),
            type_id,
            flags: if param.default.is_some() { 1 } else { 0 },
            default_const: match &param.default {
                Some(default) => builder.const_id(default)?,
                None => u32::MAX,
            },
        });
    }

    for op in &function.body {
        builder.lower_op(op, &mut locals)?;
    }

    if !builder.ends_with_return() {
        let nothing = builder.add_register(TYPE_NOTHING, 0);
        builder.push(OPCODE_LOAD_DEFAULT, vec![nothing, TYPE_NOTHING]);
        builder.push(OPCODE_RETURN_OK, vec![nothing]);
    }

    let mut flags = if function.visibility == "export" {
        0
    } else {
        FUNCTION_FLAG_PRIVATE
    };
    if function.kind == "sub" {
        flags |= FUNCTION_FLAG_SUB | FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.returns == "Nothing" {
        flags |= FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.isolated {
        flags |= FUNCTION_FLAG_ISOLATED;
    }

    Ok(Function {
        name: builder.strings.intern(&function.name),
        kind: FUNCTION_BYTECODE,
        flags,
        return_type: builder.type_id(&function.returns),
        params,
        registers: builder.registers,
        code: builder.code,
        cleanups: builder.cleanups,
    })
}

struct FunctionBuilder<'a> {
    strings: &'a mut StringPool,
    types: &'a mut TypeTable,
    constants: &'a mut ConstPool,
    function_ids: &'a HashMap<String, u32>,
    function_return_types: &'a HashMap<String, u32>,
    function_return_type_names: &'a HashMap<String, String>,
    type_model: &'a TypeModel,
    external_function_abi_hashes: &'a HashMap<String, [u8; ABI_HASH_LEN]>,
    used_imported_functions: &'a mut HashSet<String>,
    registers: Vec<Register>,
    code: Vec<Instruction>,
    cleanups: Vec<Cleanup>,
    next_cleanup_id: u32,
}

#[derive(Clone)]
pub(crate) struct ValueSlot {
    pub(crate) register: u32,
    pub(crate) type_name: String,
}

pub(crate) trait BuiltinCallLowerer {
    fn lower_value(
        &mut self,
        value: &IrValue,
        locals: &HashMap<String, ValueSlot>,
    ) -> Result<ValueSlot, String>;
    fn type_id(&mut self, type_name: &str) -> u32;
    fn add_register(&mut self, type_id: u32, flags: u32) -> u32;
    fn push(&mut self, opcode: u16, operands: Vec<u32>);
    fn push_string_const(&mut self, value: &str) -> Result<ValueSlot, String>;
    fn push_integer_const(&mut self, value: i64) -> Result<ValueSlot, String>;
}

impl<'a> FunctionBuilder<'a> {
    fn new(
        strings: &'a mut StringPool,
        types: &'a mut TypeTable,
        constants: &'a mut ConstPool,
        function_ids: &'a HashMap<String, u32>,
        function_return_types: &'a HashMap<String, u32>,
        function_return_type_names: &'a HashMap<String, String>,
        type_model: &'a TypeModel,
        external_function_abi_hashes: &'a HashMap<String, [u8; ABI_HASH_LEN]>,
        used_imported_functions: &'a mut HashSet<String>,
    ) -> Self {
        Self {
            strings,
            types,
            constants,
            function_ids,
            function_return_types,
            function_return_type_names,
            type_model,
            external_function_abi_hashes,
            used_imported_functions,
            registers: Vec::new(),
            code: Vec::new(),
            cleanups: Vec::new(),
            next_cleanup_id: 0,
        }
    }

    fn lower_op(
        &mut self,
        op: &IrOp,
        locals: &mut HashMap<String, ValueSlot>,
    ) -> Result<(), String> {
        match op {
            IrOp::Bind {
                mutable,
                name,
                type_,
                value,
            } => {
                let type_id = self.type_id(type_);
                let mut flags = 0;
                if *mutable {
                    flags |= REGISTER_FLAG_MUTABLE_LOCAL_CELL;
                }
                let register = self.add_register(type_id, flags);
                locals.insert(
                    name.clone(),
                    ValueSlot {
                        register,
                        type_name: type_.clone(),
                    },
                );
                if let Some(value) = value {
                    let value_register = self.lower_value(value, locals)?;
                    self.push_move_like(type_id, register, value_register.register);
                } else {
                    self.push(OPCODE_LOAD_DEFAULT, vec![register, type_id]);
                }
                Ok(())
            }
            IrOp::Assign { name, value } => {
                let target = locals
                    .get(name)
                    .cloned()
                    .ok_or_else(|| format!("IR assigns unknown local `{name}`"))?;
                let value = self.lower_value(value, locals)?;
                self.push_move_like(
                    self.registers[target.register as usize].type_id,
                    target.register,
                    value.register,
                );
                Ok(())
            }
            IrOp::Return { value } => {
                let register = match value {
                    Some(value) => self.lower_value(value, locals)?.register,
                    None => {
                        let register = self.add_register(TYPE_NOTHING, 0);
                        self.push(OPCODE_LOAD_DEFAULT, vec![register, TYPE_NOTHING]);
                        register
                    }
                };
                self.push(OPCODE_RETURN_OK, vec![register]);
                Ok(())
            }
            IrOp::Fail { error } => {
                let register = self.lower_value(error, locals)?.register;
                self.push(OPCODE_RETURN_ERR, vec![register]);
                Ok(())
            }
            IrOp::Eval { value } => {
                self.lower_value(value, locals)?;
                Ok(())
            }
            IrOp::If {
                condition,
                then_body,
                else_body,
            } => {
                let condition = self.lower_value(condition, locals)?;
                let else_jump = self.push_jump(OPCODE_BRANCH_IF_FALSE, vec![condition.register]);
                let mut then_locals = locals.clone();
                self.lower_ops(then_body, &mut then_locals)?;
                let end_jump = self.push_jump(OPCODE_BRANCH, Vec::new());
                self.patch_jump(else_jump);
                let mut else_locals = locals.clone();
                self.lower_ops(else_body, &mut else_locals)?;
                self.patch_jump(end_jump);
                Ok(())
            }
            IrOp::Match { value, cases } => {
                let matched = self.lower_value(value, locals)?;
                let end_jumps = self.lower_match_cases(&matched, cases, locals)?;
                for jump in end_jumps {
                    self.patch_jump(jump);
                }
                Ok(())
            }
            IrOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                let iterable = self.lower_value(iterable, locals)?;
                let item_type = self.type_id(type_);
                let item_register = self.add_register(item_type, 0);
                let iterator_register = self.add_register(TYPE_INTEGER, 0);
                let collection_type = self.type_id(&iterable.type_name);
                let end_jump = self.push_jump(
                    OPCODE_COLLECTION_ITER_BEGIN,
                    vec![
                        iterator_register,
                        item_register,
                        collection_type,
                        iterable.register,
                    ],
                );
                let loop_pc = self.code.len() as u32;
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    ValueSlot {
                        register: item_register,
                        type_name: type_.clone(),
                    },
                );
                self.lower_ops(body, &mut nested)?;
                self.push(
                    OPCODE_COLLECTION_ITER_NEXT,
                    vec![
                        iterator_register,
                        item_register,
                        collection_type,
                        iterable.register,
                        loop_pc,
                    ],
                );
                self.patch_jump(end_jump);
                Ok(())
            }
            IrOp::Using {
                name,
                type_,
                close,
                value,
                body,
            } => {
                let type_id = self.type_id(type_);
                let value = self.lower_value(value, locals)?;
                let register = self.add_register(type_id, REGISTER_FLAG_RESOURCE);
                self.push_move_like(type_id, register, value.register);
                let close_function_id = close_function_id(close)?;
                let cleanup_id = self.next_cleanup_id;
                self.next_cleanup_id += 1;
                let enter_pc = self.code.len() as u32;
                self.push(
                    OPCODE_USING_ENTER,
                    vec![register, close_function_id, cleanup_id],
                );
                let mut nested = locals.clone();
                nested.insert(
                    name.clone(),
                    ValueSlot {
                        register,
                        type_name: type_.clone(),
                    },
                );
                self.lower_ops(body, &mut nested)?;
                self.push(OPCODE_CLOSE_RESOURCE, vec![register, close_function_id]);
                let leave_pc = self.code.len() as u32;
                self.push(OPCODE_USING_LEAVE, vec![cleanup_id]);
                self.cleanups.push(Cleanup {
                    id: cleanup_id,
                    start_pc: enter_pc,
                    end_pc: leave_pc,
                    resource_register: register,
                    close_function_id,
                });
                Ok(())
            }
        }
    }

    fn lower_ops(
        &mut self,
        ops: &[IrOp],
        locals: &mut HashMap<String, ValueSlot>,
    ) -> Result<(), String> {
        for op in ops {
            self.lower_op(op, locals)?;
        }
        Ok(())
    }

    fn lower_match_cases(
        &mut self,
        matched: &ValueSlot,
        cases: &[crate::ir::IrMatchCase],
        locals: &HashMap<String, ValueSlot>,
    ) -> Result<Vec<usize>, String> {
        let mut end_jumps = Vec::new();
        for case in cases {
            let next_jump = match &case.pattern {
                IrMatchPattern::Else => None,
                IrMatchPattern::Value(pattern) => {
                    let matched_case = self.lower_match_pattern(matched, pattern, locals)?;
                    Some(self.push_jump(OPCODE_BRANCH_IF_FALSE, vec![matched_case.register]))
                }
            };
            let mut case_locals = locals.clone();
            self.lower_ops(&case.body, &mut case_locals)?;
            end_jumps.push(self.push_jump(OPCODE_BRANCH, Vec::new()));
            if let Some(jump) = next_jump {
                self.patch_jump(jump);
            }
        }
        Ok(end_jumps)
    }

    fn lower_match_pattern(
        &mut self,
        matched: &ValueSlot,
        pattern: &IrValue,
        locals: &HashMap<String, ValueSlot>,
    ) -> Result<ValueSlot, String> {
        if let IrValue::Local(name) = pattern {
            if self.type_model.variants.contains_key(name) {
                let dst = self.add_register(TYPE_BOOLEAN, 0);
                let variant_name = self.strings.intern(name);
                self.push(
                    OPCODE_VARIANT_MATCH,
                    vec![dst, matched.register, variant_name],
                );
                return Ok(ValueSlot {
                    register: dst,
                    type_name: "Boolean".to_string(),
                });
            }
        }

        let pattern = self.lower_value(pattern, locals)?;
        Ok(self.push_equal(matched.register, pattern.register))
    }

    fn lower_value(
        &mut self,
        value: &IrValue,
        locals: &HashMap<String, ValueSlot>,
    ) -> Result<ValueSlot, String> {
        match value {
            IrValue::Const { type_, .. } => {
                let type_id = self.type_id(type_);
                let register = self.add_register(type_id, 0);
                if type_ == "Nothing" {
                    self.push(OPCODE_LOAD_DEFAULT, vec![register, type_id]);
                } else {
                    let const_id = self.const_id(value)?;
                    self.push(OPCODE_LOAD_CONST, vec![register, const_id]);
                }
                Ok(ValueSlot {
                    register,
                    type_name: type_.clone(),
                })
            }
            IrValue::Local(name) => locals
                .get(name)
                .cloned()
                .ok_or_else(|| format!("IR references unknown local `{name}`")),
            IrValue::FunctionRef { name, type_ } => {
                let function_id = self
                    .function_ids
                    .get(name)
                    .copied()
                    .or_else(|| builtins::general::builtin_function_id_for_type(name, type_))
                    .ok_or_else(|| format!("IR references unknown function `{name}`"))?;
                let type_id = self.type_id(type_);
                let register = self.add_register(type_id, 0);
                self.push(OPCODE_LOAD_FUNCTION, vec![register, function_id]);
                Ok(ValueSlot {
                    register,
                    type_name: type_.clone(),
                })
            }
            IrValue::Call { target, args } => {
                if builtins::general::is_general_call(target) {
                    return builtins::general::lower_bytecode_call(self, target, args, locals);
                }
                if builtins::strings::is_strings_call(target) {
                    return builtins::strings::lower_bytecode_call(self, target, args, locals);
                }
                if builtins::math::is_math_call(target) {
                    return builtins::math::lower_bytecode_call(self, target, args, locals);
                }
                if builtins::fs::is_fs_call(target) {
                    return builtins::fs::lower_bytecode_call(self, target, args, locals);
                }
                if builtins::io::is_io_call(target) {
                    return builtins::io::lower_bytecode_call(self, target, args, locals);
                }
                if builtins::thread::is_thread_call(target) {
                    return builtins::thread::lower_bytecode_call(self, target, args, locals);
                }

                if let Some(callee) = locals.get(target) {
                    if callee.type_name.starts_with("FUNC(") {
                        let callee = callee.clone();
                        let return_type_name = function_return_from_type(&callee.type_name)
                            .ok_or_else(|| {
                                format!(
                                    "function value `{target}` has invalid type `{}`",
                                    callee.type_name
                                )
                            })?;
                        let return_type = self.type_id(&return_type_name);
                        let result_type = self.types.result_type(self.strings, return_type);
                        let result_register = self.add_register(result_type, 0);
                        let mut operands = vec![result_register, callee.register];
                        for arg in args {
                            operands.push(self.lower_value(arg, locals)?.register);
                        }
                        self.push(OPCODE_CALL_VALUE_RESULT, operands);
                        let value_register = self.add_register(return_type, 0);
                        self.push(OPCODE_UNWRAP_RESULT, vec![value_register, result_register]);
                        return Ok(ValueSlot {
                            register: value_register,
                            type_name: return_type_name,
                        });
                    }
                }

                let function_id = *self
                    .function_ids
                    .get(target)
                    .ok_or_else(|| format!("IR references unknown function `{target}`"))?;
                if self.external_function_abi_hashes.contains_key(target) {
                    self.used_imported_functions.insert(target.clone());
                }
                let return_type = self.call_return_type(target)?;
                let return_type_name = self.call_return_type_name(target)?.to_string();
                let result_type = self.types.result_type(self.strings, return_type);
                let result_register = self.add_register(result_type, 0);
                let mut operands = vec![result_register, function_id];
                for arg in args {
                    operands.push(self.lower_value(arg, locals)?.register);
                }
                self.push(OPCODE_CALL_RESULT, operands);

                let value_register = self.add_register(return_type, 0);
                self.push(OPCODE_UNWRAP_RESULT, vec![value_register, result_register]);
                Ok(ValueSlot {
                    register: value_register,
                    type_name: return_type_name,
                })
            }
            IrValue::Binary { op, left, right } => {
                if op == "AND" {
                    return self.lower_short_circuit_and(left, right, locals);
                }
                if op == "OR" {
                    return self.lower_short_circuit_or(left, right, locals);
                }
                let left_register = self.lower_value(left, locals)?;
                let right_register = self.lower_value(right, locals)?;
                if let Some(opcode) = comparison_opcode(op) {
                    return Ok(self.push_boolean_binary(
                        opcode,
                        left_register.register,
                        right_register.register,
                    ));
                }
                if op == "XOR" {
                    return Ok(self.push_boolean_binary(
                        OPCODE_XOR,
                        left_register.register,
                        right_register.register,
                    ));
                }
                let left_type_id = self.registers[left_register.register as usize].type_id;
                let right_type_id = self.registers[right_register.register as usize].type_id;
                let type_id = if op == "&" {
                    TYPE_STRING
                } else {
                    numeric_binary_type_id(op, left_type_id, right_type_id)
                };
                let dst = self.add_register(type_id, 0);
                let opcode = match op.as_str() {
                    "+" => OPCODE_ADD,
                    "-" => OPCODE_SUB,
                    "*" => OPCODE_MUL,
                    "/" => OPCODE_DIV,
                    "DIV" => OPCODE_DIV,
                    "MOD" => OPCODE_MOD,
                    "^" => OPCODE_POW,
                    "&" => OPCODE_CONCAT,
                    _ => return Err(format!("unsupported IR binary operator `{op}`")),
                };
                self.push(
                    opcode,
                    vec![dst, left_register.register, right_register.register],
                );
                Ok(ValueSlot {
                    register: dst,
                    type_name: if type_id == TYPE_STRING {
                        "String".to_string()
                    } else if type_id == TYPE_BYTE {
                        "Byte".to_string()
                    } else if type_id == TYPE_FLOAT {
                        "Float".to_string()
                    } else if type_id == TYPE_FIXED {
                        "Fixed".to_string()
                    } else {
                        "Integer".to_string()
                    },
                })
            }
            IrValue::Unary { op, operand } => {
                let operand = self.lower_value(operand, locals)?;
                match op.as_str() {
                    "NOT" => {
                        Ok(self.push_unary(OPCODE_NOT, TYPE_BOOLEAN, "Boolean", operand.register))
                    }
                    "-" => {
                        let type_id = self.registers[operand.register as usize].type_id;
                        Ok(self.push_unary(
                            OPCODE_NEG,
                            type_id,
                            &operand.type_name,
                            operand.register,
                        ))
                    }
                    _ => Err(format!("unsupported IR unary operator `{op}`")),
                }
            }
            IrValue::Constructor { type_, args } => {
                if type_ == "Ok" {
                    let success = args
                        .first()
                        .ok_or_else(|| "IR Ok constructor is missing success value".to_string())?;
                    let success = self.lower_value(success, locals)?;
                    let result_type = self.types.result_type(
                        self.strings,
                        self.registers[success.register as usize].type_id,
                    );
                    let dst = self.add_register(result_type, 0);
                    self.push(OPCODE_LOAD_DEFAULT, vec![dst, result_type]);
                    return Ok(ValueSlot {
                        register: dst,
                        type_name: format!("Result OF {}", success.type_name),
                    });
                }

                if type_ == "Err" {
                    let error = args
                        .first()
                        .ok_or_else(|| "IR Err constructor is missing error value".to_string())?;
                    self.lower_value(error, locals)?;
                    let result_type = self.type_id("Result OF Unknown");
                    let dst = self.add_register(result_type, 0);
                    self.push(OPCODE_LOAD_DEFAULT, vec![dst, result_type]);
                    return Ok(ValueSlot {
                        register: dst,
                        type_name: "Result OF Unknown".to_string(),
                    });
                }

                if let Some(record) = self.type_model.records.get(type_).cloned() {
                    let type_id = self.type_id(type_);
                    let dst = self.add_register(type_id, 0);
                    let mut operands = vec![dst, type_id];
                    for arg in args {
                        operands.push(self.lower_value(arg, locals)?.register);
                    }
                    if operands.len() != 2 + record.fields.len() {
                        return Err(format!(
                            "IR constructor `{type_}` has {} argument(s), expected {}",
                            operands.len().saturating_sub(2),
                            record.fields.len()
                        ));
                    }
                    self.push(OPCODE_CONSTRUCT_RECORD, operands);
                    return Ok(ValueSlot {
                        register: dst,
                        type_name: type_.clone(),
                    });
                }

                if let Some(variant) = self.type_model.variants.get(type_).cloned() {
                    let union_type = self.type_id(&variant.union_name);
                    let dst = self.add_register(union_type, 0);
                    let variant_name = self.strings.intern(type_);
                    let mut operands = vec![dst, union_type, variant_name];
                    for arg in args {
                        operands.push(self.lower_value(arg, locals)?.register);
                    }
                    if operands.len() != 3 + variant.fields.len() {
                        return Err(format!(
                            "IR variant constructor `{type_}` has {} argument(s), expected {}",
                            operands.len().saturating_sub(3),
                            variant.fields.len()
                        ));
                    }
                    self.push(OPCODE_CONSTRUCT_VARIANT, operands);
                    return Ok(ValueSlot {
                        register: dst,
                        type_name: variant.union_name,
                    });
                }

                Err(format!("IR references unknown constructor `{type_}`"))
            }
            IrValue::WithUpdate {
                type_,
                target,
                updates,
            } => {
                let Some(record) = self.type_model.records.get(type_).cloned() else {
                    return Err(format!("IR WITH update target `{type_}` is not a record"));
                };
                let target = self.lower_value(target, locals)?;
                let type_id = self.type_id(type_);
                let dst = self.add_register(type_id, 0);
                let mut operands = vec![dst, type_id];
                for field in &record.fields {
                    if let Some(update) = updates.iter().find(|update| update.field == field.name) {
                        operands.push(self.lower_value(&update.value, locals)?.register);
                    } else {
                        let field_type = self.type_id(&field.type_name);
                        let field_dst = self.add_register(field_type, 0);
                        let field_name = self.strings.intern(&field.name);
                        self.push(
                            OPCODE_LOAD_FIELD,
                            vec![field_dst, target.register, field_name],
                        );
                        operands.push(field_dst);
                    }
                }
                self.push(OPCODE_CONSTRUCT_RECORD, operands);
                Ok(ValueSlot {
                    register: dst,
                    type_name: type_.clone(),
                })
            }
            IrValue::ListLiteral { type_, values } => {
                let mut value_registers = Vec::new();
                for value in values {
                    value_registers.push(self.lower_value(value, locals)?.register);
                }
                let type_id = self.type_id(type_);
                let dst = self.add_register(type_id, 0);
                let mut operands = vec![dst, type_id, values.len() as u32];
                operands.extend(value_registers);
                self.push(OPCODE_CONSTRUCT_LIST, operands);
                Ok(ValueSlot {
                    register: dst,
                    type_name: type_.clone(),
                })
            }
            IrValue::MapLiteral { type_, entries } => {
                let mut entry_registers = Vec::new();
                for (key, value) in entries {
                    entry_registers.push(self.lower_value(key, locals)?.register);
                    entry_registers.push(self.lower_value(value, locals)?.register);
                }
                let type_id = self.type_id(type_);
                let dst = self.add_register(type_id, 0);
                let mut operands = vec![dst, type_id, entries.len() as u32];
                operands.extend(entry_registers);
                self.push(OPCODE_CONSTRUCT_MAP, operands);
                Ok(ValueSlot {
                    register: dst,
                    type_name: type_.clone(),
                })
            }
            IrValue::MemberAccess { target, member } => {
                if let IrValue::Local(type_name) = target.as_ref() {
                    if let Some(enum_) = self.type_model.enums.get(type_name) {
                        let Some(ordinal) = enum_.members.iter().position(|name| name == member)
                        else {
                            return Err(format!(
                                "IR references unknown enum member `{type_name}::{member}`"
                            ));
                        };
                        let type_id = self.type_id(type_name);
                        let dst = self.add_register(type_id, 0);
                        let member_name = self.strings.intern(member);
                        self.push(
                            OPCODE_LOAD_ENUM_MEMBER,
                            vec![dst, type_id, member_name, ordinal as u32],
                        );
                        return Ok(ValueSlot {
                            register: dst,
                            type_name: type_name.clone(),
                        });
                    }
                }

                let target = self.lower_value(target, locals)?;
                if let Some((key_type, value_type)) = parse_map_entry_type(&target.type_name) {
                    let (field_index, field_type) = match member.as_str() {
                        "key" => (0, key_type),
                        "value" => (1, value_type),
                        _ => {
                            return Err(format!(
                                "IR map entry member access `{}` is not a field of `{}`",
                                member, target.type_name
                            ));
                        }
                    };
                    let field_type_id = self.type_id(&field_type);
                    let entry_type_id = self.type_id(&target.type_name);
                    let dst = self.add_register(field_type_id, 0);
                    self.push(
                        OPCODE_LOAD_MAP_ENTRY_FIELD,
                        vec![dst, target.register, entry_type_id, field_index],
                    );
                    return Ok(ValueSlot {
                        register: dst,
                        type_name: field_type,
                    });
                }
                let Some(field) = self
                    .type_model
                    .records
                    .get(&target.type_name)
                    .and_then(|record| record.fields.iter().find(|field| field.name == *member))
                    .or_else(|| {
                        self.type_model
                            .variants
                            .get(&target.type_name)
                            .and_then(|variant| {
                                variant.fields.iter().find(|field| field.name == *member)
                            })
                    })
                    .cloned()
                else {
                    return Err(format!(
                        "IR member access `{}` is not a field of `{}`",
                        member, target.type_name
                    ));
                };
                let field_type = self.type_id(&field.type_name);
                let dst = self.add_register(field_type, 0);
                let field_name = self.strings.intern(member);
                self.push(OPCODE_LOAD_FIELD, vec![dst, target.register, field_name]);
                Ok(ValueSlot {
                    register: dst,
                    type_name: field.type_name,
                })
            }
        }
    }

    fn push_move_like(&mut self, type_id: u32, dst: u32, src: u32) {
        if dst == src {
            return;
        }
        if matches!(
            type_id,
            TYPE_NOTHING
                | TYPE_BOOLEAN
                | TYPE_BYTE
                | TYPE_INTEGER
                | TYPE_FLOAT
                | TYPE_FIXED
                | TYPE_STRING
        ) {
            self.push(OPCODE_COPY, vec![dst, src]);
        } else {
            self.push(OPCODE_MOVE, vec![dst, src]);
        }
    }

    fn call_return_type(&self, target: &str) -> Result<u32, String> {
        if let Some(return_type) = builtins::call_return_type_name(target) {
            return Ok(match return_type {
                "Nothing" => TYPE_NOTHING,
                "Boolean" => TYPE_BOOLEAN,
                "Integer" => TYPE_INTEGER,
                "Float" => TYPE_FLOAT,
                "Fixed" => TYPE_FIXED,
                "String" => TYPE_STRING,
                "Byte" => TYPE_BYTE,
                "File" => TYPE_FILE_HANDLE,
                _ => return Err(format!("unsupported built-in return type `{return_type}`")),
            });
        }
        self.function_return_types
            .get(target)
            .copied()
            .ok_or_else(|| format!("unsupported call target `{target}`"))
    }

    fn call_return_type_name(&self, target: &str) -> Result<&str, String> {
        if let Some(return_type) = builtins::call_return_type_name(target) {
            return Ok(return_type);
        }
        self.function_return_type_names
            .get(target)
            .map(String::as_str)
            .ok_or_else(|| format!("unsupported call target `{target}`"))
    }

    fn type_id(&mut self, name: &str) -> u32 {
        self.types.type_id(self.strings, name)
    }

    fn const_id(&mut self, value: &IrValue) -> Result<u32, String> {
        self.constants.add(self.strings, value)
    }

    fn add_register(&mut self, type_id: u32, flags: u32) -> u32 {
        let id = self.registers.len() as u32;
        self.registers.push(Register { type_id, flags });
        id
    }

    fn push(&mut self, opcode: u16, operands: Vec<u32>) {
        self.code.push(Instruction { opcode, operands });
    }

    fn push_jump(&mut self, opcode: u16, mut operands: Vec<u32>) -> usize {
        operands.push(u32::MAX);
        let index = self.code.len();
        self.push(opcode, operands);
        index
    }

    fn patch_jump(&mut self, instruction_index: usize) {
        let target = self.code.len() as u32;
        let operands = &mut self.code[instruction_index].operands;
        let last = operands
            .last_mut()
            .expect("branch instructions reserve a target operand");
        *last = target;
    }

    fn push_equal(&mut self, left: u32, right: u32) -> ValueSlot {
        self.push_boolean_binary(OPCODE_EQUAL, left, right)
    }

    fn push_boolean_binary(&mut self, opcode: u16, left: u32, right: u32) -> ValueSlot {
        let dst = self.add_register(TYPE_BOOLEAN, 0);
        self.push(opcode, vec![dst, left, right]);
        ValueSlot {
            register: dst,
            type_name: "Boolean".to_string(),
        }
    }

    fn push_unary(
        &mut self,
        opcode: u16,
        type_id: u32,
        type_name: &str,
        operand: u32,
    ) -> ValueSlot {
        let dst = self.add_register(type_id, 0);
        self.push(opcode, vec![dst, operand]);
        ValueSlot {
            register: dst,
            type_name: type_name.to_string(),
        }
    }

    fn push_boolean_const(&mut self, value: bool) -> Result<u32, String> {
        let register = self.add_register(TYPE_BOOLEAN, 0);
        let constant = IrValue::Const {
            type_: "Boolean".to_string(),
            value: value.to_string(),
        };
        let constant_id = self.const_id(&constant)?;
        self.push(OPCODE_LOAD_CONST, vec![register, constant_id]);
        Ok(register)
    }

    fn lower_short_circuit_and(
        &mut self,
        left: &IrValue,
        right: &IrValue,
        locals: &HashMap<String, ValueSlot>,
    ) -> Result<ValueSlot, String> {
        let dst = self.push_boolean_const(false)?;
        let left = self.lower_value(left, locals)?;
        let end_jump = self.push_jump(OPCODE_BRANCH_IF_FALSE, vec![left.register]);
        let right = self.lower_value(right, locals)?;
        self.push(OPCODE_COPY, vec![dst, right.register]);
        self.patch_jump(end_jump);
        Ok(ValueSlot {
            register: dst,
            type_name: "Boolean".to_string(),
        })
    }

    fn lower_short_circuit_or(
        &mut self,
        left: &IrValue,
        right: &IrValue,
        locals: &HashMap<String, ValueSlot>,
    ) -> Result<ValueSlot, String> {
        let dst = self.push_boolean_const(true)?;
        let left = self.lower_value(left, locals)?;
        let end_jump = self.push_jump(OPCODE_BRANCH_IF_TRUE, vec![left.register]);
        let right = self.lower_value(right, locals)?;
        self.push(OPCODE_COPY, vec![dst, right.register]);
        self.patch_jump(end_jump);
        Ok(ValueSlot {
            register: dst,
            type_name: "Boolean".to_string(),
        })
    }

    fn ends_with_return(&self) -> bool {
        self.code
            .last()
            .is_some_and(|instruction| instruction.opcode == OPCODE_RETURN_OK)
    }
}

fn comparison_opcode(op: &str) -> Option<u16> {
    match op {
        "=" => Some(OPCODE_EQUAL),
        "<>" => Some(OPCODE_NOT_EQUAL),
        "<" => Some(OPCODE_LESS),
        "<=" => Some(OPCODE_LESS_EQUAL),
        ">" => Some(OPCODE_GREATER),
        ">=" => Some(OPCODE_GREATER_EQUAL),
        _ => None,
    }
}

fn function_return_from_type(type_name: &str) -> Option<String> {
    type_name
        .strip_prefix("FUNC(")
        .or_else(|| type_name.strip_prefix("ISOLATED FUNC("))
        .and_then(|rest| rest.split_once(") AS "))
        .map(|(_, return_type)| return_type.to_string())
}

struct FunctionTypeSignature {
    isolated: bool,
    params: Vec<String>,
    returns: String,
}

fn parse_function_type(type_name: &str) -> Option<FunctionTypeSignature> {
    let (isolated, rest) = if let Some(rest) = type_name.strip_prefix("ISOLATED FUNC(") {
        (true, rest)
    } else {
        (false, type_name.strip_prefix("FUNC(")?)
    };
    let (params, returns) = split_function_type_rest(rest)?;
    Some(FunctionTypeSignature {
        isolated,
        params: split_top_level_types(params),
        returns: returns.to_string(),
    })
}

fn split_function_type_rest(rest: &str) -> Option<(&str, &str)> {
    let mut depth = 0usize;
    let bytes = rest.as_bytes();
    for index in 0..bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' if depth == 0 && rest[index..].starts_with(") AS ") => {
                return Some((&rest[..index], &rest[index + 5..]));
            }
            b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    None
}

fn split_top_level_types(params: &str) -> Vec<String> {
    if params.trim().is_empty() {
        return Vec::new();
    }

    let mut result = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (index, ch) in params.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                result.push(params[start..index].trim().to_string());
                start = index + 1;
            }
            _ => {}
        }
    }
    result.push(params[start..].trim().to_string());
    result
}

fn close_function_id(name: &str) -> Result<u32, String> {
    match name {
        "fs.close" => Ok(BUILTIN_FS_CLOSE_FUNCTION_ID),
        _ => Err(format!("unsupported resource close function `{name}`")),
    }
}

impl BuiltinCallLowerer for FunctionBuilder<'_> {
    fn lower_value(
        &mut self,
        value: &IrValue,
        locals: &HashMap<String, ValueSlot>,
    ) -> Result<ValueSlot, String> {
        FunctionBuilder::lower_value(self, value, locals)
    }

    fn add_register(&mut self, type_id: u32, flags: u32) -> u32 {
        FunctionBuilder::add_register(self, type_id, flags)
    }

    fn type_id(&mut self, type_name: &str) -> u32 {
        FunctionBuilder::type_id(self, type_name)
    }

    fn push(&mut self, opcode: u16, operands: Vec<u32>) {
        FunctionBuilder::push(self, opcode, operands);
    }

    fn push_string_const(&mut self, value: &str) -> Result<ValueSlot, String> {
        let register = self.add_register(TYPE_STRING, 0);
        let constant = IrValue::Const {
            type_: "String".to_string(),
            value: value.to_string(),
        };
        let constant_id = self.const_id(&constant)?;
        self.push(OPCODE_LOAD_CONST, vec![register, constant_id]);
        Ok(ValueSlot {
            register,
            type_name: "String".to_string(),
        })
    }

    fn push_integer_const(&mut self, value: i64) -> Result<ValueSlot, String> {
        let register = self.add_register(TYPE_INTEGER, 0);
        let constant = IrValue::Const {
            type_: "Integer".to_string(),
            value: value.to_string(),
        };
        let constant_id = self.const_id(&constant)?;
        self.push(OPCODE_LOAD_CONST, vec![register, constant_id]);
        Ok(ValueSlot {
            register,
            type_name: "Integer".to_string(),
        })
    }
}

const OPCODE_MOVE: u16 = 10;
const OPCODE_COPY: u16 = 11;

impl StringPool {
    fn new() -> Self {
        Self { values: Vec::new() }
    }

    fn intern(&mut self, value: &str) -> u32 {
        if let Some(index) = self.values.iter().position(|existing| existing == value) {
            return index as u32;
        }
        let index = self.values.len() as u32;
        self.values.push(value.to_string());
        index
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.values.len() as u32);
        for value in &self.values {
            put_bytes(&mut bytes, value.as_bytes());
        }
        bytes
    }
}

impl TypeTable {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
            ids: HashMap::new(),
        }
    }

    fn reserve_source_type(
        &mut self,
        strings: &mut StringPool,
        package: &str,
        ir_type: &IrType,
    ) -> u32 {
        let kind = match ir_type.kind.as_str() {
            "type" => 1,
            "union" => 2,
            "enum" => 3,
            _ => 1,
        };
        self.add_entry(strings, package, &ir_type.name, kind, Vec::new())
    }

    fn populate_source_payloads(
        &mut self,
        strings: &mut StringPool,
        ir_types: &[IrType],
    ) -> Result<(), String> {
        let source_types = ir_types
            .iter()
            .map(|ir_type| (ir_type.name.as_str(), ir_type))
            .collect::<HashMap<_, _>>();

        for ir_type in ir_types {
            let id = *self
                .ids
                .get(&ir_type.name)
                .ok_or_else(|| format!("source type `{}` was not reserved", ir_type.name))?;
            let payload = source_type_payload(strings, self, &source_types, ir_type)?;
            self.entries[(id - FIRST_TABLE_TYPE_ID) as usize].payload = payload;
        }

        Ok(())
    }

    fn type_id(&mut self, strings: &mut StringPool, name: &str) -> u32 {
        match name {
            "Nothing" => TYPE_NOTHING,
            "Boolean" => TYPE_BOOLEAN,
            "Integer" => TYPE_INTEGER,
            "Float" => TYPE_FLOAT,
            "Fixed" => TYPE_FIXED,
            "String" => TYPE_STRING,
            "File" => TYPE_FILE_HANDLE,
            name if name.starts_with("List OF ") => {
                let element = self.type_id(strings, name.trim_start_matches("List OF "));
                self.list_type(strings, element)
            }
            name if name.starts_with("Result OF ") => {
                let success = self.type_id(strings, name.trim_start_matches("Result OF "));
                self.result_type(strings, success)
            }
            name if name.starts_with("Thread OF ") => {
                if let Some((message, output)) = builtins::thread::thread_parts(name) {
                    let message = self.type_id(strings, message);
                    let output = self.type_id(strings, output);
                    self.thread_type(strings, message, output)
                } else {
                    self.add_entry(strings, "", name, 7, Vec::new())
                }
            }
            name if name.starts_with("FUNC(") => self.function_type(strings, name),
            name if name.starts_with("ISOLATED FUNC(") => self.function_type(strings, name),
            name if name.starts_with("Map OF ") => {
                let rest = name.trim_start_matches("Map OF ");
                if let Some((key, value)) = rest.split_once(" TO ") {
                    let key = self.type_id(strings, key);
                    let value = self.type_id(strings, value);
                    self.map_type(strings, key, value)
                } else {
                    self.add_entry(strings, "", name, 5, Vec::new())
                }
            }
            name if name.starts_with("MapEntry OF ") => {
                let rest = name.trim_start_matches("MapEntry OF ");
                if let Some((key, value)) = rest.split_once(" TO ") {
                    let key = self.type_id(strings, key);
                    let value = self.type_id(strings, value);
                    self.map_entry_type(strings, key, value)
                } else {
                    self.add_entry(strings, "", name, 9, Vec::new())
                }
            }
            "Byte" => TYPE_BYTE,
            "Error" => {
                strings.intern("code");
                strings.intern("message");
                TYPE_ERROR
            }
            "TerminalSize" => {
                strings.intern("columns");
                strings.intern("rows");
                TYPE_TERMINAL_SIZE
            }
            _ => {
                if let Some(id) = self.ids.get(name) {
                    *id
                } else {
                    self.add_entry(strings, "", name, 1, Vec::new())
                }
            }
        }
    }

    fn result_type(&mut self, strings: &mut StringPool, success_type: u32) -> u32 {
        let name = format!("Result#{success_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, success_type);
        self.add_entry(strings, "", &name, 6, payload)
    }

    fn list_type(&mut self, strings: &mut StringPool, element_type: u32) -> u32 {
        let name = format!("List#{element_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, element_type);
        self.add_entry(strings, "", &name, 4, payload)
    }

    fn map_type(&mut self, strings: &mut StringPool, key_type: u32, value_type: u32) -> u32 {
        let name = format!("Map#{key_type}#{value_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, key_type);
        put_u32(&mut payload, value_type);
        self.add_entry(strings, "", &name, 5, payload)
    }

    fn map_entry_type(&mut self, strings: &mut StringPool, key_type: u32, value_type: u32) -> u32 {
        let name = format!("MapEntry#{key_type}#{value_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, key_type);
        put_u32(&mut payload, value_type);
        self.add_entry(strings, "", &name, 9, payload)
    }

    fn function_type(&mut self, strings: &mut StringPool, name: &str) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let mut payload = Vec::new();
        if let Some(signature) = parse_function_type(name) {
            put_u32(&mut payload, if signature.isolated { 1 } else { 0 });
            put_u32(&mut payload, signature.params.len() as u32);
            let return_type = self.type_id(strings, &signature.returns);
            put_u32(&mut payload, return_type);
            for param in signature.params {
                let param_type = self.type_id(strings, &param);
                put_u32(&mut payload, param_type);
            }
        }
        self.add_entry(strings, "", name, 8, payload)
    }

    fn thread_type(
        &mut self,
        strings: &mut StringPool,
        message_type: u32,
        output_type: u32,
    ) -> u32 {
        let name = format!("Thread#{message_type}#{output_type}");
        if let Some(id) = self.ids.get(&name) {
            return *id;
        }

        let mut payload = Vec::new();
        put_u32(&mut payload, message_type);
        put_u32(&mut payload, output_type);
        self.add_entry(strings, "thread", &name, 7, payload)
    }

    fn add_entry(
        &mut self,
        strings: &mut StringPool,
        package: &str,
        name: &str,
        kind: u16,
        payload: Vec<u8>,
    ) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        let id = FIRST_TABLE_TYPE_ID + self.entries.len() as u32;
        self.ids.insert(name.to_string(), id);
        self.entries.push(TypeEntry {
            kind,
            name: strings.intern(name),
            owner_package: strings.intern(package),
            payload,
        });
        id
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        let entry_bytes = 20usize;
        let mut payload_offset = 4 + self.entries.len() * entry_bytes;
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u16(&mut bytes, entry.kind);
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, entry.name);
            put_u32(&mut bytes, entry.owner_package);
            put_u32(&mut bytes, payload_offset as u32);
            put_u32(&mut bytes, entry.payload.len() as u32);
            payload_offset += entry.payload.len();
        }
        for entry in &self.entries {
            bytes.extend_from_slice(&entry.payload);
        }
        bytes
    }
}

fn source_type_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    source_types: &HashMap<&str, &IrType>,
    ir_type: &IrType,
) -> Result<Vec<u8>, String> {
    let mut payload = Vec::new();
    match ir_type.kind.as_str() {
        "type" => {
            put_u32(&mut payload, ir_type.fields.len() as u32);
            for field in &ir_type.fields {
                put_field_payload(strings, types, &mut payload, field);
            }
        }
        "union" => {
            let variants = concrete_union_variants(source_types, ir_type)?;
            put_u32(&mut payload, variants.len() as u32);
            for variant in variants {
                put_u32(&mut payload, strings.intern(&variant.name));
                put_u32(&mut payload, variant.fields.len() as u32);
                for field in &variant.fields {
                    put_u32(&mut payload, strings.intern(&field.name));
                    put_u32(&mut payload, types.type_id(strings, &field.type_));
                }
            }
        }
        "enum" => {
            put_u32(&mut payload, ir_type.members.len() as u32);
            for (ordinal, member) in ir_type.members.iter().enumerate() {
                put_u32(&mut payload, strings.intern(&member.name));
                put_u32(&mut payload, ordinal as u32);
            }
        }
        _ => {}
    }
    Ok(payload)
}

fn concrete_union_variants<'a>(
    source_types: &HashMap<&str, &'a IrType>,
    ir_type: &'a IrType,
) -> Result<Vec<&'a crate::ir::IrVariant>, String> {
    let mut variants = Vec::new();
    for include in &ir_type.includes {
        let included = source_types.get(include.as_str()).ok_or_else(|| {
            format!(
                "union `{}` includes unknown union `{include}`",
                ir_type.name
            )
        })?;
        variants.extend(concrete_union_variants(source_types, included)?);
    }
    variants.extend(ir_type.variants.iter());
    Ok(variants)
}

fn put_field_payload(
    strings: &mut StringPool,
    types: &mut TypeTable,
    payload: &mut Vec<u8>,
    field: &crate::ir::IrField,
) {
    put_u32(payload, strings.intern(&field.name));
    put_u32(payload, types.type_id(strings, &field.type_));
    put_u32(
        payload,
        match field.visibility.as_deref() {
            Some("private") => 1,
            Some("package") => 2,
            Some("export") => 3,
            _ => 0,
        },
    );
}

impl ConstPool {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn add(&mut self, strings: &mut StringPool, value: &IrValue) -> Result<u32, String> {
        let entry = match value {
            IrValue::Const { type_, value } => match type_.as_str() {
                "Nothing" => ConstEntry {
                    kind: 1,
                    payload: Vec::new(),
                },
                "String" => {
                    let mut payload = Vec::new();
                    put_u32(&mut payload, strings.intern(value));
                    ConstEntry { kind: 6, payload }
                }
                "Integer" => ConstEntry {
                    kind: 3,
                    payload: value
                        .parse::<i64>()
                        .map_err(|_| format!("invalid Integer constant `{value}`"))?
                        .to_le_bytes()
                        .to_vec(),
                },
                "Float" => ConstEntry {
                    kind: 4,
                    payload: value
                        .parse::<f64>()
                        .map_err(|_| format!("invalid Float constant `{value}`"))?
                        .to_bits()
                        .to_le_bytes()
                        .to_vec(),
                },
                "Fixed" => ConstEntry {
                    kind: 5,
                    payload: fixed_raw_from_decimal(value)?.to_le_bytes().to_vec(),
                },
                "Boolean" => ConstEntry {
                    kind: 2,
                    payload: vec![if value == "true" { 1 } else { 0 }],
                },
                "Byte" => ConstEntry {
                    kind: 7,
                    payload: vec![value
                        .parse::<u8>()
                        .map_err(|_| format!("invalid Byte constant `{value}`"))?],
                },
                _ => return Err(format!("unsupported constant type `{type_}`")),
            },
            _ => return Err("only constant IR values can be stored in CONST_POOL".to_string()),
        };

        let id = self.entries.len() as u32;
        self.entries.push(entry);
        Ok(id)
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u16(&mut bytes, entry.kind);
            put_u16(&mut bytes, 0);
            put_bytes(&mut bytes, &entry.payload);
        }
        bytes
    }
}

fn fixed_raw_from_decimal(value: &str) -> Result<i64, String> {
    const SCALE: i128 = 1_i128 << 32;

    let (negative, digits) = value
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, value));
    let (whole, fractional) = digits.split_once('.').unwrap_or((digits, ""));
    if whole.is_empty() && fractional.is_empty() {
        return Err(format!("invalid Fixed constant `{value}`"));
    }
    let mut whole_value = if whole.is_empty() {
        0_i128
    } else {
        whole
            .parse::<i128>()
            .map_err(|_| format!("invalid Fixed constant `{value}`"))?
    };
    let mut fractional_value = 0_i128;
    if !fractional.is_empty() {
        let mut denominator = 1_i128;
        for digit in fractional.bytes() {
            if !digit.is_ascii_digit() {
                return Err(format!("invalid Fixed constant `{value}`"));
            }
            fractional_value = fractional_value
                .checked_mul(10)
                .and_then(|current| current.checked_add((digit - b'0') as i128))
                .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
            denominator = denominator
                .checked_mul(10)
                .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
        }
        let scaled = fractional_value
            .checked_mul(SCALE)
            .ok_or_else(|| format!("Fixed constant `{value}` has too many digits"))?;
        fractional_value = scaled / denominator;
        if (scaled % denominator) * 2 >= denominator {
            fractional_value += 1;
        }
        if fractional_value == SCALE {
            whole_value += 1;
            fractional_value = 0;
        }
    }
    let raw = whole_value
        .checked_mul(SCALE)
        .and_then(|current| current.checked_add(fractional_value))
        .ok_or_else(|| format!("Fixed constant `{value}` is out of range"))?;
    let raw = if negative { -raw } else { raw };
    i64::try_from(raw).map_err(|_| format!("Fixed constant `{value}` is out of range"))
}

impl ResourceTable {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn add_standard_file(&mut self, types: &mut TypeTable, strings: &mut StringPool) {
        let type_id = types.type_id(strings, builtins::fs::FILE_TYPE);
        self.entries.push(ResourceEntry {
            type_id,
            close_function_id: BUILTIN_FS_CLOSE_FUNCTION_ID,
            flags: RESOURCE_FLAG_NATIVE | RESOURCE_FLAG_STANDARD | RESOURCE_FLAG_CLOSE_MAY_FAIL,
        });
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u32(&mut bytes, entry.type_id);
            put_u32(&mut bytes, entry.close_function_id);
            put_u32(&mut bytes, entry.flags);
        }
        bytes
    }
}

impl ImportTable {
    fn from_metadata(strings: &mut StringPool, metadata: &BytecodeMetadata) -> Self {
        let entries = metadata
            .dependencies
            .iter()
            .map(|dependency| ImportEntry {
                package_name: strings.intern(&dependency.name),
                package_ident: strings.intern(if dependency.ident.is_empty() {
                    &dependency.name
                } else {
                    &dependency.ident
                }),
                version: strings.intern(&dependency.version),
                pin: dependency.pin,
                flags: dependency.flags,
                used_symbols: Vec::new(),
            })
            .collect();

        Self { entries }
    }

    fn record_used_imports(
        &mut self,
        strings: &mut StringPool,
        used_imported_functions: &HashSet<String>,
        external_function_abi_hashes: &HashMap<String, [u8; ABI_HASH_LEN]>,
    ) {
        let import_names = self
            .entries
            .iter()
            .map(|entry| {
                (
                    entry.package_name,
                    strings.values[entry.package_name as usize].clone(),
                )
            })
            .collect::<Vec<_>>();

        for (package_name_id, package_name) in import_names {
            let prefix = format!("{package_name}.");
            let mut symbols = used_imported_functions
                .iter()
                .filter_map(|target| {
                    let symbol_name = target.strip_prefix(&prefix)?;
                    let sig_hash = *external_function_abi_hashes.get(target)?;
                    Some(AbiUsedSymbol {
                        name: strings.intern(symbol_name),
                        sig_hash,
                    })
                })
                .collect::<Vec<_>>();
            symbols.sort_by_key(|symbol| strings.values[symbol.name as usize].clone());
            if let Some(entry) = self
                .entries
                .iter_mut()
                .find(|entry| entry.package_name == package_name_id)
            {
                entry.used_symbols = symbols;
            }
        }
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.entries.len() as u32);
        for entry in &self.entries {
            put_u32(&mut bytes, entry.package_name);
            put_u32(&mut bytes, entry.package_ident);
            put_u32(&mut bytes, entry.version);
            bytes.push(if entry.pin { 1 } else { 0 });
            put_u32(&mut bytes, entry.flags);
            put_u32(&mut bytes, entry.used_symbols.len() as u32);
            for symbol in &entry.used_symbols {
                put_u32(&mut bytes, symbol.name);
                bytes.extend_from_slice(&symbol.sig_hash);
            }
        }
        bytes
    }
}

impl AbiIndex {
    fn from_project(
        strings: &StringPool,
        types: &TypeTable,
        constants: &ConstPool,
        imports: &ImportTable,
        functions: &[Function],
    ) -> Result<Self, String> {
        let mut exports = Vec::new();
        for function in functions {
            if !is_exported_function(function) {
                continue;
            }
            let kind = if function.flags & FUNCTION_FLAG_SUB != 0 {
                BytecodeExportKind::Sub
            } else {
                BytecodeExportKind::Func
            };
            exports.push(AbiExport {
                name: function.name,
                kind,
                sig_hash: function_sig_hash(function, kind, &strings.values, types, constants)?,
            });
        }

        let dep_edges = imports
            .entries
            .iter()
            .map(|entry| AbiDepEdge {
                package_name: entry.package_name,
                package_ident: entry.package_ident,
                version_request: entry.version,
                pin: entry.pin,
                used_symbols: entry.used_symbols.clone(),
            })
            .collect();

        Ok(Self { exports, dep_edges })
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u16(&mut bytes, ABI_FORMAT_VERSION);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, self.exports.len() as u32);
        for export in &self.exports {
            put_u32(&mut bytes, export.name);
            put_u16(
                &mut bytes,
                match export.kind {
                    BytecodeExportKind::Func => 1,
                    BytecodeExportKind::Sub => 2,
                },
            );
            bytes.extend_from_slice(&export.sig_hash);
        }
        put_u32(&mut bytes, self.dep_edges.len() as u32);
        for edge in &self.dep_edges {
            put_u32(&mut bytes, edge.package_name);
            put_u32(&mut bytes, edge.package_ident);
            put_u32(&mut bytes, edge.version_request);
            bytes.push(if edge.pin { 1 } else { 0 });
            put_u32(&mut bytes, edge.used_symbols.len() as u32);
            for symbol in &edge.used_symbols {
                put_u32(&mut bytes, symbol.name);
                bytes.extend_from_slice(&symbol.sig_hash);
            }
        }
        bytes
    }
}

impl BytecodeProject {
    fn encode(&self) -> Vec<u8> {
        let mut code_section = Vec::new();
        let mut code_offsets = Vec::new();
        for function in &self.functions {
            code_offsets.push((
                code_section.len() as u64,
                function_code_length(function) as u64,
            ));
            encode_function_code(&mut code_section, function);
        }

        let mut sections = vec![
            Section::new(SECTION_MANIFEST, self.encode_manifest()),
            Section::new(SECTION_STRING_POOL, self.strings.encode()),
            Section::new(SECTION_TYPE_TABLE, self.types.encode()),
            Section::new(SECTION_CONST_POOL, self.constants.encode()),
            Section::new(SECTION_IMPORT_TABLE, self.imports.encode()),
            Section::new(SECTION_EXPORT_TABLE, self.encode_exports()),
            Section::new(SECTION_GLOBAL_TABLE, encode_empty_count()),
            Section::new(SECTION_FUNCTION_TABLE, self.encode_functions(&code_offsets)),
            Section::new(SECTION_CODE, code_section),
            Section::new(SECTION_ABI_INDEX, self.abi.encode()),
        ];
        if !self.resources.entries.is_empty() {
            sections.push(Section::new(
                SECTION_RESOURCE_TABLE,
                self.resources.encode(),
            ));
        }

        encode_sections(&sections)
    }

    fn encode_manifest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.manifest.package_name);
        put_u32(&mut bytes, self.manifest.package_ident);
        put_u32(&mut bytes, self.manifest.package_version);
        put_u32(&mut bytes, self.manifest.ident_key);
        put_u32(&mut bytes, self.manifest.author);
        put_u32(&mut bytes, self.manifest.url);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, self.imports.entries.len() as u32);
        put_u32(&mut bytes, 0);
        put_u32(&mut bytes, self.export_count());
        put_u32(&mut bytes, self.entry_function);
        put_u32(&mut bytes, self.entry_flags);
        bytes
    }

    fn encode_exports(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.export_count());
        for (index, function) in self.functions.iter().enumerate() {
            if !is_exported_function(function) {
                continue;
            }
            put_u32(&mut bytes, function.name);
            put_u16(
                &mut bytes,
                if function.flags & FUNCTION_FLAG_SUB != 0 {
                    2
                } else {
                    1
                },
            );
            put_u16(&mut bytes, 0);
            put_u32(&mut bytes, index as u32);
        }
        bytes
    }

    fn export_count(&self) -> u32 {
        self.functions
            .iter()
            .filter(|function| is_exported_function(function))
            .count() as u32
    }

    fn encode_functions(&self, code_offsets: &[(u64, u64)]) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, self.functions.len() as u32);
        for (index, function) in self.functions.iter().enumerate() {
            let (code_offset, code_length) = code_offsets[index];
            put_u32(&mut bytes, function.name);
            put_u16(&mut bytes, function.kind);
            put_u16(&mut bytes, function.flags);
            put_u32(&mut bytes, function.params.len() as u32);
            put_u32(&mut bytes, function.return_type);
            put_u32(&mut bytes, function.registers.len() as u32);
            put_u64(&mut bytes, code_offset);
            put_u64(&mut bytes, code_length);
            put_u32(&mut bytes, u32::MAX);
            put_u32(&mut bytes, function.cleanups.len() as u32);
            let cleanup_offset =
                bytes.len() + 8 + function.params.len() * 16 + function.registers.len() * 8;
            put_u64(
                &mut bytes,
                if function.cleanups.is_empty() {
                    0
                } else {
                    cleanup_offset as u64
                },
            );

            for param in &function.params {
                put_u32(&mut bytes, param.name);
                put_u32(&mut bytes, param.type_id);
                put_u32(&mut bytes, param.flags);
                put_u32(&mut bytes, param.default_const);
            }

            for register in &function.registers {
                put_u32(&mut bytes, register.type_id);
                put_u32(&mut bytes, register.flags);
            }

            for cleanup in &function.cleanups {
                put_u32(&mut bytes, cleanup.id);
                put_u32(&mut bytes, cleanup.start_pc);
                put_u32(&mut bytes, cleanup.end_pc);
                put_u32(&mut bytes, cleanup.resource_register);
                put_u32(&mut bytes, cleanup.close_function_id);
            }
        }
        bytes
    }
}

struct Section {
    id: u16,
    data: Vec<u8>,
}

impl Section {
    fn new(id: u16, data: Vec<u8>) -> Self {
        Self { id, data }
    }
}

fn encode_sections(sections: &[Section]) -> Vec<u8> {
    let section_table_size = sections.len() * 24;
    let mut offset = 16 + section_table_size;
    let mut bytes = Vec::new();

    bytes.extend_from_slice(b"MFBC");
    put_u16(&mut bytes, 1);
    put_u16(&mut bytes, 0);
    put_u32(&mut bytes, 0);
    put_u32(&mut bytes, sections.len() as u32);

    for section in sections {
        put_u16(&mut bytes, section.id);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
        put_u64(&mut bytes, offset as u64);
        put_u64(&mut bytes, section.data.len() as u64);
        offset += section.data.len();
    }

    for section in sections {
        bytes.extend_from_slice(&section.data);
    }

    bytes
}

fn encode_empty_count() -> Vec<u8> {
    let mut bytes = Vec::new();
    put_u32(&mut bytes, 0);
    bytes
}

fn function_code_length(function: &Function) -> usize {
    4 + function
        .code
        .iter()
        .map(|instruction| 8 + instruction.operands.len() * 4)
        .sum::<usize>()
}

fn encode_function_code(bytes: &mut Vec<u8>, function: &Function) {
    put_u32(bytes, function.code.len() as u32);
    for instruction in &function.code {
        put_u16(bytes, instruction.opcode);
        put_u16(bytes, 0);
        put_u16(bytes, instruction.operands.len() as u16);
        put_u16(bytes, 0);
        for operand in &instruction.operands {
            put_u32(bytes, *operand);
        }
    }
}

fn hex_dump(bytes: &[u8]) -> String {
    let mut output = String::new();
    for chunk in bytes.chunks(16) {
        for (index, byte) in chunk.iter().enumerate() {
            if index > 0 {
                output.push(' ');
            }
            output.push_str(&format!("{byte:02X}"));
        }
        output.push('\n');
    }
    output
}

fn numeric_binary_type_id(op: &str, left: u32, right: u32) -> u32 {
    let Some(left) = numeric_type_name(left) else {
        return TYPE_INTEGER;
    };
    let Some(right) = numeric_type_name(right) else {
        return TYPE_INTEGER;
    };
    match numeric::binary_result_type(op, left, right) {
        Some(numeric::TYPE_BYTE) => TYPE_BYTE,
        Some(numeric::TYPE_FIXED) => TYPE_FIXED,
        Some(numeric::TYPE_FLOAT) => TYPE_FLOAT,
        Some(numeric::TYPE_INTEGER) => TYPE_INTEGER,
        _ => TYPE_INTEGER,
    }
}

fn parse_map_entry_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("MapEntry OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn numeric_type_name(type_id: u32) -> Option<&'static str> {
    match type_id {
        TYPE_BYTE => Some(numeric::TYPE_BYTE),
        TYPE_FIXED => Some(numeric::TYPE_FIXED),
        TYPE_FLOAT => Some(numeric::TYPE_FLOAT),
        TYPE_INTEGER => Some(numeric::TYPE_INTEGER),
        _ => None,
    }
}

fn put_bytes(dst: &mut Vec<u8>, bytes: &[u8]) {
    put_u32(dst, bytes.len() as u32);
    dst.extend_from_slice(bytes);
}

fn put_u16(dst: &mut Vec<u8>, value: u16) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&value.to_le_bytes());
}

fn put_u64(dst: &mut Vec<u8>, value: u64) {
    dst.extend_from_slice(&value.to_le_bytes());
}
