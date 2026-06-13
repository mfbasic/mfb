use std::collections::{HashMap, HashSet};

use crate::arch::aarch64::{abi, ops::CodeOp};
use crate::json_string;
use crate::numeric;

use super::nir::{self, NirFunction, NirMatchPattern, NirModule, NirOp, NirRecordUpdate, NirValue};
use super::plan::NativePlan;
use super::runtime;

const RESULT_OK_TAG: &str = "0";
const RESULT_ERR_TAG: &str = "1";
const ERR_OVERFLOW_CODE: &str = "10028";
const ERR_OVERFLOW_MESSAGE: &str = "numeric overflow";
const ERR_OVERFLOW_SYMBOL: &str = "_mfb_str_error_overflow";
const ERR_UNDERFLOW_CODE: &str = "10031";
const ERR_UNDERFLOW_MESSAGE: &str = "numeric underflow";
const ERR_UNDERFLOW_SYMBOL: &str = "_mfb_str_error_underflow";
const ERR_ALLOCATION_MESSAGE: &str = "allocation failed";
const ERR_ALLOCATION_SYMBOL: &str = "_mfb_str_error_allocation";
const ERR_INDEX_OUT_OF_RANGE_CODE: &str = "10001";
const ERR_INDEX_OUT_OF_RANGE_MESSAGE: &str = "index out of range";
const ERR_INDEX_OUT_OF_RANGE_SYMBOL: &str = "_mfb_str_error_index_out_of_range";
const ERR_NOT_FOUND_CODE: &str = "10004";
const ERR_NOT_FOUND_MESSAGE: &str = "not found";
const ERR_NOT_FOUND_SYMBOL: &str = "_mfb_str_error_not_found";
const ERR_ENCODING_CODE: &str = "10019";
const ERR_ENCODING_MESSAGE: &str = "invalid encoding";
const ERR_ENCODING_SYMBOL: &str = "_mfb_str_error_encoding";
const ENTRY_ERROR_PREFIX: &str = "Code: ";
const ENTRY_ERROR_PREFIX_SYMBOL: &str = "_mfb_str_entry_error_prefix";
const ENTRY_ERROR_SEPARATOR: &str = " Message: ";
const ENTRY_ERROR_SEPARATOR_SYMBOL: &str = "_mfb_str_entry_error_separator";
const ENTRY_ERROR_NEWLINE: &str = "\n";
const ENTRY_ERROR_NEWLINE_SYMBOL: &str = "_mfb_str_entry_error_newline";
const RESULT_TAG_REGISTER: &str = abi::RETURN_REGISTER;
const RESULT_VALUE_REGISTER: &str = "x1";
const RESULT_ERROR_MESSAGE_REGISTER: &str = "x2";
const ARENA_ALLOC_SYMBOL: &str = "_mfb_arena_alloc";
const ARENA_DESTROY_SYMBOL: &str = "_mfb_arena_destroy";
const ARENA_STATE_REGISTER: &str = "x19";
const ARENA_STATE_SIZE: usize = 64;
const ARENA_DEFAULT_BLOCK_SIZE: u64 = 4096;
const ARENA_BLOCK_HEADER_SIZE: usize = 32;
const ERR_INVALID_ARGUMENT_CODE: &str = "10002";
const ERR_INVALID_ARGUMENT_MESSAGE: &str = "invalid argument";
const ERR_INVALID_ARGUMENT_SYMBOL: &str = "_mfb_str_error_invalid_argument";
const ERR_INVALID_FORMAT_CODE: &str = "10003";
const ERR_INVALID_FORMAT_MESSAGE: &str = "invalid format";
const ERR_INVALID_FORMAT_SYMBOL: &str = "_mfb_str_error_invalid_format";
const ERR_OUT_OF_MEMORY_CODE: &str = "10010";
const COLLECTION_KIND_LIST: usize = 0;
const COLLECTION_KIND_MAP: usize = 1;
const COLLECTION_HEADER_SIZE: usize = 40;
const COLLECTION_OFFSET_KIND: usize = 0;
const COLLECTION_OFFSET_KEY_TYPE: usize = 1;
const COLLECTION_OFFSET_VALUE_TYPE: usize = 2;
const COLLECTION_OFFSET_FLAGS_VERSION: usize = 3;
const COLLECTION_OFFSET_COUNT: usize = 8;
const COLLECTION_OFFSET_CAPACITY: usize = 16;
const COLLECTION_OFFSET_DATA_LENGTH: usize = 24;
const COLLECTION_OFFSET_DATA_CAPACITY: usize = 32;
const COLLECTION_ENTRY_SIZE: usize = 40;
const COLLECTION_ENTRY_OFFSET_FLAGS: usize = 0;
const COLLECTION_ENTRY_OFFSET_KEY_OFFSET: usize = 8;
const COLLECTION_ENTRY_OFFSET_KEY_LENGTH: usize = 16;
const COLLECTION_ENTRY_OFFSET_VALUE_OFFSET: usize = 24;
const COLLECTION_ENTRY_OFFSET_VALUE_LENGTH: usize = 32;
const COLLECTION_ENTRY_FLAG_USED: usize = 1;
const COLLECTION_TYPE_NONE: usize = 0;
const COLLECTION_TYPE_BOOLEAN: usize = 2;
const COLLECTION_TYPE_INTEGER: usize = 3;
const COLLECTION_TYPE_FLOAT: usize = 4;
const COLLECTION_TYPE_FIXED: usize = 5;
const COLLECTION_TYPE_STRING: usize = 6;
const COLLECTION_TYPE_BYTE: usize = 7;
const COLLECTION_TYPE_LIST: usize = 20;
const COLLECTION_TYPE_MAP: usize = 21;

pub(crate) struct NativeCodePlan {
    pub(crate) target: String,
    pub(crate) arch: String,
    pub(crate) project: String,
    pub(crate) entry_symbol: Option<String>,
    pub(crate) imports: Vec<CodeImport>,
    pub(crate) data_objects: Vec<CodeDataObject>,
    pub(crate) functions: Vec<CodeFunction>,
}

pub(crate) struct CodeFunction {
    pub(crate) name: String,
    pub(crate) symbol: String,
    pub(crate) params: Vec<CodeParam>,
    pub(crate) returns: String,
    pub(crate) frame: CodeFrame,
    pub(crate) instructions: Vec<CodeInstruction>,
    pub(crate) relocations: Vec<CodeRelocation>,
    pub(crate) stack_slots: Vec<CodeStackSlot>,
}

pub(crate) struct CodeFrame {
    pub(crate) stack_size: usize,
    pub(crate) callee_saved: Vec<String>,
}

pub(crate) struct CodeParam {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) location: String,
}

pub(crate) struct CodeInstruction {
    pub(crate) op: CodeOp,
    pub(crate) fields: Vec<(&'static str, String)>,
}

pub(crate) struct CodeRelocation {
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) kind: String,
    pub(crate) binding: String,
    pub(crate) library: Option<String>,
}

pub(crate) struct CodeImport {
    pub(crate) library: String,
    pub(crate) symbol: String,
}

pub(crate) struct CodeDataObject {
    pub(crate) symbol: String,
    pub(crate) kind: String,
    pub(crate) layout: String,
    pub(crate) align: usize,
    pub(crate) size: usize,
    pub(crate) value: String,
}

pub(crate) trait CodegenPlatform {
    fn target(&self) -> &'static str;
    fn arch(&self) -> &'static str;
    fn preserves_link_register_in_runtime_helpers(&self) -> bool;
    fn emit_program_exit(
        &self,
        from: &str,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_write(
        &self,
        from: &str,
        platform_imports: &HashMap<String, String>,
        instructions: &mut Vec<CodeInstruction>,
        relocations: &mut Vec<CodeRelocation>,
    ) -> Result<(), String>;
    fn emit_arena_map(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String>;
    fn emit_arena_unmap(&self, instructions: &mut Vec<CodeInstruction>) -> Result<(), String>;
}

pub(crate) struct CodeStackSlot {
    pub(crate) name: String,
    pub(crate) type_: String,
    pub(crate) offset: i32,
}

struct CodeBuilder<'a> {
    current_symbol: String,
    function_symbols: &'a HashMap<String, String>,
    functions: &'a HashMap<String, &'a NirFunction>,
    platform_imports: &'a HashMap<String, String>,
    type_model: TypeModel,
    string_symbols: &'a HashMap<String, String>,
    locals: HashMap<String, LocalValue>,
    instructions: Vec<CodeInstruction>,
    relocations: Vec<CodeRelocation>,
    stack_slots: Vec<CodeStackSlot>,
    used_callee_saved: Vec<String>,
    stack_size: usize,
    next_register: usize,
    next_label: usize,
}

#[derive(Clone)]
struct LocalValue {
    type_: String,
    stack_offset: usize,
    constant: Option<NirValue>,
}

#[derive(Clone)]
struct ValueResult {
    type_: String,
    location: String,
    text: String,
}

#[derive(Clone)]
struct PayloadSlot {
    slot: usize,
    type_: String,
}

#[derive(Clone)]
struct CollectionValueSlot {
    key: Option<PayloadSlot>,
    value: PayloadSlot,
}

struct CollectionTypeLayout {
    kind: usize,
    key_type_code: usize,
    value_type_code: usize,
}

#[derive(Clone)]
struct TypeModel {
    enum_members: HashMap<(String, String), usize>,
    record_fields: HashMap<String, Vec<(String, String)>>,
    union_names: HashSet<String>,
    union_variants: HashMap<String, String>,
    union_variant_tags: HashMap<String, usize>,
    union_variant_fields: HashMap<String, Vec<(String, String)>>,
}

pub(crate) fn lower_module_for_platform(
    module: &NirModule,
    native_plan: &NativePlan,
    platform: &dyn CodegenPlatform,
) -> Result<NativeCodePlan, String> {
    if module.target != platform.target() {
        return Err(format!(
            "native code platform '{}' cannot lower module target '{}'",
            platform.target(),
            module.target
        ));
    }
    let function_symbols = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), nir::function_symbol(&function.name)))
        .collect::<HashMap<_, _>>();
    let functions = module
        .functions
        .iter()
        .map(|function| (function.name.clone(), function))
        .collect::<HashMap<_, _>>();
    let platform_imports = native_plan
        .platform_imports
        .iter()
        .map(|import| (import.symbol.clone(), import.library.clone()))
        .collect::<HashMap<_, _>>();
    let imports = native_plan
        .platform_imports
        .iter()
        .map(|import| CodeImport {
            library: import.library.clone(),
            symbol: import.symbol.clone(),
        })
        .collect::<Vec<_>>();
    let string_symbols = string_symbols(module);
    let mut string_objects = string_symbols.iter().collect::<Vec<_>>();
    string_objects.sort_by(|(_, left_symbol), (_, right_symbol)| left_symbol.cmp(right_symbol));
    let data_objects = string_objects
        .into_iter()
        .map(|(value, symbol)| CodeDataObject {
            symbol: symbol.clone(),
            kind: "constant".to_string(),
            layout: "mfb.string.v1 { u64 byteLength; u8 bytes[byteLength]; u8 nul }".to_string(),
            align: 8,
            size: align(8 + value.len() + 1, 8),
            value: value.clone(),
        })
        .collect::<Vec<_>>();
    let type_model = TypeModel::from_module(module)?;
    let mut code_functions = Vec::new();

    if let Some(entry) = &module.entry {
        let entry_symbol = nir::function_symbol(&entry.name);
        code_functions.push(lower_program_entry(
            &entry_symbol,
            &entry.returns,
            &platform_imports,
            platform,
        )?);
    }
    for function in &module.functions {
        code_functions.push(lower_function(
            function,
            &function_symbols,
            &functions,
            &platform_imports,
            &string_symbols,
            type_model.clone(),
        )?);
    }
    code_functions.push(lower_arena_alloc(platform)?);
    code_functions.push(lower_arena_destroy(platform)?);
    for symbol in &native_plan.runtime_symbols {
        code_functions.push(lower_runtime_helper(symbol, &platform_imports, platform)?);
    }

    let plan = NativeCodePlan {
        target: module.target.clone(),
        arch: platform.arch().to_string(),
        project: module.project.clone(),
        entry_symbol: module.entry.as_ref().map(|_| "_main".to_string()),
        imports,
        data_objects,
        functions: code_functions,
    };
    plan.validate()?;
    Ok(plan)
}

impl NativeCodePlan {
    pub(crate) fn validate(&self) -> Result<(), String> {
        if self.target.is_empty() {
            return Err("native code plan target must not be empty".to_string());
        }
        if self.arch.is_empty() {
            return Err("native code plan arch must not be empty".to_string());
        }
        if self.project.is_empty() {
            return Err("native code plan project name must not be empty".to_string());
        }
        if self.functions.is_empty() {
            return Err("native code plan requires at least one function".to_string());
        }
        if let Some(entry_symbol) = &self.entry_symbol {
            if !self
                .functions
                .iter()
                .any(|function| &function.symbol == entry_symbol)
            {
                return Err(format!(
                    "native code plan entry symbol '{entry_symbol}' does not resolve"
                ));
            }
        }
        let defined_symbols = self
            .functions
            .iter()
            .map(|function| function.symbol.clone())
            .collect::<Vec<_>>();
        let imported_symbols = self
            .imports
            .iter()
            .map(|import| import.symbol.clone())
            .collect::<Vec<_>>();
        for import in &self.imports {
            if import.library.is_empty() || import.symbol.is_empty() {
                return Err("native code plan contains an incomplete import".to_string());
            }
        }
        let data_symbols = self
            .data_objects
            .iter()
            .map(|object| object.symbol.clone())
            .collect::<Vec<_>>();
        for object in &self.data_objects {
            if object.symbol.is_empty() || object.kind.is_empty() || object.layout.is_empty() {
                return Err("native code plan contains an incomplete data object".to_string());
            }
            if object.align == 0 || object.size == 0 {
                return Err(format!(
                    "native code data object '{}' must have nonzero size and alignment",
                    object.symbol
                ));
            }
        }
        for function in &self.functions {
            function.validate(&defined_symbols, &imported_symbols, &data_symbols)?;
        }
        Ok(())
    }

    pub(crate) fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"format\": \"mfb-native-code-plan\",\n",
                "  \"version\": 1,\n",
                "  \"target\": {},\n",
                "  \"arch\": {},\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"dataObjects\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
            json_string(&self.arch),
            json_string(&self.project),
            self.entry_symbol
                .as_ref()
                .map(|symbol| json_string(symbol))
                .unwrap_or_else(|| "null".to_string()),
            join_json(&self.imports, 2),
            join_json(&self.data_objects, 2),
            join_json(&self.functions, 2)
        )
    }
}

