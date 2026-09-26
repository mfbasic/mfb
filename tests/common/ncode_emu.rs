//! A small interpreter for the `.ncode` dump (bug-699).
//!
//! `mfb build -ncode` writes every emitted function as a list of neutral
//! instructions (`mov`, `ldr_u64`, `cmp_imm`, `b.eq`, `bl`, ...) after register
//! allocation, with the target's own register names. That vocabulary is small
//! and identical across targets, so a test can *run* an emitted function instead
//! of pattern-matching its instruction stream: feed it the arguments a real call
//! would receive, answer every external call from a model of the platform, and
//! observe what it did.
//!
//! This is what makes a handler test behavioral. "The GTK key handler injects
//! `ESC [ D` for Left" is checked by calling the handler with `GDK_KEY_Left` and
//! reading the bytes it wrote to the pipe — not by asserting which compare
//! instructions it contains, which would pin one implementation and fail a
//! correct different one.
//!
//! # What it models, and what it does not
//!
//! - Registers are a name -> `u64` map; `xzr`/`wzr` read as zero.
//! - Memory is a sparse byte map. Every data object in the dump is laid out at
//!   its own address; an external data symbol (`_OBJC_CLASS_$_NSApplication`)
//!   gets a zeroed block the platform model may fill in.
//! - `bl` to a function in the dump is a real call (recursive); `bl` to anything
//!   else goes to the [`Externals`] model, which must answer it or the run
//!   panics naming the call. There is deliberately no "return 0 for unknown
//!   externals" default: a guessed answer is exactly how an emulator lies.
//! - Condition flags are the last compare's operands; each conditional branch
//!   evaluates its own predicate over them (signed or unsigned).
//! - An op this interpreter does not know panics with the op's JSON.
//!
//! It does not model the instruction ENCODING — a correct `.ncode` stream that
//! the encoder mis-assembles runs fine here. Encoding has its own tests.

use serde_json::Value;
use std::collections::HashMap;

/// The calling convention of the dump's target: where arguments and results live.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Abi {
    /// AArch64 PCS (macOS and Linux): `x0`..`x7`, result in `x0`.
    Aarch64,
    /// Win64: `rcx`,`rdx`,`r8`,`r9`, then the stack above the 32-byte shadow space.
    Win64,
    /// System V x86-64: `rdi`,`rsi`,`rdx`,`rcx`,`r8`,`r9`, then the stack.
    SysV,
    /// RISC-V LP64: `a0`..`a7`, result in `a0`.
    Riscv,
}

impl Abi {
    pub fn for_target(target: &str) -> Abi {
        if target.starts_with("windows") {
            Abi::Win64
        } else if target.contains("x86_64") {
            Abi::SysV
        } else if target.contains("riscv64") {
            Abi::Riscv
        } else {
            Abi::Aarch64
        }
    }

    fn arg_registers(self) -> &'static [&'static str] {
        match self {
            Abi::Aarch64 => &["x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7"],
            Abi::Win64 => &["rcx", "rdx", "r8", "r9"],
            Abi::SysV => &["rdi", "rsi", "rdx", "rcx", "r8", "r9"],
            Abi::Riscv => &["a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7"],
        }
    }

    fn return_register(self) -> &'static str {
        match self {
            Abi::Aarch64 => "x0",
            Abi::Win64 | Abi::SysV => "rax",
            Abi::Riscv => "a0",
        }
    }

    fn stack_pointer(self) -> &'static str {
        match self {
            Abi::Aarch64 | Abi::Riscv => "sp",
            Abi::Win64 | Abi::SysV => "rsp",
        }
    }

    /// Registers a call may clobber (used only by the static registration scan).
    fn caller_saved(self) -> &'static [&'static str] {
        match self {
            Abi::Aarch64 => &[
                "x0", "x1", "x2", "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12",
                "x13", "x14", "x15", "x16", "x17",
            ],
            Abi::Win64 => &["rax", "rcx", "rdx", "r8", "r9", "r10", "r11"],
            Abi::SysV => &["rax", "rcx", "rdx", "rsi", "rdi", "r8", "r9", "r10", "r11"],
            Abi::Riscv => &[
                "a0", "a1", "a2", "a3", "a4", "a5", "a6", "a7", "t0", "t1", "t2", "t3", "t4", "t5",
                "t6",
            ],
        }
    }
}

