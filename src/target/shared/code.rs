use std::collections::HashMap;

use crate::arch::aarch64::{abi, ops::CodeOp};
use crate::json_string;
use crate::numeric;

use super::nir::{self, NirFunction, NirMatchPattern, NirModule, NirOp, NirValue};
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
const ERR_OUT_OF_MEMORY_CODE: &str = "10010";

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
struct TypeModel {
    enum_members: HashMap<(String, String), usize>,
    union_variants: HashMap<String, String>,
    union_variant_tags: HashMap<String, usize>,
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
                    if !data_symbols.contains(&relocation.to) {
                        return Err(format!(
                            "native code data relocation target '{}' is not a data object",
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
        let mut union_variants = HashMap::new();
        let mut union_variant_tags = HashMap::new();
        for type_ in &module.types {
            match type_.kind.as_str() {
                "enum" => {
                    for (index, member) in type_.members.iter().enumerate() {
                        enum_members.insert((type_.name.clone(), member.name.clone()), index);
                    }
                }
                "union" => {
                    for (index, variant) in type_.variants.iter().enumerate() {
                        union_variants.insert(variant.name.clone(), type_.name.clone());
                        union_variant_tags.insert(variant.name.clone(), index);
                    }
                }
                "record" | "resource" => {}
                other => {
                    return Err(format!(
                        "native code plan does not know type kind '{other}'"
                    ));
                }
            }
        }
        Ok(Self {
            enum_members,
            union_variants,
            union_variant_tags,
        })
    }
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
    if !builder
        .instructions
        .iter()
        .any(|instruction| instruction.op == CodeOp::Ret)
    {
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
    if instructions
        .iter()
        .any(|instruction| instruction.op == CodeOp::BranchLink)
        && !callee_saved
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
                        self.emit(abi::move_register(RESULT_VALUE_REGISTER, &result.location));
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
            NirValue::FunctionRef { name, type_ } => Ok(ValueResult {
                type_: type_.clone(),
                location: self
                    .function_symbols
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| name.clone()),
                text: name.clone(),
            }),
            NirValue::Call { target, args } => {
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
                    let arg = self.lower_value(&args[0])?;
                    if arg.type_ == "Byte" {
                        let register = self.allocate_register()?;
                        self.emit(abi::move_register(&register, &arg.location));
                        return Ok(ValueResult {
                            type_: "Integer".to_string(),
                            location: register,
                            text: format!("toInt({})", arg.text),
                        });
                    }
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
            NirValue::MemberAccess { target, member } => match target.as_ref() {
                NirValue::Local(type_name) => {
                    let ordinal = self
                        .type_model
                        .enum_members
                        .get(&(type_name.clone(), member.clone()))
                        .copied()
                        .ok_or_else(|| {
                            format!(
                                "native code enum member '{type_name}.{member}' does not resolve"
                            )
                        })?;
                    let register = self.allocate_register()?;
                    self.emit(abi::move_immediate(
                        &register,
                        "EnumOrdinal",
                        &ordinal.to_string(),
                    ));
                    Ok(ValueResult {
                        type_: type_name.clone(),
                        location: register,
                        text: format!("{type_name}.{member}"),
                    })
                }
                _ => Err(format!(
                    "native code plan does not lower member access '{}'",
                    member
                )),
            },
            NirValue::Binary { op, left, right } => {
                if op == "&" {
                    return self.lower_string_concat(left, right);
                }
                let left = self.lower_value(left)?;
                let right = self.lower_value(right)?;
                let register = self.allocate_register()?;
                match op.as_str() {
                    "+" => {
                        self.emit(abi::add_registers(
                            &register,
                            &left.location,
                            &right.location,
                        ));
                        if left.type_ == "Byte" && right.type_ == "Byte" {
                            let overflow_label = self.label("byte_overflow");
                            let ok_label = self.label("byte_ok");
                            self.emit(abi::compare_immediate(&register, "255"));
                            self.emit(abi::branch_hi(&overflow_label));
                            self.emit(abi::branch(&ok_label));
                            self.emit(abi::label(&overflow_label));
                            self.emit_overflow_return()?;
                            self.emit(abi::label(&ok_label));
                        }
                    }
                    "-" => {
                        if left.type_ == "Byte" && right.type_ == "Byte" {
                            let underflow_label = self.label("byte_underflow");
                            let ok_label = self.label("byte_ok");
                            self.emit(abi::compare_registers(&left.location, &right.location));
                            self.emit(abi::branch_lo(&underflow_label));
                            self.emit(abi::subtract_registers(
                                &register,
                                &left.location,
                                &right.location,
                            ));
                            self.emit(abi::branch(&ok_label));
                            self.emit(abi::label(&underflow_label));
                            self.emit_underflow_return()?;
                            self.emit(abi::label(&ok_label));
                        } else {
                            self.emit(abi::subtract_registers(
                                &register,
                                &left.location,
                                &right.location,
                            ));
                        }
                    }
                    other => {
                        return Err(format!(
                            "native code plan does not lower binary operator '{other}' yet"
                        ));
                    }
                };
                Ok(ValueResult {
                    type_: numeric_binary_result_type(op, &left.type_, &right.type_).to_string(),
                    location: register,
                    text: format!("({} {op} {})", left.text, right.text),
                })
            }
            NirValue::Unary { op, operand } => {
                let _ = operand;
                Err(format!(
                    "native code plan does not lower unary operator '{op}' yet"
                ))
            }
            NirValue::ListLiteral { .. } | NirValue::MapLiteral { .. } => {
                Err("native code plan does not lower list/map literals yet".to_string())
            }
        }
    }

    fn load_string_constant(&mut self, value: &str) -> Result<String, String> {
        let register = self.allocate_register()?;
        self.emit_load_string_constant(&register, value)?;
        Ok(register)
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
            | NirValue::ListLiteral { type_, .. }
            | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
            NirValue::Call { target, .. } | NirValue::RuntimeCall { target, .. } => {
                match target.as_str() {
                    "typeName" | "toString" => Some("String".to_string()),
                    "toInt" => Some("Integer".to_string()),
                    "toFloat" => Some("Float".to_string()),
                    "toFixed" => Some("Fixed".to_string()),
                    "toByte" => Some("Byte".to_string()),
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
            NirValue::MemberAccess { .. } => None,
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
        if abi::is_callee_saved(&register) && !self.used_callee_saved.contains(&register) {
            self.used_callee_saved.push(register.clone());
        }
        Ok(register)
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
        let message_register = self.load_string_address(ERR_OVERFLOW_MESSAGE)?;
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            ERR_OVERFLOW_CODE,
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_MESSAGE_REGISTER,
            &message_register,
        ));
        self.emit(abi::return_());
        Ok(())
    }

    fn emit_underflow_return(&mut self) -> Result<(), String> {
        let message_register = self.load_string_address(ERR_UNDERFLOW_MESSAGE)?;
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            ERR_UNDERFLOW_CODE,
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_MESSAGE_REGISTER,
            &message_register,
        ));
        self.emit(abi::return_());
        Ok(())
    }

    fn emit_allocation_error_return(&mut self) -> Result<(), String> {
        let message_register = self.load_string_address(ERR_ALLOCATION_MESSAGE)?;
        self.emit(abi::move_register(
            RESULT_VALUE_REGISTER,
            RESULT_TAG_REGISTER,
        ));
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
        let message_register = self.load_string_address(message)?;
        self.emit(abi::move_immediate(
            RESULT_TAG_REGISTER,
            "Integer",
            RESULT_ERR_TAG,
        ));
        self.emit(abi::move_immediate(
            RESULT_VALUE_REGISTER,
            "Integer",
            &(code as u64).to_string(),
        ));
        self.emit(abi::move_register(
            RESULT_ERROR_MESSAGE_REGISTER,
            &message_register,
        ));
        self.emit(abi::return_());
        Ok(())
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
            .last()
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
            CodeOp::Add | CodeOp::Sub | CodeOp::And | CodeOp::UDiv => &["dst", "lhs", "rhs"],
            CodeOp::Mvn => &["dst", "src"],
            CodeOp::MSub => &["dst", "lhs", "rhs", "minuend"],
            CodeOp::AddImm | CodeOp::SubImm => &["dst", "src", "imm"],
            CodeOp::SubSp | CodeOp::AddSp => &["imm"],
            CodeOp::CmpImm => &["lhs", "rhs"],
            CodeOp::Cmp => &["lhs", "rhs"],
            CodeOp::BranchEq
            | CodeOp::BranchNe
            | CodeOp::BranchGe
            | CodeOp::BranchHi
            | CodeOp::BranchLo
            | CodeOp::Branch
            | CodeOp::BranchLink => &["target"],
            CodeOp::BranchSelf | CodeOp::Svc | CodeOp::Ret => &[],
            CodeOp::LdrU64 | CodeOp::LdrU8 => &["dst", "base", "offset"],
            CodeOp::StrU64 | CodeOp::StrU8 => &["src", "base", "offset"],
            CodeOp::Adrp | CodeOp::AddPageOff => &["dst", "symbol"],
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
    if !values.contains(&ERR_OVERFLOW_MESSAGE.to_string()) {
        values.push(ERR_OVERFLOW_MESSAGE.to_string());
    }
    if !values.contains(&ERR_UNDERFLOW_MESSAGE.to_string()) {
        values.push(ERR_UNDERFLOW_MESSAGE.to_string());
    }
    if !values.contains(&ERR_ALLOCATION_MESSAGE.to_string()) {
        values.push(ERR_ALLOCATION_MESSAGE.to_string());
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
            let symbol = if value == ERR_OVERFLOW_MESSAGE {
                ERR_OVERFLOW_SYMBOL.to_string()
            } else if value == ERR_UNDERFLOW_MESSAGE {
                ERR_UNDERFLOW_SYMBOL.to_string()
            } else if value == ERR_ALLOCATION_MESSAGE {
                ERR_ALLOCATION_SYMBOL.to_string()
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

fn module_uses_type_name(module: &NirModule) -> bool {
    module
        .functions
        .iter()
        .any(|function| ops_use_type_name(&function.body))
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
        | NirValue::ListLiteral { type_, .. }
        | NirValue::MapLiteral { type_, .. } => Some(type_.clone()),
        NirValue::Call { target, .. } | NirValue::RuntimeCall { target, .. } => {
            match target.as_str() {
                "typeName" | "toString" => Some("String".to_string()),
                "toInt" => Some("Integer".to_string()),
                "toFloat" => Some("Float".to_string()),
                "toFixed" => Some("Fixed".to_string()),
                "toByte" => Some("Byte".to_string()),
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
        NirValue::MemberAccess { .. } => None,
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