impl CodeFunction {
    fn validate(
        &self,
        defined_symbols: &[String],
        imported_symbols: &[String],
        data_symbols: &[String],
    ) -> Result<(), String> {
        if self.name.is_empty() || self.symbol.is_empty() {
            return Err("native code function name and symbol must not be empty".to_string());
        }
        if self.instructions.is_empty() {
            return Err(format!(
                "native code function '{}' has no instructions",
                self.name
            ));
        }
        if !self
            .instructions
            .iter()
            .any(|instruction| instruction.op == CodeOp::Ret)
        {
            return Err(format!(
                "native code function '{}' has no return instruction",
                self.name
            ));
        }
        for relocation in &self.relocations {
            if relocation.from != self.symbol {
                return Err(format!(
                    "native code relocation source '{}' does not match function '{}'",
                    relocation.from, self.symbol
                ));
            }
            match relocation.binding.as_str() {
                "internal" => {
                    if !defined_symbols.contains(&relocation.to) {
                        return Err(format!(
                            "native code internal relocation target '{}' is not defined",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_some() {
                        return Err(format!(
                            "native code internal relocation '{}' must not name a library",
                            relocation.to
                        ));
                    }
                }
                "external" => {
                    if !imported_symbols.contains(&relocation.to) {
                        return Err(format!(
                            "native code external relocation target '{}' is not imported",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_none() {
                        return Err(format!(
                            "native code external relocation '{}' must name a library",
                            relocation.to
                        ));
                    }
                }
                "data" => {
                    if !data_symbols.contains(&relocation.to)
                        && !defined_symbols.contains(&relocation.to)
                    {
                        return Err(format!(
                            "native code data relocation target '{}' is not a data object or defined symbol",
                            relocation.to
                        ));
                    }
                    if relocation.library.is_some() {
                        return Err(format!(
                            "native code data relocation '{}' must not name a library",
                            relocation.to
                        ));
                    }
                }
                other => {
                    return Err(format!(
                        "native code relocation '{}' has invalid binding '{}'",
                        relocation.to, other
                    ));
                }
            }
        }
        for instruction in &self.instructions {
            instruction.validate()?;
        }
        Ok(())
    }
}

impl TypeModel {
    fn from_module(module: &NirModule) -> Result<Self, String> {
        let mut enum_members = HashMap::new();
        let mut record_fields = HashMap::new();
        let mut union_names = HashSet::new();
        let mut union_variants = HashMap::new();
        let mut union_variant_tags = HashMap::new();
        for type_ in &module.types {
            match type_.kind.as_str() {
                "type" | "record" => {
                    record_fields.insert(
                        type_.name.clone(),
                        type_
                            .fields
                            .iter()
                            .map(|field| (field.name.clone(), field.type_.clone()))
                            .collect(),
                    );
                }
                "enum" => {
                    for (index, member) in type_.members.iter().enumerate() {
                        enum_members.insert((type_.name.clone(), member.name.clone()), index);
                    }
                }
                "union" => {
                    union_names.insert(type_.name.clone());
                    for (index, variant) in expanded_nir_union_variants(module, &type_.name)
                        .iter()
                        .enumerate()
                    {
                        union_variants.insert(variant.name.clone(), type_.name.clone());
                        union_variant_tags.insert(variant.name.clone(), index);
                    }
                }
                "resource" => {}
                other => {
                    return Err(format!(
                        "native code plan does not know type kind '{other}'"
                    ));
                }
            }
        }
        Ok(Self {
            enum_members,
            record_fields,
            union_names,
            union_variants,
            union_variant_tags,
            union_variant_fields: module
                .types
                .iter()
                .filter(|type_| type_.kind == "union")
                .flat_map(|type_| {
                    expanded_nir_union_variants(module, &type_.name)
                        .into_iter()
                        .map(|variant| {
                            (
                                variant.name.clone(),
                                variant
                                    .fields
                                    .iter()
                                    .map(|field| (field.name.clone(), field.type_.clone()))
                                    .collect(),
                            )
                        })
                })
                .collect(),
        })
    }
}

fn expanded_nir_union_variants<'a>(
    module: &'a NirModule,
    union_name: &str,
) -> Vec<&'a super::nir::NirVariant> {
    let Some(type_) = module
        .types
        .iter()
        .find(|candidate| candidate.kind == "union" && candidate.name == union_name)
    else {
        return Vec::new();
    };
    let mut variants = Vec::new();
    for include in &type_.includes {
        variants.extend(expanded_nir_union_variants(module, include));
    }
    variants.extend(type_.variants.iter());
    variants
}

fn lower_program_entry(
    language_entry_symbol: &str,
    language_entry_returns: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let mut instructions = vec![
        abi::label("entry"),
        abi::subtract_stack(ARENA_STATE_SIZE),
        abi::add_immediate(ARENA_STATE_REGISTER, abi::stack_pointer(), 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 8),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 16),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 24),
        abi::branch_link(language_entry_symbol),
    ];
    let mut relocations = vec![CodeRelocation {
        from: "_main".to_string(),
        to: language_entry_symbol.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }];
    let error_label = "entry_error";
    let exit_label = "entry_exit";
    instructions.extend([
        abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG),
        abi::branch_ne(error_label),
    ]);
    if language_entry_returns == "Nothing" {
        instructions.push(abi::move_immediate(abi::return_register(), "Integer", "0"));
    } else {
        instructions.push(abi::move_register(
            abi::return_register(),
            RESULT_VALUE_REGISTER,
        ));
    }
    instructions.push(abi::branch(exit_label));
    instructions.extend([
        abi::label(error_label),
        abi::store_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 32),
        abi::move_register("x20", RESULT_ERROR_MESSAGE_REGISTER),
    ]);
    emit_write_string_object(
        ENTRY_ERROR_PREFIX_SYMBOL,
        "_main",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_integer_to_stderr(
        "_main",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_SEPARATOR_SYMBOL,
        "_main",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::load_u64(abi::string_length_register(), "x20", 0),
        abi::add_immediate(abi::string_data_register(), "x20", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(
        "_main",
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    emit_write_string_object(
        ENTRY_ERROR_NEWLINE_SYMBOL,
        "_main",
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.push(abi::move_immediate(
        abi::return_register(),
        "Integer",
        "255",
    ));
    instructions.push(abi::label(exit_label));
    instructions.extend([
        abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 32),
        abi::store_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 40),
        abi::store_u64(RESULT_ERROR_MESSAGE_REGISTER, ARENA_STATE_REGISTER, 48),
        abi::branch_link(ARENA_DESTROY_SYMBOL),
        abi::load_u64(abi::return_register(), ARENA_STATE_REGISTER, 32),
        abi::load_u64(RESULT_VALUE_REGISTER, ARENA_STATE_REGISTER, 40),
        abi::load_u64(RESULT_ERROR_MESSAGE_REGISTER, ARENA_STATE_REGISTER, 48),
    ]);
    relocations.push(CodeRelocation {
        from: "_main".to_string(),
        to: ARENA_DESTROY_SYMBOL.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    });
    platform.emit_program_exit("_main", &mut instructions, &mut relocations)?;
    Ok(CodeFunction {
        name: "program.entry".to_string(),
        symbol: "_main".to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations,
    })
}

fn lower_arena_alloc(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = Vec::new();
    instructions.extend([
        abi::label("entry"),
        abi::compare_immediate("x1", "0"),
        abi::branch_eq("arena_alloc_invalid"),
        abi::subtract_immediate("x9", "x1", 1),
        abi::and_registers("x10", "x1", "x9"),
        abi::compare_immediate("x10", "0"),
        abi::branch_ne("arena_alloc_invalid"),
        abi::compare_immediate("x0", "0"),
        abi::branch_ne("arena_alloc_size_nonzero"),
        abi::move_immediate("x0", "Integer", "1"),
        abi::label("arena_alloc_size_nonzero"),
        abi::move_register("x20", "x0"),
        abi::move_register("x21", "x1"),
        abi::label("arena_alloc_try_current"),
        abi::load_u64("x22", ARENA_STATE_REGISTER, 0),
        abi::compare_immediate("x22", "0"),
        abi::branch_eq("arena_alloc_grow"),
        abi::load_u64("x23", "x22", 16),
        abi::load_u64("x24", "x22", 24),
        abi::add_immediate("x25", "x22", ARENA_BLOCK_HEADER_SIZE),
        abi::add_registers("x26", "x25", "x24"),
        abi::compare_registers("x26", "x25"),
        abi::branch_lo("arena_alloc_oom"),
        abi::subtract_immediate("x27", "x21", 1),
        abi::move_register("x15", "x26"),
        abi::add_registers("x26", "x26", "x27"),
        abi::compare_registers("x26", "x15"),
        abi::branch_lo("arena_alloc_oom"),
        abi::bitwise_not("x27", "x27"),
        abi::and_registers("x26", "x26", "x27"),
        abi::add_registers("x28", "x26", "x20"),
        abi::compare_registers("x28", "x26"),
        abi::branch_lo("arena_alloc_oom"),
        abi::subtract_registers("x28", "x28", "x25"),
        abi::compare_registers("x28", "x23"),
        abi::branch_hi("arena_alloc_grow"),
        abi::store_u64("x28", "x22", 24),
        abi::move_immediate(abi::return_register(), "Integer", RESULT_OK_TAG),
        abi::move_register("x1", "x26"),
        abi::return_(),
        abi::label("arena_alloc_grow"),
        abi::add_registers("x23", "x20", "x21"),
        abi::compare_registers("x23", "x20"),
        abi::branch_lo("arena_alloc_oom"),
        abi::add_immediate("x23", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::move_immediate("x14", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::compare_registers("x23", "x14"),
        abi::branch_hi("arena_alloc_normal_block"),
        abi::move_immediate("x23", "Integer", &ARENA_DEFAULT_BLOCK_SIZE.to_string()),
        abi::branch("arena_alloc_map_size_ready"),
        abi::label("arena_alloc_normal_block"),
        abi::move_register("x15", "x23"),
        abi::add_immediate("x23", "x23", 4095),
        abi::compare_registers("x23", "x15"),
        abi::branch_lo("arena_alloc_oom"),
        abi::move_immediate("x24", "Integer", &(!4095_u64).to_string()),
        abi::and_registers("x23", "x23", "x24"),
        abi::label("arena_alloc_map_size_ready"),
    ]);
    platform.emit_arena_map(&mut instructions)?;
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_ge("arena_alloc_mapped"),
        abi::branch("arena_alloc_oom"),
        abi::label("arena_alloc_mapped"),
        abi::load_u64("x24", ARENA_STATE_REGISTER, 0),
        abi::store_u64("x24", abi::return_register(), 0),
        abi::store_u64("x23", abi::return_register(), 8),
        abi::subtract_immediate("x24", "x23", ARENA_BLOCK_HEADER_SIZE),
        abi::store_u64("x24", abi::return_register(), 16),
        abi::store_u64("x31", abi::return_register(), 24),
        abi::store_u64(abi::return_register(), ARENA_STATE_REGISTER, 0),
        abi::branch("arena_alloc_try_current"),
        abi::label("arena_alloc_invalid"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_INVALID_ARGUMENT_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::return_(),
        abi::label("arena_alloc_oom"),
        abi::move_immediate(abi::return_register(), "Integer", ERR_OUT_OF_MEMORY_CODE),
        abi::move_immediate("x1", "Integer", "0"),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.arena_alloc".to_string(),
        symbol: ARENA_ALLOC_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Pointer".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    })
}

fn lower_arena_destroy(platform: &dyn CodegenPlatform) -> Result<CodeFunction, String> {
    let mut instructions = Vec::new();
    instructions.extend([
        abi::label("entry"),
        abi::load_u64("x20", ARENA_STATE_REGISTER, 0),
        abi::label("arena_destroy_loop"),
        abi::compare_immediate("x20", "0"),
        abi::branch_eq("arena_destroy_done"),
        abi::load_u64("x21", "x20", 0),
        abi::load_u64("x1", "x20", 8),
        abi::move_register(abi::return_register(), "x20"),
    ]);
    platform.emit_arena_unmap(&mut instructions)?;
    instructions.extend([
        abi::move_register("x20", "x21"),
        abi::branch("arena_destroy_loop"),
        abi::label("arena_destroy_done"),
        abi::store_u64("x31", ARENA_STATE_REGISTER, 0),
        abi::return_(),
    ]);
    Ok(CodeFunction {
        name: "runtime.arena_destroy".to_string(),
        symbol: ARENA_DESTROY_SYMBOL.to_string(),
        params: Vec::new(),
        returns: "Nothing".to_string(),
        frame: CodeFrame {
            stack_size: 0,
            callee_saved: Vec::new(),
        },
        stack_slots: Vec::new(),
        instructions,
        relocations: Vec::new(),
    })
}

fn emit_write_string_object(
    symbol: &str,
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    instructions.extend([
        abi::load_page_address("x21", symbol),
        abi::add_page_offset("x21", "x21", symbol),
        abi::load_u64(abi::string_length_register(), "x21", 0),
        abi::add_immediate(abi::string_data_register(), "x21", 8),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    relocations.extend([
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: from.to_string(),
            to: symbol.to_string(),
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)
}

fn emit_write_integer_to_stderr(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    let absolute_ready_label = "entry_error_code_absolute_ready";
    let digit_loop_label = "entry_error_code_digit_loop";
    let digits_done_label = "entry_error_code_digits_done";
    let write_label = "entry_error_code_write";
    instructions.extend([
        abi::subtract_stack(64),
        abi::load_u64("x21", ARENA_STATE_REGISTER, 32),
        abi::compare_immediate("x21", "0"),
        abi::branch_ge(absolute_ready_label),
        abi::move_immediate("x22", "Integer", "0"),
        abi::subtract_registers("x21", "x22", "x21"),
        abi::label(absolute_ready_label),
        abi::add_immediate("x23", abi::stack_pointer(), 64),
        abi::move_immediate("x24", "Integer", "10"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(digit_loop_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "48"),
        abi::store_u8("x22", "x23", 0),
        abi::branch(digits_done_label),
        abi::label(digit_loop_label),
        abi::unsigned_divide_registers("x25", "x21", "x24"),
        abi::multiply_subtract_registers("x26", "x25", "x24", "x21"),
        abi::add_immediate("x26", "x26", 48),
        abi::subtract_immediate("x23", "x23", 1),
        abi::store_u8("x26", "x23", 0),
        abi::move_register("x21", "x25"),
        abi::compare_immediate("x21", "0"),
        abi::branch_ne(digit_loop_label),
        abi::label(digits_done_label),
        abi::compare_immediate("x19", "0"),
        abi::branch_ge(write_label),
        abi::subtract_immediate("x23", "x23", 1),
        abi::move_immediate("x22", "Integer", "45"),
        abi::store_u8("x22", "x23", 0),
        abi::label(write_label),
        abi::add_immediate("x27", abi::stack_pointer(), 64),
        abi::subtract_registers(abi::string_length_register(), "x27", "x23"),
        abi::move_register(abi::string_data_register(), "x23"),
        abi::move_immediate(abi::return_register(), "Integer", "2"),
    ]);
    platform.emit_write(from, platform_imports, instructions, relocations)?;
    instructions.push(abi::add_stack(64));
    Ok(())
}

fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    platform_imports: &HashMap<String, String>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let params = function
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let location = abi::argument_register(index)?;
            Ok(CodeParam {
                name: param.name.clone(),
                type_: param.type_.clone(),
                location,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;

    let mut builder = CodeBuilder {
        current_symbol: nir::function_symbol(&function.name),
        function_symbols,
        functions,
        platform_imports,
        type_model,
        string_symbols,
        locals: HashMap::new(),
        instructions: vec![abi::label("entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_label: 0,
    };
    for param in &params {
        let stack_offset = builder.allocate_stack_object(&param.name, 8);
        builder.locals.insert(
            param.name.clone(),
            LocalValue {
                type_: param.type_.clone(),
                stack_offset,
                constant: None,
            },
        );
        builder.emit(abi::store_u64(
            &param.location,
            abi::stack_pointer(),
            stack_offset,
        ));
    }
    builder.lower_ops(&function.body)?;
    if !builder.current_block_returns() {
        builder.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_OK_TAG,
        ));
        builder.emit(abi::return_());
    }
    let mut instructions = builder.instructions;
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: function.name.clone(),
        symbol: nir::function_symbol(&function.name),
        params,
        returns: function.returns.clone(),
        frame,
        instructions,
        relocations: builder.relocations,
        stack_slots,
    })
}

fn lower_runtime_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<CodeFunction, String> {
    let Some(spec) = runtime::spec_for_symbol(symbol) else {
        return Err(format!(
            "native code plan does not emit runtime helper '{symbol}'"
        ));
    };
    match spec.call {
        "io.print" => {
            let (frame, instructions, relocations) =
                lower_io_print_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: "runtime.io.print".to_string(),
                symbol: symbol.to_string(),
                params: spec
                    .abi
                    .params
                    .iter()
                    .map(|param| CodeParam {
                        name: param.name.to_string(),
                        type_: param.type_.to_string(),
                        location: param.location.to_string(),
                    })
                    .collect(),
                returns: spec.abi.returns.to_string(),
                frame,
                stack_slots: Vec::new(),
                instructions,
                relocations,
            })
        }
        other => Err(format!(
            "native code plan does not emit runtime call '{other}'"
        )),
    }
}

fn lower_io_print_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(16)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            0,
        ));
    }
    instructions.extend([
        abi::load_u64(abi::string_length_register(), abi::return_register(), 0),
        abi::add_immediate(abi::string_data_register(), abi::return_register(), 8),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        abi::move_immediate(abi::newline_scratch_register(), "Integer", "10"),
        abi::store_u64(abi::newline_scratch_register(), abi::stack_pointer(), 8),
        abi::move_immediate(abi::return_register(), "Integer", "1"),
        abi::add_immediate(abi::string_data_register(), abi::stack_pointer(), 8),
        abi::move_immediate(abi::string_length_register(), "Integer", "1"),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
            abi::add_stack(16),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: 16,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(16), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: 16,
                callee_saved: Vec::new(),
            },
            instructions,
            relocations,
        ))
    }
}

fn finalize_frame(
    instructions: &mut Vec<CodeInstruction>,
    stack_slots: &mut [CodeStackSlot],
    local_stack_size: usize,
    mut callee_saved: Vec<String>,
) -> CodeFrame {
    if instructions.iter().any(|instruction| {
        instruction.op == CodeOp::BranchLink || instruction.op == CodeOp::BranchLinkRegister
    }) && !callee_saved
        .iter()
        .any(|register| register == abi::link_register())
    {
        callee_saved.push(abi::link_register().to_string());
    }
    let save_size = callee_saved.len() * 8;
    let total_stack_size = align(save_size + local_stack_size, 16);
    if total_stack_size == 0 {
        return CodeFrame {
            stack_size: 0,
            callee_saved,
        };
    }

    for slot in stack_slots {
        slot.offset += save_size as i32;
    }
    adjust_stack_instruction_offsets(instructions, save_size);

    let mut prologue = Vec::new();
    prologue.push(abi::subtract_stack(total_stack_size));
    for (index, register) in callee_saved.iter().enumerate() {
        prologue.push(abi::store_u64(register, abi::stack_pointer(), index * 8));
    }

    let insert_at = if instructions
        .first()
        .is_some_and(|instruction| instruction.op == CodeOp::Label)
    {
        1
    } else {
        0
    };
    instructions.splice(insert_at..insert_at, prologue);

    let mut rewritten = Vec::new();
    for instruction in instructions.drain(..) {
        if instruction.op == CodeOp::Ret {
            for (index, register) in callee_saved.iter().enumerate().rev() {
                rewritten.push(abi::load_u64(register, abi::stack_pointer(), index * 8));
            }
            rewritten.push(abi::add_stack(total_stack_size));
            rewritten.push(instruction);
        } else {
            rewritten.push(instruction);
        }
    }
    *instructions = rewritten;

    CodeFrame {
        stack_size: total_stack_size,
        callee_saved,
    }
}

fn adjust_stack_instruction_offsets(instructions: &mut [CodeInstruction], offset_delta: usize) {
    if offset_delta == 0 {
        return;
    }
    for instruction in instructions {
        let stack_relative = instruction
            .fields
            .iter()
            .any(|(name, value)| matches!(*name, "base" | "src") && abi::is_stack_pointer(value));
        if !stack_relative {
            continue;
        }
        for (name, value) in &mut instruction.fields {
            if matches!(*name, "offset" | "imm") {
                if let Ok(offset) = value.parse::<usize>() {
                    *value = (offset + offset_delta).to_string();
                }
            }
        }
    }
}

impl CodeBuilder<'_> {
    fn lower_ops(&mut self, ops: &[NirOp]) -> Result<(), String> {
        for op in ops {
            match op {
                NirOp::Bind {
                    name, type_, value, ..
                } => {
                    let stack_offset = self.allocate_stack_object(name, 8);
                    let constant = value
                        .as_ref()
                        .and_then(|value| self.local_constant_value(value));
                    self.locals.insert(
                        name.clone(),
                        LocalValue {
                            type_: type_.clone(),
                            stack_offset,
                            constant,
                        },
                    );
                    if let Some(value) = value {
                        let result = self.lower_value(value)?;
                        self.emit(abi::store_u64(
                            &result.location,
                            abi::stack_pointer(),
                            stack_offset,
                        ));
                    } else if is_collection_type(type_) {
                        let result = self.lower_empty_collection(type_)?;
                        self.emit(abi::store_u64(
                            &result.location,
                            abi::stack_pointer(),
                            stack_offset,
                        ));
                    } else if let Some(fields) = self.type_model.record_fields.get(type_).cloned() {
                        let record_offset = self.allocate_stack_object(type_, 8 * fields.len());
                        for index in 0..fields.len() {
                            self.emit(abi::store_u64(
                                "x31",
                                abi::stack_pointer(),
                                record_offset + 8 * index,
                            ));
                        }
                        let register = self.allocate_register()?;
                        self.emit(abi::add_immediate(
                            &register,
                            abi::stack_pointer(),
                            record_offset,
                        ));
                        self.emit(abi::store_u64(
                            &register,
                            abi::stack_pointer(),
                            stack_offset,
                        ));
                    }
                }
                NirOp::Assign { name, value } => {
                    let stack_offset = self
                        .locals
                        .get(name)
                        .ok_or_else(|| format!("native code assignment unknown local '{name}'"))?
                        .stack_offset;
                    let result = self.lower_value(value)?;
                    self.emit(abi::store_u64(
                        &result.location,
                        abi::stack_pointer(),
                        stack_offset,
                    ));
                    let constant = self.local_constant_value(value);
                    if let Some(local) = self.locals.get_mut(name) {
                        local.constant = constant;
                    }
                }
                NirOp::Eval { value } => {
                    self.lower_value(value)?;
                }
                NirOp::Return { value } => {
                    if let Some(value) = value {
                        let result = self.lower_value(value)?;
                        if result.type_ != "Nothing" {
                            self.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
                        }
                    }
                    self.emit(abi::move_immediate(
                        RESULT_TAG_REGISTER,
                        "Integer",
                        RESULT_OK_TAG,
                    ));
                    self.emit(abi::return_());
                }
                NirOp::Fail { error } => {
                    self.emit_error_return(error)?;
                }
                NirOp::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let condition = self.lower_value(condition)?;
                    let else_label = self.label("if_else");
                    let end_label = self.label("if_end");
                    let constants_before_if = self.local_constants();
                    self.emit(abi::compare_immediate(&condition.location, "0"));
                    self.emit(abi::branch_eq(&else_label).field("reason", "ifFalse"));
                    self.lower_ops(then_body)?;
                    if !self.current_block_returns() {
                        self.emit(abi::branch(&end_label));
                    }
                    self.emit(abi::label(&else_label));
                    self.restore_local_constants(&constants_before_if);
                    self.lower_ops(else_body)?;
                    self.emit(abi::label(&end_label));
                    self.clear_local_constants();
                }
                NirOp::Match { value, cases } => {
                    let matched = self.lower_value(value)?;
                    let end_label = self.label("match_end");
                    let mut case_labels = Vec::new();
                    let mut else_label = None;
                    for case in cases {
                        let label = self.label("match_case");
                        match &case.pattern {
                            NirMatchPattern::Else => else_label = Some(label.clone()),
                            NirMatchPattern::Value(pattern) => {
                                self.lower_match_compare(&matched, pattern, &label)?;
                            }
                        }
                        case_labels.push((label, case));
                    }
                    self.emit(abi::branch(else_label.as_deref().unwrap_or(&end_label)));
                    let constants_before_match = self.local_constants();
                    for (label, case) in case_labels {
                        self.restore_local_constants(&constants_before_match);
                        self.emit(abi::label(&label));
                        self.lower_ops(&case.body)?;
                        if !self.current_block_returns() {
                            self.emit(abi::branch(&end_label));
                        }
                    }
                    self.emit(abi::label(&end_label));
                    self.clear_local_constants();
                }
                NirOp::ForEach {
                    name,
                    type_,
                    iterable,
                    body,
                } => {
                    self.lower_for_each(name, type_, iterable, body)?;
                }
                NirOp::Using {
                    name,
                    type_,
                    close,
                    value,
                    body,
                } => {
                    let stack_offset = self.allocate_stack_object(name, 8);
                    let result = self.lower_value(value)?;
                    self.locals.insert(
                        name.clone(),
                        LocalValue {
                            type_: type_.clone(),
                            stack_offset,
                            constant: self.local_constant_value(value),
                        },
                    );
                    self.emit(abi::store_u64(
                        &result.location,
                        abi::stack_pointer(),
                        stack_offset,
                    ));
                    self.lower_ops(body)?;
                    let symbol = self
                        .function_symbols
                        .get(close)
                        .cloned()
                        .unwrap_or_else(|| close.clone());
                    self.emit_call(close, &symbol, &[], None)?;
                }
            }
            self.reset_temporary_registers();
        }
        Ok(())
    }

    fn lower_for_each(
        &mut self,
        name: &str,
        type_: &str,
        iterable: &NirValue,
        body: &[NirOp],
    ) -> Result<(), String> {
        let iterable_value = self.lower_value(iterable)?;
        if !is_collection_type(&iterable_value.type_) {
            return Err(format!(
                "native code FOR EACH target '{}' is not a collection",
                iterable_value.type_
            ));
        }
        let map_entry_types = if iterable_value.type_.starts_with("Map OF ") {
            Some(map_type_parts(&iterable_value.type_).ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a valid map type",
                    iterable_value.type_
                )
            })?)
        } else {
            None
        };
        let list_element_type = iterable_value
            .type_
            .strip_prefix("List OF ")
            .map(str::to_string);
        let item_value_type = list_element_type.as_deref();
        let collection_slot = self.allocate_stack_object("for_each_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_remaining", 8);
        let local_slot = self.allocate_stack_object(name, 8);
        let entry_payload_slot = if map_entry_types.is_some() {
            Some(self.allocate_stack_object("for_each_map_entry", 16))
        } else {
            None
        };

        self.emit(abi::store_u64(
            &iterable_value.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));

        let loop_label = self.label("for_each_loop");
        let end_label = self.label("for_each_end");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&end_label));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        if let (Some(entry_payload_slot), Some((key_type, value_type))) =
            (entry_payload_slot, map_entry_types.as_ref())
        {
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let key_value = self.emit_load_collection_payload(key_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &key_value,
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let item_value = self.emit_load_collection_payload(value_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &item_value,
                abi::stack_pointer(),
                entry_payload_slot + 8,
            ));
            self.emit(abi::add_immediate(
                "x11",
                abi::stack_pointer(),
                entry_payload_slot,
            ));
            self.emit(abi::store_u64("x11", abi::stack_pointer(), local_slot));
        } else {
            let item_value_type = item_value_type.ok_or_else(|| {
                format!(
                    "native code FOR EACH target '{}' is not a list",
                    iterable_value.type_
                )
            })?;
            self.emit(abi::load_u64(
                "x11",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
            ));
            self.emit(abi::load_u64(
                "x12",
                "x10",
                COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
            ));
            self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
            let item_value =
                self.emit_load_collection_payload(item_value_type, "x8", "x11", "x12")?;
            self.emit(abi::store_u64(
                &item_value,
                abi::stack_pointer(),
                local_slot,
            ));
        }
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));

        let previous = self.locals.insert(
            name.to_string(),
            LocalValue {
                type_: type_.to_string(),
                stack_offset: local_slot,
                constant: None,
            },
        );
        self.lower_ops(body)?;
        if let Some(previous) = previous {
            self.locals.insert(name.to_string(), previous);
        } else {
            self.locals.remove(name);
        }
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&end_label));
        self.clear_local_constants();
        Ok(())
    }

    fn lower_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        if let Some(string_value) = self.static_string_value(value) {
            let register = self.load_string_constant(&string_value)?;
            return Ok(ValueResult {
                type_: "String".to_string(),
                location: register,
                text: format!("String({string_value})"),
            });
        }
        match value {
            NirValue::Const { type_, value } => {
                let register = self.allocate_register()?;
                if type_ == "String" {
                    self.emit_load_string_constant(&register, value)?;
                } else {
                    let immediate = native_immediate_value(type_, value)?;
                    self.emit(abi::move_immediate(&register, type_, &immediate));
                }
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: format!("{type_}({value})"),
                })
            }
            NirValue::Local(name) => {
                if self.type_model.union_variants.contains_key(name) {
                    return Ok(ValueResult {
                        type_: "VariantTag".to_string(),
                        location: name.clone(),
                        text: name.clone(),
                    });
                }
                let local = self
                    .locals
                    .get(name)
                    .ok_or_else(|| format!("native code local '{name}' does not resolve"))?;
                let type_ = local.type_.clone();
                let stack_offset = local.stack_offset;
                let register = self.allocate_register()?;
                self.emit(abi::load_u64(&register, abi::stack_pointer(), stack_offset));
                Ok(ValueResult {
                    type_,
                    location: register,
                    text: name.clone(),
                })
            }
            NirValue::FunctionRef { name, type_ } => {
                let symbol = self
                    .function_symbols
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone());
                let register = self.allocate_register()?;
                self.emit(abi::load_page_address(&register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol.clone(),
                    kind: "page21".to_string(),
                    binding: "data".to_string(),
                    library: None,
                });
                self.emit(abi::add_page_offset(&register, &register, &symbol));
                self.relocations.push(CodeRelocation {
                    from: self.current_symbol.clone(),
                    to: symbol,
                    kind: "pageoff12".to_string(),
                    binding: "data".to_string(),
                    library: None,
                });
                Ok(ValueResult {
                    type_: type_.clone(),
                    location: register,
                    text: name.clone(),
                })
            }
            NirValue::Call { target, args } => {
                if target == "contains" && args.len() == 2 {
                    return self.lower_collection_contains(args);
                }
                if target == "get" && args.len() == 2 {
                    return self.lower_collection_get(args);
                }
                if target == "getOr" && args.len() == 3 {
                    return self.lower_collection_get_or(args);
                }
                if target == "find" && (args.len() == 2 || args.len() == 3) {
                    return self.lower_find(args);
                }
                if target == "len" && args.len() == 1 {
                    return self.lower_len(&args[0]);
                }
                if target == "mid" && args.len() == 3 {
                    return self.lower_mid(args);
                }
                if target == "replace" && args.len() == 3 {
                    return self.lower_replace(args);
                }
                if target == "append" && args.len() == 2 {
                    return self.lower_collection_append(args);
                }
                if target == "prepend" && args.len() == 2 {
                    return self.lower_collection_prepend(args);
                }
                if target == "insert" && args.len() == 3 {
                    return self.lower_collection_insert(args);
                }
                if target == "removeAt" && args.len() == 2 {
                    return self.lower_collection_remove_at(args);
                }
                if target == "set" && args.len() == 3 {
                    return self.lower_collection_set(args);
                }
                if target == "removeKey" && args.len() == 2 {
                    return self.lower_collection_remove_key(args);
                }
                if target == "hasKey" && args.len() == 2 {
                    return self.lower_collection_has_key(args);
                }
                if target == "keys" && args.len() == 1 {
                    return self.lower_collection_keys(args);
                }
                if target == "values" && args.len() == 1 {
                    return self.lower_collection_values_builtin(args);
                }
                if target == "sum" && args.len() == 1 {
                    return self.lower_collection_sum(args);
                }
                if target == "forEach" && args.len() == 2 {
                    return self.lower_collection_for_each_call(args);
                }
                if target == "transform" && args.len() == 2 {
                    return self.lower_collection_transform_call(args);
                }
                if target == "filter" && args.len() == 2 {
                    return self.lower_collection_filter_call(args);
                }
                if target == "reduce" && args.len() == 3 {
                    return self.lower_collection_reduce_call(args);
                }
                if target == "toString" && (args.len() == 1 || args.len() == 2) {
                    return self.lower_to_string(args);
                }
                if target == "typeName" && args.len() == 1 {
                    let type_name = self.static_type_name(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name)?;
                    return Ok(ValueResult {
                        type_: "String".to_string(),
                        location: register,
                        text: format!("typeName({type_name})"),
                    });
                }
                if target == "toInt" && args.len() == 1 {
                    return self.lower_to_int(&args[0]);
                }
                if target == "toFloat" && args.len() == 1 {
                    return self.lower_to_float(&args[0]);
                }
                if target == "toFixed" && args.len() == 1 {
                    return self.lower_to_fixed(&args[0]);
                }
                if target == "toByte" && args.len() == 1 {
                    return self.lower_to_byte(&args[0]);
                }
                if target == "isNumeric" && args.len() == 1 {
                    return self.lower_is_numeric(&args[0]);
                }
                let symbol = self
                    .function_symbols
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|| target.to_string());
                self.emit_call(target, &symbol, args, None)
            }
            NirValue::RuntimeCall {
                helper,
                target,
                args,
            } => {
                if target == "typeName" && args.len() == 1 {
                    let type_name = self.static_type_name(&args[0]).ok_or_else(|| {
                        "native code cannot determine typeName argument type".to_string()
                    })?;
                    let register = self.load_string_constant(&type_name)?;
                    return Ok(ValueResult {
                        type_: "String".to_string(),
                        location: register,
                        text: format!("typeName({type_name})"),
                    });
                }
                self.emit_call(
                    target,
                    &runtime::symbol_for_call(*helper, target),
                    args,
                    Some("Nothing"),
                )
            }
            NirValue::Constructor { type_, args } => {
                let arg_values = args
                    .iter()
                    .map(|arg| self.lower_value(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                let register = self.allocate_register()?;
                if self.type_model.record_fields.contains_key(type_) {
                    let object_offset = self.allocate_stack_object(type_, 8 * arg_values.len());
                    for (index, arg) in arg_values.iter().enumerate() {
                        self.emit(abi::store_u64(
                            &arg.location,
                            abi::stack_pointer(),
                            object_offset + 8 * index,
                        ));
                    }
                    self.emit(abi::add_immediate(
                        &register,
                        abi::stack_pointer(),
                        object_offset,
                    ));
                    return Ok(ValueResult {
                        type_: type_.clone(),
                        location: register,
                        text: format!("construct {type_}({})", join_texts(&arg_values)),
                    });
                }
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(type_)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{type_}' does not resolve")
                    })?;
                let object_offset = self.allocate_stack_object(type_, 8 * (arg_values.len() + 1));
                let tag_register = self.allocate_register()?;
                self.emit(abi::move_immediate(
                    &tag_register,
                    "UnionTag",
                    &tag.to_string(),
                ));
                self.emit(abi::store_u64(
                    &tag_register,
                    abi::stack_pointer(),
                    object_offset,
                ));
                for (index, arg) in arg_values.iter().enumerate() {
                    self.emit(abi::store_u64(
                        &arg.location,
                        abi::stack_pointer(),
                        object_offset + 8 * (index + 1),
                    ));
                }
                self.emit(abi::add_immediate(
                    &register,
                    abi::stack_pointer(),
                    object_offset,
                ));
                Ok(ValueResult {
                    type_: self
                        .type_model
                        .union_variants
                        .get(type_)
                        .cloned()
                        .unwrap_or_else(|| type_.clone()),
                    location: register,
                    text: format!("construct {type_}({})", join_texts(&arg_values)),
                })
            }
            NirValue::WithUpdate {
                type_,
                target,
                updates,
            } => self.lower_with_update(type_, target, updates),
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                NirValue::Local(type_name) => {
                    if let Some(ordinal) = self
                        .type_model
                        .enum_members
                        .get(&(type_name.clone(), member.clone()))
                        .copied()
                    {
                        let register = self.allocate_register()?;
                        self.emit(abi::move_immediate(
                            &register,
                            "EnumOrdinal",
                            &ordinal.to_string(),
                        ));
                        return Ok(ValueResult {
                            type_: type_name.clone(),
                            location: register,
                            text: format!("{type_name}.{member}"),
                        });
                    }
                    self.lower_field_access(target, member)
                }
                _ => self.lower_field_access(target, member),
            },
            NirValue::Binary { op, left, right } => {
                if op == "&" {
                    return self.lower_string_concat(left, right);
                }
                if matches!(op.as_str(), "=" | "<>" | "<" | ">" | "<=" | ">=") {
                    return self.lower_comparison_binary(op, left, right);
                }
                self.lower_arithmetic_binary(op, left, right)
            }
            NirValue::Unary { op, operand } => {
                let operand = self.lower_value(operand)?;
                if op == "-" && operand.type_ == "Integer" {
                    let min_register = self.allocate_register()?;
                    let overflow_label = self.label("integer_unary_overflow");
                    let ok_label = self.label("integer_unary_ok");
                    self.emit(abi::move_immediate(
                        &min_register,
                        "Integer",
                        "9223372036854775808",
                    ));
                    self.emit(abi::compare_registers(&operand.location, &min_register));
                    self.emit(abi::branch_eq(&overflow_label));
                    let zero = self.allocate_register()?;
                    self.emit(abi::move_immediate(&zero, "Integer", "0"));
                    let register = self.allocate_register()?;
                    self.emit(abi::subtract_registers(&register, &zero, &operand.location));
                    self.emit(abi::branch(&ok_label));
                    self.emit(abi::label(&overflow_label));
                    self.emit_overflow_return()?;
                    self.emit(abi::label(&ok_label));
                    return Ok(ValueResult {
                        type_: "Integer".to_string(),
                        location: register,
                        text: format!("(-{})", operand.text),
                    });
                }
                Err(format!(
                    "native code plan does not lower unary operator '{op}' for {} yet",
                    operand.type_
                ))
            }
            NirValue::ListLiteral { type_, values } => self.lower_list_literal(type_, values),
            NirValue::MapLiteral { type_, entries } => self.lower_map_literal(type_, entries),
        }
    }

    fn lower_replace(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&value.type_) {
            let value_slot = self.allocate_stack_object("replace_list_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let old = self.lower_value(&args[1])?;
            if old.type_ != element_type {
                return Err(format!(
                    "native list replace old must be {}, got {}",
                    element_type, old.type_
                ));
            }
            let old_slot = self.allocate_stack_object("replace_list_old", 8);
            self.emit(abi::store_u64(
                &old.location,
                abi::stack_pointer(),
                old_slot,
            ));
            let new = self.lower_value(&args[2])?;
            if new.type_ != element_type {
                return Err(format!(
                    "native list replace new must be {}, got {}",
                    element_type, new.type_
                ));
            }
            let new_slot = self.allocate_stack_object("replace_list_new", 8);
            self.emit(abi::store_u64(
                &new.location,
                abi::stack_pointer(),
                new_slot,
            ));
            return self.lower_list_replace(
                value_slot,
                old_slot,
                new_slot,
                &value.type_,
                &element_type,
            );
        }
        if value.type_ != "String" {
            return Err(format!(
                "native string replace value must be String, got {}",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("replace_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let old = self.lower_value(&args[1])?;
        if old.type_ != "String" {
            return Err(format!(
                "native string replace old must be String, got {}",
                old.type_
            ));
        }
        let old_slot = self.allocate_stack_object("replace_old", 8);
        self.emit(abi::store_u64(
            &old.location,
            abi::stack_pointer(),
            old_slot,
        ));

        let new = self.lower_value(&args[2])?;
        if new.type_ != "String" {
            return Err(format!(
                "native string replace new must be String, got {}",
                new.type_
            ));
        }
        let new_slot = self.allocate_stack_object("replace_new", 8);
        self.emit(abi::store_u64(
            &new.location,
            abi::stack_pointer(),
            new_slot,
        ));

        let result_slot = self.allocate_stack_object("replace_result", 8);
        let output_len_slot = self.allocate_stack_object("replace_output_len", 8);

        let value_ptr = "x8";
        let value_len = "x9";
        let old_ptr = "x10";
        let old_len = "x11";
        let new_ptr = "x12";
        let new_len = "x13";
        let index = "x14";
        let output_len = "x15";
        let last_start = "x16";
        let match_index = "x17";
        let candidate = "x20";
        let old_cursor = "x21";
        let value_byte = "x22";
        let old_byte = "x23";
        let dest = "x24";
        let new_cursor = "x25";
        let new_index = "x26";
        for register in [
            candidate, old_cursor, value_byte, old_byte, dest, new_cursor, new_index,
        ] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let copy_original = self.label("replace_copy_original");
        let first_loop = self.label("replace_first_loop");
        let first_compare = self.label("replace_first_compare");
        let first_match = self.label("replace_first_match");
        let first_next = self.label("replace_first_next");
        let first_done = self.label("replace_first_done");
        let alloc_ok = self.label("replace_alloc_ok");
        let second_loop = self.label("replace_second_loop");
        let second_compare = self.label("replace_second_compare");
        let second_match = self.label("replace_second_match");
        let second_copy_new_loop = self.label("replace_second_copy_new_loop");
        let second_copy_new_done = self.label("replace_second_copy_new_done");
        let second_copy_one = self.label("replace_second_copy_one");
        let second_done = self.label("replace_second_done");
        let done = self.label("replace_done");
        let result = self.allocate_register()?;

        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(old_ptr, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(new_ptr, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(value_len, value_ptr, 0));
        self.emit(abi::load_u64(old_len, old_ptr, 0));
        self.emit(abi::load_u64(new_len, new_ptr, 0));
        self.emit(abi::compare_immediate(old_len, "0"));
        self.emit(abi::branch_eq(&copy_original));
        self.emit(abi::compare_registers(old_len, value_len));
        self.emit(abi::branch_hi(&copy_original));
        self.emit(abi::add_immediate(value_ptr, value_ptr, 8));
        self.emit(abi::add_immediate(old_ptr, old_ptr, 8));
        self.emit(abi::add_immediate(new_ptr, new_ptr, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_register(output_len, value_len));
        self.emit(abi::subtract_registers(last_start, value_len, old_len));

        self.emit(abi::label(&first_loop));
        self.emit(abi::compare_registers(index, last_start));
        self.emit(abi::branch_hi(&first_done));
        self.emit(abi::move_immediate(match_index, "Integer", "0"));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::move_register(old_cursor, old_ptr));
        self.emit(abi::label(&first_compare));
        self.emit(abi::compare_registers(match_index, old_len));
        self.emit(abi::branch_eq(&first_match));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::load_u8(old_byte, old_cursor, 0));
        self.emit(abi::compare_registers(value_byte, old_byte));
        self.emit(abi::branch_ne(&first_next));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(old_cursor, old_cursor, 1));
        self.emit(abi::add_immediate(match_index, match_index, 1));
        self.emit(abi::branch(&first_compare));

        self.emit(abi::label(&first_match));
        self.emit(abi::subtract_registers(output_len, output_len, old_len));
        self.emit(abi::add_registers(output_len, output_len, new_len));
        self.emit(abi::add_registers(index, index, old_len));
        self.emit(abi::branch(&first_loop));

        self.emit(abi::label(&first_next));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&first_loop));

        self.emit(abi::label(&first_done));
        self.emit(abi::store_u64(
            output_len,
            abi::stack_pointer(),
            output_len_slot,
        ));
        self.emit(abi::add_immediate(abi::return_register(), output_len, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;

        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(
            output_len,
            abi::stack_pointer(),
            output_len_slot,
        ));
        self.emit(abi::store_u64(output_len, "x1", 0));
        self.emit(abi::add_immediate(dest, "x1", 8));
        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(old_ptr, abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64(new_ptr, abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64(value_len, value_ptr, 0));
        self.emit(abi::load_u64(old_len, old_ptr, 0));
        self.emit(abi::load_u64(new_len, new_ptr, 0));
        self.emit(abi::add_immediate(value_ptr, value_ptr, 8));
        self.emit(abi::add_immediate(old_ptr, old_ptr, 8));
        self.emit(abi::add_immediate(new_ptr, new_ptr, 8));
        self.emit(abi::subtract_registers(last_start, value_len, old_len));
        self.emit(abi::move_immediate(index, "Integer", "0"));

        self.emit(abi::label(&second_loop));
        self.emit(abi::compare_registers(index, value_len));
        self.emit(abi::branch_ge(&second_done));
        self.emit(abi::compare_registers(index, last_start));
        self.emit(abi::branch_hi(&second_copy_one));
        self.emit(abi::move_immediate(match_index, "Integer", "0"));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::move_register(old_cursor, old_ptr));
        self.emit(abi::label(&second_compare));
        self.emit(abi::compare_registers(match_index, old_len));
        self.emit(abi::branch_eq(&second_match));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::load_u8(old_byte, old_cursor, 0));
        self.emit(abi::compare_registers(value_byte, old_byte));
        self.emit(abi::branch_ne(&second_copy_one));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(old_cursor, old_cursor, 1));
        self.emit(abi::add_immediate(match_index, match_index, 1));
        self.emit(abi::branch(&second_compare));

        self.emit(abi::label(&second_match));
        self.emit(abi::move_immediate(new_index, "Integer", "0"));
        self.emit(abi::move_register(new_cursor, new_ptr));
        self.emit(abi::label(&second_copy_new_loop));
        self.emit(abi::compare_registers(new_index, new_len));
        self.emit(abi::branch_eq(&second_copy_new_done));
        self.emit(abi::load_u8(value_byte, new_cursor, 0));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::add_immediate(new_cursor, new_cursor, 1));
        self.emit(abi::add_immediate(dest, dest, 1));
        self.emit(abi::add_immediate(new_index, new_index, 1));
        self.emit(abi::branch(&second_copy_new_loop));
        self.emit(abi::label(&second_copy_new_done));
        self.emit(abi::add_registers(index, index, old_len));
        self.emit(abi::branch(&second_loop));

        self.emit(abi::label(&second_copy_one));
        self.emit(abi::add_registers(candidate, value_ptr, index));
        self.emit(abi::load_u8(value_byte, candidate, 0));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::add_immediate(dest, dest, 1));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&second_loop));

        self.emit(abi::label(&second_done));
        self.emit(abi::move_immediate(value_byte, "Integer", "0"));
        self.emit(abi::store_u8(value_byte, dest, 0));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&copy_original));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), value_slot));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "replace(String, String, String)".to_string(),
        })
    }

    fn lower_list_replace(
        &mut self,
        value_slot: usize,
        old_slot: usize,
        new_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }

        let new_payload = PayloadSlot {
            slot: new_slot,
            type_: element_type.to_string(),
        };
        let new_len_slot = self.emit_payload_length_to_stack(&new_payload, "replace_new_len")?;
        let data_len_slot = self.allocate_stack_object("replace_list_data_len", 8);
        let result_slot = self.allocate_stack_object("replace_list_result", 8);
        let loop_label = self.label("replace_list_length_loop");
        let add_new = self.label("replace_list_length_add_new");
        let add_old = self.label("replace_list_length_add_old");
        let length_next = self.label("replace_list_length_next");
        let length_done = self.label("replace_list_length_done");
        let alloc_ok = self.label("replace_list_alloc_ok");
        let copy_loop = self.label("replace_list_copy_loop");
        let copy_new = self.label("replace_list_copy_new");
        let copy_old = self.label("replace_list_copy_old");
        let copy_new_string_loop = self.label("replace_list_copy_new_string_loop");
        let copy_new_string_done = self.label("replace_list_copy_new_string_done");
        let copy_old_loop = self.label("replace_list_copy_old_loop");
        let copy_done_one = self.label("replace_list_copy_done_one");
        let copy_done = self.label("replace_list_copy_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::move_immediate("x15", "Integer", "0"));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x12", "x11"));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(
            "x17",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x20",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            element_type,
            "x8",
            "x17",
            "x20",
            "x9",
            &add_new,
            &add_old,
        )?;
        self.emit(abi::label(&add_new));
        self.emit(abi::load_u64("x21", abi::stack_pointer(), new_len_slot));
        self.emit(abi::add_registers("x15", "x15", "x21"));
        self.emit(abi::branch(&length_next));
        self.emit(abi::label(&add_old));
        self.emit(abi::add_registers("x15", "x15", "x20"));
        self.emit(abi::label(&length_next));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x16", "x11", "x14"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x16",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));

        self.emit(abi::move_immediate("x13", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate("x13", "Byte", "1"));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x15", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x15", "x1", COLLECTION_OFFSET_DATA_CAPACITY));

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), old_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), new_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x11", "x14"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x12", "x11"));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x22", "x17", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            "x22",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x13",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit_collection_payload_matches_value_branch(
            element_type,
            "x8",
            "x22",
            "x23",
            "x9",
            &copy_new,
            &copy_old,
        )?;

        self.emit(abi::label(&copy_new));
        self.emit(abi::load_u64("x23", abi::stack_pointer(), new_len_slot));
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x25", "x21", "x13"));
        match element_type {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::store_u8("x24", "x25", 0));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::store_u64("x24", "x25", 0));
            }
            "String" => {
                self.emit(abi::load_u64("x24", abi::stack_pointer(), new_slot));
                self.emit(abi::add_immediate("x24", "x24", 8));
                self.emit(abi::label(&copy_new_string_loop));
                self.emit(abi::compare_immediate("x23", "0"));
                self.emit(abi::branch_eq(&copy_new_string_done));
                self.emit(abi::load_u8("x22", "x24", 0));
                self.emit(abi::store_u8("x22", "x25", 0));
                self.emit(abi::add_immediate("x24", "x24", 1));
                self.emit(abi::add_immediate("x25", "x25", 1));
                self.emit(abi::subtract_immediate("x23", "x23", 1));
                self.emit(abi::branch(&copy_new_string_loop));
                self.emit(abi::label(&copy_new_string_done));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        self.emit(abi::branch(&copy_done_one));

        self.emit(abi::label(&copy_old));
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", "x20", "x22"));
        self.emit(abi::add_registers("x25", "x21", "x13"));
        self.emit(abi::label(&copy_old_loop));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&copy_done_one));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&copy_old_loop));

        self.emit(abi::label(&copy_done_one));
        self.emit(abi::load_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x13", "x13", "x23"));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("replace({list_type}, {element_type}, {element_type})"),
        })
    }

    fn lower_to_string(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        let value_slot = self.allocate_stack_object("to_string_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let precision_slot = self.allocate_stack_object("to_string_precision", 8);
        if let Some(precision) = args.get(1) {
            let precision = self.lower_value(precision)?;
            if precision.type_ != "Byte" {
                return Err(format!(
                    "native toString precision must be Byte, got {}",
                    precision.type_
                ));
            }
            self.emit(abi::store_u64(
                &precision.location,
                abi::stack_pointer(),
                precision_slot,
            ));
        } else {
            self.emit(abi::move_immediate("x8", "Byte", "2"));
            self.emit(abi::store_u64("x8", abi::stack_pointer(), precision_slot));
        }

        self.reset_temporary_registers();
        let value_register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &value_register,
            abi::stack_pointer(),
            value_slot,
        ));

        match value.type_.as_str() {
            "String" => Ok(ValueResult {
                type_: "String".to_string(),
                location: value_register,
                text: format!("toString({})", value.text),
            }),
            "Boolean" => self.lower_boolean_to_string(&value_register),
            "Byte" => self.emit_integer_to_string_value(&value_register, false),
            "Integer" => self.emit_integer_to_string_value(&value_register, true),
            "List OF Byte" => self.emit_byte_list_to_string_value(&value_register),
            "Fixed" => {
                let precision = self.allocate_register()?;
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit_fixed_to_string_value(&value_register, &precision)
            }
            "Float" => {
                let precision = self.allocate_register()?;
                let fixed_raw = self.allocate_register()?;
                self.emit(abi::load_u64(
                    &precision,
                    abi::stack_pointer(),
                    precision_slot,
                ));
                self.emit(abi::float_move_d_from_x("d0", &value_register));
                self.emit_f64_const("d1", "x17", 4_294_967_296.0);
                self.emit(abi::float_multiply_d("d0", "d0", "d1"));
                self.emit(abi::float_convert_to_signed_x(&fixed_raw, "d0"));
                self.emit_fixed_to_string_value(&fixed_raw, &precision)
            }
            other => Err(format!(
                "native toString does not accept argument type '{other}'"
            )),
        }
    }

    fn lower_boolean_to_string(&mut self, value_register: &str) -> Result<ValueResult, String> {
        let false_label = self.label("bool_string_false");
        let done = self.label("bool_string_done");
        let result = self.allocate_register()?;
        self.emit(abi::compare_immediate(value_register, "0"));
        self.emit(abi::branch_eq(&false_label));
        self.emit_load_string_constant(&result, "TRUE")?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&false_label));
        self.emit_load_string_constant(&result, "FALSE")?;
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Boolean)".to_string(),
        })
    }

    fn emit_integer_to_string_value(
        &mut self,
        source_register: &str,
        signed: bool,
    ) -> Result<ValueResult, String> {
        let buffer_slot = self.allocate_stack_object("to_string_integer_buffer", 40);
        let length_slot = self.allocate_stack_object("to_string_integer_length", 8);
        let start_slot = self.allocate_stack_object("to_string_integer_start", 8);
        let result_slot = self.allocate_stack_object("to_string_integer_result", 8);

        let value = "x8";
        let negative = "x9";
        let length = "x10";
        let cursor = "x11";
        let divisor = "x12";
        let quotient = "x13";
        let digit = "x14";
        let dst = "x15";
        let done = self.label("int_string_done");
        let nonnegative = self.label("int_string_nonnegative");
        let zero = self.label("int_string_zero");
        let loop_start = self.label("int_string_loop");
        let digits_done = self.label("int_string_digits_done");
        let sign_done = self.label("int_string_sign_done");
        let alloc_ok = self.label("int_string_alloc_ok");
        let copy_loop = self.label("int_string_copy_loop");
        let copy_done = self.label("int_string_copy_done");

        self.emit(abi::move_register(value, source_register));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::move_immediate(length, "Integer", "0"));
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_eq(&zero));
        if signed {
            self.emit(abi::branch_ge(&nonnegative));
            self.emit(abi::subtract_registers(value, "xzr", value));
            self.emit(abi::move_immediate(negative, "Integer", "1"));
            self.emit(abi::label(&nonnegative));
        }
        self.emit(abi::add_immediate(
            cursor,
            abi::stack_pointer(),
            buffer_slot + 39,
        ));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_eq(&digits_done));
        self.emit(abi::unsigned_divide_registers(quotient, value, divisor));
        self.emit(abi::multiply_subtract_registers(
            digit, quotient, divisor, value,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::move_register(value, quotient));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&zero));
        self.emit(abi::add_immediate(
            cursor,
            abi::stack_pointer(),
            buffer_slot + 39,
        ));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'0' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::move_immediate(length, "Integer", "1"));

        self.emit(abi::label(&digits_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&sign_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'-' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::label(&sign_done));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(cursor, abi::stack_pointer(), start_slot));

        self.emit(abi::add_immediate(abi::return_register(), length, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, "x1", 0));
        self.emit(abi::add_immediate(dst, "x1", 8));
        self.emit(abi::load_u64(cursor, abi::stack_pointer(), start_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8(digit, cursor, 0));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(length, length, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(digit, "Integer", "0"));
        self.emit(abi::store_u8(digit, dst, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Integer)".to_string(),
        })
    }

    fn emit_byte_list_to_string_value(
        &mut self,
        source_register: &str,
    ) -> Result<ValueResult, String> {
        let list_slot = self.allocate_stack_object("to_string_byte_list", 8);
        let length_slot = self.allocate_stack_object("to_string_byte_list_length", 8);
        let data_slot = self.allocate_stack_object("to_string_byte_list_data", 8);
        let result_slot = self.allocate_stack_object("to_string_byte_list_result", 8);

        let list = "x8";
        let length = "x9";
        let index = "x10";
        let offset = "x11";
        let byte = "x12";
        let byte2 = "x13";
        let byte3 = "x14";
        let byte4 = "x15";
        let result = "x16";
        let dst = "x17";

        let validate_loop = self.label("byte_list_string_validate_loop");
        let validate_done = self.label("byte_list_string_validate_done");
        let invalid = self.label("byte_list_string_invalid");
        let ascii = self.label("byte_list_string_ascii");
        let two = self.label("byte_list_string_two");
        let three = self.label("byte_list_string_three");
        let three_e0 = self.label("byte_list_string_three_e0");
        let three_ed = self.label("byte_list_string_three_ed");
        let three_mid = self.label("byte_list_string_three_mid");
        let four = self.label("byte_list_string_four");
        let four_f0 = self.label("byte_list_string_four_f0");
        let four_f4 = self.label("byte_list_string_four_f4");
        let four_mid = self.label("byte_list_string_four_mid");
        let alloc_ok = self.label("byte_list_string_alloc_ok");
        let copy_loop = self.label("byte_list_string_copy_loop");
        let copy_done = self.label("byte_list_string_copy_done");

        self.emit(abi::move_register(list, source_register));
        self.emit(abi::store_u64(list, abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64(length, list, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit_collection_data_pointer(offset, list);
        self.emit(abi::store_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::move_immediate(index, "Integer", "0"));

        self.emit(abi::label(&validate_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&validate_done));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte, offset, 0));

        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_lo(&ascii));

        self.emit(abi::compare_immediate(byte, "194"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte, "224"));
        self.emit(abi::branch_lo(&two));
        self.emit(abi::compare_immediate(byte, "240"));
        self.emit(abi::branch_lo(&three));
        self.emit(abi::compare_immediate(byte, "245"));
        self.emit(abi::branch_lo(&four));
        self.emit(abi::branch(&invalid));

        self.emit(abi::label(&ascii));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&two));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three));
        self.emit(abi::add_immediate(index, index, 2));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::subtract_immediate(index, index, 2));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 1));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 2));
        self.emit(abi::load_u8(byte3, offset, 0));
        self.emit(abi::compare_immediate(byte3, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte3, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::compare_immediate(byte, "224"));
        self.emit(abi::branch_eq(&three_e0));
        self.emit(abi::compare_immediate(byte, "237"));
        self.emit(abi::branch_eq(&three_ed));
        self.emit(abi::branch(&three_mid));

        self.emit(abi::label(&three_e0));
        self.emit(abi::compare_immediate(byte2, "160"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three_ed));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "160"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&three_mid));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four));
        self.emit(abi::add_immediate(index, index, 3));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::subtract_immediate(index, index, 3));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 1));
        self.emit(abi::load_u8(byte2, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 2));
        self.emit(abi::load_u8(byte3, offset, 0));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::add_immediate(offset, offset, 3));
        self.emit(abi::load_u8(byte4, offset, 0));
        for continuation in [byte3, byte4] {
            self.emit(abi::compare_immediate(continuation, "128"));
            self.emit(abi::branch_lo(&invalid));
            self.emit(abi::compare_immediate(continuation, "192"));
            self.emit(abi::branch_ge(&invalid));
        }
        self.emit(abi::compare_immediate(byte, "240"));
        self.emit(abi::branch_eq(&four_f0));
        self.emit(abi::compare_immediate(byte, "244"));
        self.emit(abi::branch_eq(&four_f4));
        self.emit(abi::branch(&four_mid));

        self.emit(abi::label(&four_f0));
        self.emit(abi::compare_immediate(byte2, "144"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four_f4));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "144"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&four_mid));
        self.emit(abi::compare_immediate(byte2, "128"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte2, "192"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::add_immediate(index, index, 4));
        self.emit(abi::branch(&validate_loop));

        self.emit(abi::label(&invalid));
        self.emit_encoding_error_return()?;

        self.emit(abi::label(&validate_done));
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64(length, "x1", 0));
        self.emit(abi::add_immediate(dst, "x1", 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::load_u64(list, abi::stack_pointer(), list_slot));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(offset, abi::stack_pointer(), data_slot));
        self.emit(abi::add_registers(offset, offset, index));
        self.emit(abi::load_u8(byte, offset, 0));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::branch(&copy_loop));

        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, dst, 0));
        self.emit(abi::load_u64(result, abi::stack_pointer(), result_slot));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: result.to_string(),
            text: "toString(List OF Byte)".to_string(),
        })
    }

    fn emit_fixed_to_string_value(
        &mut self,
        source_register: &str,
        precision_register: &str,
    ) -> Result<ValueResult, String> {
        let buffer_slot = self.allocate_stack_object("to_string_fixed_buffer", 48);
        let integer_start_slot = self.allocate_stack_object("to_string_fixed_integer_start", 8);
        let integer_len_slot = self.allocate_stack_object("to_string_fixed_integer_len", 8);
        let total_len_slot = self.allocate_stack_object("to_string_fixed_total_len", 8);
        let magnitude_slot = self.allocate_stack_object("to_string_fixed_magnitude", 8);
        let precision_slot = self.allocate_stack_object("to_string_fixed_precision", 8);
        let result_slot = self.allocate_stack_object("to_string_fixed_result", 8);

        let raw = "x8";
        let negative = "x9";
        let int_part = "x10";
        let frac_part = "x11";
        let cursor = "x12";
        let length = "x13";
        let divisor = "x14";
        let quotient = "x15";
        let digit = "x16";
        let precision = "x17";
        let total_len = "x20";
        let dst = "x21";
        let counter = "x22";
        let scale = "x23";
        for register in [total_len, dst, counter, scale] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let nonnegative = self.label("fixed_string_nonnegative");
        let integer_zero = self.label("fixed_string_integer_zero");
        let integer_loop = self.label("fixed_string_integer_loop");
        let integer_done = self.label("fixed_string_integer_done");
        let sign_done = self.label("fixed_string_sign_done");
        let no_fraction = self.label("fixed_string_no_fraction");
        let alloc_ok = self.label("fixed_string_alloc_ok");
        let copy_integer_loop = self.label("fixed_string_copy_integer_loop");
        let copy_integer_done = self.label("fixed_string_copy_integer_done");
        let fraction_loop = self.label("fixed_string_fraction_loop");
        let fraction_done = self.label("fixed_string_fraction_done");

        self.emit(abi::move_register(raw, source_register));
        self.emit(abi::move_register(precision, precision_register));
        self.emit(abi::store_u64(
            precision,
            abi::stack_pointer(),
            precision_slot,
        ));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::compare_immediate(raw, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit(abi::subtract_registers(raw, "xzr", raw));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::label(&nonnegative));
        self.emit(abi::store_u64(raw, abi::stack_pointer(), magnitude_slot));
        self.emit(abi::shift_right_immediate(int_part, raw, 32));
        self.emit(abi::shift_left_immediate(frac_part, raw, 32));
        self.emit(abi::shift_right_immediate(frac_part, frac_part, 32));
        self.emit(abi::move_immediate(length, "Integer", "0"));
        self.emit(abi::add_immediate(
            cursor,
            abi::stack_pointer(),
            buffer_slot + 47,
        ));
        self.emit(abi::compare_immediate(int_part, "0"));
        self.emit(abi::branch_eq(&integer_zero));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::label(&integer_loop));
        self.emit(abi::compare_immediate(int_part, "0"));
        self.emit(abi::branch_eq(&integer_done));
        self.emit(abi::unsigned_divide_registers(quotient, int_part, divisor));
        self.emit(abi::multiply_subtract_registers(
            digit, quotient, divisor, int_part,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::move_register(int_part, quotient));
        self.emit(abi::branch(&integer_loop));

        self.emit(abi::label(&integer_zero));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'0' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::move_immediate(length, "Integer", "1"));

        self.emit(abi::label(&integer_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&sign_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'-' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, cursor, 0));
        self.emit(abi::subtract_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(length, length, 1));
        self.emit(abi::label(&sign_done));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::store_u64(
            cursor,
            abi::stack_pointer(),
            integer_start_slot,
        ));
        self.emit(abi::store_u64(
            length,
            abi::stack_pointer(),
            integer_len_slot,
        ));
        self.emit(abi::move_register(total_len, length));
        self.emit(abi::compare_immediate(precision, "0"));
        self.emit(abi::branch_eq(&no_fraction));
        self.emit(abi::add_immediate(total_len, total_len, 1));
        self.emit(abi::add_registers(total_len, total_len, precision));
        self.emit(abi::label(&no_fraction));
        self.emit(abi::store_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));

        self.emit(abi::add_immediate(abi::return_register(), total_len, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(
            total_len,
            abi::stack_pointer(),
            total_len_slot,
        ));
        self.emit(abi::store_u64(total_len, "x1", 0));
        self.emit(abi::add_immediate(dst, "x1", 8));
        self.emit(abi::load_u64(
            cursor,
            abi::stack_pointer(),
            integer_start_slot,
        ));
        self.emit(abi::load_u64(
            length,
            abi::stack_pointer(),
            integer_len_slot,
        ));
        self.emit(abi::label(&copy_integer_loop));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&copy_integer_done));
        self.emit(abi::load_u8(digit, cursor, 0));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::subtract_immediate(length, length, 1));
        self.emit(abi::branch(&copy_integer_loop));
        self.emit(abi::label(&copy_integer_done));

        self.emit(abi::load_u64(
            precision,
            abi::stack_pointer(),
            precision_slot,
        ));
        self.emit(abi::compare_immediate(precision, "0"));
        self.emit(abi::branch_eq(&fraction_done));
        self.emit(abi::move_immediate(
            digit,
            "Integer",
            &(b'.' as u64).to_string(),
        ));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::load_u64(raw, abi::stack_pointer(), magnitude_slot));
        self.emit(abi::shift_left_immediate(frac_part, raw, 32));
        self.emit(abi::shift_right_immediate(frac_part, frac_part, 32));
        self.emit(abi::move_immediate(counter, "Integer", "0"));
        self.emit(abi::move_immediate(divisor, "Integer", "10"));
        self.emit(abi::move_immediate(scale, "Integer", "4294967296"));
        self.emit(abi::label(&fraction_loop));
        self.emit(abi::compare_registers(counter, precision));
        self.emit(abi::branch_eq(&fraction_done));
        self.emit(abi::multiply_registers(frac_part, frac_part, divisor));
        self.emit(abi::unsigned_divide_registers(digit, frac_part, scale));
        self.emit(abi::multiply_subtract_registers(
            frac_part, digit, scale, frac_part,
        ));
        self.emit(abi::add_immediate(digit, digit, b'0' as usize));
        self.emit(abi::store_u8(digit, dst, 0));
        self.emit(abi::add_immediate(dst, dst, 1));
        self.emit(abi::add_immediate(counter, counter, 1));
        self.emit(abi::branch(&fraction_loop));
        self.emit(abi::label(&fraction_done));
        self.emit(abi::move_immediate(digit, "Integer", "0"));
        self.emit(abi::store_u8(digit, dst, 0));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "toString(Fixed)".to_string(),
        })
    }

    fn lower_to_int(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ == "Byte" {
            let register = self.allocate_register()?;
            self.emit(abi::move_register(&register, &value.location));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("toInt({})", value.text),
            });
        }
        let value_slot = self.allocate_stack_object("to_int_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        match value.type_.as_str() {
            "Fixed" => self.emit_fixed_to_int_value(&source),
            "Float" => self.emit_float_to_int_value(&source),
            "String" => self.emit_string_to_int_value(&source),
            other => Err(format!(
                "native toInt does not accept argument type '{other}'"
            )),
        }
    }

    fn emit_fixed_to_int_value(&mut self, source_register: &str) -> Result<ValueResult, String> {
        let value = "x8";
        let result = self.allocate_register()?;
        let nonnegative = self.label("fixed_to_int_nonnegative");
        let done = self.label("fixed_to_int_done");
        self.emit(abi::move_register(value, source_register));
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit(abi::subtract_registers(&result, "xzr", value));
        self.emit(abi::shift_right_immediate(&result, &result, 32));
        self.emit(abi::subtract_registers(&result, "xzr", &result));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&nonnegative));
        self.emit(abi::arithmetic_shift_right_immediate(&result, value, 32));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "toInt(Fixed)".to_string(),
        })
    }

    fn emit_float_to_int_value(&mut self, source_register: &str) -> Result<ValueResult, String> {
        let bits = "x8";
        let exponent = "x9";
        let mantissa = "x10";
        let sign = "x11";
        let mask = "x12";
        let ok = self.label("float_to_int_ok");
        let check_edge = self.label("float_to_int_check_edge");
        let edge_sign_ok = self.label("float_to_int_edge_sign_ok");
        let overflow = self.label("float_to_int_overflow");
        let invalid = self.label("float_to_int_invalid");
        let result = self.allocate_register()?;

        self.emit(abi::move_register(bits, source_register));
        self.emit(abi::shift_right_immediate(exponent, bits, 52));
        self.emit(abi::move_immediate(mask, "Integer", "2047"));
        self.emit(abi::and_registers(exponent, exponent, mask));
        self.emit(abi::compare_immediate(exponent, "2047"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::compare_immediate(exponent, "1086"));
        self.emit(abi::branch_lt(&ok));
        self.emit(abi::branch_eq(&check_edge));
        self.emit(abi::branch(&overflow));

        self.emit(abi::label(&check_edge));
        self.emit(abi::shift_right_immediate(sign, bits, 63));
        self.emit(abi::compare_immediate(sign, "1"));
        self.emit(abi::branch_eq(&edge_sign_ok));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge_sign_ok));
        self.emit(abi::move_immediate(mask, "Integer", "4503599627370495"));
        self.emit(abi::and_registers(mantissa, bits, mask));
        self.emit(abi::compare_immediate(mantissa, "0"));
        self.emit(abi::branch_ne(&overflow));

        self.emit(abi::label(&ok));
        self.emit(abi::float_move_d_from_x("d0", bits));
        self.emit(abi::float_convert_to_signed_x(&result, "d0"));
        let done = self.label("float_to_int_done");
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_invalid_format_return()?;
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "toInt(Float)".to_string(),
        })
    }

    fn emit_string_to_int_value(&mut self, source_register: &str) -> Result<ValueResult, String> {
        let string = "x8";
        let length = "x9";
        let index = "x10";
        let cursor = "x11";
        let byte = "x12";
        let acc = "x13";
        let negative = "x14";
        let digit = "x15";
        let cutoff = "x16";
        let cutlim = "x17";
        let ten = "x6";
        let invalid = self.label("string_to_int_invalid");
        let overflow = self.label("string_to_int_overflow");
        let first_not_minus = self.label("string_to_int_first_not_minus");
        let sign_done = self.label("string_to_int_sign_done");
        let loop_start = self.label("string_to_int_loop");
        let loop_done = self.label("string_to_int_done");
        let cutoff_equal = self.label("string_to_int_cutoff_equal");
        let digit_ok = self.label("string_to_int_digit_ok");
        let positive = self.label("string_to_int_positive");
        let done = self.label("string_to_int_return");
        let result = self.allocate_register()?;

        self.emit(abi::move_register(string, source_register));
        self.emit(abi::load_u64(length, string, 0));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::add_immediate(cursor, string, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_immediate(acc, "Integer", "0"));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "45"));
        self.emit(abi::branch_ne(&first_not_minus));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&sign_done));
        self.emit(abi::label(&first_not_minus));
        self.emit(abi::compare_immediate(byte, "43"));
        self.emit(abi::branch_ne(&sign_done));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::label(&sign_done));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::move_immediate(cutoff, "Integer", "922337203685477580"));
        self.emit(abi::move_immediate(cutlim, "Integer", "7"));
        self.emit(abi::compare_immediate(negative, "0"));
        let limit_ready = self.label("string_to_int_limit_ready");
        self.emit(abi::branch_eq(&limit_ready));
        self.emit(abi::move_immediate(cutlim, "Integer", "8"));
        self.emit(abi::label(&limit_ready));
        self.emit(abi::move_immediate(ten, "Integer", "10"));

        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&loop_done));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "48"));
        self.emit(abi::branch_lo(&invalid));
        self.emit(abi::compare_immediate(byte, "57"));
        self.emit(abi::branch_hi(&invalid));
        self.emit(abi::subtract_immediate(digit, byte, 48));
        self.emit(abi::compare_registers(acc, cutoff));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::branch_eq(&cutoff_equal));
        self.emit(abi::branch(&digit_ok));
        self.emit(abi::label(&cutoff_equal));
        self.emit(abi::compare_registers(digit, cutlim));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::label(&digit_ok));
        self.emit(abi::multiply_registers(acc, acc, ten));
        self.emit(abi::add_registers(acc, acc, digit));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&loop_done));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&positive));
        self.emit(abi::subtract_registers(&result, "xzr", acc));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&positive));
        self.emit(abi::move_register(&result, acc));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_invalid_format_return()?;
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "toInt(String)".to_string(),
        })
    }

    fn lower_to_byte(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "Integer" {
            return Err(format!(
                "native toByte does not accept argument type '{}'",
                value.type_
            ));
        }
        let result = self.allocate_register()?;
        let overflow = self.label("to_byte_overflow");
        let ok = self.label("to_byte_ok");
        self.emit(abi::compare_immediate(&value.location, "0"));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::compare_immediate(&value.location, "255"));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::move_register(&result, &value.location));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(ValueResult {
            type_: "Byte".to_string(),
            location: result,
            text: format!("toByte({})", value.text),
        })
    }

    fn lower_to_float(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let value_slot = self.allocate_stack_object("to_float_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register()?;
        match value.type_.as_str() {
            "Integer" => {
                self.emit(abi::signed_convert_to_float_d("d0", &source));
                self.emit(abi::float_move_x_from_d(&result, "d0"));
            }
            "Fixed" => {
                let temp = ValueResult {
                    type_: "Fixed".to_string(),
                    location: source,
                    text: value.text.clone(),
                };
                self.load_numeric_as_double("d0", &temp)?;
                self.emit(abi::float_move_x_from_d(&result, "d0"));
            }
            "String" => {
                let invalid = self.label("to_float_invalid");
                let overflow = self.label("to_float_overflow");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                self.emit_double_overflow_check("d0", &overflow);
                self.emit(abi::float_move_x_from_d(&result, "d0"));
                let done = self.label("to_float_done");
                self.emit(abi::branch(&done));
                self.emit(abi::label(&invalid));
                self.emit_invalid_format_return()?;
                self.emit(abi::label(&overflow));
                self.emit_overflow_return()?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toFloat does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Float".to_string(),
            location: result,
            text: format!("toFloat({})", value.text),
        })
    }

    fn lower_to_fixed(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        let value_slot = self.allocate_stack_object("to_fixed_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let result = self.allocate_register()?;
        match value.type_.as_str() {
            "Integer" => {
                self.emit_integer_to_fixed_value(&source, &result)?;
            }
            "Float" => {
                self.emit_float_bits_to_fixed_value(&source, &result)?;
            }
            "String" => {
                let invalid = self.label("to_fixed_invalid");
                let overflow = self.label("to_fixed_overflow");
                self.emit_parse_decimal_string_to_double(&source, &invalid)?;
                self.emit_double_overflow_check("d0", &overflow);
                let parsed_bits = "x8";
                self.emit(abi::float_move_x_from_d(parsed_bits, "d0"));
                self.emit_float_bits_to_fixed_value(parsed_bits, &result)?;
                let done = self.label("to_fixed_done");
                self.emit(abi::branch(&done));
                self.emit(abi::label(&invalid));
                self.emit_invalid_format_return()?;
                self.emit(abi::label(&overflow));
                self.emit_overflow_return()?;
                self.emit(abi::label(&done));
            }
            other => {
                return Err(format!(
                    "native toFixed does not accept argument type '{other}'"
                ))
            }
        }
        Ok(ValueResult {
            type_: "Fixed".to_string(),
            location: result,
            text: format!("toFixed({})", value.text),
        })
    }

    fn lower_is_numeric(&mut self, arg: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(arg)?;
        if value.type_ != "String" {
            return Err(format!(
                "native isNumeric does not accept argument type '{}'",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("is_numeric_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));
        self.reset_temporary_registers();
        let source = self.allocate_register()?;
        self.emit(abi::load_u64(&source, abi::stack_pointer(), value_slot));
        let invalid = self.label("is_numeric_false");
        let done = self.label("is_numeric_done");
        let result = self.allocate_register()?;
        self.emit_parse_decimal_string_to_double(&source, &invalid)?;
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("isNumeric({})", value.text),
        })
    }

    fn emit_integer_to_fixed_value(&mut self, source: &str, result: &str) -> Result<(), String> {
        let min = self.allocate_register()?;
        let max = self.allocate_register()?;
        let overflow = self.label("int_to_fixed_overflow");
        let ok = self.label("int_to_fixed_ok");
        self.emit(abi::move_immediate(&min, "Integer", "18446744071562067968"));
        self.emit(abi::compare_registers(source, &min));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::move_immediate(&max, "Integer", "2147483647"));
        self.emit(abi::compare_registers(source, &max));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::shift_left_immediate(result, source, 32));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    fn emit_float_bits_to_fixed_value(&mut self, source: &str, result: &str) -> Result<(), String> {
        let exponent = "x9";
        let mask = "x10";
        let sign = "x11";
        let mantissa = "x12";
        let invalid = self.label("float_to_fixed_invalid");
        let overflow = self.label("float_to_fixed_overflow");
        let ok = self.label("float_to_fixed_ok");
        let edge = self.label("float_to_fixed_edge");
        let edge_negative = self.label("float_to_fixed_edge_negative");
        let range_ok = self.label("float_to_fixed_range_ok");
        self.emit(abi::move_register("x8", source));
        self.emit(abi::shift_right_immediate(exponent, "x8", 52));
        self.emit(abi::move_immediate(mask, "Integer", "2047"));
        self.emit(abi::and_registers(exponent, exponent, mask));
        self.emit(abi::compare_immediate(exponent, "2047"));
        self.emit(abi::branch_eq(&invalid));
        self.emit(abi::compare_immediate(exponent, "1054"));
        self.emit(abi::branch_lt(&range_ok));
        self.emit(abi::branch_eq(&edge));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge));
        self.emit(abi::shift_right_immediate(sign, "x8", 63));
        self.emit(abi::compare_immediate(sign, "1"));
        self.emit(abi::branch_eq(&edge_negative));
        self.emit(abi::branch(&overflow));
        self.emit(abi::label(&edge_negative));
        self.emit(abi::move_immediate(mask, "Integer", "4503599627370495"));
        self.emit(abi::and_registers(mantissa, "x8", mask));
        self.emit(abi::compare_immediate(mantissa, "0"));
        self.emit(abi::branch_ne(&overflow));
        self.emit(abi::label(&range_ok));
        self.emit(abi::float_move_d_from_x("d0", "x8"));
        self.emit_f64_const("d1", "x17", 4_294_967_296.0);
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.emit(abi::float_convert_to_signed_x(result, "d0"));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&invalid));
        self.emit_invalid_format_return()?;
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    fn emit_parse_decimal_string_to_double(
        &mut self,
        source_register: &str,
        invalid_label: &str,
    ) -> Result<(), String> {
        let string = "x8";
        let length = "x9";
        let index = "x10";
        let cursor = "x11";
        let byte = "x12";
        let digit = "x13";
        let negative = "x14";
        let seen_digit = "x15";
        let ten_bits = "x16";
        let dot_seen = "x17";
        let exponent = "x4";
        let exponent_negative = "x3";
        let exponent_ten = "x2";
        let loop_start = self.label("parse_decimal_loop");
        let after_sign = self.label("parse_decimal_after_sign");
        let not_minus = self.label("parse_decimal_not_minus");
        let sign_done = self.label("parse_decimal_sign_done");
        let dot = self.label("parse_decimal_dot");
        let frac_digit = self.label("parse_decimal_frac_digit");
        let int_digit = self.label("parse_decimal_int_digit");
        let next = self.label("parse_decimal_next");
        let finish = self.label("parse_decimal_finish");
        let positive = self.label("parse_decimal_positive");
        let exponent_start = self.label("parse_decimal_exponent_start");
        let exponent_not_minus = self.label("parse_decimal_exponent_not_minus");
        let exponent_sign_done = self.label("parse_decimal_exponent_sign_done");
        let exponent_loop = self.label("parse_decimal_exponent_loop");
        let exponent_apply = self.label("parse_decimal_exponent_apply");
        let exponent_multiply_loop = self.label("parse_decimal_exponent_multiply_loop");
        let exponent_divide_loop = self.label("parse_decimal_exponent_divide_loop");
        let exponent_apply_done = self.label("parse_decimal_exponent_apply_done");
        self.emit(abi::move_register(string, source_register));
        self.emit(abi::load_u64(length, string, 0));
        self.emit(abi::compare_immediate(length, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::add_immediate(cursor, string, 8));
        self.emit(abi::move_immediate(index, "Integer", "0"));
        self.emit(abi::move_immediate(negative, "Integer", "0"));
        self.emit(abi::move_immediate(seen_digit, "Integer", "0"));
        self.emit(abi::move_immediate(dot_seen, "Integer", "0"));
        self.emit(abi::move_immediate(exponent_ten, "Integer", "10"));
        self.emit(abi::move_immediate("x0", "Integer", "0"));
        self.emit(abi::signed_convert_to_float_d("d0", "x0"));
        self.emit_f64_const("d1", ten_bits, 10.0);
        self.emit_f64_const("d3", "x7", 1.0);
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "45"));
        self.emit(abi::branch_ne(&not_minus));
        self.emit(abi::move_immediate(negative, "Integer", "1"));
        self.emit(abi::branch(&after_sign));
        self.emit(abi::label(&not_minus));
        self.emit(abi::compare_immediate(byte, "43"));
        self.emit(abi::branch_ne(&sign_done));
        self.emit(abi::label(&after_sign));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(invalid_label));
        self.emit(abi::label(&sign_done));

        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&finish));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "46"));
        self.emit(abi::branch_eq(&dot));
        self.emit(abi::compare_immediate(byte, "69"));
        self.emit(abi::branch_eq(&exponent_start));
        self.emit(abi::compare_immediate(byte, "101"));
        self.emit(abi::branch_eq(&exponent_start));
        self.emit(abi::compare_immediate(byte, "48"));
        self.emit(abi::branch_lo(invalid_label));
        self.emit(abi::compare_immediate(byte, "57"));
        self.emit(abi::branch_hi(invalid_label));
        self.emit(abi::subtract_immediate(digit, byte, 48));
        self.emit(abi::signed_convert_to_float_d("d2", digit));
        self.emit(abi::move_immediate(seen_digit, "Integer", "1"));
        self.emit(abi::compare_immediate(dot_seen, "0"));
        self.emit(abi::branch_ne(&frac_digit));
        self.emit(abi::label(&int_digit));
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.emit(abi::float_add_d("d0", "d0", "d2"));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&frac_digit));
        self.emit(abi::float_multiply_d("d3", "d3", "d1"));
        self.emit(abi::float_divide_d("d2", "d2", "d3"));
        self.emit(abi::float_add_d("d0", "d0", "d2"));
        self.emit(abi::branch(&next));
        self.emit(abi::label(&dot));
        self.emit(abi::compare_immediate(dot_seen, "0"));
        self.emit(abi::branch_ne(invalid_label));
        self.emit(abi::move_immediate(dot_seen, "Integer", "1"));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&exponent_start));
        self.emit(abi::compare_immediate(seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(invalid_label));
        self.emit(abi::move_immediate(exponent, "Integer", "0"));
        self.emit(abi::move_immediate(exponent_negative, "Integer", "0"));
        self.emit(abi::move_immediate(seen_digit, "Integer", "0"));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "45"));
        self.emit(abi::branch_ne(&exponent_not_minus));
        self.emit(abi::move_immediate(exponent_negative, "Integer", "1"));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&exponent_sign_done));
        self.emit(abi::label(&exponent_not_minus));
        self.emit(abi::compare_immediate(byte, "43"));
        self.emit(abi::branch_ne(&exponent_sign_done));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::label(&exponent_sign_done));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(invalid_label));

        self.emit(abi::label(&exponent_loop));
        self.emit(abi::compare_registers(index, length));
        self.emit(abi::branch_ge(&exponent_apply));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::compare_immediate(byte, "48"));
        self.emit(abi::branch_lo(invalid_label));
        self.emit(abi::compare_immediate(byte, "57"));
        self.emit(abi::branch_hi(invalid_label));
        self.emit(abi::subtract_immediate(digit, byte, 48));
        self.emit(abi::multiply_registers(exponent, exponent, exponent_ten));
        self.emit(abi::add_registers(exponent, exponent, digit));
        self.emit(abi::move_immediate(seen_digit, "Integer", "1"));
        self.emit(abi::add_immediate(index, index, 1));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::branch(&exponent_loop));

        self.emit(abi::label(&exponent_apply));
        self.emit(abi::compare_immediate(seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::compare_immediate(exponent_negative, "0"));
        self.emit(abi::branch_ne(&exponent_divide_loop));
        self.emit(abi::label(&exponent_multiply_loop));
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&exponent_apply_done));
        self.emit(abi::float_multiply_d("d0", "d0", "d1"));
        self.emit(abi::subtract_immediate(exponent, exponent, 1));
        self.emit(abi::branch(&exponent_multiply_loop));
        self.emit(abi::label(&exponent_divide_loop));
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_eq(&exponent_apply_done));
        self.emit(abi::float_divide_d("d0", "d0", "d1"));
        self.emit(abi::subtract_immediate(exponent, exponent, 1));
        self.emit(abi::branch(&exponent_divide_loop));
        self.emit(abi::label(&exponent_apply_done));
        self.emit(abi::move_immediate(seen_digit, "Integer", "1"));
        self.emit(abi::branch(&finish));

        self.emit(abi::label(&finish));
        self.emit(abi::compare_immediate(seen_digit, "0"));
        self.emit(abi::branch_eq(invalid_label));
        self.emit(abi::compare_immediate(negative, "0"));
        self.emit(abi::branch_eq(&positive));
        self.emit(abi::float_negate_d("d0", "d0"));
        self.emit(abi::label(&positive));
        Ok(())
    }

    fn emit_double_overflow_check(&mut self, source: &str, overflow_label: &str) {
        self.emit(abi::float_move_x_from_d("x6", source));
        self.emit(abi::shift_right_immediate("x7", "x6", 52));
        self.emit(abi::move_immediate("x5", "Integer", "2047"));
        self.emit(abi::and_registers("x7", "x7", "x5"));
        self.emit(abi::compare_immediate("x7", "2047"));
        self.emit(abi::branch_eq(overflow_label));
    }

    fn lower_arithmetic_binary(
        &mut self,
        op: &str,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let left_slot = self.allocate_stack_object("arith_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(right)?;
        let right_slot = self.allocate_stack_object("arith_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let left_text = left.text.clone();
        let right_text = right.text.clone();
        let result_type = numeric_binary_result_type(op, &left.type_, &right.type_).to_string();
        self.reset_temporary_registers();
        let left_register = self.allocate_register()?;
        let right_register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &left_register,
            abi::stack_pointer(),
            left_slot,
        ));
        self.emit(abi::load_u64(
            &right_register,
            abi::stack_pointer(),
            right_slot,
        ));
        let left = ValueResult {
            type_: left.type_,
            location: left_register,
            text: left_text,
        };
        let right = ValueResult {
            type_: right.type_,
            location: right_register,
            text: right_text,
        };
        let register = self.allocate_register()?;
        match result_type.as_str() {
            "Byte" | "Integer" => {
                self.emit_integer_binary(op, &left, &right, &register, result_type == "Byte")?;
            }
            "Fixed" => self.emit_fixed_binary(op, &left, &right, &register)?,
            "Float" => self.emit_float_binary(op, &left, &right, &register)?,
            other => {
                return Err(format!(
                    "native code plan cannot lower arithmetic result type '{other}'"
                ));
            }
        }
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }

    fn lower_comparison_binary(
        &mut self,
        op: &str,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        let left_slot = self.allocate_stack_object("cmp_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(right)?;
        let right_slot = self.allocate_stack_object("cmp_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        self.reset_temporary_registers();
        let left_register = self.allocate_register()?;
        let right_register = self.allocate_register()?;
        let result = self.allocate_register()?;
        let true_label = self.label("cmp_true");
        let done_label = self.label("cmp_done");
        self.emit(abi::load_u64(
            &left_register,
            abi::stack_pointer(),
            left_slot,
        ));
        self.emit(abi::load_u64(
            &right_register,
            abi::stack_pointer(),
            right_slot,
        ));
        self.emit(abi::compare_registers(&left_register, &right_register));
        match op {
            "=" => self.emit(abi::branch_eq(&true_label)),
            "<>" => self.emit(abi::branch_ne(&true_label)),
            "<" => self.emit(abi::branch_lt(&true_label)),
            ">" => self.emit(abi::branch_gt(&true_label)),
            "<=" => self.emit(abi::branch_le(&true_label)),
            ">=" => self.emit(abi::branch_ge(&true_label)),
            other => {
                return Err(format!(
                    "native code plan does not lower comparison operator '{other}'"
                ));
            }
        }
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::branch(&done_label));
        self.emit(abi::label(&true_label));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::label(&done_label));
        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("({} {op} {})", left.text, right.text),
        })
    }

    fn emit_integer_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
        byte_result: bool,
    ) -> Result<(), String> {
        match op {
            "+" => {
                self.emit(abi::add_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
                if byte_result {
                    self.emit_byte_upper_bound_check(dst)?;
                }
            }
            "-" => {
                if byte_result {
                    let underflow_label = self.label("byte_underflow");
                    let ok_label = self.label("byte_ok");
                    self.emit(abi::compare_registers(&left.location, &right.location));
                    self.emit(abi::branch_lo(&underflow_label));
                    self.emit(abi::subtract_registers(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                    self.emit(abi::branch(&ok_label));
                    self.emit(abi::label(&underflow_label));
                    self.emit_underflow_return()?;
                    self.emit(abi::label(&ok_label));
                } else {
                    self.emit(abi::subtract_registers_set_flags(
                        dst,
                        &left.location,
                        &right.location,
                    ));
                    self.emit_overflow_if_flags_set()?;
                }
            }
            "*" => {
                self.emit_checked_integer_multiply(dst, &left.location, &right.location)?;
                if byte_result {
                    self.emit_byte_upper_bound_check(dst)?;
                }
            }
            "/" | "DIV" => {
                self.emit_nonzero_or_invalid(&right.location)?;
                self.emit_integer_division_overflow_check(&left.location, &right.location)?;
                self.emit(abi::signed_divide_registers(
                    dst,
                    &left.location,
                    &right.location,
                ));
            }
            "MOD" => {
                self.emit_nonzero_or_invalid(&right.location)?;
                self.emit_integer_division_overflow_check(&left.location, &right.location)?;
                let quotient = self.allocate_register()?;
                self.emit(abi::signed_divide_registers(
                    &quotient,
                    &left.location,
                    &right.location,
                ));
                self.emit(abi::multiply_subtract_registers(
                    dst,
                    &quotient,
                    &right.location,
                    &left.location,
                ));
            }
            "^" => self.emit_integer_pow(dst, &left.location, &right.location, byte_result)?,
            other => {
                return Err(format!(
                    "native code plan does not lower integer operator '{other}'"
                ));
            }
        }
        Ok(())
    }

    fn emit_fixed_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
    ) -> Result<(), String> {
        match op {
            "+" => {
                self.emit(abi::add_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            "-" => {
                self.emit(abi::subtract_registers_set_flags(
                    dst,
                    &left.location,
                    &right.location,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            "*" => self.emit_fixed_multiply(dst, &left.location, &right.location)?,
            "/" => self.emit_fixed_divide(dst, &left.location, &right.location)?,
            "MOD" => {
                self.emit_fixed_divide(dst, &left.location, &right.location)?;
                let product = self.allocate_register()?;
                self.emit_fixed_multiply(&product, dst, &right.location)?;
                self.emit(abi::subtract_registers_set_flags(
                    dst,
                    &left.location,
                    &product,
                ));
                self.emit_overflow_if_flags_set()?;
            }
            "^" => self.emit_fixed_pow(dst, &left.location, &right.location)?,
            other => {
                return Err(format!(
                    "native code plan does not lower Fixed operator '{other}'"
                ));
            }
        }
        Ok(())
    }

    fn emit_float_binary(
        &mut self,
        op: &str,
        left: &ValueResult,
        right: &ValueResult,
        dst: &str,
    ) -> Result<(), String> {
        self.load_numeric_as_double("d0", left)?;
        self.load_numeric_as_double("d1", right)?;
        match op {
            "+" => self.emit(abi::float_add_d("d0", "d0", "d1")),
            "-" => self.emit(abi::float_subtract_d("d0", "d0", "d1")),
            "*" => self.emit(abi::float_multiply_d("d0", "d0", "d1")),
            "/" | "DIV" => {
                self.emit(abi::float_compare_zero_d("d1"));
                let nonzero = self.label("float_divisor_nonzero");
                self.emit(abi::branch_ne(&nonzero));
                self.emit_invalid_argument_return()?;
                self.emit(abi::label(&nonzero));
                self.emit(abi::float_divide_d("d0", "d0", "d1"));
            }
            "^" => self.emit_float_pow("d0", "d1")?,
            other => {
                return Err(format!(
                    "native code plan does not lower Float operator '{other}'"
                ));
            }
        }
        self.emit(abi::float_move_x_from_d(dst, "d0"));
        Ok(())
    }

    fn emit_overflow_if_flags_set(&mut self) -> Result<(), String> {
        let ok_label = self.label("overflow_ok");
        self.emit(abi::branch_vc(&ok_label));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    fn emit_checked_integer_add(&mut self, dst: &str, lhs: &str, rhs: &str) -> Result<(), String> {
        self.emit(abi::add_registers_set_flags(dst, lhs, rhs));
        self.emit_overflow_if_flags_set()
    }

    fn emit_byte_upper_bound_check(&mut self, value: &str) -> Result<(), String> {
        let overflow_label = self.label("byte_overflow");
        let ok_label = self.label("byte_ok");
        self.emit(abi::compare_immediate(value, "255"));
        self.emit(abi::branch_hi(&overflow_label));
        self.emit(abi::branch(&ok_label));
        self.emit(abi::label(&overflow_label));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    fn emit_checked_integer_multiply(
        &mut self,
        dst: &str,
        left: &str,
        right: &str,
    ) -> Result<(), String> {
        let high = self.allocate_register()?;
        let sign = self.allocate_register()?;
        let ok_label = self.label("mul_ok");
        self.emit(abi::multiply_registers(dst, left, right));
        self.emit(abi::signed_multiply_high_registers(&high, left, right));
        self.emit(abi::arithmetic_shift_right_immediate(&sign, dst, 63));
        self.emit(abi::compare_registers(&high, &sign));
        self.emit(abi::branch_eq(&ok_label));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    fn emit_nonzero_or_invalid(&mut self, value: &str) -> Result<(), String> {
        let ok_label = self.label("nonzero");
        self.emit(abi::compare_immediate(value, "0"));
        self.emit(abi::branch_ne(&ok_label));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&ok_label));
        Ok(())
    }

    fn emit_integer_division_overflow_check(
        &mut self,
        left: &str,
        right: &str,
    ) -> Result<(), String> {
        let min = self.allocate_register()?;
        let minus_one = self.allocate_register()?;
        let not_min = self.label("div_not_min");
        let ok = self.label("div_overflow_ok");
        self.emit(abi::move_immediate(&min, "Integer", "9223372036854775808"));
        self.emit(abi::compare_registers(left, &min));
        self.emit(abi::branch_ne(&not_min));
        self.emit(abi::move_immediate(
            &minus_one,
            "Integer",
            &u64::MAX.to_string(),
        ));
        self.emit(abi::compare_registers(right, &minus_one));
        self.emit(abi::branch_ne(&ok));
        self.emit_overflow_return()?;
        self.emit(abi::label(&not_min));
        self.emit(abi::label(&ok));
        Ok(())
    }

    fn emit_integer_pow(
        &mut self,
        dst: &str,
        base: &str,
        exponent: &str,
        byte_result: bool,
    ) -> Result<(), String> {
        let loop_label = self.label("pow_loop");
        let done_label = self.label("pow_done");
        let nonnegative = self.label("pow_nonnegative");
        let remaining = self.allocate_register()?;
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&nonnegative));
        self.emit(abi::move_register(&remaining, exponent));
        self.emit(abi::move_immediate(dst, "Integer", "1"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit_checked_integer_multiply(dst, dst, base)?;
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        if byte_result {
            self.emit_byte_upper_bound_check(dst)?;
        }
        Ok(())
    }

    fn emit_fixed_multiply(&mut self, dst: &str, left: &str, right: &str) -> Result<(), String> {
        let high = self.allocate_register()?;
        let shifted_high = self.allocate_register()?;
        let max_high = self.allocate_register()?;
        let min_high = self.allocate_register()?;
        let overflow = self.label("fixed_mul_overflow");
        let ok = self.label("fixed_mul_ok");
        self.emit(abi::multiply_registers(dst, left, right));
        self.emit(abi::signed_multiply_high_registers(&high, left, right));
        self.emit(abi::move_immediate(&max_high, "Integer", "2147483647"));
        self.emit(abi::compare_registers(&high, &max_high));
        self.emit(abi::branch_gt(&overflow));
        self.emit(abi::move_immediate(
            &min_high,
            "Integer",
            &(-2147483648_i64 as u64).to_string(),
        ));
        self.emit(abi::compare_registers(&high, &min_high));
        self.emit(abi::branch_lt(&overflow));
        self.emit(abi::shift_right_immediate(dst, dst, 32));
        self.emit(abi::shift_left_immediate(&shifted_high, &high, 32));
        self.emit(abi::or_registers(dst, &shifted_high, dst));
        self.emit(abi::branch(&ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&ok));
        Ok(())
    }

    fn emit_fixed_divide(&mut self, dst: &str, left: &str, right: &str) -> Result<(), String> {
        self.emit_nonzero_or_invalid(right)?;
        let lhs_abs = self.allocate_register()?;
        let rhs_abs = self.allocate_register()?;
        let sign = self.allocate_register()?;
        let integer = self.allocate_register()?;
        let remainder = self.allocate_register()?;
        let fraction = self.allocate_register()?;
        let counter = self.allocate_register()?;
        let bit = self.allocate_register()?;
        self.emit(abi::move_register(&lhs_abs, left));
        self.emit(abi::move_register(&rhs_abs, right));
        self.emit(abi::exclusive_or_registers(&sign, &lhs_abs, &rhs_abs));
        self.emit_abs_i64(&lhs_abs)?;
        self.emit_abs_i64(&rhs_abs)?;
        self.emit(abi::unsigned_divide_registers(&integer, &lhs_abs, &rhs_abs));
        self.emit(abi::multiply_subtract_registers(
            &remainder, &integer, &rhs_abs, &lhs_abs,
        ));
        let max_integer = self.allocate_register()?;
        let overflow = self.label("fixed_div_overflow");
        let integer_ok = self.label("fixed_div_integer_ok");
        self.emit(abi::move_immediate(&max_integer, "Integer", "2147483647"));
        self.emit(abi::compare_registers(&integer, &max_integer));
        self.emit(abi::branch_hi(&overflow));
        self.emit(abi::shift_left_immediate(dst, &integer, 32));
        self.emit(abi::move_immediate(&fraction, "Integer", "0"));
        self.emit(abi::move_immediate(&counter, "Integer", "32"));
        self.emit(abi::branch(&integer_ok));
        self.emit(abi::label(&overflow));
        self.emit_overflow_return()?;
        self.emit(abi::label(&integer_ok));

        let loop_start = self.label("fixed_div_loop");
        let skip_subtract = self.label("fixed_div_skip_subtract");
        let done = self.label("fixed_div_done");
        self.emit(abi::label(&loop_start));
        self.emit(abi::compare_immediate(&counter, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::shift_left_immediate(&remainder, &remainder, 1));
        self.emit(abi::shift_left_immediate(&fraction, &fraction, 1));
        self.emit(abi::compare_registers(&remainder, &rhs_abs));
        self.emit(abi::branch_lo(&skip_subtract));
        self.emit(abi::subtract_registers(&remainder, &remainder, &rhs_abs));
        self.emit(abi::move_immediate(&bit, "Integer", "1"));
        self.emit(abi::or_registers(&fraction, &fraction, &bit));
        self.emit(abi::label(&skip_subtract));
        self.emit(abi::subtract_immediate(&counter, &counter, 1));
        self.emit(abi::branch(&loop_start));

        self.emit(abi::label(&done));
        self.emit(abi::or_registers(dst, dst, &fraction));
        let negative = self.label("fixed_div_negative");
        let quotient_done = self.label("fixed_div_signed");
        self.emit(abi::compare_immediate(&sign, "0"));
        self.emit(abi::branch_lt(&negative));
        self.emit(abi::compare_immediate(dst, "0"));
        self.emit(abi::branch_ge(&quotient_done));
        self.emit_overflow_return()?;
        self.emit(abi::label(&negative));
        self.emit_neg_i64(dst)?;
        self.emit(abi::label(&quotient_done));
        Ok(())
    }

    fn emit_fixed_pow(&mut self, dst: &str, base: &str, exponent: &str) -> Result<(), String> {
        let one_raw = 1_u64 << 32;
        let remaining = self.allocate_register()?;
        let whole = self.allocate_register()?;
        let nonnegative = self.label("fixed_pow_nonnegative");
        let loop_label = self.label("fixed_pow_loop");
        let done_label = self.label("fixed_pow_done");
        self.emit(abi::compare_immediate(exponent, "0"));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&nonnegative));
        self.emit(abi::arithmetic_shift_right_immediate(&whole, exponent, 32));
        self.emit(abi::shift_left_immediate(&remaining, &whole, 32));
        self.emit(abi::compare_registers(&remaining, exponent));
        let exponent_is_whole = self.label("fixed_pow_whole");
        self.emit(abi::branch_eq(&exponent_is_whole));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&exponent_is_whole));
        self.emit(abi::move_register(&remaining, &whole));
        self.emit(abi::move_immediate(dst, "Fixed", &one_raw.to_string()));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&remaining, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit_fixed_multiply(dst, dst, base)?;
        self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        Ok(())
    }

    fn emit_abs_i64(&mut self, register: &str) -> Result<(), String> {
        let positive = self.label("abs_positive");
        self.emit(abi::compare_immediate(register, "0"));
        self.emit(abi::branch_ge(&positive));
        self.emit_neg_i64(register)?;
        self.emit(abi::label(&positive));
        Ok(())
    }

    fn emit_neg_i64(&mut self, register: &str) -> Result<(), String> {
        self.emit(abi::subtract_registers(register, "xzr", register));
        Ok(())
    }

    fn load_numeric_as_double(&mut self, dst: &str, value: &ValueResult) -> Result<(), String> {
        match value.type_.as_str() {
            "Float" => self.emit(abi::float_move_d_from_x(dst, &value.location)),
            "Byte" | "Integer" => self.emit(abi::signed_convert_to_float_d(dst, &value.location)),
            "Fixed" => {
                self.emit(abi::signed_convert_to_float_d(dst, &value.location));
                self.emit_f64_const("d7", "x17", 4_294_967_296.0);
                self.emit(abi::float_divide_d(dst, dst, "d7"));
            }
            other => {
                return Err(format!(
                    "native Float arithmetic cannot load operand type '{other}'"
                ));
            }
        }
        Ok(())
    }

    fn emit_f64_const(&mut self, dst: &str, scratch: &str, value: f64) {
        self.emit(abi::move_immediate(
            scratch,
            "Integer",
            &value.to_bits().to_string(),
        ));
        self.emit(abi::float_move_d_from_x(dst, scratch));
    }

    fn emit_float_pow(&mut self, dst: &str, exponent: &str) -> Result<(), String> {
        let nonnegative = self.label("float_pow_nonnegative");
        let exponent_whole = self.label("float_pow_whole");
        let loop_label = self.label("float_pow_loop");
        let done_label = self.label("float_pow_done");
        self.emit(abi::float_compare_zero_d(exponent));
        self.emit(abi::branch_ge(&nonnegative));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&nonnegative));
        let exponent_int = self.allocate_register()?;
        let exponent_roundtrip = self.allocate_register()?;
        let exponent_bits = self.allocate_register()?;
        let scratch = self.allocate_register()?;
        self.emit(abi::float_convert_to_signed_x(&exponent_int, exponent));
        self.emit(abi::signed_convert_to_float_d("d2", &exponent_int));
        self.emit(abi::float_move_x_from_d(&exponent_roundtrip, "d2"));
        self.emit(abi::float_move_x_from_d(&exponent_bits, exponent));
        self.emit(abi::compare_registers(&exponent_roundtrip, &exponent_bits));
        self.emit(abi::branch_eq(&exponent_whole));
        self.emit_invalid_argument_return()?;
        self.emit(abi::label(&exponent_whole));
        self.emit_f64_const("d2", &scratch, 1.0);
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(&exponent_int, "0"));
        self.emit(abi::branch_eq(&done_label));
        self.emit(abi::float_multiply_d("d2", "d2", dst));
        self.emit(abi::subtract_immediate(&exponent_int, &exponent_int, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done_label));
        self.emit_f64_const("d7", &scratch, 0.0);
        self.emit(abi::float_add_d(dst, "d2", "d7"));
        Ok(())
    }

    fn lower_find(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let haystack = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&haystack.type_) {
            let haystack_slot = self.allocate_stack_object("find_list_haystack", 8);
            self.emit(abi::store_u64(
                &haystack.location,
                abi::stack_pointer(),
                haystack_slot,
            ));
            let needle = self.lower_value(&args[1])?;
            let needle_slot = self.allocate_stack_object("find_list_needle", 8);
            self.emit(abi::store_u64(
                &needle.location,
                abi::stack_pointer(),
                needle_slot,
            ));
            let start_slot = self.allocate_stack_object("find_list_start", 8);
            if let Some(start) = args.get(2) {
                let start = self.lower_value(start)?;
                if start.type_ != "Integer" {
                    return Err(format!(
                        "native list find start must be Integer, got {}",
                        start.type_
                    ));
                }
                self.emit(abi::store_u64(
                    &start.location,
                    abi::stack_pointer(),
                    start_slot,
                ));
            } else {
                self.emit(abi::move_immediate("x8", "Integer", "0"));
                self.emit(abi::store_u64("x8", abi::stack_pointer(), start_slot));
            }

            if needle.type_ == element_type {
                return self.lower_list_find_item(
                    haystack_slot,
                    needle_slot,
                    start_slot,
                    &haystack.type_,
                    &element_type,
                );
            }
            if needle.type_ == haystack.type_ {
                return self.lower_list_find_sublist(
                    haystack_slot,
                    needle_slot,
                    start_slot,
                    &haystack.type_,
                    &element_type,
                );
            }
            return Err(format!(
                "native list find needle must be {} or {}, got {}",
                element_type, haystack.type_, needle.type_
            ));
        }
        if haystack.type_ != "String" {
            return Err(format!(
                "native string find haystack must be String, got {}",
                haystack.type_
            ));
        }
        let haystack_slot = self.allocate_stack_object("find_haystack", 8);
        self.emit(abi::store_u64(
            &haystack.location,
            abi::stack_pointer(),
            haystack_slot,
        ));

        let needle = self.lower_value(&args[1])?;
        if needle.type_ != "String" {
            return Err(format!(
                "native string find needle must be String, got {}",
                needle.type_
            ));
        }
        let needle_slot = self.allocate_stack_object("find_needle", 8);
        self.emit(abi::store_u64(
            &needle.location,
            abi::stack_pointer(),
            needle_slot,
        ));

        let start_slot = self.allocate_stack_object("find_start", 8);
        if let Some(start) = args.get(2) {
            let start = self.lower_value(start)?;
            if start.type_ != "Integer" {
                return Err(format!(
                    "native string find start must be Integer, got {}",
                    start.type_
                ));
            }
            self.emit(abi::store_u64(
                &start.location,
                abi::stack_pointer(),
                start_slot,
            ));
        } else {
            self.emit(abi::move_immediate("x8", "Integer", "0"));
            self.emit(abi::store_u64("x8", abi::stack_pointer(), start_slot));
        }

        let result_slot = self.allocate_stack_object("find_result", 8);
        let haystack_ptr = "x8";
        let needle_ptr = "x9";
        let haystack_len = "x10";
        let needle_len = "x11";
        let start = "x12";
        let scalar_index = "x13";
        let cursor = "x14";
        let remaining = "x15";
        let byte = "x16";
        let mask = "x17";
        let candidate = "x20";
        let compare_remaining = "x21";
        let needle_cursor = "x22";
        let haystack_byte = "x23";
        let needle_byte = "x24";
        for register in [
            candidate,
            compare_remaining,
            needle_cursor,
            haystack_byte,
            needle_byte,
        ] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let locate_start = self.label("find_locate_start");
        let locate_continue = self.label("find_locate_continue");
        let start_ready = self.label("find_start_ready");
        let search_loop = self.label("find_search_loop");
        let compare_loop = self.label("find_compare_loop");
        let advance_candidate = self.label("find_advance_candidate");
        let skip_continuation = self.label("find_skip_continuation");
        let candidate_ready = self.label("find_candidate_ready");
        let found = self.label("find_found");
        let invalid_start = self.label("find_invalid_start");
        let not_found = self.label("find_not_found");

        self.emit(abi::load_u64(
            haystack_ptr,
            abi::stack_pointer(),
            haystack_slot,
        ));
        self.emit(abi::load_u64(needle_ptr, abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64(haystack_len, haystack_ptr, 0));
        self.emit(abi::load_u64(needle_len, needle_ptr, 0));
        self.emit(abi::load_u64(start, abi::stack_pointer(), start_slot));
        self.emit(abi::move_immediate(scalar_index, "Integer", "0"));
        self.emit(abi::add_immediate(cursor, haystack_ptr, 8));
        self.emit(abi::move_register(remaining, haystack_len));
        self.emit(abi::move_immediate(mask, "Integer", "192"));

        self.emit(abi::label(&locate_start));
        self.emit(abi::compare_registers(scalar_index, start));
        self.emit(abi::branch_eq(&start_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_start));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_eq(&locate_continue));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::label(&locate_continue));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&locate_start));

        self.emit(abi::label(&start_ready));
        self.emit(abi::compare_immediate(needle_len, "0"));
        self.emit(abi::branch_eq(&found));

        self.emit(abi::label(&search_loop));
        self.emit(abi::compare_registers(remaining, needle_len));
        self.emit(abi::branch_lo(&not_found));
        self.emit(abi::move_register(candidate, cursor));
        self.emit(abi::add_immediate(needle_cursor, needle_ptr, 8));
        self.emit(abi::move_register(compare_remaining, needle_len));

        self.emit(abi::label(&compare_loop));
        self.emit(abi::compare_immediate(compare_remaining, "0"));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::load_u8(haystack_byte, candidate, 0));
        self.emit(abi::load_u8(needle_byte, needle_cursor, 0));
        self.emit(abi::compare_registers(haystack_byte, needle_byte));
        self.emit(abi::branch_ne(&advance_candidate));
        self.emit(abi::add_immediate(candidate, candidate, 1));
        self.emit(abi::add_immediate(needle_cursor, needle_cursor, 1));
        self.emit(abi::subtract_immediate(
            compare_remaining,
            compare_remaining,
            1,
        ));
        self.emit(abi::branch(&compare_loop));

        self.emit(abi::label(&advance_candidate));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::label(&skip_continuation));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&candidate_ready));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_ne(&candidate_ready));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&skip_continuation));
        self.emit(abi::label(&candidate_ready));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&search_loop));

        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64(
            scalar_index,
            abi::stack_pointer(),
            result_slot,
        ));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));

        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: "find(String, String)".to_string(),
        })
    }

    fn lower_list_find_item(
        &mut self,
        haystack_slot: usize,
        needle_slot: usize,
        start_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let result_slot = self.allocate_stack_object("find_list_result", 8);
        let valid_start = self.label("list_find_item_valid_start");
        let loop_label = self.label("list_find_item_loop");
        let found = self.label("list_find_item_found");
        let next = self.label("list_find_item_next");
        let invalid_start = self.label("list_find_item_invalid_start");
        let not_found = self.label("list_find_item_not_found");
        let done = self.label("list_find_item_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), haystack_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate("x11", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_gt(&invalid_start));
        self.emit(abi::move_register("x12", "x11"));
        self.emit(abi::move_immediate(
            "x13",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x14", "x12", "x13"));
        self.emit(abi::add_immediate("x15", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x15", "x15", "x14"));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x12", "x10"));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::load_u64(
            "x16",
            "x15",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x17",
            "x15",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            element_type,
            "x8",
            "x16",
            "x17",
            "x9",
            &found,
            &next,
        )?;

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate("x15", "x15", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: format!("find({list_type}, {element_type})"),
        })
    }

    fn lower_list_find_sublist(
        &mut self,
        haystack_slot: usize,
        needle_slot: usize,
        start_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }

        let result_slot = self.allocate_stack_object("find_sublist_result", 8);
        let valid_start = self.label("list_find_sublist_valid_start");
        let empty_found = self.label("list_find_sublist_empty_found");
        let outer_loop = self.label("list_find_sublist_outer_loop");
        let compare_loop = self.label("list_find_sublist_compare_loop");
        let compare_next = self.label("list_find_sublist_compare_next");
        let found = self.label("list_find_sublist_found");
        let advance_outer = self.label("list_find_sublist_advance_outer");
        let invalid_start = self.label("list_find_sublist_invalid_start");
        let not_found = self.label("list_find_sublist_not_found");
        let done = self.label("list_find_sublist_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), haystack_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), needle_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), start_slot));
        self.emit(abi::compare_immediate("x12", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_start));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x12", "x10"));
        self.emit(abi::branch_gt(&invalid_start));
        self.emit(abi::compare_immediate("x11", "0"));
        self.emit(abi::branch_eq(&empty_found));

        self.emit(abi::move_register("x13", "x12"));
        self.emit(abi::label(&outer_loop));
        self.emit(abi::add_registers("x14", "x13", "x11"));
        self.emit(abi::compare_registers("x14", "x10"));
        self.emit(abi::branch_gt(&not_found));
        self.emit(abi::move_immediate("x14", "Integer", "0"));

        self.emit(abi::label(&compare_loop));
        self.emit(abi::compare_registers("x14", "x11"));
        self.emit(abi::branch_eq(&found));
        self.emit(abi::add_registers("x15", "x13", "x14"));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x15", "x16"));
        self.emit(abi::add_immediate("x17", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x17", "x17", "x15"));
        self.emit(abi::multiply_registers("x20", "x14", "x16"));
        self.emit(abi::add_immediate("x25", "x9", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x20", "x25", "x20"));
        self.emit(abi::load_u64(
            "x21",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64(
            "x23",
            "x20",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x24",
            "x20",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payloads_match_branch(
            element_type,
            "x8",
            "x21",
            "x22",
            "x9",
            "x23",
            "x24",
            &compare_next,
            &advance_outer,
        )?;

        self.emit(abi::label(&compare_next));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::branch(&compare_loop));

        self.emit(abi::label(&advance_outer));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::branch(&outer_loop));

        self.emit(abi::label(&empty_found));
        self.emit(abi::store_u64("x12", abi::stack_pointer(), result_slot));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid_start));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&found));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), result_slot));
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: "Integer".to_string(),
            location: result,
            text: format!("find({list_type}, {list_type}) over {element_type}"),
        })
    }

    fn lower_mid(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let value = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&value.type_) {
            let value_slot = self.allocate_stack_object("mid_list_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let start = self.lower_value(&args[1])?;
            if start.type_ != "Integer" {
                return Err(format!(
                    "native list mid start must be Integer, got {}",
                    start.type_
                ));
            }
            let start_slot = self.allocate_stack_object("mid_list_start", 8);
            self.emit(abi::store_u64(
                &start.location,
                abi::stack_pointer(),
                start_slot,
            ));
            let count = self.lower_value(&args[2])?;
            if count.type_ != "Integer" {
                return Err(format!(
                    "native list mid count must be Integer, got {}",
                    count.type_
                ));
            }
            let count_slot = self.allocate_stack_object("mid_list_count", 8);
            self.emit(abi::store_u64(
                &count.location,
                abi::stack_pointer(),
                count_slot,
            ));
            return self.lower_list_mid(
                value_slot,
                start_slot,
                count_slot,
                &value.type_,
                &element_type,
            );
        }
        if value.type_ != "String" {
            return Err(format!(
                "native string mid value must be String, got {}",
                value.type_
            ));
        }
        let value_slot = self.allocate_stack_object("mid_value", 8);
        self.emit(abi::store_u64(
            &value.location,
            abi::stack_pointer(),
            value_slot,
        ));

        let start = self.lower_value(&args[1])?;
        if start.type_ != "Integer" {
            return Err(format!(
                "native string mid start must be Integer, got {}",
                start.type_
            ));
        }
        let start_slot = self.allocate_stack_object("mid_start", 8);
        self.emit(abi::store_u64(
            &start.location,
            abi::stack_pointer(),
            start_slot,
        ));

        let count = self.lower_value(&args[2])?;
        if count.type_ != "Integer" {
            return Err(format!(
                "native string mid count must be Integer, got {}",
                count.type_
            ));
        }
        let count_slot = self.allocate_stack_object("mid_count", 8);
        self.emit(abi::store_u64(
            &count.location,
            abi::stack_pointer(),
            count_slot,
        ));

        let result_slot = self.allocate_stack_object("mid_result", 8);
        let start_ptr_slot = self.allocate_stack_object("mid_start_ptr", 8);
        let byte_len_slot = self.allocate_stack_object("mid_byte_len", 8);
        let value_ptr = "x8";
        let string_len = "x9";
        let cursor = "x10";
        let remaining = "x11";
        let scalar_index = "x12";
        let start_index = "x13";
        let count_value = "x14";
        let end_index = "x15";
        let byte = "x16";
        let mask = "x17";
        let start_ptr = "x20";
        let end_ptr = "x21";
        let copy_src = "x22";
        let copy_dst = "x23";
        let copy_remaining = "x24";
        let byte_len = "x25";
        for register in [
            start_ptr,
            end_ptr,
            copy_src,
            copy_dst,
            copy_remaining,
            byte_len,
        ] {
            if abi::is_callee_saved(register)
                && !self.used_callee_saved.iter().any(|saved| saved == register)
            {
                self.used_callee_saved.push(register.to_string());
            }
        }

        let locate_start = self.label("mid_locate_start");
        let locate_start_continue = self.label("mid_locate_start_continue");
        let locate_start_advanced = self.label("mid_locate_start_advanced");
        let start_ready = self.label("mid_start_ready");
        let locate_end = self.label("mid_locate_end");
        let locate_end_continue = self.label("mid_locate_end_continue");
        let locate_end_advanced = self.label("mid_locate_end_advanced");
        let end_ready = self.label("mid_end_ready");
        let alloc_ok = self.label("mid_alloc_ok");
        let copy_loop = self.label("mid_copy_loop");
        let copy_done = self.label("mid_copy_done");
        let invalid_range = self.label("mid_invalid_range");

        self.emit(abi::load_u64(value_ptr, abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64(start_index, abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64(count_value, abi::stack_pointer(), count_slot));
        self.emit(abi::compare_immediate(start_index, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_immediate(count_value, "0"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::add_registers(end_index, start_index, count_value));
        self.emit(abi::compare_registers(end_index, start_index));
        self.emit(abi::branch_lo(&invalid_range));
        self.emit(abi::load_u64(string_len, value_ptr, 0));
        self.emit(abi::add_immediate(cursor, value_ptr, 8));
        self.emit(abi::move_register(start_ptr, cursor));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::move_register(remaining, string_len));
        self.emit(abi::move_immediate(scalar_index, "Integer", "0"));
        self.emit(abi::move_immediate(mask, "Integer", "192"));

        self.emit(abi::label(&locate_start));
        self.emit(abi::compare_registers(scalar_index, start_index));
        self.emit(abi::branch_eq(&start_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_range));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::label(&locate_start_continue));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&locate_start_advanced));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_ne(&locate_start_advanced));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&locate_start_continue));
        self.emit(abi::label(&locate_start_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&locate_start));

        self.emit(abi::label(&start_ready));
        self.emit(abi::move_register(start_ptr, cursor));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::label(&locate_end));
        self.emit(abi::compare_registers(scalar_index, end_index));
        self.emit(abi::branch_eq(&end_ready));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&invalid_range));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::label(&locate_end_continue));
        self.emit(abi::compare_immediate(remaining, "0"));
        self.emit(abi::branch_eq(&locate_end_advanced));
        self.emit(abi::load_u8(byte, cursor, 0));
        self.emit(abi::and_registers(byte, byte, mask));
        self.emit(abi::compare_immediate(byte, "128"));
        self.emit(abi::branch_ne(&locate_end_advanced));
        self.emit(abi::add_immediate(cursor, cursor, 1));
        self.emit(abi::subtract_immediate(remaining, remaining, 1));
        self.emit(abi::branch(&locate_end_continue));
        self.emit(abi::label(&locate_end_advanced));
        self.emit(abi::add_immediate(scalar_index, scalar_index, 1));
        self.emit(abi::branch(&locate_end));

        self.emit(abi::label(&end_ready));
        self.emit(abi::move_register(end_ptr, cursor));
        self.emit(abi::subtract_registers(byte_len, end_ptr, start_ptr));
        self.emit(abi::store_u64(
            start_ptr,
            abi::stack_pointer(),
            start_ptr_slot,
        ));
        self.emit(abi::store_u64(
            byte_len,
            abi::stack_pointer(),
            byte_len_slot,
        ));
        self.emit(abi::add_immediate(abi::return_register(), byte_len, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64(byte_len, abi::stack_pointer(), byte_len_slot));
        self.emit(abi::store_u64(byte_len, "x1", 0));
        self.emit(abi::load_u64(
            start_ptr,
            abi::stack_pointer(),
            start_ptr_slot,
        ));
        self.emit(abi::move_register(copy_src, start_ptr));
        self.emit(abi::add_immediate(copy_dst, "x1", 8));
        self.emit(abi::move_register(copy_remaining, byte_len));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate(copy_remaining, "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8(byte, copy_src, 0));
        self.emit(abi::store_u8(byte, copy_dst, 0));
        self.emit(abi::add_immediate(copy_src, copy_src, 1));
        self.emit(abi::add_immediate(copy_dst, copy_dst, 1));
        self.emit(abi::subtract_immediate(copy_remaining, copy_remaining, 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate(byte, "Integer", "0"));
        self.emit(abi::store_u8(byte, copy_dst, 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        let done = self.label("mid_done");
        self.emit(abi::branch(&done));

        self.emit(abi::label(&invalid_range));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: result,
            text: "mid(String, Integer, Integer)".to_string(),
        })
    }

    fn lower_list_mid(
        &mut self,
        value_slot: usize,
        start_slot: usize,
        count_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }

        let data_len_slot = self.allocate_stack_object("mid_list_data_len", 8);
        let result_slot = self.allocate_stack_object("mid_list_result", 8);
        let valid_start = self.label("mid_list_valid_start");
        let valid_count = self.label("mid_list_valid_count");
        let range_ok = self.label("mid_list_range_ok");
        let length_loop = self.label("mid_list_length_loop");
        let length_done = self.label("mid_list_length_done");
        let alloc_ok = self.label("mid_list_alloc_ok");
        let copy_loop = self.label("mid_list_copy_loop");
        let copy_entry = self.label("mid_list_copy_entry");
        let copy_bytes = self.label("mid_list_copy_bytes");
        let copy_bytes_done = self.label("mid_list_copy_bytes_done");
        let copy_done = self.label("mid_list_copy_done");
        let invalid_range = self.label("mid_list_invalid_range");
        let done = self.label("mid_list_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_ge(&valid_count));
        self.emit(abi::branch(&invalid_range));
        self.emit(abi::label(&valid_count));
        self.emit(abi::compare_registers("x9", "x11"));
        self.emit(abi::branch_gt(&invalid_range));
        self.emit(abi::add_registers("x12", "x9", "x10"));
        self.emit(abi::compare_registers("x12", "x9"));
        self.emit(abi::branch_lt(&invalid_range));
        self.emit(abi::compare_registers("x12", "x11"));
        self.emit(abi::branch_le(&range_ok));
        self.emit(abi::branch(&invalid_range));

        self.emit(abi::label(&range_ok));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::store_u64("x13", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x9", "x14"));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x16", "x16", "x15"));

        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers("x13", "x10"));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64(
            "x17",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64("x20", abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_registers("x20", "x20", "x17"));
        self.emit(abi::store_u64("x20", abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::branch(&length_loop));

        self.emit(abi::label(&length_done));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x10", "x14"));
        self.emit(abi::load_u64("x16", abi::stack_pointer(), data_len_slot));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x15",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x16",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));

        self.emit(abi::move_immediate("x13", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate("x13", "Byte", "1"));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), count_slot));
        self.emit(abi::store_u64("x10", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x10", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x16", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x16", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x16", "x1", COLLECTION_OFFSET_DATA_CAPACITY));

        self.emit(abi::load_u64("x8", abi::stack_pointer(), value_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), start_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), count_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x9", "x14"));
        self.emit(abi::add_immediate("x16", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x16", "x16", "x15"));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::multiply_registers("x21", "x10", "x14"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x12", "Integer", "0"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));

        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x12", "x10"));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::label(&copy_entry));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x22", "x17", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            "x22",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            "x16",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            "x13",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", "x20", "x22"));
        self.emit(abi::add_registers("x25", "x21", "x13"));

        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        self.emit(abi::load_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x13", "x13", "x23"));
        self.emit(abi::add_immediate("x16", "x16", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));

        self.emit(abi::label(&invalid_range));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&copy_done));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("mid({list_type}, Integer, Integer) over {element_type}"),
        })
    }

    fn lower_collection_get(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("get_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("get_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));

        if let Some(element_type) = list_element_type(&collection.type_) {
            if key.type_ != "Integer" {
                return Err(format!(
                    "native collection get list index must be Integer, got {}",
                    key.type_
                ));
            }
            return self.lower_list_get(
                collection_slot,
                key_slot,
                &collection.type_,
                &element_type,
            );
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            if key.type_ != key_type {
                return Err(format!(
                    "native collection get map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            return self.lower_map_get(
                collection_slot,
                key_slot,
                &collection.type_,
                &key_type,
                &value_type,
            );
        }

        Err(format!(
            "native collection get does not accept {}",
            collection.type_
        ))
    }

    fn lower_collection_contains(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("contains_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let item = self.lower_value(&args[1])?;
        let item_slot = self.allocate_stack_object("contains_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));

        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection contains does not accept {}",
                collection.type_
            ));
        };
        if item.type_ != element_type {
            return Err(format!(
                "native collection contains item must be {}, got {}",
                element_type, item.type_
            ));
        }

        self.reset_temporary_registers();
        let collection_register = self.allocate_register()?;
        let item_register = self.allocate_register()?;
        let count = self.allocate_register()?;
        let index = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let result = self.allocate_register()?;
        let loop_label = self.label("contains_loop");
        let found = self.label("contains_found");
        let next = self.label("contains_next");
        let not_found = self.label("contains_not_found");
        let done = self.label("contains_done");

        self.emit(abi::load_u64(
            &collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(
            &item_register,
            abi::stack_pointer(),
            item_slot,
        ));
        self.emit(abi::load_u64(
            &count,
            &collection_register,
            COLLECTION_OFFSET_COUNT,
        ));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::add_immediate(
            &entry,
            &collection_register,
            COLLECTION_HEADER_SIZE,
        ));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_collection_payload_match_branch(
            &element_type,
            &collection_register,
            &value_offset,
            &value_length,
            &item_register,
            &found,
            &next,
        )?;

        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("contains({}, {})", collection.type_, element_type),
        })
    }

    fn lower_collection_get_or(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("get_or_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));

        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("get_or_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));

        let default = self.lower_value(&args[2])?;
        let default_slot = self.allocate_stack_object("get_or_default", 8);
        self.emit(abi::store_u64(
            &default.location,
            abi::stack_pointer(),
            default_slot,
        ));

        if let Some(element_type) = list_element_type(&collection.type_) {
            if key.type_ != "Integer" {
                return Err(format!(
                    "native collection getOr list index must be Integer, got {}",
                    key.type_
                ));
            }
            if default.type_ != element_type {
                return Err(format!(
                    "native collection getOr default must be {}, got {}",
                    element_type, default.type_
                ));
            }
            return self.lower_list_get_or(
                collection_slot,
                key_slot,
                default_slot,
                &collection.type_,
                &element_type,
            );
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            if key.type_ != key_type {
                return Err(format!(
                    "native collection getOr map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            if default.type_ != value_type {
                return Err(format!(
                    "native collection getOr default must be {}, got {}",
                    value_type, default.type_
                ));
            }
            return self.lower_map_get_or(
                collection_slot,
                key_slot,
                default_slot,
                &collection.type_,
                &key_type,
                &value_type,
            );
        }

        Err(format!(
            "native collection getOr does not accept {}",
            collection.type_
        ))
    }

    fn lower_collection_has_key(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let collection_slot = self.allocate_stack_object("has_key_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let key = self.lower_value(&args[1])?;
        let key_slot = self.allocate_stack_object("has_key_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));

        let Some((key_type, _)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection hasKey does not accept {}",
                collection.type_
            ));
        };
        if key.type_ != key_type {
            return Err(format!(
                "native collection hasKey key must be {}, got {}",
                key_type, key.type_
            ));
        }

        self.reset_temporary_registers();
        let result = self.allocate_register()?;
        let loop_label = self.label("has_key_loop");
        let found = self.label("has_key_found");
        let next = self.label("has_key_next");
        let not_found = self.label("has_key_not_found");
        let done = self.label("has_key_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::load_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x14",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            &key_type, "x8", "x13", "x14", "x9", &found, &next,
        )?;
        self.emit(abi::label(&found));
        self.emit(abi::move_immediate(&result, "Boolean", "true"));
        self.emit(abi::branch(&done));
        self.emit(abi::label(&next));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&not_found));
        self.emit(abi::move_immediate(&result, "Boolean", "false"));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: "Boolean".to_string(),
            location: result,
            text: format!("hasKey({}, {})", collection.type_, key_type),
        })
    }

    fn lower_collection_keys(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some((key_type, _)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection keys does not accept {}",
                collection.type_
            ));
        };
        self.lower_map_projection(&collection, &key_type, true)
    }

    fn lower_collection_values_builtin(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some((_, value_type)) = map_type_parts(&collection.type_) else {
            return Err(format!(
                "native collection values does not accept {}",
                collection.type_
            ));
        };
        self.lower_map_projection(&collection, &value_type, false)
    }

    fn lower_map_projection(
        &mut self,
        collection: &ValueResult,
        element_type: &str,
        project_key: bool,
    ) -> Result<ValueResult, String> {
        let collection_slot = self.allocate_stack_object("map_projection_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let layout = CollectionTypeLayout::from_type(&format!("List OF {element_type}"))
            .ok_or_else(|| {
                format!("native code collection type 'List OF {element_type}' is not supported")
            })?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let data_len_slot = self.allocate_stack_object("map_projection_data_len", 8);
        let result_slot = self.allocate_stack_object("map_projection_result", 8);
        let length_loop = self.label("map_projection_length_loop");
        let length_done = self.label("map_projection_length_done");
        let alloc_ok = self.label("map_projection_alloc_ok");
        let copy_loop = self.label("map_projection_copy_loop");
        let copy_bytes = self.label("map_projection_copy_bytes");
        let copy_bytes_done = self.label("map_projection_copy_bytes_done");
        let copy_done = self.label("map_projection_copy_done");
        let offset_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET
        };
        let length_field = if project_key {
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH
        } else {
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH
        };

        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&length_loop));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_ge(&length_done));
        self.emit(abi::load_u64("x13", "x12", length_field));
        self.emit(abi::add_registers("x11", "x11", "x13"));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x10", "x10", 1));
        self.emit(abi::branch(&length_loop));
        self.emit(abi::label(&length_done));
        self.emit(abi::store_u64("x11", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x9", "x14"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x15",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x11",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::move_immediate("x13", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x13",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate("x13", "Byte", "1"));
        self.emit(abi::store_u8("x13", "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x9", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x9", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x11", "x1", COLLECTION_OFFSET_DATA_CAPACITY));

        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::move_immediate(
            "x14",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x9", "x14"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8("x22", "x17", COLLECTION_ENTRY_OFFSET_FLAGS));
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x22",
            "x17",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64("x22", "x12", offset_field));
        self.emit(abi::load_u64("x23", "x12", length_field));
        self.emit(abi::store_u64(
            "x11",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", "x20", "x22"));
        self.emit(abi::add_registers("x25", "x21", "x11"));
        self.emit(abi::label(&copy_bytes));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&copy_bytes_done));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&copy_bytes));
        self.emit(abi::label(&copy_bytes_done));
        self.emit(abi::load_u64(
            "x23",
            "x17",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x11", "x11", "x23"));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x17", "x17", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x10", "x10", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: format!("List OF {element_type}"),
            location: result,
            text: if project_key {
                format!("keys({})", collection.type_)
            } else {
                format!("values({})", collection.type_)
            },
        })
    }

    fn lower_collection_sum(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection sum does not accept {}",
                collection.type_
            ));
        };
        if !matches!(element_type.as_str(), "Integer" | "Float" | "Fixed") {
            return Err(format!(
                "native collection sum does not accept {}",
                collection.type_
            ));
        }
        let collection_slot = self.allocate_stack_object("sum_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let loop_label = self.label("sum_loop");
        let done = self.label("sum_done");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x10", "Integer", "0"));
        self.emit(abi::add_immediate("x11", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate("x14", &element_type, "0"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers("x10", "x9"));
        self.emit(abi::branch_ge(&done));
        self.emit(abi::load_u64(
            "x12",
            "x11",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit_collection_data_pointer("x15", "x8");
        self.emit(abi::add_registers("x15", "x15", "x12"));
        match element_type.as_str() {
            "Integer" => {
                self.emit(abi::load_u64("x16", "x15", 0));
                self.emit_checked_integer_add("x14", "x14", "x16")?;
            }
            "Float" => {
                self.emit(abi::load_u64("x16", "x15", 0));
                self.emit(abi::float_move_d_from_x("d0", "x14"));
                self.emit(abi::float_move_d_from_x("d1", "x16"));
                self.emit(abi::float_add_d("d0", "d0", "d1"));
                self.emit(abi::float_move_x_from_d("x14", "d0"));
            }
            "Fixed" => {
                self.emit(abi::load_u64("x16", "x15", 0));
                self.emit_checked_integer_add("x14", "x14", "x16")?;
            }
            _ => unreachable!(),
        }
        self.emit(abi::add_immediate("x11", "x11", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x10", "x10", 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::move_register(&result, "x14"));
        Ok(ValueResult {
            type_: element_type,
            location: result,
            text: format!("sum({})", collection.type_),
        })
    }

    fn lower_collection_for_each_call(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection forEach does not accept {}",
                collection.type_
            ));
        };
        let action = self.lower_value(&args[1])?;
        if !action.type_.starts_with("FUNC(") {
            return Err(format!(
                "native collection forEach action must be a function, got {}",
                action.type_
            ));
        }
        if action.location == "void" {
            return Err(
                "native collection forEach action does not have a callable location".to_string(),
            );
        }
        let action_slot = self.allocate_stack_object("for_each_call_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let collection_slot = self.allocate_stack_object("for_each_call_collection", 8);
        let cursor_slot = self.allocate_stack_object("for_each_call_cursor", 8);
        let remaining_slot = self.allocate_stack_object("for_each_call_remaining", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
        let loop_label = self.label("for_each_call_loop");
        let ok_label = self.label("for_each_call_ok");
        let done = self.label("for_each_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64(
            "x11",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x12",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        let item = self.emit_load_collection_payload(&element_type, "x8", "x11", "x12")?;
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        Ok(ValueResult {
            type_: "Nothing".to_string(),
            location: "void".to_string(),
            text: format!("forEach({}, {})", collection.type_, action.text),
        })
    }

    fn lower_collection_transform_call(
        &mut self,
        args: &[NirValue],
    ) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection transform does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("transform_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection transform action must be a function, got {}",
                action.type_
            )
        })?;
        self.require_direct_callable("transform", &action)?;
        let action_slot = self.allocate_stack_object("transform_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let output_list_type = format!("List OF {output_type}");
        let output = self.lower_empty_collection(&output_list_type)?;
        let output_slot = self.allocate_stack_object("transform_output", 8);
        let cursor_slot = self.allocate_stack_object("transform_cursor", 8);
        let remaining_slot = self.allocate_stack_object("transform_remaining", 8);
        self.emit(abi::store_u64(
            &output.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

        let loop_label = self.label("transform_call_loop");
        let ok_label = self.label("transform_call_ok");
        let done = self.label("transform_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));

        let item_slot = self.allocate_stack_object("transform_item", 8);
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            item_slot,
        ));
        let singleton = self.lower_collection_values(
            &output_list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: output_type.clone(),
                },
            }],
            "transform item",
        )?;
        let singleton_slot = self.allocate_stack_object("transform_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            singleton_slot,
        ));
        let index_slot = self.allocate_stack_object("transform_append_index", 8);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), output_slot));
        self.emit(abi::load_u64("x8", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), index_slot));
        let updated = self.lower_list_insert_collection(
            output_slot,
            index_slot,
            singleton_slot,
            &output_list_type,
            &output_type,
        )?;
        self.emit(abi::store_u64(
            &updated.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: output_list_type,
            location: result,
            text: format!("transform({}, {})", collection.type_, action.text),
        })
    }

    fn lower_collection_filter_call(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection filter does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("filter_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let action = self.lower_value(&args[1])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection filter predicate must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != "Boolean" {
            return Err(format!(
                "native collection filter predicate must return Boolean, got {output_type}"
            ));
        }
        self.require_direct_callable("filter", &action)?;
        let action_slot = self.allocate_stack_object("filter_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let output = self.lower_empty_collection(&collection.type_)?;
        let output_slot = self.allocate_stack_object("filter_output", 8);
        let cursor_slot = self.allocate_stack_object("filter_cursor", 8);
        let remaining_slot = self.allocate_stack_object("filter_remaining", 8);
        let item_slot = self.allocate_stack_object("filter_item", 8);
        self.emit(abi::store_u64(
            &output.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

        let loop_label = self.label("filter_call_loop");
        let ok_label = self.label("filter_call_ok");
        let keep_label = self.label("filter_call_keep");
        let skip_label = self.label("filter_call_skip");
        let done = self.label("filter_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::store_u64(&item, abi::stack_pointer(), item_slot));
        self.emit(abi::move_register(&abi::argument_register(0)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));
        self.emit(abi::compare_immediate(RESULT_VALUE_REGISTER, "0"));
        self.emit(abi::branch_ne(&keep_label));
        self.emit(abi::branch(&skip_label));
        self.emit(abi::label(&keep_label));
        let singleton = self.lower_collection_values(
            &collection.type_,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.clone(),
                },
            }],
            "filter item",
        )?;
        let singleton_slot = self.allocate_stack_object("filter_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            singleton_slot,
        ));
        let index_slot = self.allocate_stack_object("filter_append_index", 8);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), output_slot));
        self.emit(abi::load_u64("x8", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), index_slot));
        let updated = self.lower_list_insert_collection(
            output_slot,
            index_slot,
            singleton_slot,
            &collection.type_,
            &element_type,
        )?;
        self.emit(abi::store_u64(
            &updated.location,
            abi::stack_pointer(),
            output_slot,
        ));
        self.emit(abi::label(&skip_label));
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), output_slot));
        Ok(ValueResult {
            type_: collection.type_.clone(),
            location: result,
            text: format!("filter({}, {})", collection.type_, action.text),
        })
    }

    fn lower_collection_reduce_call(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&collection.type_) else {
            return Err(format!(
                "native collection reduce does not accept {}",
                collection.type_
            ));
        };
        let collection_slot = self.allocate_stack_object("reduce_collection", 8);
        self.emit(abi::store_u64(
            &collection.location,
            abi::stack_pointer(),
            collection_slot,
        ));
        let initial = self.lower_value(&args[1])?;
        let accumulator_slot = self.allocate_stack_object("reduce_accumulator", 8);
        self.emit(abi::store_u64(
            &initial.location,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        let action = self.lower_value(&args[2])?;
        let output_type = callable_return_type(&action.type_).ok_or_else(|| {
            format!(
                "native collection reduce reducer must be a function, got {}",
                action.type_
            )
        })?;
        if output_type != initial.type_ {
            return Err(format!(
                "native collection reduce reducer must return {}, got {output_type}",
                initial.type_
            ));
        }
        self.require_direct_callable("reduce", &action)?;
        let action_slot = self.allocate_stack_object("reduce_action", 8);
        self.emit(abi::store_u64(
            &action.location,
            abi::stack_pointer(),
            action_slot,
        ));
        let cursor_slot = self.allocate_stack_object("reduce_cursor", 8);
        let remaining_slot = self.allocate_stack_object("reduce_remaining", 8);
        self.initialize_collection_loop_slots(collection_slot, cursor_slot, remaining_slot);

        let loop_label = self.label("reduce_call_loop");
        let ok_label = self.label("reduce_call_ok");
        let done = self.label("reduce_call_done");
        self.emit(abi::label(&loop_label));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::compare_immediate("x9", "0"));
        self.emit(abi::branch_eq(&done));
        let item = self.load_collection_loop_item(collection_slot, cursor_slot, &element_type)?;
        self.emit(abi::load_u64(
            &abi::argument_register(0)?,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        self.emit(abi::move_register(&abi::argument_register(1)?, &item));
        self.emit(abi::load_u64("x17", abi::stack_pointer(), action_slot));
        self.emit_direct_callable_branch("x17");
        self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
        self.emit(abi::branch_eq(&ok_label));
        self.emit(abi::return_());
        self.emit(abi::label(&ok_label));
        self.emit(abi::store_u64(
            RESULT_VALUE_REGISTER,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        self.advance_collection_loop(cursor_slot, remaining_slot, &loop_label);
        self.emit(abi::label(&done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(
            &result,
            abi::stack_pointer(),
            accumulator_slot,
        ));
        Ok(ValueResult {
            type_: initial.type_,
            location: result,
            text: format!(
                "reduce({}, {}, {})",
                collection.type_, initial.text, action.text
            ),
        })
    }

    fn require_direct_callable(&self, name: &str, action: &ValueResult) -> Result<(), String> {
        if !action.type_.starts_with("FUNC(") {
            return Err(format!(
                "native collection {name} action must be a function, got {}",
                action.type_
            ));
        }
        if action.location == "void" {
            return Err(format!(
                "native collection {name} action does not have a callable location"
            ));
        }
        Ok(())
    }

    fn emit_direct_callable_branch(&mut self, location: &str) {
        self.emit_callable_branch(location);
    }

    fn emit_callable_branch(&mut self, location: &str) {
        if location.starts_with('x') {
            self.emit(abi::branch_link_register(location));
            return;
        }
        self.emit(abi::branch_link(location));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: location.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
    }

    fn initialize_collection_loop_slots(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        remaining_slot: usize,
    ) {
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
    }

    fn load_collection_loop_item(
        &mut self,
        collection_slot: usize,
        cursor_slot: usize,
        element_type: &str,
    ) -> Result<String, String> {
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64(
            "x11",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x12",
            "x10",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit_load_collection_payload(element_type, "x8", "x11", "x12")
    }

    fn advance_collection_loop(
        &mut self,
        cursor_slot: usize,
        remaining_slot: usize,
        loop_label: &str,
    ) {
        self.emit(abi::load_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::add_immediate("x10", "x10", COLLECTION_ENTRY_SIZE));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), cursor_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::subtract_immediate("x9", "x9", 1));
        self.emit(abi::store_u64("x9", abi::stack_pointer(), remaining_slot));
        self.emit(abi::branch(loop_label));
    }

    fn lower_collection_append(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection append does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("append_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        let insert_slot =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("append_index", 8);
        self.emit(abi::load_u64("x8", abi::stack_pointer(), list_slot));
        self.emit(abi::load_u64("x8", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), index_slot));
        self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )
    }

    fn lower_collection_prepend(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection prepend does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("prepend_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let item = self.lower_value(&args[1])?;
        if item.type_ == list.type_ {
            return Err("native collection prepend expects a single item, not a list".to_string());
        }
        let insert_slot =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        let index_slot = self.allocate_stack_object("prepend_index", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), index_slot));
        self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )
    }

    fn lower_collection_insert(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection insert does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("insert_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection insert index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("insert_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        let item = self.lower_value(&args[2])?;
        if item.type_ == list.type_ {
            return Err("native collection insert expects a single item, not a list".to_string());
        }
        let insert_slot =
            self.collection_argument_as_list_slot(&list.type_, &element_type, item)?;
        self.lower_list_insert_collection(
            list_slot,
            index_slot,
            insert_slot,
            &list.type_,
            &element_type,
        )
    }

    fn collection_argument_as_list_slot(
        &mut self,
        list_type: &str,
        element_type: &str,
        item: ValueResult,
    ) -> Result<usize, String> {
        if item.type_ == list_type {
            let slot = self.allocate_stack_object("collection_insert_list", 8);
            self.emit(abi::store_u64(&item.location, abi::stack_pointer(), slot));
            return Ok(slot);
        }
        if item.type_ != element_type {
            return Err(format!(
                "native collection list item must be {}, got {}",
                element_type, item.type_
            ));
        }
        let item_slot = self.allocate_stack_object("collection_insert_item", 8);
        self.emit(abi::store_u64(
            &item.location,
            abi::stack_pointer(),
            item_slot,
        ));
        let singleton = self.lower_collection_values(
            list_type,
            vec![CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot: item_slot,
                    type_: element_type.to_string(),
                },
            }],
            "singleton list",
        )?;
        let slot = self.allocate_stack_object("collection_insert_singleton", 8);
        self.emit(abi::store_u64(
            &singleton.location,
            abi::stack_pointer(),
            slot,
        ));
        Ok(slot)
    }

    fn lower_collection_remove_at(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let list = self.lower_value(&args[0])?;
        let Some(element_type) = list_element_type(&list.type_) else {
            return Err(format!(
                "native collection removeAt does not accept {}",
                list.type_
            ));
        };
        let list_slot = self.allocate_stack_object("remove_at_list", 8);
        self.emit(abi::store_u64(
            &list.location,
            abi::stack_pointer(),
            list_slot,
        ));
        let index = self.lower_value(&args[1])?;
        if index.type_ != "Integer" {
            return Err(format!(
                "native collection removeAt index must be Integer, got {}",
                index.type_
            ));
        }
        let index_slot = self.allocate_stack_object("remove_at_index", 8);
        self.emit(abi::store_u64(
            &index.location,
            abi::stack_pointer(),
            index_slot,
        ));
        self.lower_list_remove_at(list_slot, index_slot, &list.type_, &element_type)
    }

    fn lower_collection_set(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let collection = self.lower_value(&args[0])?;
        if let Some(element_type) = list_element_type(&collection.type_) {
            let list_slot = self.allocate_stack_object("set_list", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                list_slot,
            ));
            let index = self.lower_value(&args[1])?;
            if index.type_ != "Integer" {
                return Err(format!(
                    "native collection set list index must be Integer, got {}",
                    index.type_
                ));
            }
            let index_slot = self.allocate_stack_object("set_index", 8);
            self.emit(abi::store_u64(
                &index.location,
                abi::stack_pointer(),
                index_slot,
            ));
            let item = self.lower_value(&args[2])?;
            if item.type_ != element_type {
                return Err(format!(
                    "native collection set list item must be {}, got {}",
                    element_type, item.type_
                ));
            }
            let singleton_slot =
                self.collection_argument_as_list_slot(&collection.type_, &element_type, item)?;
            let removed =
                self.lower_list_remove_at(list_slot, index_slot, &collection.type_, &element_type)?;
            let removed_slot = self.allocate_stack_object("set_removed_list", 8);
            self.emit(abi::store_u64(
                &removed.location,
                abi::stack_pointer(),
                removed_slot,
            ));
            return self.lower_list_insert_collection(
                removed_slot,
                index_slot,
                singleton_slot,
                &collection.type_,
                &element_type,
            );
        }

        if let Some((key_type, value_type)) = map_type_parts(&collection.type_) {
            let map_slot = self.allocate_stack_object("set_map", 8);
            self.emit(abi::store_u64(
                &collection.location,
                abi::stack_pointer(),
                map_slot,
            ));
            let key = self.lower_value(&args[1])?;
            if key.type_ != key_type {
                return Err(format!(
                    "native collection set map key must be {}, got {}",
                    key_type, key.type_
                ));
            }
            let key_slot = self.allocate_stack_object("set_map_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(&args[2])?;
            if value.type_ != value_type {
                return Err(format!(
                    "native collection set map value must be {}, got {}",
                    value_type, value.type_
                ));
            }
            let value_slot = self.allocate_stack_object("set_map_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            let without =
                self.lower_map_remove_key(map_slot, key_slot, &collection.type_, &key_type)?;
            let without_slot = self.allocate_stack_object("set_map_without", 8);
            self.emit(abi::store_u64(
                &without.location,
                abi::stack_pointer(),
                without_slot,
            ));
            let singleton = self.lower_collection_values(
                &collection.type_,
                vec![CollectionValueSlot {
                    key: Some(PayloadSlot {
                        slot: key_slot,
                        type_: key_type.clone(),
                    }),
                    value: PayloadSlot {
                        slot: value_slot,
                        type_: value_type,
                    },
                }],
                "singleton map",
            )?;
            let singleton_slot = self.allocate_stack_object("set_map_singleton", 8);
            self.emit(abi::store_u64(
                &singleton.location,
                abi::stack_pointer(),
                singleton_slot,
            ));
            return self.lower_map_concat(without_slot, singleton_slot, &collection.type_);
        }

        Err(format!(
            "native collection set does not accept {} yet",
            collection.type_
        ))
    }

    fn lower_collection_remove_key(&mut self, args: &[NirValue]) -> Result<ValueResult, String> {
        let map = self.lower_value(&args[0])?;
        let Some((key_type, _)) = map_type_parts(&map.type_) else {
            return Err(format!(
                "native collection removeKey does not accept {}",
                map.type_
            ));
        };
        let map_slot = self.allocate_stack_object("remove_key_map", 8);
        self.emit(abi::store_u64(
            &map.location,
            abi::stack_pointer(),
            map_slot,
        ));
        let key = self.lower_value(&args[1])?;
        if key.type_ != key_type {
            return Err(format!(
                "native collection removeKey key must be {}, got {}",
                key_type, key.type_
            ));
        }
        let key_slot = self.allocate_stack_object("remove_key_key", 8);
        self.emit(abi::store_u64(
            &key.location,
            abi::stack_pointer(),
            key_slot,
        ));
        self.lower_map_remove_key(map_slot, key_slot, &map.type_, &key_type)
    }

    fn lower_list_insert_collection(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        insert_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("list_insert_result", 8);
        let valid_start = self.label("list_insert_valid_start");
        let alloc_ok = self.label("list_insert_alloc_ok");
        let invalid = self.label("list_insert_invalid");
        let done = self.label("list_insert_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x10", "x11"));
        self.emit(abi::branch_gt(&invalid));
        self.emit(abi::load_u64("x12", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers("x13", "x11", "x12"));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x15", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers("x15", "x14", "x15"));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x13", "x16"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x17",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", "x13", "x15");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), insert_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x13", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x13", "x16"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));

        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit_copy_collection_entries(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            "x10",
            "list_insert_prefix",
        )?;
        self.emit(abi::add_immediate("x12", "x9", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x9");
        self.emit(abi::load_u64("x14", "x9", COLLECTION_OFFSET_COUNT));
        self.emit_copy_collection_entries(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            "x14",
            "list_insert_inserted",
        )?;
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::subtract_registers("x14", "x14", "x10"));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x15", "x10", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x15"));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit_copy_collection_entries(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            "x14",
            "list_insert_suffix",
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("list update {list_type} over {element_type}"),
        })
    }

    fn lower_list_remove_at(
        &mut self,
        base_slot: usize,
        index_slot: usize,
        list_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(list_type)
            .ok_or_else(|| format!("native code collection type '{list_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("list_remove_result", 8);
        let valid_start = self.label("list_remove_valid_start");
        let alloc_ok = self.label("list_remove_alloc_ok");
        let invalid = self.label("list_remove_invalid");
        let done = self.label("list_remove_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_immediate("x10", "0"));
        self.emit(abi::branch_ge(&valid_start));
        self.emit(abi::branch(&invalid));
        self.emit(abi::label(&valid_start));
        self.emit(abi::compare_registers("x10", "x11"));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x10", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x17"));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64(
            "x15",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::subtract_registers("x15", "x14", "x15"));
        self.emit(abi::subtract_immediate("x13", "x11", 1));
        self.emit(abi::multiply_registers("x17", "x13", "x16"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x17",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", "x13", "x15");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), base_slot));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x13", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x13", "x16"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit_copy_collection_entries(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            "x10",
            "list_remove_prefix",
        )?;
        self.emit(abi::load_u64("x10", abi::stack_pointer(), index_slot));
        self.emit(abi::load_u64("x14", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::subtract_registers("x14", "x14", "x10"));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::add_immediate("x15", "x10", 1));
        self.emit(abi::multiply_registers("x15", "x15", "x16"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_registers("x12", "x12", "x15"));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit_copy_collection_entries(
            "x12",
            "x20",
            "x17",
            "x21",
            "x13",
            "x14",
            "list_remove_suffix",
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: list_type.to_string(),
            location: result,
            text: format!("removeAt({list_type}, Integer) over {element_type}"),
        })
    }

    fn emit_write_list_header_from_registers(
        &mut self,
        layout: &CollectionTypeLayout,
        collection: &str,
        count: &str,
        data_len: &str,
    ) {
        self.emit(abi::move_immediate("x22", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x22", collection, COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x22", collection, COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8(
            "x22",
            collection,
            COLLECTION_OFFSET_VALUE_TYPE,
        ));
        self.emit(abi::move_immediate("x22", "Byte", "1"));
        self.emit(abi::store_u8(
            "x22",
            collection,
            COLLECTION_OFFSET_FLAGS_VERSION,
        ));
        self.emit(abi::store_u64(count, collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64(
            count,
            collection,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::store_u64(
            data_len,
            collection,
            COLLECTION_OFFSET_DATA_LENGTH,
        ));
        self.emit(abi::store_u64(
            data_len,
            collection,
            COLLECTION_OFFSET_DATA_CAPACITY,
        ));
    }

    fn emit_copy_collection_entries(
        &mut self,
        source_entry: &str,
        source_data: &str,
        dest_entry: &str,
        dest_data: &str,
        dest_data_offset: &str,
        count: &str,
        label_prefix: &str,
    ) -> Result<(), String> {
        let loop_label = self.label(&format!("{label_prefix}_loop"));
        let bytes_loop = self.label(&format!("{label_prefix}_bytes"));
        let bytes_done = self.label(&format!("{label_prefix}_bytes_done"));
        let done = self.label(&format!("{label_prefix}_done"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(count, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::move_immediate("x22", "Integer", "0"));
        self.emit(abi::store_u64(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            "x22",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", source_data, "x22"));
        self.emit(abi::add_registers("x25", dest_data, dest_data_offset));
        self.emit(abi::label(&bytes_loop));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&bytes_done));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&bytes_loop));
        self.emit(abi::label(&bytes_done));
        self.emit(abi::load_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            "x23",
        ));
        self.emit(abi::add_immediate(
            source_entry,
            source_entry,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::add_immediate(
            dest_entry,
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(count, count, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        Ok(())
    }

    fn lower_map_concat(
        &mut self,
        left_slot: usize,
        right_slot: usize,
        map_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("map_concat_result", 8);
        let alloc_ok = self.label("map_concat_alloc_ok");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::load_u64("x11", "x9", COLLECTION_OFFSET_COUNT));
        self.emit(abi::add_registers("x12", "x10", "x11"));
        self.emit(abi::load_u64("x13", "x8", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::load_u64("x14", "x9", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::add_registers("x14", "x13", "x14"));
        self.emit(abi::move_immediate(
            "x15",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x16", "x12", "x15"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x16",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x14",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", "x12", "x14");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::load_u64("x12", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x15",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x12", "x15"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit_copy_map_entries("x12", "x20", "x17", "x21", "x13", "x10", "map_concat_left")?;
        self.emit(abi::add_immediate("x12", "x9", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x9");
        self.emit(abi::load_u64("x10", "x9", COLLECTION_OFFSET_COUNT));
        self.emit_copy_map_entries("x12", "x20", "x17", "x21", "x13", "x10", "map_concat_right")?;
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("map concat {map_type}"),
        })
    }

    fn lower_map_remove_key(
        &mut self,
        map_slot: usize,
        key_slot: usize,
        map_type: &str,
        key_type: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(map_type)
            .ok_or_else(|| format!("native code collection type '{map_type}' is not supported"))?;
        for register in ["x20", "x21", "x22", "x23", "x24", "x25"] {
            self.mark_register_used(register);
        }
        let result_slot = self.allocate_stack_object("map_remove_result", 8);
        let scan_loop = self.label("map_remove_scan_loop");
        let scan_keep = self.label("map_remove_scan_keep");
        let scan_next = self.label("map_remove_scan_next");
        let scan_done = self.label("map_remove_scan_done");
        let alloc_ok = self.label("map_remove_alloc_ok");
        let copy_loop = self.label("map_remove_copy_loop");
        let copy_keep = self.label("map_remove_copy_keep");
        let copy_next = self.label("map_remove_copy_next");
        let copy_done = self.label("map_remove_copy_done");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::move_immediate("x14", "Integer", "0"));
        self.emit(abi::move_immediate("x15", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::label(&scan_loop));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_ge(&scan_done));
        self.emit(abi::load_u64(
            "x13",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x16",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "x8", "x13", "x16", "x9", &scan_next, &scan_keep,
        )?;
        self.emit(abi::label(&scan_keep));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::load_u64(
            "x16",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::load_u64(
            "x17",
            "x12",
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x15", "x15", "x16"));
        self.emit(abi::add_registers("x15", "x15", "x17"));
        self.emit(abi::label(&scan_next));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::branch(&scan_loop));
        self.emit(abi::label(&scan_done));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x17", "x14", "x16"));
        self.emit(abi::add_immediate(
            abi::return_register(),
            "x17",
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(
            abi::return_register(),
            abi::return_register(),
            "x15",
        ));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit_write_list_header_from_registers(&layout, "x1", "x14", "x15");

        self.emit(abi::load_u64("x8", abi::stack_pointer(), map_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x10", "x8", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate("x11", "Integer", "0"));
        self.emit(abi::move_immediate("x13", "Integer", "0"));
        self.emit(abi::add_immediate("x12", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::add_immediate("x17", "x1", COLLECTION_HEADER_SIZE));
        self.emit_collection_data_pointer("x20", "x8");
        self.emit(abi::load_u64("x14", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(
            "x16",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x21", "x14", "x16"));
        self.emit(abi::add_registers("x21", "x17", "x21"));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_registers("x11", "x10"));
        self.emit(abi::branch_ge(&copy_done));
        self.emit(abi::load_u64(
            "x14",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x15",
            "x12",
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_matches_value_branch(
            key_type, "x8", "x14", "x15", "x9", &copy_next, &copy_keep,
        )?;
        self.emit(abi::label(&copy_keep));
        self.emit_copy_one_map_entry("x12", "x20", "x17", "x21", "x13");
        self.emit(abi::label(&copy_next));
        self.emit(abi::add_immediate("x12", "x12", COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(ValueResult {
            type_: map_type.to_string(),
            location: result,
            text: format!("removeKey({map_type}, {key_type})"),
        })
    }

    fn emit_copy_map_entries(
        &mut self,
        source_entry: &str,
        source_data: &str,
        dest_entry: &str,
        dest_data: &str,
        dest_data_offset: &str,
        count: &str,
        label_prefix: &str,
    ) -> Result<(), String> {
        let loop_label = self.label(&format!("{label_prefix}_loop"));
        let done = self.label(&format!("{label_prefix}_done"));
        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_immediate(count, "0"));
        self.emit(abi::branch_eq(&done));
        self.emit_copy_one_map_entry(
            source_entry,
            source_data,
            dest_entry,
            dest_data,
            dest_data_offset,
        );
        self.emit(abi::add_immediate(
            source_entry,
            source_entry,
            COLLECTION_ENTRY_SIZE,
        ));
        self.emit(abi::subtract_immediate(count, count, 1));
        self.emit(abi::branch(&loop_label));
        self.emit(abi::label(&done));
        Ok(())
    }

    fn emit_copy_one_map_entry(
        &mut self,
        source_entry: &str,
        source_data: &str,
        dest_entry: &str,
        dest_data: &str,
        dest_data_offset: &str,
    ) {
        let key_loop = self.label("map_entry_key_copy_loop");
        let key_done = self.label("map_entry_key_copy_done");
        let value_loop = self.label("map_entry_value_copy_loop");
        let value_done = self.label("map_entry_value_copy_done");
        self.emit(abi::move_immediate(
            "x22",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            "x22",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_FLAGS,
        ));
        self.emit(abi::load_u64(
            "x22",
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            source_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers("x24", source_data, "x22"));
        self.emit(abi::add_registers("x25", dest_data, dest_data_offset));
        self.emit(abi::label(&key_loop));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&key_done));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&key_loop));
        self.emit(abi::label(&key_done));
        self.emit(abi::load_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            "x23",
        ));

        self.emit(abi::load_u64(
            "x22",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            "x23",
            source_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::store_u64(
            dest_data_offset,
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::store_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers("x24", source_data, "x22"));
        self.emit(abi::add_registers("x25", dest_data, dest_data_offset));
        self.emit(abi::label(&value_loop));
        self.emit(abi::compare_immediate("x23", "0"));
        self.emit(abi::branch_eq(&value_done));
        self.emit(abi::load_u8("x22", "x24", 0));
        self.emit(abi::store_u8("x22", "x25", 0));
        self.emit(abi::add_immediate("x24", "x24", 1));
        self.emit(abi::add_immediate("x25", "x25", 1));
        self.emit(abi::subtract_immediate("x23", "x23", 1));
        self.emit(abi::branch(&value_loop));
        self.emit(abi::label(&value_done));
        self.emit(abi::load_u64(
            "x23",
            dest_entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit(abi::add_registers(
            dest_data_offset,
            dest_data_offset,
            "x23",
        ));
        self.emit(abi::add_immediate(
            dest_entry,
            dest_entry,
            COLLECTION_ENTRY_SIZE,
        ));
    }

    fn lower_list_get(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        collection_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let index = self.allocate_register()?;
        let count = self.allocate_register()?;
        let entry_offset = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let invalid = self.label("list_get_invalid");
        let done = self.label("list_get_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), key_slot));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_lt(&invalid));
        self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&invalid));
        self.emit(abi::move_immediate(
            &entry_offset,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(
            &entry_offset,
            &index,
            &entry_offset,
        ));
        self.emit(abi::add_immediate(
            &entry,
            &collection,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&entry, &entry, &entry_offset));
        self.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        let result = self.emit_load_collection_payload(
            element_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&invalid));
        self.emit_index_out_of_range_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: element_type.to_string(),
            location: result,
            text: format!("get({collection_type}, Integer)"),
        })
    }

    fn lower_map_get(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        collection_type: &str,
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let key = self.allocate_register()?;
        let count = self.allocate_register()?;
        let index = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let key_offset = self.allocate_register()?;
        let key_length = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let loop_label = self.label("map_get_loop");
        let found = self.label("map_get_found");
        let next = self.label("map_get_next");
        let not_found = self.label("map_get_not_found");
        let done = self.label("map_get_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::add_immediate(
            &entry,
            &collection,
            COLLECTION_HEADER_SIZE,
        ));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&not_found));
        self.emit(abi::load_u64(
            &key_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &key_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_match_branch(
            key_type,
            &collection,
            &key_offset,
            &key_length,
            &key,
            &found,
            &next,
        )?;

        self.emit(abi::label(&found));
        self.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        let result = self.emit_load_collection_payload(
            value_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&not_found));
        self.emit_not_found_return()?;
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: value_type.to_string(),
            location: result,
            text: format!("get({collection_type}, {key_type})"),
        })
    }

    fn lower_list_get_or(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        default_slot: usize,
        collection_type: &str,
        element_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let index = self.allocate_register()?;
        let count = self.allocate_register()?;
        let entry_offset = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let use_default = self.label("list_get_or_default");
        let done = self.label("list_get_or_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&index, abi::stack_pointer(), key_slot));
        self.emit(abi::compare_immediate(&index, "0"));
        self.emit(abi::branch_lt(&use_default));
        self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&use_default));
        self.emit(abi::move_immediate(
            &entry_offset,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(
            &entry_offset,
            &index,
            &entry_offset,
        ));
        self.emit(abi::add_immediate(
            &entry,
            &collection,
            COLLECTION_HEADER_SIZE,
        ));
        self.emit(abi::add_registers(&entry, &entry, &entry_offset));
        self.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        let result = self.emit_load_collection_payload(
            element_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));
        self.emit(abi::label(&use_default));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), default_slot));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: element_type.to_string(),
            location: result,
            text: format!("getOr({collection_type}, Integer, {element_type})"),
        })
    }

    fn lower_map_get_or(
        &mut self,
        collection_slot: usize,
        key_slot: usize,
        default_slot: usize,
        collection_type: &str,
        key_type: &str,
        value_type: &str,
    ) -> Result<ValueResult, String> {
        self.reset_temporary_registers();
        let collection = self.allocate_register()?;
        let key = self.allocate_register()?;
        let count = self.allocate_register()?;
        let index = self.allocate_register()?;
        let entry = self.allocate_register()?;
        let key_offset = self.allocate_register()?;
        let key_length = self.allocate_register()?;
        let value_offset = self.allocate_register()?;
        let value_length = self.allocate_register()?;
        let loop_label = self.label("map_get_or_loop");
        let found = self.label("map_get_or_found");
        let next = self.label("map_get_or_next");
        let use_default = self.label("map_get_or_default");
        let done = self.label("map_get_or_done");

        self.emit(abi::load_u64(
            &collection,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64(&key, abi::stack_pointer(), key_slot));
        self.emit(abi::load_u64(&count, &collection, COLLECTION_OFFSET_COUNT));
        self.emit(abi::move_immediate(&index, "Integer", "0"));
        self.emit(abi::add_immediate(
            &entry,
            &collection,
            COLLECTION_HEADER_SIZE,
        ));

        self.emit(abi::label(&loop_label));
        self.emit(abi::compare_registers(&index, &count));
        self.emit(abi::branch_ge(&use_default));
        self.emit(abi::load_u64(
            &key_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
        ));
        self.emit(abi::load_u64(
            &key_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
        ));
        self.emit_collection_payload_match_branch(
            key_type,
            &collection,
            &key_offset,
            &key_length,
            &key,
            &found,
            &next,
        )?;

        self.emit(abi::label(&found));
        self.emit(abi::load_u64(
            &value_offset,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64(
            &value_length,
            &entry,
            COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        let result = self.emit_load_collection_payload(
            value_type,
            &collection,
            &value_offset,
            &value_length,
        )?;
        self.emit(abi::branch(&done));

        self.emit(abi::label(&next));
        self.emit(abi::add_immediate(&entry, &entry, COLLECTION_ENTRY_SIZE));
        self.emit(abi::add_immediate(&index, &index, 1));
        self.emit(abi::branch(&loop_label));

        self.emit(abi::label(&use_default));
        self.emit(abi::load_u64(&result, abi::stack_pointer(), default_slot));
        self.emit(abi::label(&done));

        Ok(ValueResult {
            type_: value_type.to_string(),
            location: result,
            text: format!("getOr({collection_type}, {key_type}, {value_type})"),
        })
    }

    fn lower_len(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        let value = self.lower_value(value)?;
        if value.type_ == "String" {
            let count_slot = self.allocate_stack_object("len_string_count", 8);
            let remaining = self.allocate_register()?;
            let cursor = self.allocate_register()?;
            let byte = self.allocate_register()?;
            let mask = self.allocate_register()?;
            let loop_label = self.label("len_string_loop");
            let continuation_label = self.label("len_string_continuation");
            let next_label = self.label("len_string_next");
            let done_label = self.label("len_string_done");
            self.emit(abi::move_immediate(&byte, "Integer", "0"));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::load_u64(&remaining, &value.location, 0));
            self.emit(abi::add_immediate(&cursor, &value.location, 8));
            self.emit(abi::move_immediate(&mask, "Integer", "192"));
            self.emit(abi::label(&loop_label));
            self.emit(abi::compare_immediate(&remaining, "0"));
            self.emit(abi::branch_eq(&done_label));
            self.emit(abi::load_u8(&byte, &cursor, 0));
            self.emit(abi::and_registers(&byte, &byte, &mask));
            self.emit(abi::compare_immediate(&byte, "128"));
            self.emit(abi::branch_eq(&continuation_label));
            self.emit(abi::load_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::add_immediate(&byte, &byte, 1));
            self.emit(abi::store_u64(&byte, abi::stack_pointer(), count_slot));
            self.emit(abi::branch(&next_label));
            self.emit(abi::label(&continuation_label));
            self.emit(abi::label(&next_label));
            self.emit(abi::add_immediate(&cursor, &cursor, 1));
            self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
            self.emit(abi::branch(&loop_label));
            self.emit(abi::label(&done_label));
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(&register, abi::stack_pointer(), count_slot));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            });
        } else if is_collection_type(&value.type_) {
            let register = self.allocate_register()?;
            self.emit(abi::load_u64(
                &register,
                &value.location,
                COLLECTION_OFFSET_COUNT,
            ));
            return Ok(ValueResult {
                type_: "Integer".to_string(),
                location: register,
                text: format!("len({})", value.text),
            });
        } else {
            return Err(format!(
                "native len does not accept argument type '{}'",
                value.type_
            ));
        }
    }

    fn lower_empty_collection(&mut self, type_: &str) -> Result<ValueResult, String> {
        self.lower_collection_values(type_, Vec::new(), "empty collection")
    }

    fn lower_list_literal(
        &mut self,
        type_: &str,
        values: &[NirValue],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for value in values {
            let value = self.lower_value(value)?;
            let slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(&value.location, abi::stack_pointer(), slot));
            slots.push(CollectionValueSlot {
                key: None,
                value: PayloadSlot {
                    slot,
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "list")
    }

    fn lower_map_literal(
        &mut self,
        type_: &str,
        entries: &[(NirValue, NirValue)],
    ) -> Result<ValueResult, String> {
        let mut slots = Vec::new();
        for (key, value) in entries {
            let key = self.lower_value(key)?;
            let key_slot = self.allocate_stack_object("collection_key", 8);
            self.emit(abi::store_u64(
                &key.location,
                abi::stack_pointer(),
                key_slot,
            ));
            let value = self.lower_value(value)?;
            let value_slot = self.allocate_stack_object("collection_value", 8);
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                value_slot,
            ));
            slots.push(CollectionValueSlot {
                key: Some(PayloadSlot {
                    slot: key_slot,
                    type_: key.type_,
                }),
                value: PayloadSlot {
                    slot: value_slot,
                    type_: value.type_,
                },
            });
        }
        self.lower_collection_values(type_, slots, "map")
    }

    fn lower_collection_values(
        &mut self,
        type_: &str,
        slots: Vec<CollectionValueSlot>,
        label: &str,
    ) -> Result<ValueResult, String> {
        let layout = CollectionTypeLayout::from_type(type_)
            .ok_or_else(|| format!("native code collection type '{type_}' is not supported"))?;
        let count = slots.len();
        let data_len_slot = self.allocate_stack_object("collection_data_len", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_len_slot));
        for slot in &slots {
            if let Some(key) = &slot.key {
                self.emit_add_payload_length(data_len_slot, key)?;
            }
            self.emit_add_payload_length(data_len_slot, &slot.value)?;
        }

        let collection_slot = self.allocate_stack_object("collection_literal", 8);
        let alloc_ok = self.label("collection_alloc_ok");
        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_len_slot));
        self.emit(abi::move_immediate(
            "x9",
            "Integer",
            &(COLLECTION_HEADER_SIZE + count * COLLECTION_ENTRY_SIZE).to_string(),
        ));
        self.emit(abi::add_registers(abi::return_register(), "x8", "x9"));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), collection_slot));

        self.emit_write_collection_header(&layout, count, data_len_slot);

        let data_offset_slot = self.allocate_stack_object("collection_data_offset", 8);
        self.emit(abi::move_immediate("x8", "Integer", "0"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_offset_slot));

        for (index, slot) in slots.iter().enumerate() {
            self.emit_write_collection_entry(collection_slot, index, slot, data_offset_slot)?;
        }
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &register,
            abi::stack_pointer(),
            collection_slot,
        ));
        Ok(ValueResult {
            type_: type_.to_string(),
            location: register,
            text: format!("{label} {type_}"),
        })
    }

    fn emit_write_collection_header(
        &mut self,
        layout: &CollectionTypeLayout,
        count: usize,
        data_len_slot: usize,
    ) {
        self.emit(abi::move_immediate("x8", "Byte", &layout.kind.to_string()));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_KIND));
        self.emit(abi::move_immediate(
            "x8",
            "Byte",
            &layout.key_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_KEY_TYPE));
        self.emit(abi::move_immediate(
            "x8",
            "Byte",
            &layout.value_type_code.to_string(),
        ));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_VALUE_TYPE));
        self.emit(abi::move_immediate("x8", "Byte", "1"));
        self.emit(abi::store_u8("x8", "x1", COLLECTION_OFFSET_FLAGS_VERSION));
        self.emit(abi::move_immediate("x8", "Integer", &count.to_string()));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_COUNT));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_len_slot));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_LENGTH));
        self.emit(abi::store_u64("x8", "x1", COLLECTION_OFFSET_DATA_CAPACITY));
    }

    fn emit_write_collection_entry(
        &mut self,
        collection_slot: usize,
        index: usize,
        slot: &CollectionValueSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        let entry_offset = COLLECTION_HEADER_SIZE + index * COLLECTION_ENTRY_SIZE;
        let key_len_slot = if let Some(key) = &slot.key {
            Some(self.emit_payload_length_to_stack(key, "collection_key_len")?)
        } else {
            None
        };
        let value_len_slot =
            self.emit_payload_length_to_stack(&slot.value, "collection_value_len")?;
        let collection_register = "x8";
        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));

        self.emit(abi::move_immediate(
            "x9",
            "Byte",
            &COLLECTION_ENTRY_FLAG_USED.to_string(),
        ));
        self.emit(abi::store_u8(
            "x9",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_FLAGS,
        ));

        if let Some(key_len_slot) = key_len_slot {
            self.emit(abi::load_u64("x10", abi::stack_pointer(), data_offset_slot));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::load_u64("x11", abi::stack_pointer(), key_len_slot));
            self.emit(abi::store_u64(
                "x11",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
            self.emit_copy_payload_to_collection(
                collection_slot,
                key_len_slot,
                slot.key.as_ref().unwrap(),
                data_offset_slot,
            )?;
        } else {
            self.emit(abi::move_immediate("x10", "Integer", "0"));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_OFFSET,
            ));
            self.emit(abi::store_u64(
                "x10",
                collection_register,
                entry_offset + COLLECTION_ENTRY_OFFSET_KEY_LENGTH,
            ));
        }

        self.emit(abi::load_u64(
            collection_register,
            abi::stack_pointer(),
            collection_slot,
        ));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::store_u64(
            "x10",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_OFFSET,
        ));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), value_len_slot));
        self.emit(abi::store_u64(
            "x11",
            collection_register,
            entry_offset + COLLECTION_ENTRY_OFFSET_VALUE_LENGTH,
        ));
        self.emit_copy_payload_to_collection(
            collection_slot,
            value_len_slot,
            &slot.value,
            data_offset_slot,
        )?;
        Ok(())
    }

    fn emit_add_payload_length(
        &mut self,
        total_slot: usize,
        payload: &PayloadSlot,
    ) -> Result<(), String> {
        let len_slot = self.emit_payload_length_to_stack(payload, "collection_payload_len")?;
        self.emit(abi::load_u64("x8", abi::stack_pointer(), total_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), total_slot));
        Ok(())
    }

    fn emit_payload_length_to_stack(
        &mut self,
        payload: &PayloadSlot,
        label: &str,
    ) -> Result<usize, String> {
        let len_slot = self.allocate_stack_object(label, 8);
        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::move_immediate("x8", "Integer", "1"));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::move_immediate("x8", "Integer", "8"));
            }
            "String" => {
                self.emit(abi::load_u64("x8", abi::stack_pointer(), payload.slot));
                self.emit(abi::load_u64("x8", "x8", 0));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        self.emit(abi::store_u64("x8", abi::stack_pointer(), len_slot));
        Ok(len_slot)
    }

    fn emit_copy_payload_to_collection(
        &mut self,
        collection_slot: usize,
        len_slot: usize,
        payload: &PayloadSlot,
        data_offset_slot: usize,
    ) -> Result<(), String> {
        self.emit(abi::load_u64("x8", abi::stack_pointer(), collection_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::add_immediate("x10", "x8", COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64("x11", "x8", COLLECTION_OFFSET_CAPACITY));
        self.emit(abi::move_immediate(
            "x12",
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers("x11", "x11", "x12"));
        self.emit(abi::add_registers("x10", "x10", "x11"));
        self.emit(abi::add_registers("x10", "x10", "x9"));

        match payload.type_.as_str() {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u8("x12", "x10", 0));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::store_u64("x12", "x10", 0));
            }
            "String" => {
                let loop_label = self.label("collection_copy_string_loop");
                let done_label = self.label("collection_copy_string_done");
                self.emit(abi::load_u64("x12", abi::stack_pointer(), payload.slot));
                self.emit(abi::add_immediate("x12", "x12", 8));
                self.emit(abi::load_u64("x13", abi::stack_pointer(), len_slot));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x13", "0"));
                self.emit(abi::branch_eq(&done_label));
                self.emit(abi::load_u8("x14", "x12", 0));
                self.emit(abi::store_u8("x14", "x10", 0));
                self.emit(abi::add_immediate("x12", "x12", 1));
                self.emit(abi::add_immediate("x10", "x10", 1));
                self.emit(abi::subtract_immediate("x13", "x13", 1));
                self.emit(abi::branch(&loop_label));
                self.emit(abi::label(&done_label));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }

        self.emit(abi::load_u64("x8", abi::stack_pointer(), data_offset_slot));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), len_slot));
        self.emit(abi::add_registers("x8", "x8", "x9"));
        self.emit(abi::store_u64("x8", abi::stack_pointer(), data_offset_slot));
        Ok(())
    }

    fn emit_collection_data_pointer(&mut self, dst: &str, collection: &str) {
        let capacity = "x6";
        let entry_size = "x7";
        self.emit(abi::move_register(capacity, collection));
        self.emit(abi::add_immediate(dst, collection, COLLECTION_HEADER_SIZE));
        self.emit(abi::load_u64(
            capacity,
            capacity,
            COLLECTION_OFFSET_CAPACITY,
        ));
        self.emit(abi::move_immediate(
            entry_size,
            "Integer",
            &COLLECTION_ENTRY_SIZE.to_string(),
        ));
        self.emit(abi::multiply_registers(capacity, capacity, entry_size));
        self.emit(abi::add_registers(dst, dst, capacity));
    }

    fn emit_load_collection_payload(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
    ) -> Result<String, String> {
        let collection_input = "x3";
        let offset_input = "x4";
        let length_input = "x5";
        self.emit(abi::move_register(collection_input, collection));
        self.emit(abi::move_register(offset_input, offset));
        self.emit(abi::move_register(length_input, length));
        let data = self.allocate_register()?;
        self.emit_collection_data_pointer(&data, collection_input);
        self.emit(abi::add_registers(&data, &data, offset_input));
        match type_ {
            "Boolean" | "Byte" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u8(&result, &data, 0));
                Ok(result)
            }
            "Integer" | "Float" | "Fixed" => {
                let result = self.allocate_register()?;
                self.emit(abi::load_u64(&result, &data, 0));
                Ok(result)
            }
            "String" => self.emit_materialize_string_from_bytes(&data, length_input),
            other => Err(format!(
                "native collection packed payload does not support type '{other}'"
            )),
        }
    }

    fn emit_materialize_string_from_bytes(
        &mut self,
        source: &str,
        length: &str,
    ) -> Result<String, String> {
        let source_slot = self.allocate_stack_object("collection_string_source", 8);
        let length_slot = self.allocate_stack_object("collection_string_length", 8);
        let result_slot = self.allocate_stack_object("collection_string_result", 8);
        let alloc_ok = self.label("collection_string_alloc_ok");
        let copy_loop = self.label("collection_string_copy_loop");
        let copy_done = self.label("collection_string_copy_done");

        self.emit(abi::store_u64(source, abi::stack_pointer(), source_slot));
        self.emit(abi::store_u64(length, abi::stack_pointer(), length_slot));
        self.emit(abi::add_immediate(abi::return_register(), length, 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::store_u64("x1", abi::stack_pointer(), result_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), length_slot));
        self.emit(abi::store_u64("x12", "x1", 0));
        self.emit(abi::add_immediate("x13", "x1", 8));
        self.emit(abi::load_u64("x14", abi::stack_pointer(), source_slot));
        self.emit(abi::label(&copy_loop));
        self.emit(abi::compare_immediate("x12", "0"));
        self.emit(abi::branch_eq(&copy_done));
        self.emit(abi::load_u8("x15", "x14", 0));
        self.emit(abi::store_u8("x15", "x13", 0));
        self.emit(abi::add_immediate("x14", "x14", 1));
        self.emit(abi::add_immediate("x13", "x13", 1));
        self.emit(abi::subtract_immediate("x12", "x12", 1));
        self.emit(abi::branch(&copy_loop));
        self.emit(abi::label(&copy_done));
        self.emit(abi::move_immediate("x15", "Integer", "0"));
        self.emit(abi::store_u8("x15", "x13", 0));
        let result = self.allocate_register()?;
        self.emit(abi::load_u64(&result, abi::stack_pointer(), result_slot));
        Ok(result)
    }

    fn emit_collection_payload_match_branch(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
        value: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        let data = self.allocate_register()?;
        self.emit_collection_data_pointer(&data, collection);
        self.emit(abi::add_registers(&data, &data, offset));
        match type_ {
            "Boolean" | "Byte" => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u8(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                let candidate = self.allocate_register()?;
                self.emit(abi::load_u64(&candidate, &data, 0));
                self.emit(abi::compare_registers(&candidate, value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let value_len = self.allocate_register()?;
                let value_cursor = self.allocate_register()?;
                let remaining = self.allocate_register()?;
                let packed_byte = self.allocate_register()?;
                let value_byte = self.allocate_register()?;
                let loop_label = self.label("collection_string_match_loop");
                self.emit(abi::load_u64(&value_len, value, 0));
                self.emit(abi::compare_registers(length, &value_len));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&value_cursor, value, 8));
                self.emit(abi::move_register(&remaining, length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate(&remaining, "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8(&packed_byte, &data, 0));
                self.emit(abi::load_u8(&value_byte, &value_cursor, 0));
                self.emit(abi::compare_registers(&packed_byte, &value_byte));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate(&data, &data, 1));
                self.emit(abi::add_immediate(&value_cursor, &value_cursor, 1));
                self.emit(abi::subtract_immediate(&remaining, &remaining, 1));
                self.emit(abi::branch(&loop_label));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    fn emit_collection_payload_matches_value_branch(
        &mut self,
        type_: &str,
        collection: &str,
        offset: &str,
        length: &str,
        value: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit(abi::move_register("x2", collection));
        self.emit(abi::move_register("x3", offset));
        self.emit_collection_data_pointer("x2", "x2");
        self.emit(abi::add_registers("x2", "x2", "x3"));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::compare_registers("x6", value));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_string_value_match_loop");
                self.emit(abi::load_u64("x3", value, 0));
                self.emit(abi::compare_registers(length, "x3"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x4", value, 8));
                self.emit(abi::move_register("x5", length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    fn emit_collection_payloads_match_branch(
        &mut self,
        type_: &str,
        left_collection: &str,
        left_offset: &str,
        left_length: &str,
        right_collection: &str,
        right_offset: &str,
        right_length: &str,
        equal_label: &str,
        not_equal_label: &str,
    ) -> Result<(), String> {
        self.emit(abi::move_register("x2", left_collection));
        self.emit(abi::move_register("x3", left_offset));
        self.emit(abi::move_register("x4", right_collection));
        self.emit(abi::move_register("x5", right_offset));
        self.emit_collection_data_pointer("x2", "x2");
        self.emit(abi::add_registers("x2", "x2", "x3"));
        self.emit_collection_data_pointer("x4", "x4");
        self.emit(abi::add_registers("x4", "x4", "x5"));
        match type_ {
            "Boolean" | "Byte" => {
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "Integer" | "Float" | "Fixed" => {
                self.emit(abi::load_u64("x6", "x2", 0));
                self.emit(abi::load_u64("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::branch(not_equal_label));
            }
            "String" => {
                let loop_label = self.label("collection_payload_string_match_loop");
                self.emit(abi::compare_registers(left_length, right_length));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::move_register("x5", left_length));
                self.emit(abi::label(&loop_label));
                self.emit(abi::compare_immediate("x5", "0"));
                self.emit(abi::branch_eq(equal_label));
                self.emit(abi::load_u8("x6", "x2", 0));
                self.emit(abi::load_u8("x7", "x4", 0));
                self.emit(abi::compare_registers("x6", "x7"));
                self.emit(abi::branch_ne(not_equal_label));
                self.emit(abi::add_immediate("x2", "x2", 1));
                self.emit(abi::add_immediate("x4", "x4", 1));
                self.emit(abi::subtract_immediate("x5", "x5", 1));
                self.emit(abi::branch(&loop_label));
            }
            other => {
                return Err(format!(
                    "native collection packed payload does not support type '{other}'"
                ));
            }
        }
        Ok(())
    }

    fn load_string_constant(&mut self, value: &str) -> Result<String, String> {
        let register = self.allocate_register()?;
        self.emit_load_string_constant(&register, value)?;
        Ok(register)
    }

    fn lower_field_access(
        &mut self,
        target: &NirValue,
        member: &str,
    ) -> Result<ValueResult, String> {
        let target_value = self.lower_value(target)?;
        let (field_index, field_type, payload_offset) =
            if let Some((key_type, value_type)) = parse_map_entry_type(&target_value.type_) {
                match member {
                    "key" => (0, key_type, 0),
                    "value" => (1, value_type, 0),
                    _ => {
                        return Err(format!(
                            "native code map entry '{}' has no field '{}'",
                            target_value.type_, member
                        ));
                    }
                }
            } else if let Some(fields) = self.type_model.record_fields.get(&target_value.type_) {
                let Some((index, (_, field_type))) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == member)
                else {
                    return Err(format!(
                        "native code record '{}' has no field '{}'",
                        target_value.type_, member
                    ));
                };
                (index, field_type.clone(), 0)
            } else if let Some(fields) = self
                .type_model
                .union_variant_fields
                .get(&target_value.type_)
            {
                let Some((index, (_, field_type))) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, (name, _))| name == member)
                else {
                    return Err(format!(
                        "native code variant '{}' has no field '{}'",
                        target_value.type_, member
                    ));
                };
                (index, field_type.clone(), 8)
            } else if self.type_model.union_names.contains(&target_value.type_) {
                let matches = self
                    .type_model
                    .union_variant_fields
                    .values()
                    .filter_map(|fields| {
                        fields
                            .iter()
                            .enumerate()
                            .find(|(_, (name, _))| name == member)
                            .map(|(index, (_, field_type))| (index, field_type.clone()))
                    })
                    .collect::<Vec<_>>();
                let Some((index, field_type)) = matches.first().cloned() else {
                    return Err(format!(
                        "native code union '{}' has no payload field '{}'",
                        target_value.type_, member
                    ));
                };
                (index, field_type, 8)
            } else {
                return Err(format!(
                    "native code field access target '{}' is not a record or variant",
                    target_value.type_
                ));
            };
        let register = self.allocate_register()?;
        self.emit(abi::load_u64(
            &register,
            &target_value.location,
            payload_offset + 8 * field_index,
        ));
        Ok(ValueResult {
            type_: field_type,
            location: register,
            text: format!("{}.{}", target_value.text, member),
        })
    }

    fn lower_with_update(
        &mut self,
        type_: &str,
        target: &NirValue,
        updates: &[NirRecordUpdate],
    ) -> Result<ValueResult, String> {
        let fields = self
            .type_model
            .record_fields
            .get(type_)
            .cloned()
            .ok_or_else(|| format!("native code WITH target '{type_}' is not a record"))?;
        let target_value = self.lower_value(target)?;
        let register = self.allocate_register()?;
        let object_offset = self.allocate_stack_object(type_, 8 * fields.len());
        for (index, _) in fields.iter().enumerate() {
            let scratch = self.allocate_register()?;
            self.emit(abi::load_u64(&scratch, &target_value.location, 8 * index));
            self.emit(abi::store_u64(
                &scratch,
                abi::stack_pointer(),
                object_offset + 8 * index,
            ));
        }
        for update in updates {
            let Some(index) = fields
                .iter()
                .position(|(field_name, _)| field_name == &update.field)
            else {
                return Err(format!(
                    "native code WITH update references unknown field '{}'",
                    update.field
                ));
            };
            let value = self.lower_value(&update.value)?;
            self.emit(abi::store_u64(
                &value.location,
                abi::stack_pointer(),
                object_offset + 8 * index,
            ));
        }
        self.emit(abi::add_immediate(
            &register,
            abi::stack_pointer(),
            object_offset,
        ));
        Ok(ValueResult {
            type_: type_.to_string(),
            location: register,
            text: format!("with {}", target_value.text),
        })
    }

    fn lower_string_concat(
        &mut self,
        left: &NirValue,
        right: &NirValue,
    ) -> Result<ValueResult, String> {
        let left = self.lower_value(left)?;
        if left.type_ != "String" {
            return Err(format!(
                "native string concat left operand must be String, got {}",
                left.type_
            ));
        }
        let left_slot = self.allocate_stack_object("concat_left", 8);
        self.emit(abi::store_u64(
            &left.location,
            abi::stack_pointer(),
            left_slot,
        ));
        let right = self.lower_value(right)?;
        if right.type_ != "String" {
            return Err(format!(
                "native string concat right operand must be String, got {}",
                right.type_
            ));
        }
        let right_slot = self.allocate_stack_object("concat_right", 8);
        self.emit(abi::store_u64(
            &right.location,
            abi::stack_pointer(),
            right_slot,
        ));
        let total_slot = self.allocate_stack_object("concat_total", 8);

        let alloc_ok = self.label("string_concat_alloc_ok");
        let left_loop = self.label("string_concat_left_loop");
        let left_done = self.label("string_concat_left_done");
        let right_loop = self.label("string_concat_right_loop");
        let right_done = self.label("string_concat_right_done");

        self.emit(abi::load_u64("x11", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x12", abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate("x13", "x11", 8));
        self.emit(abi::add_immediate("x15", "x12", 8));
        self.emit(abi::load_u64("x8", "x11", 0));
        self.emit(abi::load_u64("x9", "x12", 0));
        self.emit(abi::add_registers("x10", "x8", "x9"));
        self.emit(abi::store_u64("x10", abi::stack_pointer(), total_slot));
        self.emit(abi::add_immediate(abi::return_register(), "x10", 9));
        self.emit(abi::move_immediate("x1", "Integer", "8"));
        self.emit(abi::branch_link(ARENA_ALLOC_SYMBOL));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: ARENA_ALLOC_SYMBOL.to_string(),
            kind: "branch26".to_string(),
            binding: "internal".to_string(),
            library: None,
        });
        self.emit(abi::compare_immediate(
            abi::return_register(),
            RESULT_OK_TAG,
        ));
        self.emit(abi::branch_eq(&alloc_ok));
        self.emit_allocation_error_return()?;
        self.emit(abi::label(&alloc_ok));
        self.emit(abi::load_u64("x11", abi::stack_pointer(), left_slot));
        self.emit(abi::load_u64("x15", abi::stack_pointer(), right_slot));
        self.emit(abi::add_immediate("x15", "x15", 8));
        self.emit(abi::load_u64("x8", "x11", 0));
        self.emit(abi::add_immediate("x11", "x11", 8));
        self.emit(abi::load_u64("x9", abi::stack_pointer(), right_slot));
        self.emit(abi::load_u64("x9", "x9", 0));
        self.emit(abi::load_u64("x10", abi::stack_pointer(), total_slot));
        self.emit(abi::store_u64("x10", "x1", 0));
        self.emit(abi::add_immediate("x12", "x1", 8));
        self.emit(abi::move_register("x14", "x8"));
        self.emit(abi::label(&left_loop));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&left_done));
        self.emit(abi::load_u8("x16", "x11", 0));
        self.emit(abi::store_u8("x16", "x12", 0));
        self.emit(abi::add_immediate("x11", "x11", 1));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::branch(&left_loop));
        self.emit(abi::label(&left_done));
        self.emit(abi::move_register("x14", "x9"));
        self.emit(abi::label(&right_loop));
        self.emit(abi::compare_immediate("x14", "0"));
        self.emit(abi::branch_eq(&right_done));
        self.emit(abi::load_u8("x16", "x15", 0));
        self.emit(abi::store_u8("x16", "x12", 0));
        self.emit(abi::add_immediate("x15", "x15", 1));
        self.emit(abi::add_immediate("x12", "x12", 1));
        self.emit(abi::subtract_immediate("x14", "x14", 1));
        self.emit(abi::branch(&right_loop));
        self.emit(abi::label(&right_done));
        self.emit(abi::move_immediate("x16", "Integer", "0"));
        self.emit(abi::store_u8("x16", "x12", 0));

        Ok(ValueResult {
            type_: "String".to_string(),
            location: "x1".to_string(),
            text: format!("({} & {})", left.text, right.text),
        })
    }

    fn emit_load_string_constant(&mut self, register: &str, value: &str) -> Result<(), String> {
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        self.emit(abi::load_page_address(register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(register, register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        Ok(())
    }

    fn local_constant_value(&self, value: &NirValue) -> Option<NirValue> {
        match value {
            NirValue::Const { .. } => Some(value.clone()),
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.clone()),
            NirValue::Call { target, args } if target == "toString" && args.len() == 1 => self
                .static_primitive_text(&args[0])
                .map(|value| NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }),
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
                    .map(|value| NirValue::Const {
                        type_: "String".to_string(),
                        value,
                    })
            }
            NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                self.static_type_name(&args[0])
                    .map(|value| NirValue::Const {
                        type_: "String".to_string(),
                        value,
                    })
            }
            NirValue::Binary { op, .. } if op == "&" => {
                self.static_string_value(value)
                    .map(|value| NirValue::Const {
                        type_: "String".to_string(),
                        value,
                    })
            }
            _ => None,
        }
    }

    fn static_string_value(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_string_value(constant)),
            NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
                self.static_primitive_text(&args[0])
            }
            NirValue::RuntimeCall { target, args, .. }
                if target == "toString" && args.len() == 1 =>
            {
                self.static_primitive_text(&args[0])
            }
            NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. }
                if target == "typeName" && args.len() == 1 =>
            {
                self.static_type_name(&args[0])
            }
            NirValue::Binary { op, left, right } if op == "&" => {
                let left = self.static_string_value(left)?;
                let right = self.static_string_value(right)?;
                Some(format!("{left}{right}"))
            }
            _ => None,
        }
    }

    fn static_primitive_text(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, value } => match type_.as_str() {
                "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
                "Boolean" => match value.as_str() {
                    "true" => Some("TRUE".to_string()),
                    "false" => Some("FALSE".to_string()),
                    _ => None,
                },
                _ => None,
            },
            NirValue::Local(name) => self
                .locals
                .get(name)
                .and_then(|local| local.constant.as_ref())
                .and_then(|constant| self.static_primitive_text(constant)),
            _ => None,
        }
    }

    fn static_type_name(&self, value: &NirValue) -> Option<String> {
        match value {
            NirValue::Const { type_, .. } => Some(type_.clone()),
            NirValue::Local(name) => self.locals.get(name).map(|local| local.type_.clone()),
            NirValue::FunctionRef { type_, .. }
            | NirValue::Constructor { type_, .. }
            | NirValue::WithUpdate { type_, .. }
            | NirValue::ListLiteral { type_, .. }
            | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
            NirValue::Call { target, .. } | NirValue::RuntimeCall { target, .. } => {
                match target.as_str() {
                    "replace" | "typeName" | "toString" => Some("String".to_string()),
                    "find" | "len" | "toInt" => Some("Integer".to_string()),
                    "mid" => Some("String".to_string()),
                    "toFloat" => Some("Float".to_string()),
                    "toFixed" => Some("Fixed".to_string()),
                    "toByte" => Some("Byte".to_string()),
                    "isNumeric" => Some("Boolean".to_string()),
                    _ => None,
                }
            }
            NirValue::Binary { op, left, right } => {
                if matches!(
                    op.as_str(),
                    "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
                ) {
                    return Some("Boolean".to_string());
                }
                if op == "&" {
                    return Some("String".to_string());
                }
                let left = self.static_type_name(left)?;
                let right = self.static_type_name(right)?;
                Some(numeric_binary_result_type(op, &left, &right).to_string())
            }
            NirValue::Unary { op, operand } => {
                if op == "NOT" {
                    Some("Boolean".to_string())
                } else {
                    self.static_type_name(operand)
                }
            }
            NirValue::MemberAccess { target, member } => {
                let target_type = self.static_type_name(target)?;
                let (key_type, value_type) = parse_map_entry_type(&target_type)?;
                match member.as_str() {
                    "key" => Some(key_type),
                    "value" => Some(value_type),
                    _ => None,
                }
            }
        }
    }

    fn lower_match_compare(
        &mut self,
        matched: &ValueResult,
        pattern: &NirValue,
        label: &str,
    ) -> Result<(), String> {
        match pattern {
            NirValue::MemberAccess { target, member } => {
                let NirValue::Local(type_name) = target.as_ref() else {
                    return Err("native code enum match pattern must name enum type".to_string());
                };
                let ordinal = self
                    .type_model
                    .enum_members
                    .get(&(type_name.clone(), member.clone()))
                    .copied()
                    .ok_or_else(|| {
                        format!("native code enum member '{type_name}.{member}' does not resolve")
                    })?;
                self.emit(abi::compare_immediate(
                    &matched.location,
                    &ordinal.to_string(),
                ));
                self.emit(abi::branch_eq(label));
            }
            NirValue::Local(variant) if self.type_model.union_variants.contains_key(variant) => {
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(variant)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{variant}' does not resolve")
                    })?;
                let tag_register = self.allocate_register()?;
                self.emit(abi::load_u64(&tag_register, &matched.location, 0));
                self.emit(abi::compare_immediate(&tag_register, &tag.to_string()));
                self.emit(abi::branch_eq(label));
            }
            _ => {
                let _ = (matched, pattern, label);
                return Err(
                    "native code plan does not lower non-enum/non-union match comparisons yet"
                        .to_string(),
                );
            }
        }
        Ok(())
    }

    fn emit_call(
        &mut self,
        target: &str,
        symbol: &str,
        args: &[NirValue],
        return_type: Option<&str>,
    ) -> Result<ValueResult, String> {
        let arg_values = args
            .iter()
            .map(|arg| self.lower_value(arg))
            .collect::<Result<Vec<_>, _>>()?;
        for (index, arg) in arg_values.iter().enumerate() {
            self.emit(abi::move_register(
                &abi::argument_register(index)?,
                &arg.location,
            ));
        }
        self.emit(abi::branch_link(symbol));
        let (binding, library) = if let Some(library) = self.platform_imports.get(symbol) {
            ("external".to_string(), Some(library.clone()))
        } else {
            ("internal".to_string(), None)
        };
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.to_string(),
            kind: "branch26".to_string(),
            binding,
            library,
        });
        let result_type = return_type
            .map(|type_| type_.to_string())
            .or_else(|| {
                self.functions
                    .get(target)
                    .map(|function| function.returns.clone())
            })
            .unwrap_or_else(|| "Unknown".to_string());
        if result_type == "Nothing" {
            return Ok(ValueResult {
                type_: result_type,
                location: "void".to_string(),
                text: format!("call {target}({})", join_texts(&arg_values)),
            });
        }
        if return_type.is_none() {
            let ok_label = self.label("call_ok");
            self.emit(abi::compare_immediate(RESULT_TAG_REGISTER, RESULT_OK_TAG));
            self.emit(abi::branch_eq(&ok_label));
            self.emit(abi::return_());
            self.emit(abi::label(&ok_label));
        }
        let register = self.allocate_register()?;
        self.emit(abi::move_register(&register, RESULT_VALUE_REGISTER));
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    fn allocate_register(&mut self) -> Result<String, String> {
        let register = abi::temporary_register(self.next_register)?;
        self.next_register += 1;
        self.mark_register_used(&register);
        Ok(register)
    }

    fn mark_register_used(&mut self, register: &str) {
        if abi::is_callee_saved(register)
            && !self.used_callee_saved.iter().any(|saved| saved == register)
        {
            self.used_callee_saved.push(register.to_string());
        }
    }

    fn reset_temporary_registers(&mut self) {
        self.next_register = 8;
    }

    fn local_constants(&self) -> HashMap<String, Option<NirValue>> {
        self.locals
            .iter()
            .map(|(name, local)| (name.clone(), local.constant.clone()))
            .collect()
    }

    fn restore_local_constants(&mut self, constants: &HashMap<String, Option<NirValue>>) {
        for (name, local) in &mut self.locals {
            local.constant = constants.get(name).cloned().unwrap_or(None);
        }
    }

    fn clear_local_constants(&mut self) {
        for local in self.locals.values_mut() {
            local.constant = None;
        }
    }

    fn allocate_stack_object(&mut self, name: &str, size: usize) -> usize {
        let offset = self.stack_size;
        let size = align(size, 8);
        self.stack_size += size;
        self.stack_slots.push(CodeStackSlot {
            name: format!("{name}_{}", self.stack_slots.len()),
            type_: name.to_string(),
            offset: offset as i32,
        });
        offset
    }

    fn label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}_{}", self.next_label);
        self.next_label += 1;
        label
    }

    fn emit(&mut self, instruction: CodeInstruction) {
        self.instructions.push(instruction);
    }

    fn emit_overflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE)
    }

    fn emit_underflow_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_UNDERFLOW_CODE, ERR_UNDERFLOW_MESSAGE)
    }

    fn emit_invalid_argument_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INVALID_ARGUMENT_CODE, ERR_INVALID_ARGUMENT_MESSAGE)
    }

    fn emit_invalid_format_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INVALID_FORMAT_CODE, ERR_INVALID_FORMAT_MESSAGE)
    }

    fn emit_allocation_error_return(&mut self) -> Result<(), String> {
        self.emit_error_register_return(RESULT_TAG_REGISTER, ERR_ALLOCATION_MESSAGE)
    }

    fn emit_index_out_of_range_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_INDEX_OUT_OF_RANGE_CODE, ERR_INDEX_OUT_OF_RANGE_MESSAGE)
    }

    fn emit_not_found_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_NOT_FOUND_CODE, ERR_NOT_FOUND_MESSAGE)
    }

    fn emit_encoding_error_return(&mut self) -> Result<(), String> {
        self.emit_error_code_return(ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE)
    }

    fn emit_error_code_return(&mut self, code: &str, message: &str) -> Result<(), String> {
        let code_register = self.allocate_register()?;
        self.emit(abi::move_immediate(&code_register, "Integer", code));
        self.emit_error_register_return(&code_register, message)
    }

    fn emit_error_register_return(
        &mut self,
        code_register: &str,
        message: &str,
    ) -> Result<(), String> {
        let message_register = self.load_string_address(message)?;
        self.emit(abi::move_register(RESULT_VALUE_REGISTER, code_register));
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_MESSAGE_REGISTER,
            &message_register,
        ));
        self.emit(abi::return_());
        Ok(())
    }

    fn emit_error_return(&mut self, error: &NirValue) -> Result<(), String> {
        let NirValue::Constructor { type_, args } = error else {
            return Err("native code fail expects Error constructor".to_string());
        };
        if type_ != "Error" || args.len() != 2 {
            return Err("native code fail expects Error[code, message]".to_string());
        }
        let code = integer_constant_value(&args[0])
            .ok_or_else(|| "native code fail expects constant Error code".to_string())?;
        let message = string_constant_value(&args[1])
            .ok_or_else(|| "native code fail expects constant Error message".to_string())?;
        self.emit_error_code_return(&(code as u64).to_string(), message)
    }

    fn load_string_address(&mut self, value: &str) -> Result<String, String> {
        let symbol = self
            .string_symbols
            .get(value)
            .ok_or_else(|| format!("native code string literal '{value}' has no data object"))?
            .clone();
        let register = self.allocate_register()?;
        self.emit(abi::load_page_address(&register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        self.emit(abi::add_page_offset(&register, &register, &symbol));
        self.relocations.push(CodeRelocation {
            from: self.current_symbol.clone(),
            to: symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        });
        Ok(register)
    }

    fn current_block_returns(&self) -> bool {
        self.instructions
            .iter()
            .rev()
            .find(|instruction| instruction.op != CodeOp::Label)
            .is_some_and(|instruction| instruction.op == CodeOp::Ret)
    }
}

impl CodeInstruction {
    pub(crate) fn new(op: &str) -> Self {
        Self {
            op: CodeOp::from_mnemonic(op).unwrap_or_else(|err| panic!("{err}")),
            fields: Vec::new(),
        }
    }

    pub(crate) fn field(mut self, name: &'static str, value: &str) -> Self {
        self.fields.push((name, value.to_string()));
        self
    }

    fn validate(&self) -> Result<(), String> {
        let required: &[&str] = match self.op {
            CodeOp::Label => &["name"],
            CodeOp::Mov => &["dst", "src"],
            CodeOp::MovImm => &["dst", "value"],
            CodeOp::Add
            | CodeOp::Adds
            | CodeOp::Sub
            | CodeOp::Subs
            | CodeOp::And
            | CodeOp::Orr
            | CodeOp::Eor
            | CodeOp::Mul
            | CodeOp::SMulH
            | CodeOp::SDiv
            | CodeOp::UDiv
            | CodeOp::FAddD
            | CodeOp::FSubD
            | CodeOp::FMulD
            | CodeOp::FDivD => &["dst", "lhs", "rhs"],
            CodeOp::Mvn => &["dst", "src"],
            CodeOp::MSub => &["dst", "lhs", "rhs", "minuend"],
            CodeOp::LslImm | CodeOp::LsrImm | CodeOp::AsrImm => &["dst", "src", "shift"],
            CodeOp::AddImm | CodeOp::SubImm => &["dst", "src", "imm"],
            CodeOp::SubSp | CodeOp::AddSp => &["imm"],
            CodeOp::CmpImm => &["lhs", "rhs"],
            CodeOp::Cmp => &["lhs", "rhs"],
            CodeOp::BranchEq
            | CodeOp::BranchNe
            | CodeOp::BranchGe
            | CodeOp::BranchLt
            | CodeOp::BranchGt
            | CodeOp::BranchLe
            | CodeOp::BranchVc
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::Branch
            | CodeOp::BranchLink => &["target"],
            CodeOp::BranchLinkRegister => &["register"],
            CodeOp::BranchSelf | CodeOp::Svc | CodeOp::Ret => &[],
            CodeOp::LdrU64 | CodeOp::LdrU8 => &["dst", "base", "offset"],
            CodeOp::StrU64 | CodeOp::StrU8 => &["src", "base", "offset"],
            CodeOp::Adrp | CodeOp::AddPageOff => &["dst", "symbol"],
            CodeOp::FMovXFromD
            | CodeOp::FMovDFromX
            | CodeOp::FNegD
            | CodeOp::SCvtfDFromX
            | CodeOp::FCvtzsXFromD => &["dst", "src"],
            CodeOp::FCmpZeroD => &["src"],
        };
        for name in required {
            if !self.fields.iter().any(|(field, _)| field == name) {
                return Err(format!(
                    "native code instruction '{}' missing field '{}'",
                    self.op.mnemonic(),
                    name
                ));
            }
        }
        Ok(())
    }
}

trait ToCodeJson {
    fn to_json(&self, indent: usize) -> String;
}

impl ToCodeJson for CodeFunction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{\n",
                "{}  \"name\": {},\n",
                "{}  \"symbol\": {},\n",
                "{}  \"returns\": {},\n",
                "{}  \"frame\": {},\n",
                "{}  \"params\": [{}\n{}  ],\n",
                "{}  \"stackSlots\": [{}\n{}  ],\n",
                "{}  \"instructions\": [{}\n{}  ],\n",
                "{}  \"relocations\": [{}\n{}  ]\n",
                "{}}}"
            ),
            pad,
            pad,
            json_string(&self.name),
            pad,
            json_string(&self.symbol),
            pad,
            json_string(&self.returns),
            pad,
            self.frame.to_json(indent + 2),
            pad,
            join_json(&self.params, indent + 2),
            pad,
            pad,
            join_json(&self.stack_slots, indent + 2),
            pad,
            pad,
            join_json(&self.instructions, indent + 2),
            pad,
            pad,
            join_json(&self.relocations, indent + 2),
            pad,
            pad
        )
    }
}

