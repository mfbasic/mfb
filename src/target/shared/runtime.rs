use crate::arch::aarch64::abi;
use crate::builtins;
use crate::ir::{IrOp, IrProject, IrValue};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuntimeHelper {
    Fs,
    General,
    Io,
    Math,
    Net,
    Strings,
    Thread,
    Tls,
}

impl RuntimeHelper {
    pub fn name(self) -> &'static str {
        match self {
            RuntimeHelper::Fs => "fs",
            RuntimeHelper::General => "general",
            RuntimeHelper::Io => "io",
            RuntimeHelper::Math => "math",
            RuntimeHelper::Net => "net",
            RuntimeHelper::Strings => "strings",
            RuntimeHelper::Thread => "thread",
            RuntimeHelper::Tls => "tls",
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

const STRING_VALUE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "value",
    type_: "String",
    location: "x0",
}];

const STRING_VALUE_PATTERN_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "pattern",
        type_: "String",
        location: "x1",
    },
];

const STRING_LIST_SEPARATOR_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "values",
        type_: "List OF String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "separator",
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

pub(crate) const STRINGS_TRIM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.trim",
    symbol: "_mfb_rt_strings_strings_trim",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_TRIM_START_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.trimStart",
    symbol: "_mfb_rt_strings_strings_trimStart",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_TRIM_END_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.trimEnd",
    symbol: "_mfb_rt_strings_strings_trimEnd",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_UPPER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.upper",
    symbol: "_mfb_rt_strings_strings_upper",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_LOWER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.lower",
    symbol: "_mfb_rt_strings_strings_lower",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_CASE_FOLD_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.caseFold",
    symbol: "_mfb_rt_strings_strings_caseFold",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_NORMALIZE_NFC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.normalizeNfc",
    symbol: "_mfb_rt_strings_strings_normalizeNfc",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_GRAPHEMES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.graphemes",
    symbol: "_mfb_rt_strings_strings_graphemes",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_STARTS_WITH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.startsWith",
    symbol: "_mfb_rt_strings_strings_startsWith",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_ENDS_WITH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.endsWith",
    symbol: "_mfb_rt_strings_strings_endsWith",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_CONTAINS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.contains",
    symbol: "_mfb_rt_strings_strings_contains",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_SPLIT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.split",
    symbol: "_mfb_rt_strings_strings_split",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PATTERN_PARAMS,
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_JOIN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.join",
    symbol: "_mfb_rt_strings_strings_join",
    abi: RuntimeHelperAbi {
        params: STRING_LIST_SEPARATOR_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const STRINGS_BYTE_LEN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Strings,
    call: "strings.byteLen",
    symbol: "_mfb_rt_strings_strings_byteLen",
    abi: RuntimeHelperAbi {
        params: STRING_VALUE_PARAMS,
        returns: "Integer",
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

const THREAD_START_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "f",
        type_: "ISOLATED FUNC(ThreadWorker OF Msg TO Out, In) AS Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "data",
        type_: "In",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "inboundLimit",
        type_: "Integer",
        location: "x2",
    },
    RuntimeAbiParam {
        name: "outboundLimit",
        type_: "Integer",
        location: "x3",
    },
];

const THREAD_HANDLE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "t",
    type_: "Thread OF Msg TO Out",
    location: "x0",
}];

const THREAD_WORKER_HANDLE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "t",
    type_: "ThreadWorker OF Msg TO Out",
    location: "x0",
}];

const THREAD_SEND_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "data",
        type_: "Msg",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x2",
    },
];

const THREAD_POLL_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "ms",
        type_: "Integer",
        location: "x1",
    },
];

const THREAD_RECEIVE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "ThreadWorker OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x1",
    },
];

const THREAD_PARENT_RECEIVE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "Thread OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x1",
    },
];

const THREAD_WORKER_SEND_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "t",
        type_: "ThreadWorker OF Msg TO Out",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "data",
        type_: "Msg",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x2",
    },
];

