use crate::ir::{IrFunction, IrOp, IrProject, IrType, IrValue};
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

const TYPE_NOTHING: u32 = 1;
const TYPE_BOOLEAN: u32 = 2;
const TYPE_INTEGER: u32 = 3;
const TYPE_FLOAT: u32 = 4;
const TYPE_FIXED: u32 = 5;
const TYPE_STRING: u32 = 6;

const FUNCTION_BYTECODE: u16 = 1;
const FUNCTION_BUILTIN: u16 = 4;

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
const OPCODE_CONCAT: u16 = 40;
const OPCODE_CALL_RESULT: u16 = 60;
const OPCODE_UNWRAP_RESULT: u16 = 61;
const OPCODE_RETURN_OK: u16 = 70;

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

pub struct NativePlan {
    pub prints: Vec<Vec<u8>>,
    pub exit_code: u8,
}

pub fn native_plan(ir: &IrProject) -> Result<NativePlan, String> {
    let entry = ir
        .entry
        .as_ref()
        .ok_or_else(|| "native executable output requires an executable entry point".to_string())?;
    let functions = ir
        .functions
        .iter()
        .map(|function| (function.name.as_str(), function))
        .collect::<HashMap<_, _>>();
    let entry_function = functions
        .get(entry.name.as_str())
        .ok_or_else(|| format!("entry function `{}` was not lowered to IR", entry.name))?;
    let mut evaluator = NativePlanEvaluator {
        functions: &functions,
        prints: Vec::new(),
    };
    let args = if entry.accepts_args {
        vec!["".to_string()]
    } else {
        Vec::new()
    };
    let returned = evaluator.eval_function(entry_function, args)?;
    let exit_code = if entry.returns == "Integer" {
        returned
            .ok_or_else(|| format!("FUNC entry `{}` did not return an Integer", entry.name))?
            .parse::<u8>()
            .map_err(|_| {
                format!(
                    "native build requires FUNC entry `{}` to return an Integer in the process exit-code range 0..255",
                    entry.name
                )
            })?
    } else {
        0
    };
    Ok(NativePlan {
        prints: evaluator.prints,
        exit_code,
    })
}

struct NativePlanEvaluator<'a> {
    functions: &'a HashMap<&'a str, &'a IrFunction>,
    prints: Vec<Vec<u8>>,
}

impl<'a> NativePlanEvaluator<'a> {
    fn eval_function(
        &mut self,
        function: &'a IrFunction,
        args: Vec<String>,
    ) -> Result<Option<String>, String> {
        let mut locals = HashMap::new();
        for (index, param) in function.params.iter().enumerate() {
            let value = if let Some(value) = args.get(index) {
                value.clone()
            } else if let Some(default) = &param.default {
                self.eval_string(default, &locals)?
            } else {
                return Err(format!(
                    "native build cannot call `{}` without argument `{}`",
                    function.name, param.name
                ));
            };
            locals.insert(param.name.clone(), value);
        }

        for op in &function.body {
            match op {
                IrOp::Bind { name, value, .. } => {
                    let value = match value {
                        Some(value) => self.eval_string(value, &locals)?,
                        None => String::new(),
                    };
                    locals.insert(name.clone(), value);
                }
                IrOp::Return { value } => {
                    return match value {
                        Some(value) => Ok(Some(self.eval_string(value, &locals)?)),
                        None => Ok(None),
                    };
                }
                IrOp::Eval { value } => {
                    self.eval_effect(value, &locals)?;
                }
            }
        }

        Ok(None)
    }

    fn eval_effect(
        &mut self,
        value: &'a IrValue,
        locals: &HashMap<String, String>,
    ) -> Result<(), String> {
        match value {
            IrValue::Call { target, args } if target == "io.print" => {
                let Some(arg) = args.first() else {
                    return Err(
                        "native build requires io.print to receive one argument".to_string()
                    );
                };
                let mut value = self.eval_string(arg, locals)?.into_bytes();
                value.push(b'\n');
                self.prints.push(value);
                Ok(())
            }
            IrValue::Call { .. } => {
                self.eval_string(value, locals)?;
                Ok(())
            }
            IrValue::Local(_) | IrValue::Const { .. } | IrValue::Binary { .. } => {
                self.eval_string(value, locals)?;
                Ok(())
            }
        }
    }

