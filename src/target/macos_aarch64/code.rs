use std::collections::HashMap;

use crate::json_string;

use super::nir::{self, NirFunction, NirMatchPattern, NirModule, NirOp, NirValue};
use super::plan::NativePlan;
use super::runtime;

pub(crate) struct NativeCodePlan {
    pub(crate) target: String,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodeOp {
    Label,
    Mov,
    MovImm,
    Add,
    AddImm,
    SubSp,
    AddSp,
    CmpImm,
    BranchEq,
    Branch,
    BranchLink,
    BranchSelf,
    Svc,
    Ret,
    LdrU64,
    StrU64,
    Adrp,
    AddPageOff,
}

impl CodeOp {
    pub(crate) fn mnemonic(self) -> &'static str {
        match self {
            CodeOp::Label => "label",
            CodeOp::Mov => "mov",
            CodeOp::MovImm => "mov_imm",
            CodeOp::Add => "add",
            CodeOp::AddImm => "add_imm",
            CodeOp::SubSp => "sub_sp",
            CodeOp::AddSp => "add_sp",
            CodeOp::CmpImm => "cmp_imm",
            CodeOp::BranchEq => "b.eq",
            CodeOp::Branch => "b",
            CodeOp::BranchLink => "bl",
            CodeOp::BranchSelf => "branch_self",
            CodeOp::Svc => "svc",
            CodeOp::Ret => "ret",
            CodeOp::LdrU64 => "ldr_u64",
            CodeOp::StrU64 => "str_u64",
            CodeOp::Adrp => "adrp",
            CodeOp::AddPageOff => "add_pageoff",
        }
    }

