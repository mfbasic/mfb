use super::*;

pub(crate) const IO_PRINT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.print",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const IO_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.write",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const IO_PRINT_ERROR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.printError",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const IO_WRITE_ERROR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.writeError",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const IO_FLUSH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.flush",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const IO_IS_BUFFERED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.isBuffered",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const IO_SET_BUFFERED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.setBuffered",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const IO_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.input",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const IO_READ_LINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.readLine",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const IO_READ_CHAR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.readChar",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const IO_READ_BYTE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.readByte",
    abi: RuntimeHelperAbi { returns: "Byte" },
};

pub(crate) const IO_POLL_INPUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.pollInput",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const IO_IS_INPUT_TERMINAL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.isInputTerminal",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const IO_IS_OUTPUT_TERMINAL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.isOutputTerminal",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const IO_IS_ERROR_TERMINAL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Io,
    call: "io.isErrorTerminal",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

// bug-70 once asserted here that `io.flush` declared a truthful non-empty
// clobber set rather than the `clobbers: &[]` it originally shipped with. The
// per-spec `clobbers` field itself was deleted by bug-329 (it repeated one
// constant at every spec and nothing read the register names), so a false
// empty declaration can no longer exist. The real clobber model — every
// internal `bl _mfb_*` destroys all of `x0`–`x17` — lives in the register
// allocator's call-clobber masks (`regalloc/analysis.rs`).