impl CodeFrame {
    fn to_json(&self, _indent: usize) -> String {
        format!(
            "{{ \"stackSize\": {}, \"calleeSaved\": [{}] }}",
            self.stack_size,
            json_string_list(&self.callee_saved)
        )
    }
}

impl ToCodeJson for CodeParam {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"location\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            json_string(&self.location)
        )
    }
}

impl ToCodeJson for CodeInstruction {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let mut fields = vec![format!("\"op\": {}", json_string(self.op.mnemonic()))];
        fields.extend(
            self.fields
                .iter()
                .map(|(name, value)| format!("\"{name}\": {}", json_string(value))),
        );
        format!("\n{}{{ {} }}", pad, fields.join(", "))
    }
}

impl ToCodeJson for CodeRelocation {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        let library = self
            .library
            .as_ref()
            .map(|library| json_string(library))
            .unwrap_or_else(|| "null".to_string());
        format!(
            "\n{}{{ \"from\": {}, \"to\": {}, \"kind\": {}, \"binding\": {}, \"library\": {} }}",
            pad,
            json_string(&self.from),
            json_string(&self.to),
            json_string(&self.kind),
            json_string(&self.binding),
            library
        )
    }
}

impl ToCodeJson for CodeImport {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"library\": {}, \"symbol\": {} }}",
            pad,
            json_string(&self.library),
            json_string(&self.symbol)
        )
    }
}

