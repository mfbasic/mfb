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

const FS_PATH_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "path",
    type_: "String",
    location: abi::RETURN_REGISTER,
}];

const FS_PATH_MODE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "path",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "mode",
        type_: "String",
        location: "x1",
    },
];

const FS_FILE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "file",
    type_: "File",
    location: "x0",
}];

const FS_FILE_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "file",
        type_: "File",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: "x1",
    },
];

const FS_PATH_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "path",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: "x1",
    },
];

const FS_TWO_PATH_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "base",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "child",
        type_: "String",
        location: "x1",
    },
];

const FS_FILE_BYTE_LIST_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "file",
        type_: "File",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: "x1",
    },
];

const FS_PATH_BYTE_LIST_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "path",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: "x1",
    },
];

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

pub(crate) const FS_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.exists",
    symbol: "_mfb_rt_fs_fs_exists",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_FILE_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.fileExists",
    symbol: "_mfb_rt_fs_fs_fileExists",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_DIRECTORY_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.directoryExists",
    symbol: "_mfb_rt_fs_fs_directoryExists",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CURRENT_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.currentDirectory",
    symbol: "_mfb_rt_fs_fs_currentDirectory",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_TEMP_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.tempDirectory",
    symbol: "_mfb_rt_fs_fs_tempDirectory",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_SET_CURRENT_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.setCurrentDirectory",
    symbol: "_mfb_rt_fs_fs_setCurrentDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_DELETE_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.deleteFile",
    symbol: "_mfb_rt_fs_fs_deleteFile",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CREATE_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createDirectory",
    symbol: "_mfb_rt_fs_fs_createDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CREATE_DIRECTORIES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createDirectories",
    symbol: "_mfb_rt_fs_fs_createDirectories",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_DELETE_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.deleteDirectory",
    symbol: "_mfb_rt_fs_fs_deleteDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_LIST_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.listDirectory",
    symbol: "_mfb_rt_fs_fs_listDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_OPEN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.open",
    symbol: "_mfb_rt_fs_fs_open",
    abi: RuntimeHelperAbi {
        params: FS_PATH_MODE_PARAMS,
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_OPEN_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openFile",
    symbol: "_mfb_rt_fs_fs_openFile",
    abi: RuntimeHelperAbi {
        params: FS_PATH_MODE_PARAMS,
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_OPEN_FILE_NO_FOLLOW_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openFileNoFollow",
    symbol: "_mfb_rt_fs_fs_openFileNoFollow",
    abi: RuntimeHelperAbi {
        params: FS_PATH_MODE_PARAMS,
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CREATE_TEMP_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createTempFile",
    symbol: "_mfb_rt_fs_fs_createTempFile",
    abi: RuntimeHelperAbi {
        params: &[RuntimeAbiParam {
            name: "directory",
            type_: "String",
            location: "x0",
        }],
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.close",
    symbol: "_mfb_rt_fs_fs_close",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_ALL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeAll",
    symbol: "_mfb_rt_fs_fs_writeAll",
    abi: RuntimeHelperAbi {
        params: FS_FILE_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readText",
    symbol: "_mfb_rt_fs_fs_readText",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readBytes",
    symbol: "_mfb_rt_fs_fs_readBytes",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeText",
    symbol: "_mfb_rt_fs_fs_writeText",
    abi: RuntimeHelperAbi {
        params: FS_PATH_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_TEXT_ATOMIC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeTextAtomic",
    symbol: "_mfb_rt_fs_fs_writeTextAtomic",
    abi: RuntimeHelperAbi {
        params: FS_PATH_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeBytes",
    symbol: "_mfb_rt_fs_fs_writeBytes",
    abi: RuntimeHelperAbi {
        params: FS_PATH_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_BYTES_ATOMIC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeBytesAtomic",
    symbol: "_mfb_rt_fs_fs_writeBytesAtomic",
    abi: RuntimeHelperAbi {
        params: FS_PATH_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_APPEND_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.appendText",
    symbol: "_mfb_rt_fs_fs_appendText",
    abi: RuntimeHelperAbi {
        params: FS_PATH_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_APPEND_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.appendBytes",
    symbol: "_mfb_rt_fs_fs_appendBytes",
    abi: RuntimeHelperAbi {
        params: FS_PATH_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_LINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readLine",
    symbol: "_mfb_rt_fs_fs_readLine",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_ALL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readAll",
    symbol: "_mfb_rt_fs_fs_readAll",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_ALL_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readAllBytes",
    symbol: "_mfb_rt_fs_fs_readAllBytes",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_ALL_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeAllBytes",
    symbol: "_mfb_rt_fs_fs_writeAllBytes",
    abi: RuntimeHelperAbi {
        params: FS_FILE_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_EOF_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.eof",
    symbol: "_mfb_rt_fs_fs_eof",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CANONICAL_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.canonicalPath",
    symbol: "_mfb_rt_fs_fs_canonicalPath",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_IS_WITHIN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.isWithin",
    symbol: "_mfb_rt_fs_fs_isWithin",
    abi: RuntimeHelperAbi {
        params: FS_TWO_PATH_PARAMS,
        returns: "Boolean",
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
        FS_FILE_EXISTS_SPEC,
        FS_DIRECTORY_EXISTS_SPEC,
        FS_EXISTS_SPEC,
        FS_CURRENT_DIRECTORY_SPEC,
        FS_TEMP_DIRECTORY_SPEC,
        FS_SET_CURRENT_DIRECTORY_SPEC,
        FS_DELETE_FILE_SPEC,
        FS_CREATE_DIRECTORY_SPEC,
        FS_CREATE_DIRECTORIES_SPEC,
        FS_DELETE_DIRECTORY_SPEC,
        FS_LIST_DIRECTORY_SPEC,
        FS_OPEN_SPEC,
        FS_OPEN_FILE_SPEC,
        FS_OPEN_FILE_NO_FOLLOW_SPEC,
        FS_CREATE_TEMP_FILE_SPEC,
        FS_CLOSE_SPEC,
        FS_WRITE_ALL_SPEC,
        FS_READ_TEXT_SPEC,
        FS_READ_BYTES_SPEC,
        FS_WRITE_TEXT_SPEC,
        FS_WRITE_TEXT_ATOMIC_SPEC,
        FS_WRITE_BYTES_SPEC,
        FS_WRITE_BYTES_ATOMIC_SPEC,
        FS_APPEND_TEXT_SPEC,
        FS_APPEND_BYTES_SPEC,
        FS_READ_LINE_SPEC,
        FS_READ_ALL_SPEC,
        FS_READ_ALL_BYTES_SPEC,
        FS_WRITE_ALL_BYTES_SPEC,
        FS_EOF_SPEC,
        FS_CANONICAL_PATH_SPEC,
        FS_IS_WITHIN_SPEC,
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
            | "fs.pathBaseName"
            | "fs.pathDirName"
            | "fs.pathExtension"
            | "fs.pathJoin"
            | "fs.pathNormalize"
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