pub(crate) const THREAD_START_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.start",
    symbol: "_mfb_rt_thread_thread_start",
    abi: RuntimeHelperAbi {
        params: THREAD_START_PARAMS,
        returns: "Thread OF Msg TO Out",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_IS_RUNNING_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isRunning",
    symbol: "_mfb_rt_thread_thread_isRunning",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_WAIT_FOR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.waitFor",
    symbol: "_mfb_rt_thread_thread_waitFor",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Out",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_CANCEL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.cancel",
    symbol: "_mfb_rt_thread_thread_cancel",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_DROP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.drop",
    symbol: "_mfb_rt_thread_thread_drop",
    abi: RuntimeHelperAbi {
        params: THREAD_HANDLE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_SEND_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.send",
    symbol: "_mfb_rt_thread_thread_send",
    abi: RuntimeHelperAbi {
        params: THREAD_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.poll",
    symbol: "_mfb_rt_thread_thread_poll",
    abi: RuntimeHelperAbi {
        params: THREAD_POLL_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.read",
    symbol: "_mfb_rt_thread_thread_read",
    abi: RuntimeHelperAbi {
        params: THREAD_PARENT_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_RECEIVE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.receive",
    symbol: "_mfb_rt_thread_thread_receive",
    abi: RuntimeHelperAbi {
        params: THREAD_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_EMIT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.emit",
    symbol: "_mfb_rt_thread_thread_emit",
    abi: RuntimeHelperAbi {
        params: THREAD_WORKER_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_IS_CANCELLED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.isCancelled",
    symbol: "_mfb_rt_thread_thread_isCancelled",
    abi: RuntimeHelperAbi {
        params: THREAD_WORKER_HANDLE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// Resource plane (§7): `thread::transfer`/`thread::accept` mirror `send`/`receive`
// but run on a separate per-thread resource queue.
pub(crate) const THREAD_TRANSFER_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.transferResource",
    symbol: "_mfb_rt_thread_thread_transferResource",
    abi: RuntimeHelperAbi {
        params: THREAD_SEND_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const THREAD_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Thread,
    call: "thread.acceptResource",
    symbol: "_mfb_rt_thread_thread_acceptResource",
    abi: RuntimeHelperAbi {
        params: THREAD_RECEIVE_PARAMS,
        returns: "Msg",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

const NET_HOST_PORT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "host",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "port",
        type_: "Integer",
        location: "x1",
    },
];

const NET_CONNECT_TCP_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "host",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "port",
        type_: "Integer",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x2",
    },
];

const NET_LISTEN_TCP_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "host",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "port",
        type_: "Integer",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "backlog",
        type_: "Integer",
        location: "x2",
    },
];

const NET_SOCKET_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "sock",
    type_: "Socket",
    location: "x0",
}];

const NET_SOCKET_TIMEOUT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x1",
    },
];

const NET_LISTENER_TIMEOUT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "listener",
        type_: "Listener",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x1",
    },
];

const NET_SOCKET_INT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "maxBytes",
        type_: "Integer",
        location: "x1",
    },
];

const NET_SOCKET_BYTES_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: "x1",
    },
];

const NET_SOCKET_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "Socket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: "x1",
    },
];

pub(crate) const NET_LOOKUP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.lookup",
    symbol: "_mfb_rt_net_net_lookup",
    abi: RuntimeHelperAbi {
        params: NET_HOST_PORT_PARAMS,
        returns: "List OF Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_CONNECT_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.connectTcp",
    symbol: "_mfb_rt_net_net_connectTcp",
    abi: RuntimeHelperAbi {
        params: NET_CONNECT_TCP_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

const NET_CONNECT_TCP_ADDR_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "address",
        type_: "Address",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x1",
    },
];