    fn from_mnemonic(op: &str) -> Result<Self, String> {
        match op {
            "label" => Ok(CodeOp::Label),
            "mov" => Ok(CodeOp::Mov),
            "mov_imm" => Ok(CodeOp::MovImm),
            "add" => Ok(CodeOp::Add),
            "add_imm" => Ok(CodeOp::AddImm),
            "sub_sp" => Ok(CodeOp::SubSp),
            "add_sp" => Ok(CodeOp::AddSp),
            "cmp_imm" => Ok(CodeOp::CmpImm),
            "b.eq" => Ok(CodeOp::BranchEq),
            "b" => Ok(CodeOp::Branch),
            "bl" => Ok(CodeOp::BranchLink),
            "branch_self" => Ok(CodeOp::BranchSelf),
            "svc" => Ok(CodeOp::Svc),
            "ret" => Ok(CodeOp::Ret),
            "ldr_u64" => Ok(CodeOp::LdrU64),
            "str_u64" => Ok(CodeOp::StrU64),
            "adrp" => Ok(CodeOp::Adrp),
            "add_pageoff" => Ok(CodeOp::AddPageOff),
            other => Err(format!("native code op '{other}' is not encodable")),
        }
    }
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CodegenPlatform {
    MacosAarch64,
    LinuxAarch64,
}

impl CodegenPlatform {
    fn from_target(target: &str) -> Result<Self, String> {
        match target {
            "macos-aarch64" => Ok(Self::MacosAarch64),
            "linux-aarch64" => Ok(Self::LinuxAarch64),
            other => Err(format!(
                "native code plan target '{other}' does not match a supported aarch64 target"
            )),
        }
    }
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
    location: String,
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

pub(crate) fn lower_module(
    module: &NirModule,
    native_plan: &NativePlan,
) -> Result<NativeCodePlan, String> {
    lower_module_for_platform(
        module,
        native_plan,
        CodegenPlatform::from_target(&module.target)?,
    )
}

pub(crate) fn lower_module_for_platform(
    module: &NirModule,
    native_plan: &NativePlan,
    platform: CodegenPlatform,
) -> Result<NativeCodePlan, String> {
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
    let data_objects = string_symbols
        .iter()
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
        code_functions.push(lower_program_entry(&entry_symbol, &entry.returns, platform));
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
    for symbol in &native_plan.runtime_symbols {
        code_functions.push(lower_runtime_helper(symbol, &platform_imports, platform)?);
    }

    let plan = NativeCodePlan {
        target: module.target.clone(),
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
        if !matches!(self.target.as_str(), "macos-aarch64" | "linux-aarch64") {
            return Err(format!(
                "native code plan target '{}' does not match a supported aarch64 target",
                self.target
            ));
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
                "  \"arch\": \"aarch64\",\n",
                "  \"project\": {},\n",
                "  \"entrySymbol\": {},\n",
                "  \"imports\": [{}\n  ],\n",
                "  \"dataObjects\": [{}\n  ],\n",
                "  \"functions\": [{}\n  ]\n",
                "}}\n"
            ),
            json_string(&self.target),
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
    platform: CodegenPlatform,
) -> CodeFunction {
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        CodeInstruction::new("bl").field("target", language_entry_symbol),
    ];
    if language_entry_returns == "Nothing" {
        instructions.push(
            CodeInstruction::new("mov_imm")
                .field("dst", "x0")
                .field("type", "Integer")
                .field("value", "0"),
        );
    }
    let mut relocations = vec![CodeRelocation {
        from: "_main".to_string(),
        to: language_entry_symbol.to_string(),
        kind: "branch26".to_string(),
        binding: "internal".to_string(),
        library: None,
    }];
    match platform {
        CodegenPlatform::MacosAarch64 => {
            instructions.extend([
                CodeInstruction::new("bl").field("target", "_exit"),
                CodeInstruction::new("branch_self"),
                CodeInstruction::new("ret"),
            ]);
            relocations.push(CodeRelocation {
                from: "_main".to_string(),
                to: "_exit".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some("libSystem".to_string()),
            });
        }
        CodegenPlatform::LinuxAarch64 => {
            instructions.extend([
                CodeInstruction::new("mov_imm")
                    .field("dst", "x8")
                    .field("type", "Integer")
                    .field("value", "93"),
                CodeInstruction::new("svc"),
                CodeInstruction::new("branch_self"),
                CodeInstruction::new("ret"),
            ]);
        }
    }
    CodeFunction {
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
    }
}

fn lower_function(
    function: &NirFunction,
    function_symbols: &HashMap<String, String>,
    functions: &HashMap<String, &NirFunction>,
    platform_imports: &HashMap<String, String>,
    string_symbols: &HashMap<String, String>,
    type_model: TypeModel,
) -> Result<CodeFunction, String> {
    let mut locals = HashMap::new();
    let params = function
        .params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let location = argument_register(index)?;
            locals.insert(
                param.name.clone(),
                LocalValue {
                    type_: param.type_.clone(),
                    location: location.clone(),
                },
            );
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
        locals,
        instructions: vec![CodeInstruction::new("label").field("name", "entry")],
        relocations: Vec::new(),
        stack_slots: Vec::new(),
        used_callee_saved: Vec::new(),
        stack_size: 0,
        next_register: 8,
        next_label: 0,
    };
    builder.lower_ops(&function.body)?;
    if !builder
        .instructions
        .iter()
        .any(|instruction| instruction.op == CodeOp::Ret)
    {
        builder.emit(CodeInstruction::new("ret"));
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
    platform: CodegenPlatform,
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
    platform: CodegenPlatform,
) -> Result<(CodeFrame, Vec<CodeInstruction>, Vec<CodeRelocation>), String> {
    let mut instructions = vec![
        CodeInstruction::new("label").field("name", "entry"),
        CodeInstruction::new("sub_sp").field("imm", "16"),
    ];
    let mut relocations = Vec::new();
    if platform == CodegenPlatform::MacosAarch64 {
        instructions.push(
            CodeInstruction::new("str_u64")
                .field("src", "x30")
                .field("base", "sp")
                .field("offset", "0"),
        );
    }
    instructions.extend([
        CodeInstruction::new("ldr_u64")
            .field("dst", "x2")
            .field("base", "x0")
            .field("offset", "0"),
        CodeInstruction::new("add_imm")
            .field("dst", "x1")
            .field("src", "x0")
            .field("imm", "8"),
        CodeInstruction::new("mov_imm")
            .field("dst", "x0")
            .field("type", "Integer")
            .field("value", "1"),
    ]);
    emit_platform_write(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    instructions.extend([
        CodeInstruction::new("mov_imm")
            .field("dst", "x9")
            .field("type", "Integer")
            .field("value", "10"),
        CodeInstruction::new("str_u64")
            .field("src", "x9")
            .field("base", "sp")
            .field("offset", "8"),
        CodeInstruction::new("mov_imm")
            .field("dst", "x0")
            .field("type", "Integer")
            .field("value", "1"),
        CodeInstruction::new("add_imm")
            .field("dst", "x1")
            .field("src", "sp")
            .field("imm", "8"),
        CodeInstruction::new("mov_imm")
            .field("dst", "x2")
            .field("type", "Integer")
            .field("value", "1"),
    ]);
    emit_platform_write(
        symbol,
        platform_imports,
        platform,
        &mut instructions,
        &mut relocations,
    )?;
    if platform == CodegenPlatform::MacosAarch64 {
        instructions.extend([
            CodeInstruction::new("ldr_u64")
                .field("dst", "x30")
                .field("base", "sp")
                .field("offset", "0"),
            CodeInstruction::new("add_sp").field("imm", "16"),
            CodeInstruction::new("ret"),
        ]);
        Ok((
            CodeFrame {
                stack_size: 16,
                callee_saved: vec!["x30".to_string()],
            },
            instructions,
            relocations,
        ))
    } else {
        instructions.extend([
            CodeInstruction::new("add_sp").field("imm", "16"),
            CodeInstruction::new("ret"),
        ]);
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

fn emit_platform_write(
    from: &str,
    platform_imports: &HashMap<String, String>,
    platform: CodegenPlatform,
    instructions: &mut Vec<CodeInstruction>,
    relocations: &mut Vec<CodeRelocation>,
) -> Result<(), String> {
    match platform {
        CodegenPlatform::MacosAarch64 => {
            let library = platform_imports
                .get("_write")
                .ok_or_else(|| "io.print runtime helper requires _write import".to_string())?
                .clone();
            instructions.push(CodeInstruction::new("bl").field("target", "_write"));
            relocations.push(CodeRelocation {
                from: from.to_string(),
                to: "_write".to_string(),
                kind: "branch26".to_string(),
                binding: "external".to_string(),
                library: Some(library),
            });
        }
        CodegenPlatform::LinuxAarch64 => {
            instructions.extend([
                CodeInstruction::new("mov_imm")
                    .field("dst", "x8")
                    .field("type", "Integer")
                    .field("value", "64"),
                CodeInstruction::new("svc"),
            ]);
        }
    }
    Ok(())
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
        && !callee_saved.iter().any(|register| register == "x30")
    {
        callee_saved.push("x30".to_string());
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
    prologue.push(CodeInstruction::new("sub_sp").field("imm", &total_stack_size.to_string()));
    for (index, register) in callee_saved.iter().enumerate() {
        prologue.push(
            CodeInstruction::new("str_u64")
                .field("src", register)
                .field("base", "sp")
                .field("offset", &(index * 8).to_string()),
        );
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
                rewritten.push(
                    CodeInstruction::new("ldr_u64")
                        .field("dst", register)
                        .field("base", "sp")
                        .field("offset", &(index * 8).to_string()),
                );
            }
            rewritten
                .push(CodeInstruction::new("add_sp").field("imm", &total_stack_size.to_string()));
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
            .any(|(name, value)| matches!(*name, "base" | "src") && value == "sp");
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
                    let register = self.allocate_register();
                    self.locals.insert(
                        name.clone(),
                        LocalValue {
                            type_: type_.clone(),
                            location: register.clone(),
                        },
                    );
                    self.allocate_stack_object(name, 8);
                    if let Some(value) = value {
                        let result = self.lower_value(value)?;
                        self.emit(
                            CodeInstruction::new("mov")
                                .field("dst", &register)
                                .field("src", &result.location),
                        );
                    }
                }
                NirOp::Assign { name, value } => {
                    let dst = self
                        .locals
                        .get(name)
                        .ok_or_else(|| format!("native code assignment unknown local '{name}'"))?
                        .location
                        .clone();
                    let result = self.lower_value(value)?;
                    self.emit(
                        CodeInstruction::new("mov")
                            .field("dst", &dst)
                            .field("src", &result.location),
                    );
                }
                NirOp::Eval { value } => {
                    self.lower_value(value)?;
                }
                NirOp::Return { value } => {
                    if let Some(value) = value {
                        let result = self.lower_value(value)?;
                        self.emit(
                            CodeInstruction::new("mov")
                                .field("dst", "x0")
                                .field("src", &result.location),
                        );
                    }
                    self.emit(CodeInstruction::new("ret"));
                }
                NirOp::If {
                    condition,
                    then_body,
                    else_body,
                } => {
                    let condition = self.lower_value(condition)?;
                    let else_label = self.label("if_else");
                    let end_label = self.label("if_end");
                    self.emit(
                        CodeInstruction::new("cmp_imm")
                            .field("lhs", &condition.location)
                            .field("rhs", "0"),
                    );
                    self.emit(
                        CodeInstruction::new("b.eq")
                            .field("target", &else_label)
                            .field("reason", "ifFalse"),
                    );
                    self.lower_ops(then_body)?;
                    if !self.current_block_returns() {
                        self.emit(CodeInstruction::new("b").field("target", &end_label));
                    }
                    self.emit(CodeInstruction::new("label").field("name", &else_label));
                    self.lower_ops(else_body)?;
                    self.emit(CodeInstruction::new("label").field("name", &end_label));
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
                    self.emit(
                        CodeInstruction::new("b")
                            .field("target", else_label.as_deref().unwrap_or(&end_label)),
                    );
                    for (label, case) in case_labels {
                        self.emit(CodeInstruction::new("label").field("name", &label));
                        self.lower_ops(&case.body)?;
                        if !self.current_block_returns() {
                            self.emit(CodeInstruction::new("b").field("target", &end_label));
                        }
                    }
                    self.emit(CodeInstruction::new("label").field("name", &end_label));
                }
                NirOp::Using {
                    name,
                    type_,
                    close,
                    value,
                    body,
                } => {
                    let register = self.allocate_register();
                    let result = self.lower_value(value)?;
                    self.locals.insert(
                        name.clone(),
                        LocalValue {
                            type_: type_.clone(),
                            location: register.clone(),
                        },
                    );
                    self.emit(
                        CodeInstruction::new("mov")
                            .field("dst", &register)
                            .field("src", &result.location),
                    );
                    self.lower_ops(body)?;
                    let symbol = self
                        .function_symbols
                        .get(close)
                        .cloned()
                        .unwrap_or_else(|| close.clone());
                    self.emit_call(close, &symbol, &[], None)?;
                }
            }
        }
        Ok(())
    }

    fn lower_value(&mut self, value: &NirValue) -> Result<ValueResult, String> {
        match value {
            NirValue::Const { type_, value } => {
                let register = self.allocate_register();
                if type_ == "String" {
                    let symbol = self
                        .string_symbols
                        .get(value)
                        .ok_or_else(|| {
                            format!("native code string literal '{value}' has no data object")
                        })?
                        .clone();
                    self.emit(
                        CodeInstruction::new("adrp")
                            .field("dst", &register)
                            .field("symbol", &symbol),
                    );
                    self.relocations.push(CodeRelocation {
                        from: self.current_symbol.clone(),
                        to: symbol.clone(),
                        kind: "page21".to_string(),
                        binding: "data".to_string(),
                        library: None,
                    });
                    self.emit(
                        CodeInstruction::new("add_pageoff")
                            .field("dst", &register)
                            .field("src", &register)
                            .field("symbol", &symbol),
                    );
                    self.relocations.push(CodeRelocation {
                        from: self.current_symbol.clone(),
                        to: symbol,
                        kind: "pageoff12".to_string(),
                        binding: "data".to_string(),
                        library: None,
                    });
                } else {
                    self.emit(
                        CodeInstruction::new("mov_imm")
                            .field("dst", &register)
                            .field("type", type_)
                            .field("value", value),
                    );
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
                Ok(ValueResult {
                    type_: local.type_.clone(),
                    location: local.location.clone(),
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
            } => self.emit_call(
                target,
                &runtime::symbol_for_call(*helper, target),
                args,
                Some("Nothing"),
            ),
            NirValue::Constructor { type_, args } => {
                let arg_values = args
                    .iter()
                    .map(|arg| self.lower_value(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                let register = self.allocate_register();
                let tag = self
                    .type_model
                    .union_variant_tags
                    .get(type_)
                    .copied()
                    .ok_or_else(|| {
                        format!("native code union variant '{type_}' does not resolve")
                    })?;
                let object_offset = self.allocate_stack_object(type_, 8 * (arg_values.len() + 1));
                let tag_register = self.allocate_register();
                self.emit(
                    CodeInstruction::new("mov_imm")
                        .field("dst", &tag_register)
                        .field("type", "UnionTag")
                        .field("value", &tag.to_string()),
                );
                self.emit(
                    CodeInstruction::new("str_u64")
                        .field("src", &tag_register)
                        .field("base", "sp")
                        .field("offset", &object_offset.to_string()),
                );
                for (index, arg) in arg_values.iter().enumerate() {
                    self.emit(
                        CodeInstruction::new("str_u64")
                            .field("src", &arg.location)
                            .field("base", "sp")
                            .field("offset", &(object_offset + 8 * (index + 1)).to_string()),
                    );
                }
                self.emit(
                    CodeInstruction::new("add_imm")
                        .field("dst", &register)
                        .field("src", "sp")
                        .field("imm", &object_offset.to_string()),
                );
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
                    let register = self.allocate_register();
                    self.emit(
                        CodeInstruction::new("mov_imm")
                            .field("dst", &register)
                            .field("type", "EnumOrdinal")
                            .field("value", &ordinal.to_string()),
                    );
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
                let left = self.lower_value(left)?;
                let right = self.lower_value(right)?;
                let register = self.allocate_register();
                let instruction = match op.as_str() {
                    "+" => "add",
                    other => {
                        return Err(format!(
                            "native code plan does not lower binary operator '{other}' yet"
                        ));
                    }
                };
                self.emit(
                    CodeInstruction::new(instruction)
                        .field("dst", &register)
                        .field("lhs", &left.location)
                        .field("rhs", &right.location),
                );
                Ok(ValueResult {
                    type_: left.type_.clone(),
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
                self.emit(
                    CodeInstruction::new("cmp_imm")
                        .field("lhs", &matched.location)
                        .field("rhs", &ordinal.to_string()),
                );
                self.emit(CodeInstruction::new("b.eq").field("target", label));
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
                let tag_register = self.allocate_register();
                self.emit(
                    CodeInstruction::new("ldr_u64")
                        .field("dst", &tag_register)
                        .field("base", &matched.location)
                        .field("offset", "0"),
                );
                self.emit(
                    CodeInstruction::new("cmp_imm")
                        .field("lhs", &tag_register)
                        .field("rhs", &tag.to_string()),
                );
                self.emit(CodeInstruction::new("b.eq").field("target", label));
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
            self.emit(
                CodeInstruction::new("mov")
                    .field("dst", &argument_register(index)?)
                    .field("src", &arg.location),
            );
        }
        self.emit(CodeInstruction::new("bl").field("target", symbol));
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
        let register = self.allocate_register();
        self.emit(
            CodeInstruction::new("mov")
                .field("dst", &register)
                .field("src", "x0"),
        );
        Ok(ValueResult {
            type_: result_type,
            location: register,
            text: format!("call {target}({})", join_texts(&arg_values)),
        })
    }

    fn allocate_register(&mut self) -> String {
        let register = match self.next_register {
            8..=17 => format!("x{}", self.next_register),
            18 => "x19".to_string(),
            19 => "x20".to_string(),
            20 => "x21".to_string(),
            21 => "x22".to_string(),
            22 => "x23".to_string(),
            23 => "x24".to_string(),
            24 => "x25".to_string(),
            25 => "x26".to_string(),
            26 => "x27".to_string(),
            27 => "x28".to_string(),
            other => panic!("native code plan exhausted physical registers at allocation {other}"),
        };
        self.next_register += 1;
        if is_callee_saved(&register) && !self.used_callee_saved.contains(&register) {
            self.used_callee_saved.push(register.clone());
        }
        register
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

    fn current_block_returns(&self) -> bool {
        self.instructions
            .last()
            .is_some_and(|instruction| instruction.op == CodeOp::Ret)
    }
}

impl CodeInstruction {
    fn new(op: &str) -> Self {
        Self {
            op: CodeOp::from_mnemonic(op).unwrap_or_else(|err| panic!("{err}")),
            fields: Vec::new(),
        }
    }

    fn field(mut self, name: &'static str, value: &str) -> Self {
        self.fields.push((name, value.to_string()));
        self
    }

    fn validate(&self) -> Result<(), String> {
        let required: &[&str] = match self.op {
            CodeOp::Label => &["name"],
            CodeOp::Mov => &["dst", "src"],
            CodeOp::MovImm => &["dst", "value"],
            CodeOp::Add => &["dst", "lhs", "rhs"],
            CodeOp::AddImm => &["dst", "src", "imm"],
            CodeOp::SubSp | CodeOp::AddSp => &["imm"],
            CodeOp::CmpImm => &["lhs", "rhs"],
            CodeOp::BranchEq | CodeOp::Branch | CodeOp::BranchLink => &["target"],
            CodeOp::BranchSelf | CodeOp::Svc | CodeOp::Ret => &[],
            CodeOp::LdrU64 => &["dst", "base", "offset"],
            CodeOp::StrU64 => &["src", "base", "offset"],
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

fn argument_register(index: usize) -> Result<String, String> {
    if index < 8 {
        Ok(format!("x{index}"))
    } else {
        Err(format!(
            "native code plan cannot pass argument {index}; stack arguments are not implemented"
        ))
    }
}

fn string_symbols(module: &NirModule) -> HashMap<String, String> {
    let mut values = Vec::new();
    for function in &module.functions {
        collect_string_values_from_ops(&function.body, &mut values);
    }
    values
        .into_iter()
        .enumerate()
        .map(|(index, value)| (value, format!("_mfb_str_{index}")))
        .collect()
}

fn collect_string_values_from_ops(ops: &[NirOp], values: &mut Vec<String>) {
    for op in ops {
        match op {
            NirOp::Bind { value, .. } | NirOp::Return { value } => {
                if let Some(value) = value {
                    collect_string_values_from_value(value, values);
                }
            }
            NirOp::Assign { value, .. } | NirOp::Eval { value } => {
                collect_string_values_from_value(value, values);
            }
            NirOp::If {
                condition,
                then_body,
                else_body,
            } => {
                collect_string_values_from_value(condition, values);
                collect_string_values_from_ops(then_body, values);
                collect_string_values_from_ops(else_body, values);
            }
            NirOp::Match { value, cases } => {
                collect_string_values_from_value(value, values);
                for case in cases {
                    if let NirMatchPattern::Value(value) = &case.pattern {
                        collect_string_values_from_value(value, values);
                    }
                    collect_string_values_from_ops(&case.body, values);
                }
            }
            NirOp::Using { value, body, .. } => {
                collect_string_values_from_value(value, values);
                collect_string_values_from_ops(body, values);
            }
        }
    }
}

fn collect_string_values_from_value(value: &NirValue, values: &mut Vec<String>) {
    match value {
        NirValue::Const { type_, value } if type_ == "String" => {
            if !values.contains(value) {
                values.push(value.clone());
            }
        }
        NirValue::Call { args, .. }
        | NirValue::RuntimeCall { args, .. }
        | NirValue::Constructor { args, .. } => {
            for arg in args {
                collect_string_values_from_value(arg, values);
            }
        }
        NirValue::ListLiteral { values: items, .. } => {
            for item in items {
                collect_string_values_from_value(item, values);
            }
        }
        NirValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                collect_string_values_from_value(key, values);
                collect_string_values_from_value(value, values);
            }
        }
        NirValue::MemberAccess { target, .. } => collect_string_values_from_value(target, values),
        NirValue::Binary { left, right, .. } => {
            collect_string_values_from_value(left, values);
            collect_string_values_from_value(right, values);
        }
        NirValue::Unary { operand, .. } => collect_string_values_from_value(operand, values),
        NirValue::Const { .. } | NirValue::Local(_) | NirValue::FunctionRef { .. } => {}
    }
}

fn align(value: usize, alignment: usize) -> usize {
    if alignment == 0 {
        return value;
    }
    value.div_ceil(alignment) * alignment
}

fn is_callee_saved(register: &str) -> bool {
    matches!(
        register,
        "x19" | "x20" | "x21" | "x22" | "x23" | "x24" | "x25" | "x26" | "x27" | "x28"
    )
}

fn join_texts(values: &[ValueResult]) -> String {
    values
        .iter()
        .map(|value| value.text.clone())
        .collect::<Vec<_>>()
        .join(", ")
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