impl ToCodeJson for CodeDataObject {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            concat!(
                "\n{}{{ \"symbol\": {}, \"kind\": {}, \"layout\": {}, ",
                "\"align\": {}, \"size\": {}, \"value\": {} }}"
            ),
            pad,
            json_string(&self.symbol),
            json_string(&self.kind),
            json_string(&self.layout),
            self.align,
            self.size,
            json_string(&self.value)
        )
    }
}

impl ToCodeJson for CodeStackSlot {
    fn to_json(&self, indent: usize) -> String {
        let pad = " ".repeat(indent);
        format!(
            "\n{}{{ \"name\": {}, \"type\": {}, \"offset\": {} }}",
            pad,
            json_string(&self.name),
            json_string(&self.type_),
            self.offset
        )
    }
}

fn string_symbols(module: &NirModule) -> HashMap<String, String> {
    let mut values = Vec::new();
    if module_uses_type_name(module) {
        collect_type_name_values(module, &mut values);
    }
    for function in &module.functions {
        collect_string_values_from_ops(&function.body, &mut values);
    }
    for value in [
        ERR_INVALID_ARGUMENT_MESSAGE,
        ERR_OVERFLOW_MESSAGE,
        ERR_UNDERFLOW_MESSAGE,
        ERR_ALLOCATION_MESSAGE,
    ] {
        push_string_value(&mut values, value.to_string());
    }
    if module_uses_call(module, "find")
        || module_uses_call(module, "mid")
        || module_uses_call(module, "get")
        || module_uses_call(module, "append")
        || module_uses_call(module, "prepend")
        || module_uses_call(module, "insert")
        || module_uses_call(module, "transform")
        || module_uses_call(module, "filter")
        || module_uses_call(module, "removeAt")
        || module_uses_call(module, "set")
    {
        push_string_value(&mut values, ERR_INDEX_OUT_OF_RANGE_MESSAGE.to_string());
    }
    if module_uses_call(module, "find") || module_uses_call(module, "get") {
        push_string_value(&mut values, ERR_NOT_FOUND_MESSAGE.to_string());
    }
    if module_uses_call(module, "toString") {
        push_string_value(&mut values, "TRUE".to_string());
        push_string_value(&mut values, "FALSE".to_string());
        push_string_value(&mut values, ERR_ENCODING_MESSAGE.to_string());
    }
    for value in [
        ENTRY_ERROR_PREFIX,
        ENTRY_ERROR_SEPARATOR,
        ENTRY_ERROR_NEWLINE,
    ] {
        if !values.contains(&value.to_string()) {
            values.push(value.to_string());
        }
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| {
            let symbol = if let Some(symbol) = standard_error_message_symbol(&value) {
                symbol.to_string()
            } else if value == ENTRY_ERROR_PREFIX {
                ENTRY_ERROR_PREFIX_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_SEPARATOR {
                ENTRY_ERROR_SEPARATOR_SYMBOL.to_string()
            } else if value == ENTRY_ERROR_NEWLINE {
                ENTRY_ERROR_NEWLINE_SYMBOL.to_string()
            } else {
                format!("_mfb_str_{index}")
            };
            (value, symbol)
        })
        .collect()
}

fn standard_error_messages() -> &'static [(&'static str, &'static str, &'static str)] {
    &[
        (
            ERR_INVALID_ARGUMENT_CODE,
            ERR_INVALID_ARGUMENT_MESSAGE,
            ERR_INVALID_ARGUMENT_SYMBOL,
        ),
        (
            ERR_INVALID_FORMAT_CODE,
            ERR_INVALID_FORMAT_MESSAGE,
            ERR_INVALID_FORMAT_SYMBOL,
        ),
        (ERR_OVERFLOW_CODE, ERR_OVERFLOW_MESSAGE, ERR_OVERFLOW_SYMBOL),
        (
            ERR_UNDERFLOW_CODE,
            ERR_UNDERFLOW_MESSAGE,
            ERR_UNDERFLOW_SYMBOL,
        ),
        (
            ERR_OUT_OF_MEMORY_CODE,
            ERR_ALLOCATION_MESSAGE,
            ERR_ALLOCATION_SYMBOL,
        ),
        (
            ERR_INDEX_OUT_OF_RANGE_CODE,
            ERR_INDEX_OUT_OF_RANGE_MESSAGE,
            ERR_INDEX_OUT_OF_RANGE_SYMBOL,
        ),
        (
            ERR_NOT_FOUND_CODE,
            ERR_NOT_FOUND_MESSAGE,
            ERR_NOT_FOUND_SYMBOL,
        ),
        (ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE, ERR_ENCODING_SYMBOL),
    ]
}

