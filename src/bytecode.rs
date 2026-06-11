use crate::builtins;
use crate::ir::{IrFunction, IrMatchPattern, IrOp, IrProject, IrType, IrValue};
use std::collections::HashMap;
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

pub(crate) const TYPE_NOTHING: u32 = 1;
pub(crate) const TYPE_BOOLEAN: u32 = 2;
pub(crate) const TYPE_INTEGER: u32 = 3;
pub(crate) const TYPE_FLOAT: u32 = 4;
pub(crate) const TYPE_FIXED: u32 = 5;
pub(crate) const TYPE_STRING: u32 = 6;
pub(crate) const TYPE_BYTE: u32 = 7;

const FUNCTION_BYTECODE: u16 = 1;

const FUNCTION_FLAG_PRIVATE: u16 = 1 << 1;
const FUNCTION_FLAG_SUB: u16 = 1 << 3;
const FUNCTION_FLAG_RETURNS_NOTHING: u16 = 1 << 5;

const REGISTER_FLAG_PARAMETER: u32 = 1 << 0;
const REGISTER_FLAG_MUTABLE_LOCAL_CELL: u32 = 1 << 1;
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
pub(crate) const OPCODE_WRITE_STDOUT: u16 = 50;
const OPCODE_CALL_RESULT: u16 = 60;
const OPCODE_UNWRAP_RESULT: u16 = 61;
const OPCODE_LOAD_FUNCTION: u16 = 62;
const OPCODE_CALL_VALUE_RESULT: u16 = 63;
const OPCODE_RETURN_OK: u16 = 70;
const OPCODE_CONSTRUCT_RECORD: u16 = 80;
const OPCODE_CONSTRUCT_VARIANT: u16 = 81;
const OPCODE_LOAD_FIELD: u16 = 82;
const OPCODE_LOAD_ENUM_MEMBER: u16 = 83;
const OPCODE_CONSTRUCT_LIST: u16 = 84;
const OPCODE_CONSTRUCT_MAP: u16 = 85;
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

pub fn write_bytecode_hex(
    project_dir: &Path,
    ir: &IrProject,
    version: &str,
) -> Result<PathBuf, String> {
    let bytes = build_bytecode_bytes(ir, version)?;
    let hex_path = project_dir.join(format!("{}.hex", ir.name));
    fs::write(&hex_path, hex_dump(&bytes))
        .map_err(|err| format!("failed to write '{}': {err}", hex_path.display()))?;
    Ok(hex_path)
}

pub fn build_bytecode_bytes(ir: &IrProject, version: &str) -> Result<Vec<u8>, String> {
    Ok(lower_project(ir, version)?.encode())
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
    Result,
    Other,
}

pub struct NativeProgram {
    pub entry_function: u32,
    pub entry_returns_integer: bool,
    pub types: NativeTypeLayouts,
    pub functions: Vec<NativeFunction>,
    pub constants: Vec<NativeConst>,
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
    Integer(i64),
    Float(f64),
    Fixed,
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
pub const NATIVE_OPCODE_WRITE_STDOUT: u16 = OPCODE_WRITE_STDOUT;
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

pub fn native_program(ir: &IrProject) -> Result<NativeProgram, String> {
    let project = lower_project(ir, "native")?;
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
        types: native_type_layouts(ir, &project.types.ids, &project.strings),
        functions,
        constants,
    })
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
        5 => Ok(NativeConst::Fixed),
        6 => {
            let string_id = read_u32(&constant.payload, 0);
            let value = strings.get(&string_id).cloned().ok_or_else(|| {
                format!("String constant references missing string pool entry {string_id}")
            })?;
            Ok(NativeConst::String(value))
        }
        _ => Ok(NativeConst::Other),
    }
}