/// The platform: every call the emulated code makes to something outside the dump.
pub trait Externals {
    /// Answer the external call `name`. Read arguments with [`Machine::arg`].
    /// Return `None` for a call this model does not know — the run then panics
    /// naming it, rather than continuing on an invented answer.
    fn call(&mut self, machine: &mut Machine, name: &str) -> Option<u64>;

    /// Whether this model answers a call to a function that IS in the dump —
    /// to stand in for a runtime helper that is not what the test is about
    /// (the arena allocator, the stdin log reader).
    fn intercepts(&self, _name: &str) -> bool {
        false
    }
}

struct Function<'a> {
    instructions: &'a [Value],
    labels: HashMap<String, usize>,
}

const DATA_BASE: u64 = 0x1000_0000;
const FUNCTION_BASE: u64 = 0x2000_0000;
const EXTERNAL_BASE: u64 = 0x3000_0000;
const HEAP_BASE: u64 = 0x5000_0000;
const STACK_TOP: u64 = 0x7000_0000;
/// Guard against a handler that loops forever (a wrong branch in the emulated
/// code, or a platform model that never lets a loop finish).
const STEP_LIMIT: u64 = 5_000_000;
const PAGE: usize = 4096;

pub struct Machine<'a> {
    pub abi: Abi,
    functions: HashMap<String, Function<'a>>,
    regs: HashMap<String, u64>,
    /// Sparse memory in 4 KiB pages: the Unicode tables alone are megabytes, and
    /// a byte map of them is both slow to load and large.
    pages: HashMap<u64, Box<[u8; PAGE]>>,
    symbol_address: HashMap<String, u64>,
    address_symbol: HashMap<u64, String>,
    next_external: u64,
    next_heap: u64,
    flags: (u64, u64),
    steps: u64,
    halted: bool,
}

fn field<'v>(instruction: &'v Value, name: &str) -> &'v str {
    instruction
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("ncode instruction has no `{name}`: {instruction}"))
}

fn parse_int(text: &str) -> u64 {
    match text {
        "true" => return 1,
        "false" => return 0,
        _ => {}
    }
    if let Some(hex) = text.strip_prefix("0x") {
        return u64::from_str_radix(hex, 16).unwrap_or_else(|_| panic!("bad immediate {text}"));
    }
    text.parse::<i128>()
        .unwrap_or_else(|_| panic!("bad immediate {text}")) as u64
}

fn decode_hex(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|i| u8::from_str_radix(&text[2 * i..2 * i + 2], 16).expect("hex data object"))
        .collect()
}

/// The initial bytes of one data object, as the linker would lay them out.
fn data_object_bytes(object: &Value) -> Vec<u8> {
    let kind = object["kind"].as_str().unwrap_or("");
    let value = object["value"].as_str().unwrap_or("");
    let layout = object["layout"].as_str().unwrap_or("");
    let size = object["size"].as_u64().unwrap_or(0) as usize;
    let mut bytes = if kind == "raw" {
        decode_hex(value)
    } else if layout.starts_with("mfb.string") {
        let text = value.as_bytes();
        let mut out = (text.len() as u64).to_le_bytes().to_vec();
        out.extend_from_slice(text);
        out.push(0);
        out
    } else {
        value.as_bytes().to_vec()
    };
    if bytes.len() < size {
        bytes.resize(size, 0);
    }
    bytes
}