fn standard_error_message_symbol(message: &str) -> Option<&'static str> {
    standard_error_messages()
        .iter()
        .find_map(|(_, candidate, symbol)| (*candidate == message).then_some(*symbol))
}

fn module_uses_type_name(module: &NirModule) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_use_type_name(&function.body))
}

fn module_uses_call(module: &NirModule, target: &str) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_use_call(&function.body, target))
}

fn ops_use_call(ops: &[NirOp], target: &str) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { value, .. } | NirOp::Return { value } => {
            value.as_ref().is_some_and(|value| value_uses_call(value, target))
        }
        NirOp::Fail { error } => value_uses_call(error, target),
        NirOp::Assign { value, .. } | NirOp::Eval { value } => value_uses_call(value, target),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_call(condition, target)
                || ops_use_call(then_body, target)
                || ops_use_call(else_body, target)
        }
        NirOp::Match { value, cases } => {
            value_uses_call(value, target)
                || cases.iter().any(|case| {
                    matches!(&case.pattern, NirMatchPattern::Value(value) if value_uses_call(value, target))
                        || ops_use_call(&case.body, target)
                })
        }
        NirOp::ForEach { iterable, body, .. } => {
            value_uses_call(iterable, target) || ops_use_call(body, target)
        }
        NirOp::Using { value, body, .. } => {
            value_uses_call(value, target) || ops_use_call(body, target)
        }
    })
}