    fn eval_string(
        &mut self,
        value: &'a IrValue,
        locals: &HashMap<String, String>,
    ) -> Result<String, String> {
        match value {
            IrValue::Const { type_, value } if type_ == "String" => Ok(value.clone()),
            IrValue::Const { type_, value } if type_ == "Integer" => Ok(value.clone()),
            IrValue::Local(name) => locals
                .get(name)
                .cloned()
                .ok_or_else(|| format!("native build references unknown local `{name}`")),
            IrValue::Binary { op, left, right } if op == "&" => {
                let mut value = self.eval_string(left, locals)?;
                value.push_str(&self.eval_string(right, locals)?);
                Ok(value)
            }
            IrValue::Call { target, args } => {
                let function = self.functions.get(target.as_str()).ok_or_else(|| {
                    format!("native build cannot call unknown function `{target}`")
                })?;
                let args = args
                    .iter()
                    .map(|arg| self.eval_string(arg, locals))
                    .collect::<Result<Vec<_>, _>>()?;
                self.eval_function(function, args)?.ok_or_else(|| {
                    format!("native build requires `{target}` to return a String value")
                })
            }
            IrValue::Const { type_, .. } => Err(format!(
                "native build does not support {type_} values outside built-in lowering yet"
            )),
            IrValue::Binary { op, .. } => Err(format!(
                "native build does not support binary operator `{op}` outside built-in lowering yet"
            )),
        }
    }
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

    let mut constants = ConstPool::new();
    let mut function_ids = HashMap::new();
    let mut function_return_types = HashMap::new();
    for (index, function) in ir.functions.iter().enumerate() {
        function_ids.insert(function.name.clone(), index as u32);
        let return_type = types.type_id(&mut strings, &function.returns);
        function_return_types.insert(function.name.clone(), return_type);
    }

