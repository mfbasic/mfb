use super::*;

use crate::arch::aarch64::abi;

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