fn value_uses_call(value: &NirValue, target: &str) -> bool {
    match value {
        NirValue::Call { target: call, args }
        | NirValue::RuntimeCall {
            target: call, args, ..
        } => call == target || args.iter().any(|arg| value_uses_call(arg, target)),
        NirValue::Constructor { args, .. } => args.iter().any(|arg| value_uses_call(arg, target)),
        NirValue::WithUpdate {
            target: updated,
            updates,
            ..
        } => {
            value_uses_call(updated, target)
                || updates
                    .iter()
                    .any(|update| value_uses_call(&update.value, target))
        }
        NirValue::ListLiteral { values, .. } => {
            values.iter().any(|value| value_uses_call(value, target))
        }
        NirValue::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(key, value)| value_uses_call(key, target) || value_uses_call(value, target)),
        NirValue::MemberAccess { target: value, .. } => value_uses_call(value, target),
        NirValue::Binary { left, right, .. } => {
            value_uses_call(left, target) || value_uses_call(right, target)
        }
        NirValue::Unary { operand, .. } => value_uses_call(operand, target),
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => false,
    }
}

fn ops_use_type_name(ops: &[NirOp]) -> bool {
    ops.iter().any(|op| match op {
        NirOp::Bind { value, .. } | NirOp::Return { value } => {
            value.as_ref().is_some_and(value_uses_type_name)
        }
        NirOp::Fail { error } => value_uses_type_name(error),
        NirOp::Assign { value, .. } | NirOp::Eval { value } => value_uses_type_name(value),
        NirOp::If {
            condition,
            then_body,
            else_body,
        } => {
            value_uses_type_name(condition)
                || ops_use_type_name(then_body)
                || ops_use_type_name(else_body)
        }
        NirOp::Match { value, cases } => value_uses_type_name(value) || cases.iter().any(|case| {
            matches!(&case.pattern, NirMatchPattern::Value(value) if value_uses_type_name(value))
                || ops_use_type_name(&case.body)
        }),
        NirOp::ForEach { iterable, body, .. } => {
            value_uses_type_name(iterable) || ops_use_type_name(body)
        }
        NirOp::Using { value, body, .. } => value_uses_type_name(value) || ops_use_type_name(body),
    })
}