pub(crate) const NET_CONNECT_TCP_ADDR_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.connectTcpAddr",
    symbol: "_mfb_rt_net_net_connectTcpAddr",
    abi: RuntimeHelperAbi {
        params: NET_CONNECT_TCP_ADDR_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_LISTEN_TCP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.listenTcp",
    symbol: "_mfb_rt_net_net_listenTcp",
    abi: RuntimeHelperAbi {
        params: NET_LISTEN_TCP_PARAMS,
        returns: "Listener",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_ACCEPT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.accept",
    symbol: "_mfb_rt_net_net_accept",
    abi: RuntimeHelperAbi {
        params: NET_LISTENER_TIMEOUT_PARAMS,
        returns: "Socket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_POLL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.poll",
    symbol: "_mfb_rt_net_net_poll",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.read",
    symbol: "_mfb_rt_net_net_read",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_INT_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.readText",
    symbol: "_mfb_rt_net_net_readText",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_INT_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.write",
    symbol: "_mfb_rt_net_net_write",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_BYTES_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.writeText",
    symbol: "_mfb_rt_net_net_writeText",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.close",
    symbol: "_mfb_rt_net_net_close",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_LOCAL_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.localAddress",
    symbol: "_mfb_rt_net_net_localAddress",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_REMOTE_ADDRESS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.remoteAddress",
    symbol: "_mfb_rt_net_net_remoteAddress",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_PARAMS,
        returns: "Address",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SET_READ_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setReadTimeout",
    symbol: "_mfb_rt_net_net_setReadTimeout",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SET_WRITE_TIMEOUT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.setWriteTimeout",
    symbol: "_mfb_rt_net_net_setWriteTimeout",
    abi: RuntimeHelperAbi {
        params: NET_SOCKET_TIMEOUT_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// ---------------------------------------------------------------------------
// UDP datagram sockets
// ---------------------------------------------------------------------------

const NET_UDP_SOCKET_INT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "UdpSocket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "maxBytes",
        type_: "Integer",
        location: "x1",
    },
];

const NET_SEND_TO_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "UdpSocket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "address",
        type_: "Address",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: "x2",
    },
];

const NET_SEND_TEXT_TO_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "UdpSocket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "address",
        type_: "Address",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: "x2",
    },
];

pub(crate) const NET_BIND_UDP_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.bindUdp",
    symbol: "_mfb_rt_net_net_bindUdp",
    abi: RuntimeHelperAbi {
        params: NET_HOST_PORT_PARAMS,
        returns: "UdpSocket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_RECEIVE_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveFrom",
    symbol: "_mfb_rt_net_net_receiveFrom",
    abi: RuntimeHelperAbi {
        params: NET_UDP_SOCKET_INT_PARAMS,
        returns: "Datagram",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_RECEIVE_TEXT_FROM_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.receiveTextFrom",
    symbol: "_mfb_rt_net_net_receiveTextFrom",
    abi: RuntimeHelperAbi {
        params: NET_UDP_SOCKET_INT_PARAMS,
        returns: "DatagramText",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SEND_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTo",
    symbol: "_mfb_rt_net_net_sendTo",
    abi: RuntimeHelperAbi {
        params: NET_SEND_TO_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const NET_SEND_TEXT_TO_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Net,
    call: "net.sendTextTo",
    symbol: "_mfb_rt_net_net_sendTextTo",
    abi: RuntimeHelperAbi {
        params: NET_SEND_TEXT_TO_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

// ---------------------------------------------------------------------------
// TLS (transport-layer security; Linux/OpenSSL backend, plan-03-net.md §4)
// ---------------------------------------------------------------------------

const TLS_CONNECT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "host",
        type_: "String",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "port",
        type_: "Integer",
        location: "x1",
    },
    RuntimeAbiParam {
        name: "timeoutMs",
        type_: "Integer",
        location: "x2",
    },
    RuntimeAbiParam {
        name: "serverName",
        type_: "String",
        location: "x3",
    },
];

const TLS_SOCKET_INT_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "TlsSocket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "maxBytes",
        type_: "Integer",
        location: "x1",
    },
];

const TLS_SOCKET_BYTES_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "TlsSocket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: "x1",
    },
];

const TLS_SOCKET_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "sock",
        type_: "TlsSocket",
        location: "x0",
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: "x1",
    },
];

const TLS_SOCKET_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "sock",
    type_: "TlsSocket",
    location: "x0",
}];

