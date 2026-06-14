use std::collections::{HashMap, HashSet};

use crate::arch::aarch64::{abi, ops::CodeOp};
use crate::builtins;
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
const ERR_OUTPUT_CODE: &str = "10015";
const ERR_OUTPUT_MESSAGE: &str = "output failure";
const ERR_OUTPUT_SYMBOL: &str = "_mfb_str_error_output";
const ERR_UNSUPPORTED_CODE: &str = "10007";
const ERR_UNSUPPORTED_MESSAGE: &str = "unsupported operation";
const ERR_UNSUPPORTED_SYMBOL: &str = "_mfb_str_error_unsupported";
const ERR_EOF_CODE: &str = "10016";
const ERR_EOF_MESSAGE: &str = "end of file";
const ERR_EOF_SYMBOL: &str = "_mfb_str_error_eof";
const ERR_ENCODING_CODE: &str = "10019";
const ERR_ENCODING_MESSAGE: &str = "invalid encoding";
const ERR_ENCODING_SYMBOL: &str = "_mfb_str_error_encoding";
const ERR_INPUT_CODE: &str = "10020";
const ERR_INPUT_MESSAGE: &str = "input failure";
const ERR_INPUT_SYMBOL: &str = "_mfb_str_error_input";
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
const UNICODE_STAGE1_SYMBOL: &str = "_mfb_unicode_stage1";
const UNICODE_STAGE2_SYMBOL: &str = "_mfb_unicode_stage2";
const UNICODE_PROPERTIES_SYMBOL: &str = "_mfb_unicode_properties";
const UNICODE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_sequences";
const UNICODE_COMBINATIONS_SECOND_SYMBOL: &str = "_mfb_unicode_combinations_second";
const UNICODE_COMBINATIONS_COMBINED_SYMBOL: &str = "_mfb_unicode_combinations_combined";
const UNICODE_NFD_ENTRIES_SYMBOL: &str = "_mfb_unicode_nfd_entries";
const UNICODE_NFD_SEQUENCES_SYMBOL: &str = "_mfb_unicode_nfd_sequences";
const UNICODE_UPPERCASE_ENTRIES_SYMBOL: &str = "_mfb_unicode_uppercase_entries";
const UNICODE_UPPERCASE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_uppercase_sequences";
const UNICODE_LOWERCASE_ENTRIES_SYMBOL: &str = "_mfb_unicode_lowercase_entries";
const UNICODE_LOWERCASE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_lowercase_sequences";
const UNICODE_CASEFOLD_ENTRIES_SYMBOL: &str = "_mfb_unicode_casefold_entries";
const UNICODE_CASEFOLD_SEQUENCES_SYMBOL: &str = "_mfb_unicode_casefold_sequences";

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
    fn emit_poll_input(
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
    let mut data_objects = string_objects
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
    if module_uses_unicode_runtime_tables(module) {
        data_objects.extend(unicode_runtime_data_objects());
    }
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
    for (name, type_, symbol) in builtin_function_refs(module) {
        code_functions.push(lower_builtin_function_wrapper(
            &name,
            &type_,
            &symbol,
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

fn lower_builtin_function_wrapper(
    name: &str,
    type_: &str,
    symbol: &str,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    platform_imports: &HashMap<String, String>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let (params, returns) = function_type_parts(type_).ok_or_else(|| {
        format!("native built-in function wrapper has malformed function type '{type_}'")
    })?;
    if params.len() != 1 || returns != "Boolean" {
        return Err(format!(
            "native built-in function wrapper expects a unary Boolean function, got '{type_}'"
        ));
    }

    let param = CodeParam {
        name: "value".to_string(),
        type_: params[0].clone(),
        location: abi::argument_register(0)?,
    };
    let mut builder = CodeBuilder {
        current_symbol: symbol.to_string(),
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

    let stack_offset = builder.allocate_stack_object("value", 8);
    builder.locals.insert(
        "value".to_string(),
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

    let result = builder.lower_value(&NirValue::Call {
        target: name.to_string(),
        args: vec![NirValue::Local("value".to_string())],
    })?;
    builder.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
    builder.emit(abi::move_immediate(
        RESULT_TAG_REGISTER,
        "Integer",
        RESULT_OK_TAG,
    ));
    builder.emit(abi::return_());

    let mut instructions = builder.instructions;
    let mut stack_slots = builder.stack_slots;
    let frame = finalize_frame(
        &mut instructions,
        &mut stack_slots,
        builder.stack_size,
        builder.used_callee_saved,
    );

    Ok(CodeFunction {
        name: format!("builtin.{name}.{type_}"),
        symbol: symbol.to_string(),
        params: vec![param],
        returns: returns,
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
        "io.print" | "io.write" | "io.printError" | "io.writeError" => {
            let (frame, instructions, relocations) = lower_io_write_helper(
                symbol,
                platform_imports,
                platform,
                matches!(spec.call, "io.printError" | "io.writeError"),
                matches!(spec.call, "io.print" | "io.printError"),
            )?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
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
        "io.flush" | "io.flushError" => {
            let instructions = vec![
                abi::label("entry"),
                abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
                abi::return_(),
            ];
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
                symbol: symbol.to_string(),
                params: Vec::new(),
                returns: spec.abi.returns.to_string(),
                frame: CodeFrame {
                    stack_size: 0,
                    callee_saved: Vec::new(),
                },
                stack_slots: Vec::new(),
                instructions,
                relocations: Vec::new(),
            })
        }
        "io.pollInput" => {
            let (frame, instructions, relocations) =
                lower_io_poll_input_helper(symbol, platform_imports, platform)?;
            Ok(CodeFunction {
                name: format!("runtime.{}", spec.call),
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

fn lower_io_write_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
    stderr: bool,
    append_newline: bool,
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
        abi::move_immediate(
            abi::return_register(),
            "Integer",
            if stderr { "2" } else { "1" },
        ),
    ]);
    platform.emit_write(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;
    let write_error = format!("{symbol}_write_error");
    let done = format!("{symbol}_done");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&write_error),
    ]);
    if append_newline {
        instructions.extend([
            abi::move_immediate(abi::newline_scratch_register(), "Integer", "10"),
            abi::store_u64(abi::newline_scratch_register(), abi::stack_pointer(), 8),
            abi::move_immediate(
                abi::return_register(),
                "Integer",
                if stderr { "2" } else { "1" },
            ),
            abi::add_immediate(abi::string_data_register(), abi::stack_pointer(), 8),
            abi::move_immediate(abi::string_length_register(), "Integer", "1"),
        ]);
        platform.emit_write(
            symbol,
            platform_imports,
            &mut instructions,
            &mut relocations,
        )?;
        instructions.extend([
            abi::compare_immediate(abi::return_register(), "0"),
            abi::branch_lt(&write_error),
        ]);
    }
    instructions.extend([
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&write_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_OUTPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let output_error_symbol = ERR_OUTPUT_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &output_error_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: output_error_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.push(abi::label(&done));
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

fn lower_io_poll_input_helper(
    symbol: &str,
    platform_imports: &HashMap<String, String>,
    platform: &dyn CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    const POLLIN_PACKED_FD0: &str = "4294967296";
    const FRAME_SIZE: usize = 48;
    const POLLFD_OFFSET: usize = 8;
    const TIMESPEC_SEC_OFFSET: usize = 16;
    const TIMESPEC_NSEC_OFFSET: usize = 24;
    const TIMEOUT_OFFSET: usize = 32;

    let mut instructions = vec![abi::label("entry"), abi::subtract_stack(FRAME_SIZE)];
    let mut relocations = Vec::new();
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.push(abi::store_u64(
            abi::link_register(),
            abi::stack_pointer(),
            0,
        ));
    }
    instructions.extend([
        abi::store_u64(abi::return_register(), abi::stack_pointer(), TIMEOUT_OFFSET),
        abi::move_immediate("x9", "Integer", POLLIN_PACKED_FD0),
        abi::store_u64("x9", abi::stack_pointer(), POLLFD_OFFSET),
    ]);

    if platform.target() == "linux-aarch64" {
        let timeout_zero = format!("{symbol}_timeout_zero");
        let timeout_positive = format!("{symbol}_timeout_positive");
        let timeout_ready = format!("{symbol}_timeout_ready");
        instructions.extend([
            abi::load_u64("x10", abi::stack_pointer(), TIMEOUT_OFFSET),
            abi::compare_immediate("x10", "0"),
            abi::branch_eq(&timeout_zero),
            abi::branch_gt(&timeout_positive),
            abi::move_immediate("x2", "Integer", "0"),
            abi::branch(&timeout_ready),
            abi::label(&timeout_zero),
            abi::store_u64("x31", abi::stack_pointer(), TIMESPEC_SEC_OFFSET),
            abi::store_u64("x31", abi::stack_pointer(), TIMESPEC_NSEC_OFFSET),
            abi::add_immediate("x2", abi::stack_pointer(), TIMESPEC_SEC_OFFSET),
            abi::branch(&timeout_ready),
            abi::label(&timeout_positive),
            abi::move_immediate("x11", "Integer", "1000"),
            abi::unsigned_divide_registers("x12", "x10", "x11"),
            abi::multiply_subtract_registers("x13", "x12", "x11", "x10"),
            abi::move_immediate("x14", "Integer", "1000000"),
            abi::multiply_registers("x13", "x13", "x14"),
            abi::store_u64("x12", abi::stack_pointer(), TIMESPEC_SEC_OFFSET),
            abi::store_u64("x13", abi::stack_pointer(), TIMESPEC_NSEC_OFFSET),
            abi::add_immediate("x2", abi::stack_pointer(), TIMESPEC_SEC_OFFSET),
            abi::label(&timeout_ready),
        ]);
    } else {
        instructions.push(abi::load_u64("x2", abi::stack_pointer(), TIMEOUT_OFFSET));
    }

    instructions.extend([
        abi::add_immediate(abi::return_register(), abi::stack_pointer(), POLLFD_OFFSET),
        abi::move_immediate("x1", "Integer", "1"),
    ]);
    platform.emit_poll_input(
        symbol,
        platform_imports,
        &mut instructions,
        &mut relocations,
    )?;

    let poll_error = format!("{symbol}_poll_error");
    let poll_ready = format!("{symbol}_poll_ready");
    let done = format!("{symbol}_done");
    instructions.extend([
        abi::compare_immediate(abi::return_register(), "0"),
        abi::branch_lt(&poll_error),
        abi::branch_gt(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "0"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_ready),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Boolean", "1"),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_OK_TAG),
        abi::branch(&done),
        abi::label(&poll_error),
        abi::move_immediate(RESULT_VALUE_REGISTER, "Integer", ERR_INPUT_CODE),
        abi::move_immediate(RESULT_TAG_REGISTER, "Integer", RESULT_ERR_TAG),
    ]);
    let input_error_symbol = ERR_INPUT_SYMBOL.to_string();
    instructions.push(
        CodeInstruction::new("adrp")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &input_error_symbol),
    );
    instructions.push(
        CodeInstruction::new("add_pageoff")
            .field("dst", RESULT_ERROR_MESSAGE_REGISTER)
            .field("src", RESULT_ERROR_MESSAGE_REGISTER)
            .field("symbol", &input_error_symbol),
    );
    relocations.extend([
        CodeRelocation {
            from: symbol.to_string(),
            to: input_error_symbol.clone(),
            kind: "page21".to_string(),
            binding: "data".to_string(),
            library: None,
        },
        CodeRelocation {
            from: symbol.to_string(),
            to: input_error_symbol,
            kind: "pageoff12".to_string(),
            binding: "data".to_string(),
            library: None,
        },
    ]);
    instructions.push(abi::label(&done));
    if platform.preserves_link_register_in_runtime_helpers() {
        instructions.extend([
            abi::load_u64(abi::link_register(), abi::stack_pointer(), 0),
            abi::add_stack(FRAME_SIZE),
            abi::return_(),
        ]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
                callee_saved: vec![abi::link_register().to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([abi::add_stack(FRAME_SIZE), abi::return_()]);
        Ok((
            CodeFrame {
                stack_size: FRAME_SIZE,
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

mod builder_collection_layout;
mod builder_collection_queries;
mod builder_collection_updates;
mod builder_control;
mod builder_conversions;
mod builder_misc;
mod builder_numeric;
mod builder_search;
mod builder_strings;
mod builder_strings_package;
mod builder_values;
mod private;

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
            CodeOp::LdrU64 | CodeOp::LdrU32 | CodeOp::LdrU16 | CodeOp::LdrU8 => {
                &["dst", "base", "offset"]
            }
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
    if module_uses_any_call(
        module,
        &[
            "io.print",
            "io.write",
            "io.printError",
            "io.writeError",
            "io.flush",
            "io.flushError",
        ],
    ) {
        push_string_value(&mut values, ERR_OUTPUT_MESSAGE.to_string());
    }
    if module_uses_any_call(
        module,
        &["io.input", "io.readLine", "io.readChar", "io.readByte"],
    ) {
        push_string_value(&mut values, ERR_EOF_MESSAGE.to_string());
        push_string_value(&mut values, ERR_INPUT_MESSAGE.to_string());
    }
    if module_uses_call(module, "io.pollInput") {
        push_string_value(&mut values, ERR_INPUT_MESSAGE.to_string());
    }
    if module_uses_call(module, "io.terminalSize") {
        push_string_value(&mut values, ERR_UNSUPPORTED_MESSAGE.to_string());
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
        (
            ERR_UNSUPPORTED_CODE,
            ERR_UNSUPPORTED_MESSAGE,
            ERR_UNSUPPORTED_SYMBOL,
        ),
        (ERR_OUTPUT_CODE, ERR_OUTPUT_MESSAGE, ERR_OUTPUT_SYMBOL),
        (ERR_EOF_CODE, ERR_EOF_MESSAGE, ERR_EOF_SYMBOL),
        (ERR_ENCODING_CODE, ERR_ENCODING_MESSAGE, ERR_ENCODING_SYMBOL),
        (ERR_INPUT_CODE, ERR_INPUT_MESSAGE, ERR_INPUT_SYMBOL),
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

fn module_uses_any_call(module: &NirModule, targets: &[&str]) -> bool {
    targets.iter().any(|target| module_uses_call(module, target))
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

fn module_uses_unicode_runtime_tables(module: &NirModule) -> bool {
    module.functions.iter().any(|function| {
        let mut constants = HashMap::new();
        let mut types = HashMap::new();
        ops_use_unicode_runtime_tables(&function.body, &mut constants, &mut types)
    })
}

fn ops_use_unicode_runtime_tables(
    ops: &[NirOp],
    constants: &mut HashMap<String, NirValue>,
    types: &mut HashMap<String, String>,
) -> bool {
    for op in ops {
        match op {
            NirOp::Bind {
                name, type_, value, ..
            } => {
                types.insert(name.clone(), type_.clone());
                if let Some(value) = value {
                    if value_uses_unicode_runtime_tables(value, constants, types) {
                        return true;
                    }
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
            NirOp::Assign { name, value } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
                if let Some(constant) = local_constant_value_with_constants(value, constants, types)
                {
                    constants.insert(name.clone(), constant);
                } else {
                    constants.remove(name);
                }
            }
            NirOp::Eval { value } | NirOp::Fail { error: value } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
            }
            NirOp::Return { value } => {
                if value
                    .as_ref()
                    .is_some_and(|value| value_uses_unicode_runtime_tables(value, constants, types))
                {
                    return true;
                }
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                if value_uses_unicode_runtime_tables(condition, constants, types) {
                    return true;
                }
                let mut then_constants = constants.clone();
                let mut then_types = types.clone();
                let mut else_constants = constants.clone();
                let mut else_types = types.clone();
                if ops_use_unicode_runtime_tables(then_body, &mut then_constants, &mut then_types)
                    || ops_use_unicode_runtime_tables(
                        else_body,
                        &mut else_constants,
                        &mut else_types,
                    )
                {
                    return true;
                }
            }
            NirOp::Match { value, cases } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        if value_uses_unicode_runtime_tables(value, constants, types) {
                            return true;
                        }
                    }
                    let mut case_constants = constants.clone();
                    let mut case_types = types.clone();
                    if ops_use_unicode_runtime_tables(
                        &case.body,
                        &mut case_constants,
                        &mut case_types,
                    ) {
                        return true;
                    }
                }
            }
            NirOp::ForEach {
                name,
                type_,
                iterable,
                body,
            } => {
                if value_uses_unicode_runtime_tables(iterable, constants, types) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
            }
            NirOp::Using {
                name,
                type_,
                value,
                body,
                ..
            } => {
                if value_uses_unicode_runtime_tables(value, constants, types) {
                    return true;
                }
                let mut body_constants = constants.clone();
                let mut body_types = types.clone();
                body_constants.remove(name);
                body_types.insert(name.clone(), type_.clone());
                if ops_use_unicode_runtime_tables(body, &mut body_constants, &mut body_types) {
                    return true;
                }
            }
        }
    }
    false
}

fn value_uses_unicode_runtime_tables(
    value: &NirValue,
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    match value {
        NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. } => {
            matches!(
                target.as_str(),
                "strings.upper"
                    | "strings.lower"
                    | "strings.caseFold"
                    | "strings.normalizeNfc"
                    | "strings.graphemes"
            ) && !unicode_string_call_is_static(target, args, constants, types)
                || args
                    .iter()
                    .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types))
        }
        NirValue::Constructor { args, .. } => args
            .iter()
            .any(|arg| value_uses_unicode_runtime_tables(arg, constants, types)),
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            value_uses_unicode_runtime_tables(target, constants, types)
                || updates.iter().any(|update| {
                    value_uses_unicode_runtime_tables(&update.value, constants, types)
                })
        }
        NirValue::ListLiteral { values, .. } => values
            .iter()
            .any(|value| value_uses_unicode_runtime_tables(value, constants, types)),
        NirValue::MapLiteral { entries, .. } => entries.iter().any(|(key, value)| {
            value_uses_unicode_runtime_tables(key, constants, types)
                || value_uses_unicode_runtime_tables(value, constants, types)
        }),
        NirValue::MemberAccess { target, .. } => {
            value_uses_unicode_runtime_tables(target, constants, types)
        }
        NirValue::Binary { left, right, .. } => {
            value_uses_unicode_runtime_tables(left, constants, types)
                || value_uses_unicode_runtime_tables(right, constants, types)
        }
        NirValue::Unary { operand, .. } => {
            value_uses_unicode_runtime_tables(operand, constants, types)
        }
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => false,
    }
}

fn unicode_string_call_is_static(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> bool {
    matches!(
        target,
        "strings.upper"
            | "strings.lower"
            | "strings.caseFold"
            | "strings.normalizeNfc"
            | "strings.graphemes"
    ) && args.len() == 1
        && static_string_value_with_constants(&args[0], constants, types).is_some()
}

fn unicode_runtime_data_objects() -> Vec<CodeDataObject> {
    let tables = crate::unicode_runtime_tables::tables();
    vec![
        raw_data_object(
            UNICODE_STAGE1_SYMBOL,
            "u16 utf8proc stage1 property index table",
            tables.stage1.len() * 2,
            crate::unicode_runtime_tables::stage1_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_STAGE2_SYMBOL,
            "u16 utf8proc stage2 property index table",
            tables.stage2.len() * 2,
            crate::unicode_runtime_tables::stage2_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_PROPERTIES_SYMBOL,
            "mfb.unicode.property.v1 records, 24 bytes each",
            tables.properties.len() * 24,
            crate::unicode_runtime_tables::properties_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_SEQUENCES_SYMBOL,
            "u16 utf8proc sequence table",
            tables.sequences.len() * 2,
            crate::unicode_runtime_tables::sequences_hex(),
            2,
        ),
        raw_data_object(
            UNICODE_COMBINATIONS_SECOND_SYMBOL,
            "u32 utf8proc composition second codepoint table",
            tables.combinations_second.len() * 4,
            crate::unicode_runtime_tables::combinations_second_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_COMBINATIONS_COMBINED_SYMBOL,
            "u32 utf8proc composition combined codepoint table",
            tables.combinations_combined.len() * 4,
            crate::unicode_runtime_tables::combinations_combined_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_NFD_ENTRIES_SYMBOL,
            "mfb.unicode.nfd_entry.v1 records, 16 bytes each",
            tables.nfd_entries.len() * 16,
            crate::unicode_runtime_tables::nfd_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_NFD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode NFD sequence table",
            tables.nfd_sequences.len() * 4,
            crate::unicode_runtime_tables::nfd_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_UPPERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 uppercase records, 16 bytes each",
            tables.uppercase_entries.len() * 16,
            crate::unicode_runtime_tables::uppercase_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_UPPERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode uppercase sequence table",
            tables.uppercase_sequences.len() * 4,
            crate::unicode_runtime_tables::uppercase_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_LOWERCASE_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 lowercase records, 16 bytes each",
            tables.lowercase_entries.len() * 16,
            crate::unicode_runtime_tables::lowercase_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_LOWERCASE_SEQUENCES_SYMBOL,
            "u32 flattened Unicode lowercase sequence table",
            tables.lowercase_sequences.len() * 4,
            crate::unicode_runtime_tables::lowercase_sequences_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_CASEFOLD_ENTRIES_SYMBOL,
            "mfb.unicode.mapping_entry.v1 casefold records, 16 bytes each",
            tables.casefold_entries.len() * 16,
            crate::unicode_runtime_tables::casefold_entries_hex(),
            4,
        ),
        raw_data_object(
            UNICODE_CASEFOLD_SEQUENCES_SYMBOL,
            "u32 flattened Unicode casefold sequence table",
            tables.casefold_sequences.len() * 4,
            crate::unicode_runtime_tables::casefold_sequences_hex(),
            4,
        ),
    ]
}

fn raw_data_object(
    symbol: &str,
    layout: &str,
    size: usize,
    value: String,
    alignment: usize,
) -> CodeDataObject {
    CodeDataObject {
        symbol: symbol.to_string(),
        kind: "raw".to_string(),
        layout: layout.to_string(),
        align: alignment,
        size: align(size, alignment),
        value,
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
    if let NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. } = value {
        if target == "strings.graphemes" && args.len() == 1 {
            if let Some(value) = static_string_value_with_constants(&args[0], constants, types) {
                for grapheme in crate::unicode_backend::graphemes(&value) {
                    push_string_value(values, grapheme);
                }
            }
        }
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
        NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. }
            if strings_package_static_string_value(target, args, constants, types).is_some() =>
        {
            strings_package_static_string_value(target, args, constants, types).map(|value| {
                NirValue::Const {
                    type_: "String".to_string(),
                    value,
                }
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
        NirValue::Call { target, args } | NirValue::RuntimeCall { target, args, .. } => {
            strings_package_static_string_value(target, args, constants, types)
        }
        NirValue::Binary { op, left, right } if op == "&" => {
            let left = static_string_value_with_constants(left, constants, types)?;
            let right = static_string_value_with_constants(right, constants, types)?;
            Some(format!("{left}{right}"))
        }
        _ => None,
    }
}

fn strings_package_static_string_value(
    target: &str,
    args: &[NirValue],
    constants: &HashMap<String, NirValue>,
    types: &HashMap<String, String>,
) -> Option<String> {
    let value = args
        .first()
        .and_then(|arg| static_string_value_with_constants(arg, constants, types))?;
    match target {
        "strings.upper" if args.len() == 1 => Some(crate::unicode_backend::upper(&value)),
        "strings.lower" if args.len() == 1 => Some(crate::unicode_backend::lower(&value)),
        "strings.caseFold" if args.len() == 1 => Some(crate::unicode_backend::case_fold(&value)),
        "strings.normalizeNfc" if args.len() == 1 => {
            Some(crate::unicode_backend::normalize_nfc(&value))
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
                "strings.trim"
                | "strings.trimStart"
                | "strings.trimEnd"
                | "strings.upper"
                | "strings.lower"
                | "strings.caseFold"
                | "strings.normalizeNfc"
                | "strings.join" => Some("String".to_string()),
                "strings.graphemes" | "strings.split" => Some("List OF String".to_string()),
                "strings.startsWith" | "strings.endsWith" | "strings.contains" => {
                    Some("Boolean".to_string())
                }
                "strings.byteLen" => Some("Integer".to_string()),
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

fn function_type_parts(type_: &str) -> Option<(Vec<String>, String)> {
    let rest = type_.strip_prefix("FUNC(")?;
    let (params, returns) = rest.split_once(") AS ")?;
    let params = if params.trim().is_empty() {
        Vec::new()
    } else {
        params.split(", ").map(str::to_string).collect()
    };
    Some((params, returns.to_string()))
}

fn builtin_function_symbol_for_type(name: &str, type_: &str) -> Option<String> {
    builtins::general::builtin_function_id_for_type(name, type_)?;
    Some(format!(
        "_mfb_builtin_{}_{}",
        nir::symbol_fragment(name),
        nir::symbol_fragment(type_)
    ))
}

fn builtin_function_refs(module: &NirModule) -> Vec<(String, String, String)> {
    let mut refs = Vec::new();
    let mut seen = HashSet::new();
    for function in &module.functions {
        collect_builtin_function_refs_in_ops(&function.body, &mut refs, &mut seen);
    }
    refs
}

fn collect_builtin_function_refs_in_ops(
    ops: &[NirOp],
    refs: &mut Vec<(String, String, String)>,
    seen: &mut HashSet<String>,
) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } => {
                if let Some(value) = value {
                    collect_builtin_function_refs_in_value(value, refs, seen);
                }
            }
            NirOp::Assign { value, .. } | NirOp::Eval { value } | NirOp::Fail { error: value } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
            NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_builtin_function_refs_in_value(value, refs, seen);
                }
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_builtin_function_refs_in_value(condition, refs, seen);
                collect_builtin_function_refs_in_ops(then_body, refs, seen);
                collect_builtin_function_refs_in_ops(else_body, refs, seen);
            }
            NirOp::Match { value, cases } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
                for case in cases {
                    if let NirMatchPattern::Value(pattern) = &case.pattern {
                        collect_builtin_function_refs_in_value(pattern, refs, seen);
                    }
                    collect_builtin_function_refs_in_ops(&case.body, refs, seen);
                }
            }
            NirOp::ForEach { iterable, body, .. } => {
                collect_builtin_function_refs_in_value(iterable, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
            NirOp::Using { value, body, .. } => {
                collect_builtin_function_refs_in_value(value, refs, seen);
                collect_builtin_function_refs_in_ops(body, refs, seen);
            }
        }
    }
}

fn collect_builtin_function_refs_in_value(
    value: &NirValue,
    refs: &mut Vec<(String, String, String)>,
    seen: &mut HashSet<String>,
) {
    match value {
        NirValue::FunctionRef { name, type_ } => {
            if let Some(symbol) = builtin_function_symbol_for_type(name, type_) {
                let key = format!("{name}\0{type_}");
                if seen.insert(key) {
                    refs.push((name.clone(), type_.clone(), symbol));
                }
            }
        }
        NirValue::Call { args, .. } | NirValue::RuntimeCall { args, .. } => {
            for arg in args {
                collect_builtin_function_refs_in_value(arg, refs, seen);
            }
        }
        NirValue::Constructor { args, .. } => {
            for value in args {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::WithUpdate {
            target, updates, ..
        } => {
            collect_builtin_function_refs_in_value(target, refs, seen);
            for update in updates {
                collect_builtin_function_refs_in_value(&update.value, refs, seen);
            }
        }
        NirValue::ListLiteral { values, .. } => {
            for value in values {
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_builtin_function_refs_in_value(key, refs, seen);
                collect_builtin_function_refs_in_value(value, refs, seen);
            }
        }
        NirValue::Binary { left, right, .. } => {
            collect_builtin_function_refs_in_value(left, refs, seen);
            collect_builtin_function_refs_in_value(right, refs, seen);
        }
        NirValue::Unary { operand, .. } => {
            collect_builtin_function_refs_in_value(operand, refs, seen);
        }
        NirValue::MemberAccess { target, .. } => {
            collect_builtin_function_refs_in_value(target, refs, seen);
        }
        NirValue::Const { .. } | NirValue::Local(_) => {}
    }
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