fn value_uses_type_name(value: &NirValue) -> bool {
    let direct = match value {
        NirValue::Call { target, .. } | NirValue::RuntimeCall { target, .. } => {
            target == "typeName"
        }
        _ => false,
    };
    direct
        || match value {
            NirValue::Call { args, .. }
            | NirValue::RuntimeCall { args, .. }
            | NirValue::Constructor { args, .. } => args.iter().any(value_uses_type_name),
            NirValue::WithUpdate {
                target, updates, ..
            } => {
                value_uses_type_name(target)
                    || updates
                        .iter()
                        .any(|update| value_uses_type_name(&update.value))
            }
            NirValue::ListLiteral { values, .. } => values.iter().any(value_uses_type_name),
            NirValue::MapLiteral { entries, .. } => entries
                .iter()
                .any(|(key, value)| value_uses_type_name(key) || value_uses_type_name(value)),
            NirValue::MemberAccess { target, .. } => value_uses_type_name(target),
            NirValue::Binary { left, right, .. } => {
                value_uses_type_name(left) || value_uses_type_name(right)
            }
            NirValue::Unary { operand, .. } => value_uses_type_name(operand),
            NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => false,
        }
}

fn collect_type_name_values(module: &NirModule, values: &mut Vec<String>) {
    for value in [
        "Boolean", "Byte", "Error", "Fixed", "Float", "Integer", "Nothing", "String",
    ] {
        push_string_value(values, value.to_string());
    }
    for type_ in &module.types {
        push_string_value(values, type_.name.clone());
        for field in &type_.fields {
            push_string_value(values, field.type_.clone());
        }
        for variant in &type_.variants {
            push_string_value(values, variant.name.clone());
            for field in &variant.fields {
                push_string_value(values, field.type_.clone());
            }
        }
    }
    for function in &module.functions {
        push_string_value(values, function.returns.clone());
        for param in &function.params {
            push_string_value(values, param.type_.clone());
        }
        collect_type_name_values_from_ops(&function.body, values);
    }
}