pub(crate) const TLS_CONNECT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.connect",
    symbol: "_mfb_rt_tls_tls_connect",
    abi: RuntimeHelperAbi {
        params: TLS_CONNECT_PARAMS,
        returns: "TlsSocket",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_READ_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.read",
    symbol: "_mfb_rt_tls_tls_read",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_INT_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.readText",
    symbol: "_mfb_rt_tls_tls_readText",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_INT_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_WRITE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.write",
    symbol: "_mfb_rt_tls_tls_write",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_BYTES_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.writeText",
    symbol: "_mfb_rt_tls_tls_writeText",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const TLS_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Tls,
    call: "tls.close",
    symbol: "_mfb_rt_tls_tls_close",
    abi: RuntimeHelperAbi {
        params: TLS_SOCKET_PARAMS,
        returns: "Nothing",
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
        STRINGS_TRIM_SPEC,
        STRINGS_TRIM_START_SPEC,
        STRINGS_TRIM_END_SPEC,
        STRINGS_UPPER_SPEC,
        STRINGS_LOWER_SPEC,
        STRINGS_CASE_FOLD_SPEC,
        STRINGS_NORMALIZE_NFC_SPEC,
        STRINGS_GRAPHEMES_SPEC,
        STRINGS_STARTS_WITH_SPEC,
        STRINGS_ENDS_WITH_SPEC,
        STRINGS_CONTAINS_SPEC,
        STRINGS_SPLIT_SPEC,
        STRINGS_JOIN_SPEC,
        STRINGS_BYTE_LEN_SPEC,
        THREAD_START_SPEC,
        THREAD_IS_RUNNING_SPEC,
        THREAD_WAIT_FOR_SPEC,
        THREAD_CANCEL_SPEC,
        THREAD_DROP_SPEC,
        THREAD_SEND_SPEC,
        THREAD_POLL_SPEC,
        THREAD_READ_SPEC,
        THREAD_RECEIVE_SPEC,
        THREAD_EMIT_SPEC,
        THREAD_TRANSFER_SPEC,
        THREAD_ACCEPT_SPEC,
        THREAD_IS_CANCELLED_SPEC,
        NET_LOOKUP_SPEC,
        NET_CONNECT_TCP_SPEC,
        NET_CONNECT_TCP_ADDR_SPEC,
        NET_LISTEN_TCP_SPEC,
        NET_ACCEPT_SPEC,
        NET_POLL_SPEC,
        NET_READ_SPEC,
        NET_READ_TEXT_SPEC,
        NET_WRITE_SPEC,
        NET_WRITE_TEXT_SPEC,
        NET_CLOSE_SPEC,
        NET_LOCAL_ADDRESS_SPEC,
        NET_REMOTE_ADDRESS_SPEC,
        NET_SET_READ_TIMEOUT_SPEC,
        NET_SET_WRITE_TIMEOUT_SPEC,
        NET_BIND_UDP_SPEC,
        NET_RECEIVE_FROM_SPEC,
        NET_RECEIVE_TEXT_FROM_SPEC,
        NET_SEND_TO_SPEC,
        NET_SEND_TEXT_TO_SPEC,
        TLS_CONNECT_SPEC,
        TLS_READ_SPEC,
        TLS_READ_TEXT_SPEC,
        TLS_WRITE_SPEC,
        TLS_WRITE_TEXT_SPEC,
        TLS_CLOSE_SPEC,
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
    } else if builtins::net::is_net_call(name) {
        Some(RuntimeHelper::Net)
    } else if builtins::tls::is_tls_call(name) {
        Some(RuntimeHelper::Tls)
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
            | "math.abs"
            | "math.min"
            | "math.max"
            | "math.clamp"
            | "math.floor"
            | "math.ceil"
            | "math.round"
            | "math.sqrt"
            | "math.pow"
            | "math.exp"
            | "math.log"
            | "math.log10"
            | "math.sin"
            | "math.cos"
            | "math.tan"
            | "math.asin"
            | "math.acos"
            | "math.atan"
            | "math.atan2"
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
    // Resource unions drop by dispatching to each variant's close op, so a bind
    // of a resource-union type pulls in every variant's close helper.
    let resource_union_closes: HashMap<String, Vec<&'static str>> = ir
        .types
        .iter()
        .filter(|type_| type_.kind == "union")
        .filter_map(|type_| {
            let closes: Vec<&'static str> = type_
                .variants
                .iter()
                .map(|variant| crate::builtins::resource_close_function(&variant.name))
                .collect::<Option<Vec<_>>>()?;
            if closes.is_empty() {
                return None;
            }
            Some((type_.name.clone(), closes))
        })
        .collect();
    for function in &ir.functions {
        push_op_helpers(&function.body, &resource_union_closes, &mut helpers);
    }
    helpers
}

fn push_op_helpers(
    ops: &[IrOp],
    resource_union_closes: &HashMap<String, Vec<&'static str>>,
    helpers: &mut Vec<RuntimeHelper>,
) {
    for op in ops {
        match op {
            IrOp::Bind { type_, value, .. } => {
                if let Some(close) = crate::builtins::resource_close_function(type_) {
                    if let Some(helper) = helper_for_call(close) {
                        push_unique(helpers, helper);
                    }
                }
                if let Some(closes) = resource_union_closes.get(type_) {
                    for close in closes {
                        if let Some(helper) = helper_for_call(close) {
                            push_unique(helpers, helper);
                        }
                    }
                }
                if let Some(value) = value {
                    push_value_helpers(value, helpers);
                }
            }
            IrOp::Fail { error } => {
                push_value_helpers(error, helpers);
            }
            IrOp::Assign { value, .. }
            | IrOp::AssignGlobal { value, .. }
            | IrOp::StateAssign { value, .. }
            | IrOp::Eval { value } => {
                push_value_helpers(value, helpers);
            }
            IrOp::Return { value } => {
                if let Some(value) = value {
                    push_value_helpers(value, helpers);
                }
            }
            IrOp::ExitLoop { .. } | IrOp::ContinueLoop { .. } => {}
            IrOp::ExitProgram { code } => push_value_helpers(code, helpers),
            IrOp::If {
                condition,
                then_body,
                else_body,
            } => {
                push_value_helpers(condition, helpers);
                push_op_helpers(then_body, resource_union_closes, helpers);
                push_op_helpers(else_body, resource_union_closes, helpers);
            }
            IrOp::Match { value, cases } => {
                push_value_helpers(value, helpers);
                for case in cases {
                    push_op_helpers(&case.body, resource_union_closes, helpers);
                }
            }
            IrOp::While {
                condition, body, ..
            } => {
                push_value_helpers(condition, helpers);
                push_op_helpers(body, resource_union_closes, helpers);
            }
            IrOp::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                push_value_helpers(start, helpers);
                push_value_helpers(end, helpers);
                push_value_helpers(step, helpers);
                push_op_helpers(body, resource_union_closes, helpers);
            }
            IrOp::DoUntil { body, condition } => {
                push_op_helpers(body, resource_union_closes, helpers);
                push_value_helpers(condition, helpers);
            }
            IrOp::ForEach { iterable, body, .. } => {
                push_value_helpers(iterable, helpers);
                push_op_helpers(body, resource_union_closes, helpers);
            }
            IrOp::Trap { body, .. } => {
                push_op_helpers(body, resource_union_closes, helpers);
            }
        }
    }
}