impl<'a> Machine<'a> {
    /// Load every function and data object of an `.ncode` dump.
    pub fn new(ncode: &'a Value) -> Machine<'a> {
        let target = ncode["target"].as_str().unwrap_or("");
        let abi = Abi::for_target(target);
        let mut machine = Machine {
            abi,
            functions: HashMap::new(),
            regs: HashMap::new(),
            pages: HashMap::new(),
            symbol_address: HashMap::new(),
            address_symbol: HashMap::new(),
            next_external: EXTERNAL_BASE,
            next_heap: HEAP_BASE,
            flags: (0, 0),
            steps: 0,
            halted: false,
        };
        let mut next_data = DATA_BASE;
        for object in ncode["dataObjects"].as_array().expect("dataObjects") {
            let symbol = object["symbol"].as_str().expect("data symbol").to_string();
            let bytes = data_object_bytes(object);
            let address = next_data;
            // Zero bytes need no write: an untouched page reads as zero.
            for (i, byte) in bytes.iter().enumerate() {
                if *byte != 0 {
                    machine.write_u8(address + i as u64, *byte);
                }
            }
            machine.symbol_address.insert(symbol.clone(), address);
            machine.address_symbol.insert(address, symbol);
            next_data += ((bytes.len() as u64 + 15) & !15) + 16;
        }
        for (index, function) in ncode["functions"]
            .as_array()
            .expect("functions")
            .iter()
            .enumerate()
        {
            let symbol = function["symbol"]
                .as_str()
                .expect("function symbol")
                .to_string();
            let instructions = function["instructions"].as_array().expect("instructions");
            let labels = instructions
                .iter()
                .enumerate()
                .filter(|(_, i)| i["op"] == "label")
                .map(|(pos, i)| (field(i, "name").to_string(), pos))
                .collect();
            let address = FUNCTION_BASE + 16 * index as u64;
            machine.symbol_address.insert(symbol.clone(), address);
            machine.address_symbol.insert(address, symbol.clone());
            machine.functions.insert(
                symbol,
                Function {
                    instructions: instructions.as_slice(),
                    labels,
                },
            );
        }
        machine.set_reg(abi.stack_pointer(), STACK_TOP);
        machine
    }

    pub fn has_function(&self, symbol: &str) -> bool {
        self.functions.contains_key(symbol)
    }

    /// The address of a data object, function, or (allocated on first use)
    /// external symbol.
    pub fn address_of(&mut self, symbol: &str) -> u64 {
        if let Some(address) = self.symbol_address.get(symbol) {
            return *address;
        }
        let address = self.next_external;
        self.next_external += 0x1000;
        self.symbol_address.insert(symbol.to_string(), address);
        self.address_symbol.insert(address, symbol.to_string());
        address
    }

    /// The symbol whose block starts at `address`, if any.
    pub fn symbol_at(&self, address: u64) -> Option<&str> {
        self.address_symbol.get(&address).map(String::as_str)
    }

    pub fn reg(&self, name: &str) -> u64 {
        match name {
            "xzr" | "wzr" | "zero" => 0,
            _ => *self.regs.get(name).unwrap_or(&0),
        }
    }

    pub fn set_reg(&mut self, name: &str, value: u64) {
        if !matches!(name, "xzr" | "wzr" | "zero") {
            self.regs.insert(name.to_string(), value);
        }
    }

    /// Argument `index` of the call being answered (or about to be made).
    pub fn arg(&self, index: usize) -> u64 {
        let registers = self.abi.arg_registers();
        if index < registers.len() {
            return self.reg(registers[index]);
        }
        let sp = self.reg(self.abi.stack_pointer());
        let offset = match self.abi {
            Abi::Win64 => 0x20 + 8 * (index - registers.len()) as u64,
            Abi::SysV | Abi::Aarch64 | Abi::Riscv => 8 * (index - registers.len()) as u64,
        };
        self.read_u64(sp + offset)
    }

    pub fn read_u8(&self, address: u64) -> u8 {
        self.pages
            .get(&(address / PAGE as u64))
            .map_or(0, |page| page[(address % PAGE as u64) as usize])
    }

    fn write_u8(&mut self, address: u64, value: u8) {
        let page = self
            .pages
            .entry(address / PAGE as u64)
            .or_insert_with(|| Box::new([0; PAGE]));
        page[(address % PAGE as u64) as usize] = value;
    }

    pub fn read_uint(&self, address: u64, width: usize) -> u64 {
        (0..width).fold(0u64, |acc, i| {
            acc | (u64::from(self.read_u8(address + i as u64)) << (8 * i))
        })
    }

    pub fn read_u64(&self, address: u64) -> u64 {
        self.read_uint(address, 8)
    }

    pub fn write_uint(&mut self, address: u64, width: usize, value: u64) {
        for i in 0..width {
            self.write_u8(address + i as u64, (value >> (8 * i)) as u8);
        }
    }

    pub fn write_bytes(&mut self, address: u64, bytes: &[u8]) {
        for (i, byte) in bytes.iter().enumerate() {
            self.write_u8(address + i as u64, *byte);
        }
    }

    pub fn read_bytes(&self, address: u64, len: usize) -> Vec<u8> {
        (0..len).map(|i| self.read_u8(address + i as u64)).collect()
    }

    /// A NUL-terminated byte string at `address` (without the NUL).
    pub fn read_cstr(&self, address: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut at = address;
        loop {
            let byte = self.read_u8(at);
            if byte == 0 {
                return out;
            }
            out.push(byte);
            at += 1;
        }
    }

    /// Stop the program: every active call returns at once (a model's
    /// `ExitProcess`/`_exit`).
    pub fn halt(&mut self) {
        self.halted = true;
    }

    pub fn halted(&self) -> bool {
        self.halted
    }

    /// A fresh zeroed block of `len` bytes, for the platform model's objects.
    pub fn alloc(&mut self, len: usize) -> u64 {
        let address = self.next_heap;
        self.next_heap += ((len as u64 + 15) & !15) + 16;
        address
    }

    /// Call `symbol` with `args` and return its result register.
    pub fn call(&mut self, symbol: &str, args: &[u64], externals: &mut dyn Externals) -> u64 {
        let registers = self.abi.arg_registers();
        assert!(
            args.len() <= registers.len(),
            "call {symbol}: stack arguments are not supported by Machine::call"
        );
        for (register, value) in registers.iter().zip(args) {
            self.set_reg(register, *value);
        }
        self.enter(symbol, externals);
        self.reg(self.abi.return_register())
    }

    fn enter(&mut self, symbol: &str, externals: &mut dyn Externals) {
        let sp = self.abi.stack_pointer();
        if matches!(self.abi, Abi::Win64 | Abi::SysV) {
            // `call` pushes the return address; the callee's frame math counts on it.
            let value = self.reg(sp) - 8;
            self.set_reg(sp, value);
            self.write_uint(value, 8, 0xdead_beef);
        } else {
            // The link register, whatever the dump calls it.
            self.set_reg("lr", 0xdead_beef);
            self.set_reg("ra", 0xdead_beef);
        }
        let function = self
            .functions
            .get(symbol)
            .unwrap_or_else(|| panic!("no function {symbol} in the dump"));
        let instructions = function.instructions;
        let labels = function.labels.clone();
        let mut pc = 0usize;
        loop {
            self.steps += 1;
            assert!(
                self.steps < STEP_LIMIT,
                "emulation of {symbol} exceeded {STEP_LIMIT} steps"
            );
            let instruction = instructions
                .get(pc)
                .unwrap_or_else(|| panic!("{symbol}: ran off the end of the function"));
            pc += 1;
            let op = field(instruction, "op");
            let jump = |label: &str| -> usize {
                *labels
                    .get(label)
                    .unwrap_or_else(|| panic!("{symbol}: no label {label}"))
            };
            match op {
                "label" => {}
                "ret" => {
                    if matches!(self.abi, Abi::Win64 | Abi::SysV) {
                        let value = self.reg(sp) + 8;
                        self.set_reg(sp, value);
                    }
                    return;
                }
                "sub_sp" => {
                    let value = self.reg(sp) - parse_int(field(instruction, "imm"));
                    self.set_reg(sp, value);
                }
                "add_sp" => {
                    let value = self.reg(sp) + parse_int(field(instruction, "imm"));
                    self.set_reg(sp, value);
                }
                "mov" => {
                    let value = self.reg(field(instruction, "src"));
                    self.set_reg(field(instruction, "dst"), value);
                }
                "mov_imm" => {
                    let value = parse_int(field(instruction, "value"));
                    self.set_reg(field(instruction, "dst"), value);
                }
                "adrp" => {
                    let address = self.address_of(field(instruction, "symbol"));
                    self.set_reg(field(instruction, "dst"), address);
                }
                "add_pageoff" => {
                    // `adrp` already produced the whole address.
                    let value = self.reg(field(instruction, "src"));
                    self.set_reg(field(instruction, "dst"), value);
                }
                "add" | "sub" | "and" | "orr" | "or" | "eor" | "mul" | "lsl" | "lsr" => {
                    let lhs = self.reg(field(instruction, "lhs"));
                    let rhs = self.reg(field(instruction, "rhs"));
                    let value = match op {
                        "add" => lhs.wrapping_add(rhs),
                        "sub" => lhs.wrapping_sub(rhs),
                        "and" => lhs & rhs,
                        "orr" | "or" => lhs | rhs,
                        "eor" => lhs ^ rhs,
                        "mul" => lhs.wrapping_mul(rhs),
                        "lsl" => lhs.wrapping_shl(rhs as u32),
                        _ => lhs.wrapping_shr(rhs as u32),
                    };
                    self.set_reg(field(instruction, "dst"), value);
                }
                "add_imm" | "sub_imm" | "lsl_imm" | "lsr_imm" | "and_imm" => {
                    let src = self.reg(field(instruction, "src"));
                    // Shifts name their amount `shift`; the rest `imm`.
                    let amount = if op.ends_with("sl_imm") || op.ends_with("sr_imm") {
                        "shift"
                    } else {
                        "imm"
                    };
                    let imm = parse_int(field(instruction, amount));
                    let value = match op {
                        "add_imm" => src.wrapping_add(imm),
                        "sub_imm" => src.wrapping_sub(imm),
                        "lsl_imm" => src.wrapping_shl(imm as u32),
                        "lsr_imm" => src.wrapping_shr(imm as u32),
                        _ => src & imm,
                    };
                    self.set_reg(field(instruction, "dst"), value);
                }
                "cmp" => {
                    self.flags = (
                        self.reg(field(instruction, "lhs")),
                        self.reg(field(instruction, "rhs")),
                    );
                }
                "cmp_imm" => {
                    self.flags = (
                        self.reg(field(instruction, "lhs")),
                        parse_int(field(instruction, "rhs")),
                    );
                }
                "b" => pc = jump(field(instruction, "target")),
                // RISC-V has no flags: a branch compares its own two registers.
                "rv.br" => {
                    let l = self.reg(field(instruction, "lhs"));
                    let r = self.reg(field(instruction, "rhs"));
                    let taken = match field(instruction, "cond") {
                        "eq" => l == r,
                        "ne" => l != r,
                        "lt" => (l as i64) < (r as i64),
                        "ge" => (l as i64) >= (r as i64),
                        "ltu" => l < r,
                        "geu" => l >= r,
                        other => panic!("{symbol}: unknown rv.br condition {other}"),
                    };
                    if taken {
                        pc = jump(field(instruction, "target"));
                    }
                }
                "mvn" => {
                    let value = !self.reg(field(instruction, "src"));
                    self.set_reg(field(instruction, "dst"), value);
                }
                condition if condition.starts_with("b.") => {
                    let (l, r) = self.flags;
                    let (sl, sr) = (l as i64, r as i64);
                    let taken = match &condition[2..] {
                        "eq" => l == r,
                        "ne" => l != r,
                        "lt" => sl < sr,
                        "le" => sl <= sr,
                        "gt" => sl > sr,
                        "ge" => sl >= sr,
                        "lo" => l < r,
                        "ls" => l <= r,
                        "hi" => l > r,
                        "hs" => l >= r,
                        other => panic!("{symbol}: unknown condition {other}"),
                    };
                    if taken {
                        pc = jump(field(instruction, "target"));
                    }
                }
                load if load.starts_with("ldr_u") => {
                    let width = match load {
                        "ldr_u8" => 1,
                        "ldr_u16" => 2,
                        "ldr_u32" => 4,
                        "ldr_u64" => 8,
                        _ => panic!("{symbol}: unknown load {instruction}"),
                    };
                    let address = self
                        .reg(field(instruction, "base"))
                        .wrapping_add(parse_int(field(instruction, "offset")));
                    let value = self.read_uint(address, width);
                    self.set_reg(field(instruction, "dst"), value);
                }
                store if store.starts_with("str_u") => {
                    let width = match store {
                        "str_u8" => 1,
                        "str_u16" => 2,
                        "str_u32" => 4,
                        "str_u64" => 8,
                        _ => panic!("{symbol}: unknown store {instruction}"),
                    };
                    let address = self
                        .reg(field(instruction, "base"))
                        .wrapping_add(parse_int(field(instruction, "offset")));
                    let value = self.reg(field(instruction, "src"));
                    self.write_uint(address, width, value);
                }
                "bl" => {
                    let target = field(instruction, "target").to_string();
                    if self.functions.contains_key(&target) && !externals.intercepts(&target) {
                        let saved_lr = self.reg("lr");
                        self.enter(&target, externals);
                        if matches!(self.abi, Abi::Aarch64 | Abi::Riscv) {
                            self.set_reg("lr", saved_lr);
                        }
                    } else {
                        let before: Vec<u64> = self
                            .abi
                            .caller_saved()
                            .iter()
                            .map(|r| self.reg(r))
                            .collect();
                        let result = externals.call(self, &target).unwrap_or_else(|| {
                            panic!(
                                "{symbol}: the platform model does not answer external call \
                                 `{target}` (args {:#x}, {:#x}, {:#x}, {:#x})",
                                self.arg(0),
                                self.arg(1),
                                self.arg(2),
                                self.arg(3)
                            )
                        });
                        // A real call leaves every caller-saved register but the
                        // result undefined. Poison the ones the model did not set
                        // (it may return a second value, like the arena
                        // allocator's block in `x1`), so code that reads one after
                        // a call — a result taken from the wrong register — sees
                        // garbage here as it does on hardware, not a stale value.
                        //
                        // The poison is a small positive number, not an obviously
                        // bad one: what a register really holds after a call is a
                        // leftover count or index, and a huge or negative value
                        // would sail past the `count > 0` / `i < count` tests a
                        // wrong-register bug actually gets through.
                        let ret = self.abi.return_register();
                        for (i, register) in self.abi.caller_saved().iter().enumerate() {
                            if *register != ret && self.reg(register) == before[i] {
                                self.set_reg(register, 0x40 + i as u64);
                            }
                        }
                        self.set_reg(ret, result);
                    }
                    if self.halted {
                        return;
                    }
                }
                "branch_self" => panic!("{symbol}: reached a branch-to-self (a deliberate hang)"),
                _ => panic!("{symbol}: the ncode interpreter does not know op {instruction}"),
            }
        }
    }
}

/// A value the static registration scan can name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sym {
    /// The address of a symbol (`adrp`/`add_pageoff`).
    Addr(String),
    /// An immediate.
    Int(u64),
    /// The result of an external call, with its symbolic arguments.
    Call(String, Vec<Sym>),
    Unknown,
}