fn native_type_layouts(
    ir: &IrProject,
    type_ids: &HashMap<String, u32>,
    strings: &StringPool,
) -> NativeTypeLayouts {
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
            8,
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

    for ir_type in &ir.types {
        let Some(type_id) = native_type_id(type_ids, &ir_type.name) else {
            continue;
        };

        match ir_type.kind.as_str() {
            "type" => {
                let fields = native_field_layouts(strings, &ir_type.fields, 0);
                records.insert(
                    type_id,
                    NativeRecordLayout {
                        size_slots: ir_type.fields.len(),
                        fields: fields
                            .iter()
                            .cloned()
                            .map(|field| (field.name, field))
                            .collect(),
                        ordered_fields: fields,
                    },
                );
            }
            "union" => {
                let mut variants = HashMap::new();
                let mut max_payload_slots = 0usize;
                for variant in &ir_type.variants {
                    max_payload_slots = max_payload_slots.max(variant.fields.len());
                    if let Some(name_id) = string_id(strings, &variant.name) {
                        variants.insert(
                            name_id,
                            NativeVariantLayout {
                                fields: native_field_layouts(strings, &variant.fields, 1),
                            },
                        );
                    }
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

    NativeTypeLayouts { records, unions }
}

fn native_field_layouts(
    strings: &StringPool,
    fields: &[crate::ir::IrField],
    base_offset_slots: usize,
) -> Vec<NativeFieldLayout> {
    fields
        .iter()
        .enumerate()
        .filter_map(|(index, field)| {
            Some(NativeFieldLayout {
                name: string_id(strings, &field.name)?,
                offset_slots: base_offset_slots + index,
            })
        })
        .collect()
}

fn string_id(strings: &StringPool, value: &str) -> Option<u32> {
    strings
        .values
        .iter()
        .position(|existing| existing == value)
        .map(|index| index as u32)
}

fn native_type_id(type_ids: &HashMap<String, u32>, name: &str) -> Option<u32> {
    match name {
        "Nothing" => Some(TYPE_NOTHING),
        "Boolean" => Some(TYPE_BOOLEAN),
        "Integer" => Some(TYPE_INTEGER),
        "Float" => Some(TYPE_FLOAT),
        "Fixed" => Some(TYPE_FIXED),
        "String" => Some(TYPE_STRING),
        "Byte" => Some(TYPE_BYTE),
        "Error" => Some(8),
        _ => type_ids.get(name).copied(),
    }
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
    entry_function: u32,
    entry_flags: u32,
    functions: Vec<Function>,
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

fn lower_project(ir: &IrProject, version: &str) -> Result<BytecodeProject, String> {
    let mut strings = StringPool::new();
    strings.intern(&ir.name);
    strings.intern(version);
    strings.intern("");

    let mut types = TypeTable::new();
    for ir_type in &ir.types {
        types.add_source_type(&mut strings, &ir.name, ir_type);
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

    let mut functions = Vec::new();
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
        )?);
    }

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
        entry_function,
        entry_flags,
        functions,
    })
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
) -> Result<Function, String> {
    let mut builder = FunctionBuilder::new(
        strings,
        types,
        constants,
        function_ids,
        function_return_types,
        function_return_type_names,
        type_model,
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

    let mut flags = FUNCTION_FLAG_PRIVATE;
    if function.kind == "sub" {
        flags |= FUNCTION_FLAG_SUB | FUNCTION_FLAG_RETURNS_NOTHING;
    }
    if function.returns == "Nothing" {
        flags |= FUNCTION_FLAG_RETURNS_NOTHING;
    }

    Ok(Function {
        name: builder.strings.intern(&function.name),
        kind: FUNCTION_BYTECODE,
        flags,
        return_type: builder.type_id(&function.returns),
        params,
        registers: builder.registers,
        code: builder.code,
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
    registers: Vec<Register>,
    code: Vec<Instruction>,
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

    fn push_default(&mut self, register: u32, type_id: u32) {
        self.push(OPCODE_LOAD_DEFAULT, vec![register, type_id]);
    }
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
    ) -> Self {
        Self {
            strings,
            types,
            constants,
            function_ids,
            function_return_types,
            function_return_type_names,
            type_model,
            registers: Vec::new(),
            code: Vec::new(),
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
                if target == builtins::io::print::NAME {
                    return builtins::io::print::lower_bytecode_call(self, args, locals);
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
                let type_id = if op == "&" {
                    TYPE_STRING
                } else if self.registers[left_register.register as usize].type_id == TYPE_FLOAT
                    || self.registers[right_register.register as usize].type_id == TYPE_FLOAT
                    || self.registers[left_register.register as usize].type_id == TYPE_FIXED
                    || self.registers[right_register.register as usize].type_id == TYPE_FIXED
                {
                    TYPE_FLOAT
                } else {
                    TYPE_INTEGER
                };
                let dst = self.add_register(type_id, 0);
                let opcode = match op.as_str() {
                    "+" => OPCODE_ADD,
                    "-" => OPCODE_SUB,
                    "*" => OPCODE_MUL,
                    "/" => OPCODE_DIV,
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
                    } else if type_id == TYPE_FLOAT {
                        "Float".to_string()
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
                let Some(field) = self
                    .type_model
                    .records
                    .get(&target.type_name)
                    .and_then(|record| record.fields.iter().find(|field| field.name == *member))
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
            TYPE_NOTHING | TYPE_BOOLEAN | TYPE_INTEGER | TYPE_FLOAT | TYPE_FIXED | TYPE_STRING
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
        .and_then(|rest| rest.split_once(") AS "))
        .map(|(_, return_type)| return_type.to_string())
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

    fn add_source_type(
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
        let payload = source_type_payload(strings, ir_type);
        self.add_entry(strings, package, &ir_type.name, kind, payload)
    }

    fn type_id(&mut self, strings: &mut StringPool, name: &str) -> u32 {
        match name {
            "Nothing" => TYPE_NOTHING,
            "Boolean" => TYPE_BOOLEAN,
            "Integer" => TYPE_INTEGER,
            "Float" => TYPE_FLOAT,
            "Fixed" => TYPE_FIXED,
            "String" => TYPE_STRING,
            name if name.starts_with("List OF ") => {
                let element = self.type_id(strings, name.trim_start_matches("List OF "));
                self.list_type(strings, element)
            }
            name if name.starts_with("Result OF ") => {
                let success = self.type_id(strings, name.trim_start_matches("Result OF "));
                self.result_type(strings, success)
            }
            name if name.starts_with("FUNC(") => self.function_type(strings, name),
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
            "Byte" => TYPE_BYTE,
            "Error" => {
                strings.intern("code");
                strings.intern("message");
                8
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

    fn function_type(&mut self, strings: &mut StringPool, name: &str) -> u32 {
        if let Some(id) = self.ids.get(name) {
            return *id;
        }
        self.add_entry(strings, "", name, 8, Vec::new())
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
        let id = 9 + self.entries.len() as u32;
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

fn source_type_payload(strings: &mut StringPool, ir_type: &IrType) -> Vec<u8> {
    let mut payload = Vec::new();
    match ir_type.kind.as_str() {
        "type" => {
            put_u32(&mut payload, ir_type.fields.len() as u32);
            for field in &ir_type.fields {
                put_field_payload(strings, &mut payload, field);
            }
        }
        "union" => {
            put_u32(&mut payload, ir_type.includes.len() as u32);
            for include in &ir_type.includes {
                put_u32(&mut payload, strings.intern(include));
            }
            put_u32(&mut payload, ir_type.variants.len() as u32);
            for variant in &ir_type.variants {
                put_u32(&mut payload, strings.intern(&variant.name));
                put_u32(&mut payload, variant.fields.len() as u32);
                for field in &variant.fields {
                    put_field_payload(strings, &mut payload, field);
                }
            }
        }
        "enum" => {
            put_u32(&mut payload, ir_type.members.len() as u32);
            for member in &ir_type.members {
                put_u32(&mut payload, strings.intern(&member.name));
            }
        }
        _ => {}
    }
    payload
}

fn put_field_payload(strings: &mut StringPool, payload: &mut Vec<u8>, field: &crate::ir::IrField) {
    put_u32(payload, strings.intern(&field.name));
    put_u32(payload, strings.intern(&field.type_));
    put_u16(
        payload,
        match field.visibility.as_deref() {
            Some("private") => 1,
            Some("package") => 2,
            Some("export") => 3,
            _ => 0,
        },
    );
    put_u16(payload, 0);
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
                "Boolean" => ConstEntry {
                    kind: 2,
                    payload: vec![if value == "true" { 1 } else { 0 }],
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

        let sections = vec![
            Section::new(SECTION_MANIFEST, self.encode_manifest()),
            Section::new(SECTION_STRING_POOL, self.strings.encode()),
            Section::new(SECTION_TYPE_TABLE, self.types.encode()),
            Section::new(SECTION_CONST_POOL, self.constants.encode()),
            Section::new(SECTION_IMPORT_TABLE, encode_empty_count()),
            Section::new(SECTION_EXPORT_TABLE, self.encode_exports()),
            Section::new(SECTION_GLOBAL_TABLE, encode_empty_count()),
            Section::new(SECTION_FUNCTION_TABLE, self.encode_functions(&code_offsets)),
            Section::new(SECTION_CODE, code_section),
        ];

        encode_sections(&sections)
    }

    fn encode_manifest(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        put_u32(&mut bytes, 0);
        put_u32(&mut bytes, 1);
        put_u32(&mut bytes, 2);
        put_u32(&mut bytes, 2);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u16(&mut bytes, 1);
        put_u16(&mut bytes, 0);
        put_u32(&mut bytes, 0);
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
            if function.kind != FUNCTION_BYTECODE {
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
            .filter(|function| function.kind == FUNCTION_BYTECODE)
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
            put_u32(&mut bytes, 0);
            put_u64(&mut bytes, 0);

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