fn push_value_helpers(value: &IrValue, helpers: &mut Vec<RuntimeHelper>) {
    match value {
        IrValue::Call { target, args, .. } | IrValue::CallResult { target, args, .. } => {
            if !is_native_direct_call(target) {
                if let Some(helper) = helper_for_call(target) {
                    push_unique(helpers, helper);
                }
            }
            for arg in args {
                push_value_helpers(arg, helpers);
            }
        }
        IrValue::MemberAccess { target, member } => {
            if member == "result" {
                push_unique(helpers, RuntimeHelper::Thread);
            }
            push_value_helpers(target, helpers);
        }
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
        IrValue::UnionWrap { value, .. }
        | IrValue::UnionExtract { value, .. }
        | IrValue::ResultIsOk { value }
        | IrValue::ResultValue { value }
        | IrValue::ResultError { value } => {
            push_value_helpers(value, helpers);
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
        IrValue::Closure { captures, .. } => {
            for value in captures {
                push_value_helpers(value, helpers);
            }
        }
        IrValue::Capture { .. }
        | IrValue::Const { .. }
        | IrValue::Local(_)
        | IrValue::Global(_)
        | IrValue::FunctionRef { .. } => {}
    }
}

fn push_unique(helpers: &mut Vec<RuntimeHelper>, helper: RuntimeHelper) {
    if !helpers.contains(&helper) {
        helpers.push(helper);
    }
}
