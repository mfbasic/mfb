use crate::arch::aarch64::abi;
use crate::builtins;
use crate::ir::{IrOp, IrProject, IrValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeHelper {
    Fs,
    General,
    Io,
    Math,
    Strings,
    Thread,
}

impl RuntimeHelper {
    pub fn name(self) -> &'static str {
        match self {
            RuntimeHelper::Fs => "fs",
            RuntimeHelper::General => "general",
            RuntimeHelper::Io => "io",
            RuntimeHelper::Math => "math",
            RuntimeHelper::Strings => "strings",
            RuntimeHelper::Thread => "thread",
        }
    }
}

pub fn symbol_for_call(helper: RuntimeHelper, target: &str) -> String {
    format!(
        "_mfb_rt_{}_{}",
        helper.name(),
        target
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '_' {
                    ch
                } else {
                    '_'
                }
            })
            .collect::<String>()
    )
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeHelperSpec {
    pub(crate) helper: RuntimeHelper,
    pub(crate) call: &'static str,
    pub(crate) symbol: &'static str,
    pub(crate) abi: RuntimeHelperAbi,
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeHelperAbi {
    pub(crate) params: &'static [RuntimeAbiParam],
    pub(crate) returns: &'static str,
    pub(crate) clobbers: &'static [&'static str],
}

#[derive(Clone, Copy)]
pub(crate) struct RuntimeAbiParam {
    pub(crate) name: &'static str,
    pub(crate) type_: &'static str,
    pub(crate) location: &'static str,
}

const IO_PRINT_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "value",
    type_: "String",
    location: abi::RETURN_REGISTER,
}];

const IO_INPUT_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "prompt",
    type_: "String",
    location: abi::RETURN_REGISTER,
}];

const IO_POLL_INPUT_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "timeoutMs",
    type_: "Integer",
    location: abi::RETURN_REGISTER,
}];

pub(crate) const IO_PRINT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.print",
    symbol: "_mfb_rt_io_io_print",
    abi: RuntimeHelperAbi {
        params: IO_PRINT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.write",
    symbol: "_mfb_rt_io_io_write",
    abi: RuntimeHelperAbi {
        params: IO_PRINT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_PRINT_ERROR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.printError",
    symbol: "_mfb_rt_io_io_printError",
    abi: RuntimeHelperAbi {
        params: IO_PRINT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_WRITE_ERROR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.writeError",
    symbol: "_mfb_rt_io_io_writeError",
    abi: RuntimeHelperAbi {
        params: IO_PRINT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_FLUSH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.flush",
    symbol: "_mfb_rt_io_io_flush",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: &[],
    },
};

pub(crate) const IO_FLUSH_ERROR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.flushError",
    symbol: "_mfb_rt_io_io_flushError",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Nothing",
        clobbers: &[],
    },
};

pub(crate) const IO_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.input",
    symbol: "_mfb_rt_io_io_input",
    abi: RuntimeHelperAbi {
        params: IO_INPUT_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_READ_LINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.readLine",
    symbol: "_mfb_rt_io_io_readLine",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_READ_CHAR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.readChar",
    symbol: "_mfb_rt_io_io_readChar",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_READ_BYTE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.readByte",
    symbol: "_mfb_rt_io_io_readByte",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_POLL_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.pollInput",
    symbol: "_mfb_rt_io_io_pollInput",
    abi: RuntimeHelperAbi {
        params: IO_POLL_INPUT_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_IS_INPUT_TERMINAL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.isInputTerminal",
    symbol: "_mfb_rt_io_io_isInputTerminal",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_IS_OUTPUT_TERMINAL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.isOutputTerminal",
    symbol: "_mfb_rt_io_io_isOutputTerminal",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_IS_ERROR_TERMINAL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.isErrorTerminal",
    symbol: "_mfb_rt_io_io_isErrorTerminal",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const IO_TERMINAL_SIZE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.terminalSize",
    symbol: "_mfb_rt_io_io_terminalSize",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "TerminalSize",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) fn supported_helper_specs() -> &'static [RuntimeHelperSpec] {
    &[
        IO_PRINT_SPEC,
        IO_WRITE_SPEC,
        IO_PRINT_ERROR_SPEC,
        IO_WRITE_ERROR_SPEC,
        IO_FLUSH_SPEC,
        IO_FLUSH_ERROR_SPEC,
        IO_INPUT_SPEC,
        IO_READ_LINE_SPEC,
        IO_READ_CHAR_SPEC,
        IO_READ_BYTE_SPEC,
        IO_POLL_INPUT_SPEC,
        IO_IS_INPUT_TERMINAL_SPEC,
        IO_IS_OUTPUT_TERMINAL_SPEC,
        IO_IS_ERROR_TERMINAL_SPEC,
        IO_TERMINAL_SIZE_SPEC,
    ]
}

pub(crate) fn spec_for_symbol(symbol: &str) -> Option<&'static RuntimeHelperSpec> {
    supported_helper_specs()
        .iter()
        .find(|spec| spec.symbol == symbol)
}