    let mut builtin_ids = HashMap::new();
    if uses_builtin(ir, "io.print") {
        let function_id = ir.functions.len() as u32;
        function_ids.insert("io.print".to_string(), function_id);
        function_return_types.insert("io.print".to_string(), TYPE_NOTHING);
        builtin_ids.insert("io.print".to_string(), function_id);
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
        )?);
    }

    for builtin_name in builtin_ids.keys() {
        functions.push(lower_builtin(builtin_name, &mut strings)?);
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

fn uses_builtin(ir: &IrProject, name: &str) -> bool {
    ir.functions
        .iter()
        .any(|function| function.body.iter().any(|op| op_uses_call(op, name)))
}

fn op_uses_call(op: &IrOp, name: &str) -> bool {
    match op {
        IrOp::Bind { value, .. } | IrOp::Return { value } => value
            .as_ref()
            .is_some_and(|value| value_uses_call(value, name)),
        IrOp::Eval { value } => value_uses_call(value, name),
    }
}

fn value_uses_call(value: &IrValue, name: &str) -> bool {
    match value {
        IrValue::Call { target, args } => {
            target == name || args.iter().any(|arg| value_uses_call(arg, name))
        }
        IrValue::Binary { left, right, .. } => {
            value_uses_call(left, name) || value_uses_call(right, name)
        }
        IrValue::Const { .. } | IrValue::Local(_) => false,
    }
}

fn lower_function(
    function: &IrFunction,
    strings: &mut StringPool,
    types: &mut TypeTable,
    constants: &mut ConstPool,
    function_ids: &HashMap<String, u32>,
    function_return_types: &HashMap<String, u32>,
) -> Result<Function, String> {
    let mut builder = FunctionBuilder::new(
        strings,
        types,
        constants,
        function_ids,
        function_return_types,
    );
    let mut params = Vec::new();
    let mut locals = HashMap::new();

    for param in &function.params {
        let type_id = builder.type_id(&param.type_);
        let register = builder.add_register(
            type_id,
            REGISTER_FLAG_PARAMETER | REGISTER_FLAG_INITIALIZED_AT_ENTRY,
        );
        locals.insert(param.name.clone(), register);
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

fn lower_builtin(name: &str, strings: &mut StringPool) -> Result<Function, String> {
    if name != "io.print" {
        return Err(format!("unsupported built-in function `{name}`"));
    }

    Ok(Function {
        name: strings.intern(name),
        kind: FUNCTION_BUILTIN,
        flags: FUNCTION_FLAG_RETURNS_NOTHING,
        return_type: TYPE_NOTHING,
        params: vec![Param {
            name: strings.intern("value"),
            type_id: TYPE_STRING,
            flags: 0,
            default_const: u32::MAX,
        }],
        registers: Vec::new(),
        code: Vec::new(),
    })
}

struct FunctionBuilder<'a> {
    strings: &'a mut StringPool,
    types: &'a mut TypeTable,
    constants: &'a mut ConstPool,
    function_ids: &'a HashMap<String, u32>,
    function_return_types: &'a HashMap<String, u32>,
    registers: Vec<Register>,
    code: Vec<Instruction>,
}

impl<'a> FunctionBuilder<'a> {
    fn new(
        strings: &'a mut StringPool,
        types: &'a mut TypeTable,
        constants: &'a mut ConstPool,
        function_ids: &'a HashMap<String, u32>,
        function_return_types: &'a HashMap<String, u32>,
    ) -> Self {
        Self {
            strings,
            types,
            constants,
            function_ids,
            function_return_types,
            registers: Vec::new(),
            code: Vec::new(),
        }
    }

    fn lower_op(&mut self, op: &IrOp, locals: &mut HashMap<String, u32>) -> Result<(), String> {
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
                locals.insert(name.clone(), register);
                if let Some(value) = value {
                    let value_register = self.lower_value(value, locals)?;
                    self.push_move_like(type_id, register, value_register);
                } else {
                    self.push(OPCODE_LOAD_DEFAULT, vec![register, type_id]);
                }
                Ok(())
            }
            IrOp::Return { value } => {
                let register = match value {
                    Some(value) => self.lower_value(value, locals)?,
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
        }
    }

    fn lower_value(
        &mut self,
        value: &IrValue,
        locals: &HashMap<String, u32>,
    ) -> Result<u32, String> {
        match value {
            IrValue::Const { type_, .. } => {
                let type_id = self.type_id(type_);
                let register = self.add_register(type_id, 0);
                let const_id = self.const_id(value)?;
                self.push(OPCODE_LOAD_CONST, vec![register, const_id]);
                Ok(register)
            }
            IrValue::Local(name) => locals
                .get(name)
                .copied()
                .ok_or_else(|| format!("IR references unknown local `{name}`")),
            IrValue::Call { target, args } => {
                let function_id = *self
                    .function_ids
                    .get(target)
                    .ok_or_else(|| format!("IR references unknown function `{target}`"))?;
                let return_type = self.call_return_type(target)?;
                let result_type = self.types.result_type(self.strings, return_type);
                let result_register = self.add_register(result_type, 0);
                let mut operands = vec![result_register, function_id];
                for arg in args {
                    operands.push(self.lower_value(arg, locals)?);
                }
                self.push(OPCODE_CALL_RESULT, operands);

                let value_register = self.add_register(return_type, 0);
                self.push(OPCODE_UNWRAP_RESULT, vec![value_register, result_register]);
                Ok(value_register)
            }
            IrValue::Binary { op, left, right } => {
                let left_register = self.lower_value(left, locals)?;
                let right_register = self.lower_value(right, locals)?;
                let type_id = if op == "&" {
                    TYPE_STRING
                } else if self.registers[left_register as usize].type_id == TYPE_FLOAT
                    || self.registers[right_register as usize].type_id == TYPE_FLOAT
                    || self.registers[left_register as usize].type_id == TYPE_FIXED
                    || self.registers[right_register as usize].type_id == TYPE_FIXED
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
                    "&" => OPCODE_CONCAT,
                    _ => return Err(format!("unsupported IR binary operator `{op}`")),
                };
                self.push(opcode, vec![dst, left_register, right_register]);
                Ok(dst)
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
        self.function_return_types
            .get(target)
            .copied()
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

    fn ends_with_return(&self) -> bool {
        self.code
            .last()
            .is_some_and(|instruction| instruction.opcode == OPCODE_RETURN_OK)
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
        self.add_entry(strings, package, &ir_type.name, kind, Vec::new())
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
            "Byte" => 7,
            "Error" => 8,
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

impl ConstPool {
    fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    fn add(&mut self, strings: &mut StringPool, value: &IrValue) -> Result<u32, String> {
        let entry = match value {
            IrValue::Const { type_, value } => match type_.as_str() {
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