/// One call to `callee` found by [`find_calls`]: the function it sits in and the
/// symbolic value of each argument register at the call.
#[derive(Clone, Debug)]
pub struct CallSite {
    pub function: String,
    pub args: Vec<Sym>,
}

/// Every call to the external `callee` in the dump, with its arguments traced
/// symbolically through straight-line code (register moves, `adrp`, immediates,
/// earlier external calls). This is how a test finds the handler a registration
/// call installs — `class_addMethod(cls, sel, imp, types)`,
/// `g_signal_connect_data(obj, "key-released", handler, ...)` — without knowing
/// which function performs the registration.
///
/// The trace ignores branches (registration code is straight-line) and forgets
/// caller-saved registers at every call, so a value it reports was really in
/// the argument register at the call.
pub fn find_calls(ncode: &Value, callee: &str) -> Vec<CallSite> {
    let abi = Abi::for_target(ncode["target"].as_str().unwrap_or(""));
    let mut sites = Vec::new();
    for function in ncode["functions"].as_array().expect("functions") {
        let name = function["symbol"].as_str().unwrap_or("").to_string();
        let mut regs: HashMap<String, Sym> = HashMap::new();
        let get =
            |regs: &HashMap<String, Sym>, r: &str| regs.get(r).cloned().unwrap_or(Sym::Unknown);
        for instruction in function["instructions"].as_array().expect("instructions") {
            let op = instruction["op"].as_str().unwrap_or("");
            match op {
                "adrp" => {
                    regs.insert(
                        field(instruction, "dst").to_string(),
                        Sym::Addr(field(instruction, "symbol").to_string()),
                    );
                }
                "add_pageoff" => {
                    let value = get(&regs, field(instruction, "src"));
                    regs.insert(field(instruction, "dst").to_string(), value);
                }
                "mov" => {
                    let value = get(&regs, field(instruction, "src"));
                    regs.insert(field(instruction, "dst").to_string(), value);
                }
                "mov_imm" => {
                    regs.insert(
                        field(instruction, "dst").to_string(),
                        Sym::Int(parse_int(field(instruction, "value"))),
                    );
                }
                "bl" => {
                    let target = field(instruction, "target").to_string();
                    let args: Vec<Sym> =
                        abi.arg_registers().iter().map(|r| get(&regs, r)).collect();
                    if target == callee {
                        sites.push(CallSite {
                            function: name.clone(),
                            args: args.clone(),
                        });
                    }
                    for r in abi.caller_saved() {
                        regs.remove(*r);
                    }
                    regs.insert(abi.return_register().to_string(), Sym::Call(target, args));
                }
                _ => {
                    if let Some(dst) = instruction.get("dst").and_then(Value::as_str) {
                        regs.insert(dst.to_string(), Sym::Unknown);
                    }
                }
            }
        }
    }
    sites
}

/// The C string a data object holds (for a `Sym::Addr` naming a string constant).
pub fn data_cstr(ncode: &Value, symbol: &str) -> Option<String> {
    ncode["dataObjects"]
        .as_array()?
        .iter()
        .find(|o| o["symbol"] == symbol && o["kind"] == "raw")
        .map(|o| {
            let bytes = decode_hex(o["value"].as_str().unwrap_or(""));
            let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
            String::from_utf8_lossy(&bytes[..end]).into_owned()
        })
}