pub(crate) fn spec_for_call(target: &str) -> Option<&'static RuntimeHelperSpec> {
    supported_helper_specs()
        .iter()
        .find(|spec| spec.call == target)
}

pub fn helper_for_call(name: &str) -> Option<RuntimeHelper> {
    if builtins::fs::is_fs_call(name) {
        Some(RuntimeHelper::Fs)
    } else if builtins::general::is_general_call(name) {
        Some(RuntimeHelper::General)
    } else if builtins::io::is_io_call(name) {
        Some(RuntimeHelper::Io)
    } else if builtins::math::is_math_call(name) {
        Some(RuntimeHelper::Math)
    } else if builtins::strings::is_strings_call(name) {
        Some(RuntimeHelper::Strings)
    } else if builtins::thread::is_thread_call(name) {
        Some(RuntimeHelper::Thread)
    } else {
        None
    }
}

pub(crate) fn is_native_direct_call(name: &str) -> bool {
    matches!(
        name,
        "contains"
            | "append"
            | "get"
            | "getOr"
            | "hasKey"
            | "insert"
            | "find"
            | "forEach"
            | "filter"
            | "keys"
            | "len"
            | "mid"
            | "prepend"
            | "reduce"
            | "removeAt"
            | "removeKey"
            | "replace"
            | "set"
            | "sum"
            | "transform"
            | "values"
            | "toByte"
            | "toFixed"
            | "toFloat"
            | "toInt"
            | "toString"
            | "isEmpty"
            | "isEven"
            | "isNegative"
            | "isNotEmpty"
            | "isOdd"
            | "isPositive"
            | "isNumeric"
            | "isZero"
            | "strings.byteLen"
            | "strings.caseFold"
            | "strings.contains"
            | "strings.endsWith"
            | "strings.graphemes"
            | "strings.lower"
            | "strings.normalizeNfc"
            | "strings.startsWith"
            | "strings.split"
            | "strings.trim"
            | "strings.trimEnd"
            | "strings.trimStart"
            | "strings.upper"
            | "strings.join"
    )
}

pub fn required_helpers(ir: &IrProject) -> Vec<RuntimeHelper> {
    let mut helpers = Vec::new();
    for function in &ir.functions {
        push_op_helpers(&function.body, &mut helpers);
    }
    helpers
}

fn push_op_helpers(ops: &[IrOp], helpers: &mut Vec<RuntimeHelper>) {
    for op in ops {
        match op {
            IrOp::Bind { value, .. } => {
                if let Some(value) = value {
                    push_value_helpers(value, helpers);
                }
            }
            IrOp::Fail { error } => {
                push_value_helpers(error, helpers);
            }
            IrOp::Assign { value, .. } | IrOp::Eval { value } => {
                push_value_helpers(value, helpers);
            }
            IrOp::Return { value } => {
                if let Some(value) = value {
                    push_value_helpers(value, helpers);
                }
            }
            IrOp::If {
                condition,
                then_body,
                else_body,
            } => {
                push_value_helpers(condition, helpers);
                push_op_helpers(then_body, helpers);
                push_op_helpers(else_body, helpers);
            }
            IrOp::Match { value, cases } => {
                push_value_helpers(value, helpers);
                for case in cases {
                    push_op_helpers(&case.body, helpers);
                }
            }
            IrOp::ForEach { iterable, body, .. } => {
                push_value_helpers(iterable, helpers);
                push_op_helpers(body, helpers);
            }
            IrOp::Using { value, body, .. } => {
                push_value_helpers(value, helpers);
                push_op_helpers(body, helpers);
            }
        }
    }
}

fn push_value_helpers(value: &IrValue, helpers: &mut Vec<RuntimeHelper>) {
    match value {
        IrValue::Call { target, args } => {
            if !is_native_direct_call(target) {
                if let Some(helper) = helper_for_call(target) {
                    push_unique(helpers, helper);
                }
            }
            for arg in args {
                push_value_helpers(arg, helpers);
            }
        }
        IrValue::MemberAccess { target, .. } => push_value_helpers(target, helpers),
        IrValue::Binary { left, right, .. } => {
            push_value_helpers(left, helpers);
            push_value_helpers(right, helpers);
        }
        IrValue::Unary { operand, .. } => push_value_helpers(operand, helpers),
        IrValue::Constructor { args, .. } => {
            for arg in args {
                push_value_helpers(arg, helpers);
            }
        }
        IrValue::WithUpdate {
            target, updates, ..
        } => {
            push_value_helpers(target, helpers);
            for update in updates {
                push_value_helpers(&update.value, helpers);
            }
        }
        IrValue::ListLiteral { values, .. } => {
            for value in values {
                push_value_helpers(value, helpers);
            }
        }
        IrValue::MapLiteral { entries, .. } => {
            for (key, value) in entries {
                push_value_helpers(key, helpers);
                push_value_helpers(value, helpers);
            }
        }
        IrValue::Const { .. } | IrValue::Local(_) | IrValue::FunctionRef { .. } => {}
    }
}

fn push_unique(helpers: &mut Vec<RuntimeHelper>, helper: RuntimeHelper) {
    if !helpers.contains(&helper) {
        helpers.push(helper);
    }
}