fn collect_type_name_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    for op in ops {
        match op {
            NirOp::Bind { type_, value, .. } => {
                push_string_value(values, type_.clone());
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_type_name_values_from_value(value, values);
                }
            }
            NirOp::Fail { error } => collect_type_name_values_from_value(error, values),
            NirOp::Assign { value, .. } | NirOp::Eval { value } => {
                collect_type_name_values_from_value(value, values);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_type_name_values_from_value(condition, values);
                collect_type_name_values_from_ops(then_body, values);
                collect_type_name_values_from_ops(else_body, values);
            }
            NirOp::Match { value, cases } => {
                collect_type_name_values_from_value(value, values);
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        collect_type_name_values_from_value(value, values);
                    }
                    collect_type_name_values_from_ops(&case.body, values);
                }
            }
            NirOp::ForEach {
                type_,
                iterable,
                body,
                ..
            } => {
                push_string_value(values, type_.clone());
                collect_type_name_values_from_value(iterable, values);
                collect_type_name_values_from_ops(body, values);
            }
            NirOp::Using {
                type_, value, body, ..
            } => {
                push_string_value(values, type_.clone());
                collect_type_name_values_from_value(value, values);
                collect_type_name_values_from_ops(body, values);
            }
        }
    }
}

fn collect_type_name_values_from_value(value: &NirValue, values: &mut Vec<String>) {
    match value {
        NirValue::Const { type_, .. }
        | NirValue::FunctionRef { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => {
            push_string_value(values, type_.clone());
        }
        _ => {}
    }
    match value {
        NirValue::Call { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_type_name_values_from_value(arg, values);
            }
        }
        NirValue::WithUpdate {
            type_,
            target,
            updates,
        } => {
            push_string_value(values, type_.clone());
            collect_type_name_values_from_value(target, values);
            for update in updates {
                collect_type_name_values_from_value(&update.value, values);
            }
        }
        NirValue::ListLiteral { values: items, .. } => {
            for item in items {
                collect_type_name_values_from_value(item, values);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_type_name_values_from_value(key, values);
                collect_type_name_values_from_value(value, values);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_type_name_values_from_value(target, values)
        }
        NirValue::Binary { left, right, .. } => {
            collect_type_name_values_from_value(left, values);
            collect_type_name_values_from_value(right, values);
        }
        NirValue::Unary { operand, .. } => collect_type_name_values_from_value(operand, values),
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => {}
    }
}

fn collect_string_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    let mut constants = HashMap::new();
    let mut types = HashMap::new();
    collect_string_values_from_ops_with_constants(ops, values, &mut constants, &mut types);
}

fn collect_string_values_from_ops_with_constants(
    ops: &[NirOp],
    values: &mut Vec<String>,
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, String>,
) {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types);
                    if let Some(constant) =
                        local_constant_value_with_constants(value, constants, types)
                    {
                        constants.insert(name.clone(), constant);
                    } else {
                        constants.remove(name);
                    }
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values, constants, types);
                }
            }
            NirOp::Fail { error } => {
                collect_string_values_from_value(error, values, constants, types);
            }
            NirOp::Assign { name, value } => {
                collect_string_values_from_value(value, values, constants, types);
                if let Some(constant) = local_constant_value_with_constants(value, constants, types)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } => {
                collect_string_values_from_value(value, values, constants, types);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_string_values_from_value(condition, values, constants, types);
                let mut then_constants = constants.clone();
                let mut else_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_types = types.clone();
                collect_string_values_from_ops_with_constants(
                    then_body,
                    values,
                    &mut then_constants,
                    &mut then_types,
                );
                collect_string_values_from_ops_with_constants(
                    else_body,
                    values,
                    &mut else_constants,
                    &mut else_types,
                );
            }
            NirOp::Match { value, cases } => {
                collect_string_values_from_value(value, values, constants, types);
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        collect_string_values_from_value(value, values, constants, types);
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    collect_string_values_from_ops_with_constants(
                        &case.body,
                        values,
                        &mut case_constants,
                        &mut case_types,
                    );
                }
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                collect_string_values_from_value(iterable, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                );
            }
            NirOp::Using {
                name,
                type_,
                value,
                body,
                ..
            } => {
                collect_string_values_from_value(value, values, constants, types);
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_types.insert(name.clone(), type_.clone());
                collect_string_values_from_ops_with_constants(
                    body,
                    values,
                    &mut body_constants,
                    &mut body_types,
                );
            }
        }
    }
}

fn collect_string_values_from_value(
    value: &NirValue,
    values: &mut Vec<String>,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) {
    if let Some(value) = static_string_value_with_constants(value, constants, types) {
        push_string_value(values, value);
    }
    if value_may_return_invalid_format(value, constants, types) {
        push_string_value(values, ERR_INVALID_FORMAT_MESSAGE.to_string());
    }
    match value {
        NirValue::Const { type_, value } if type_ == "String" => {
            push_string_value(values, value.clone());
        }
        NirValue::Call { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_values_from_value(arg, values, constants, types);
            }
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_string_values_from_value(target, values, constants, types);
            for update in updates {
                collect_string_values_from_value(&update.value, values, constants, types);
            }
        }
        NirValue::ListLiteral { values: items, .. } => {
            for item in items {
                collect_string_values_from_value(item, values, constants, types);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_string_values_from_value(key, values, constants, types);
                collect_string_values_from_value(value, values, constants, types);
            }
        }
        NirValue::MemberAccess { target, .. } => {
            collect_string_values_from_value(target, values, constants, types)
        }
        NirValue::Binary { left, right, .. } => {
            collect_string_values_from_value(left, values, constants, types);
            collect_string_values_from_value(right, values, constants, types);
        }
        NirValue::Unary { operand, .. } => {
            collect_string_values_from_value(operand, values, constants, types)
        }
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => {}
    }
}

impl CollectionTypeLayout {
    fn from_type(type_: &str) -> Option<Self> {
        if let Some(value_type) = type_.strip_prefix("List OF ") {
            return Some(Self {
                kind: COLLECTION_KIND_LIST,
                key_type_code: COLLECTION_TYPE_NONE,
                value_type_code: collection_type_code(value_type)?,
            });
        }
        let (key_type, value_type) = map_type_parts(type_)?;
        Some(Self {
            kind: COLLECTION_KIND_MAP,
            key_type_code: collection_type_code(&key_type)?,
            value_type_code: collection_type_code(&value_type)?,
        })
    }
}

fn collection_type_code(type_: &str) -> Option<usize> {
    match type_ {
        "Boolean" => Some(COLLECTION_TYPE_BOOLEAN),
        "Byte" => Some(COLLECTION_TYPE_BYTE),
        "Integer" => Some(COLLECTION_TYPE_INTEGER),
        "Float" => Some(COLLECTION_TYPE_FLOAT),
        "Fixed" => Some(COLLECTION_TYPE_FIXED),
        "String" => Some(COLLECTION_TYPE_STRING),
        _ if type_.starts_with("List OF ") => Some(COLLECTION_TYPE_LIST),
        _ if type_.starts_with("Map OF ") => Some(COLLECTION_TYPE_MAP),
        _ => None,
    }
}

fn value_may_return_invalid_format(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    (match value {
        NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. } => {
            match target.as_str() {
                "toInt" if args.len() == 1 => {
                    static_type_name_with_types(&args[0], types).as_deref() != Some("Byte")
                }
                "toFloat" | "toFixed" | "isNumeric" => true,
                _ => false,
            }
        }
        _ => false,
    }) || match value {
        NirValue::Call { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_may_return_invalid_format(arg, constants, types)),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_may_return_invalid_format(target, constants, types)
                || updates
                    .iter()
                    .any(|update| value_may_return_invalid_format(&update.value, constants, types))
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_may_return_invalid_format(value, constants, types)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_may_return_invalid_format(key, constants, types)
                || value_may_return_invalid_format(value, constants, types)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_may_return_invalid_format(target, constants, types)
        }
        NirValue::Binary { left, right, .. } => {
            value_may_return_invalid_format(left, constants, types)
                || value_may_return_invalid_format(right, constants, types)
        }
        NirValue::Unary { operand, .. } => {
            value_may_return_invalid_format(operand, constants, types)
        }
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => false,
    }
}

fn push_string_value(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}

fn local_constant_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<NirValue> {
    match value {
        NirValue::Const { .. } => Some(value.clone()),
        NirValue::Local(name) => constants.get(name).cloned(),
        NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types).map(|value| NirValue::Const {
                type_: "String".to_string(),
                value,
            })
        }
        NirValue::Binary { op, .. } if op == "&" => {
            static_string_value_with_constants(value, constants, types).map(|value| {
                NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }
            })
        }
        _ => None,
    }
}

fn static_string_value_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value.clone()),
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| static_string_value_with_constants(constant, constants, types)),
        NirValue::Call { target, args } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::RuntimeCall { target, args, .. } if target == "toString" && args.len() == 1 => {
            static_primitive_text_with_constants(&args[0], constants)
        }
        NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. }
            if target == "typeName" && args.len() == 1 =>
        {
            static_type_name_with_types(&args[0], types)
        }
        NirValue::Binary { op, left, right } if op == "&" => {
            let left = static_string_value_with_constants(left, constants, types)?;
            let right = static_string_value_with_constants(right, constants, types)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

fn static_type_name_with_types(
    value: &NirValue,
    types: &HashMap<String, String>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, .. } => Some(type_.clone()),
        NirValue::Local(name) => types.get(name).cloned(),
        NirValue::FunctionRef { type_, .. }
        | NirValue::Constructor { type_, .. }
        | NirValue::WithUpdate { type_, .. }
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::Call { target, .. } | NirValue::RuntimeCall { target, .. } => {
            match target.as_str() {
                "replace" | "typeName" | "toString" => Some("String".to_string()),
                "find" | "len" | "toInt" => Some("Integer".to_string()),
                "mid" => Some("String".to_string()),
                "toFloat" => Some("Float".to_string()),
                "toFixed" => Some("Fixed".to_string()),
                "toByte" => Some("Byte".to_string()),
                "isNumeric" => Some("Boolean".to_string()),
                _ => None,
            }
        }
        NirValue::Binary { op, left, right } => {
            if matches!(
                op.as_str(),
                "=" | "<>" | "<" | ">" | "<=" | ">=" | "AND" | "OR" | "XOR"
            ) {
                return Some("Boolean".to_string());
            }
            if op == "&" {
                return Some("String".to_string());
            }
            let left = static_type_name_with_types(left, types)?;
            let right = static_type_name_with_types(right, types)?;
            Some(numeric_binary_result_type(op, &left, &right).to_string())
        }
        NirValue::Unary { op, operand } => {
            if op == "NOT" {
                Some("Boolean".to_string())
            } else {
                static_type_name_with_types(operand, types)
            }
        }
        NirValue::MemberAccess { target, member } => {
            let target_type = static_type_name_with_types(target, types)?;
            let (key_type, value_type) = parse_map_entry_type(&target_type)?;
            match member.as_str() {
                "key" => Some(key_type),
                "value" => Some(value_type),
                _ => None,
            }
        }
    }
}

fn static_primitive_text_with_constants(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
) -> Option<String> {
    match value {
        NirValue::Const { type_, value } => match type_.as_str() {
            "Integer" | "Byte" | "Float" | "Fixed" | "String" => Some(value.clone()),
            "Boolean" => match value.as_str() {
                "true" => Some("TRUE".to_string()),
                "false" => Some("FALSE".to_string()),
                _ => None,
            },
            _ => None,
        },
        NirValue::Local(name) => constants
            .get(name)
            .and_then(|constant| static_primitive_text_with_constants(constant, constants)),
        _ => None,
    }
}

fn align(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

fn join_texts(values: &[ValueResult]) -> String {
    values
        .iter()
        .map(|value| value.text.clone())
        .collect::<Vec<_>>()
        .join(", ")
}

fn is_collection_type(type_: &str) -> bool {
    type_.starts_with("List OF ") || type_.starts_with("Map OF ")
}

fn list_element_type(type_: &str) -> Option<String> {
    type_.strip_prefix("List OF ").map(str::to_string)
}

fn map_type_parts(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("Map OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn callable_return_type(type_: &str) -> Option<String> {
    let (_, returns) = type_.rsplit_once(") AS ")?;
    Some(returns.to_string())
}

fn parse_map_entry_type(type_: &str) -> Option<(String, String)> {
    let rest = type_.strip_prefix("MapEntry OF ")?;
    let (key, value) = rest.split_once(" TO ")?;
    Some((key.to_string(), value.to_string()))
}

fn numeric_binary_result_type(operator: &str, left: &str, right: &str) -> &'static str {
    numeric::binary_result_type(operator, left, right).unwrap_or(numeric::TYPE_INTEGER)
}

fn integer_constant_value(value: &NirValue) -> Option<i64> {
    match value {
        NirValue::Const { type_, value } if type_ == "Integer" => value.parse::<i64>().ok(),
        NirValue::Unary { op, operand } if op == "-" => {
            integer_constant_value(operand).and_then(i64::checked_neg)
        }
        _ => None,
    }
}

fn native_immediate_value(type_: &str, value: &str) -> Result<String, String> {
    match type_ {
        "Nothing" => Ok("0".to_string()),
        "Float" => Ok(value
            .parse::<f64>()
            .map_err(|_| format!("invalid Float constant `{value}`"))?
            .to_bits()
            .to_string()),
        "Fixed" => Ok(fixed_raw_from_decimal(value)?.to_string()),
        _ => Ok(value.to_string()),
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

fn string_constant_value(value: &NirValue) -> Option<&str> {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => Some(value),
        _ => None,
    }
}

fn join_json<T: ToCodeJson>(values: &[T], indent: usize) -> String {
    values
        .iter()
        .map(|value| value.to_json(indent))
        .collect::<Vec<_>>()
        .join(",")
}

fn json_string_list(values: &[String]) -> String {
    values
        .iter()
        .map(|value| json_string(value))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
fn checked_arena_used_after_alloc(
    block_base: u64,
    current_offset: u64,
    capacity: u64,
    size: u64,
    align: u64,
) -> Result<(u64, u64), u64> {
    let invalid_argument = ERR_INVALID_ARGUMENT_CODE
        .parse::<u64>()
        .expect("invalid argument code");
    let out_of_memory = ERR_OUT_OF_MEMORY_CODE
        .parse::<u64>()
        .expect("out of memory code");
    if align == 0 || !align.is_power_of_two() {
        return Err(invalid_argument);
    }
    let size = size.max(1);
    let payload_base = block_base
        .checked_add(ARENA_BLOCK_HEADER_SIZE as u64)
        .ok_or(out_of_memory)?;
    let raw = payload_base
        .checked_add(current_offset)
        .ok_or(out_of_memory)?;
    let mask = align - 1;
    let aligned = raw
        .checked_add(mask)
        .map(|value| value & !mask)
        .ok_or(out_of_memory)?;
    let end = aligned.checked_add(size).ok_or(out_of_memory)?;
    let used = end.checked_sub(payload_base).ok_or(out_of_memory)?;
    if used > capacity {
        return Err(out_of_memory);
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
            Err(10002)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, 128, 8, 3),
            Err(10002)
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
            Err(10010)
        );
    }

    #[test]
    fn arena_checks_arithmetic_overflow() {
        assert_eq!(
            checked_arena_used_after_alloc(u64::MAX - 8, 0, 128, 8, 8),
            Err(10010)
        );
        assert_eq!(
            checked_arena_used_after_alloc(0x1000, 0, u64::MAX, u64::MAX, 8),
            Err(10010)
        );
    }
}
