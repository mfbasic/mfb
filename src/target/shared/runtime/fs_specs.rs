use super::*;

use crate::target::shared::abi;

const FS_PATH_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "path",
    type_: "String",
    location: abi::ARG[0],
}];

const FS_PATH_MODE_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "path",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "mode",
        type_: "String",
        location: abi::ARG[1],
    },
];

const FS_FILE_PARAMS: &[RuntimeAbiParam] = &[RuntimeAbiParam {
    name: "file",
    type_: "File",
    location: abi::ARG[0],
}];

const FS_FILE_BOOLEAN_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "file",
        type_: "File",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "enabled",
        type_: "Boolean",
        location: abi::ARG[1],
    },
];

const FS_FILE_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "file",
        type_: "File",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: abi::ARG[1],
    },
];

const FS_PATH_STRING_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "path",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "value",
        type_: "String",
        location: abi::ARG[1],
    },
];

const FS_TWO_PATH_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "base",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "child",
        type_: "String",
        location: abi::ARG[1],
    },
];

const FS_FILE_BYTE_LIST_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "file",
        type_: "File",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: abi::ARG[1],
    },
];

const FS_PATH_BYTE_LIST_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "path",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "bytes",
        type_: "List OF Byte",
        location: abi::ARG[1],
    },
];

pub(crate) const FS_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.exists",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_FILE_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.fileExists",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_DIRECTORY_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.directoryExists",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CURRENT_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.currentDirectory",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_TEMP_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.tempDirectory",
    abi: RuntimeHelperAbi {
        params: &[],
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_SET_CURRENT_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.setCurrentDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_DELETE_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.deleteFile",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CREATE_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CREATE_DIRECTORIES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createDirectories",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_DELETE_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.deleteDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_LIST_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.listDirectory",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "List OF String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_OPEN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.open",
    abi: RuntimeHelperAbi {
        params: FS_PATH_MODE_PARAMS,
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_OPEN_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openFile",
    abi: RuntimeHelperAbi {
        params: FS_PATH_MODE_PARAMS,
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_OPEN_FILE_NO_FOLLOW_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openFileNoFollow",
    abi: RuntimeHelperAbi {
        params: FS_PATH_MODE_PARAMS,
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

const FS_OPEN_WITHIN_PARAMS: &[RuntimeAbiParam] = &[
    RuntimeAbiParam {
        name: "root",
        type_: "String",
        location: abi::ARG[0],
    },
    RuntimeAbiParam {
        name: "relPath",
        type_: "String",
        location: abi::ARG[1],
    },
    RuntimeAbiParam {
        name: "mode",
        type_: "String",
        location: abi::ARG[2],
    },
];

pub(crate) const FS_OPEN_WITHIN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openWithin",
    abi: RuntimeHelperAbi {
        params: FS_OPEN_WITHIN_PARAMS,
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CREATE_TEMP_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createTempFile",
    abi: RuntimeHelperAbi {
        params: &[RuntimeAbiParam {
            name: "directory",
            type_: "String",
            location: abi::ARG[0],
        }],
        returns: "File",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.close",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_SET_BUFFERED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.setBuffered",
    abi: RuntimeHelperAbi {
        params: FS_FILE_BOOLEAN_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_IS_BUFFERED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.isBuffered",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_FLUSH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.flush",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_ALL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeAll",
    abi: RuntimeHelperAbi {
        params: FS_FILE_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readText",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readBytes",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeText",
    abi: RuntimeHelperAbi {
        params: FS_PATH_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_TEXT_ATOMIC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeTextAtomic",
    abi: RuntimeHelperAbi {
        params: FS_PATH_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeBytes",
    abi: RuntimeHelperAbi {
        params: FS_PATH_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_BYTES_ATOMIC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeBytesAtomic",
    abi: RuntimeHelperAbi {
        params: FS_PATH_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_APPEND_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.appendText",
    abi: RuntimeHelperAbi {
        params: FS_PATH_STRING_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_APPEND_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.appendBytes",
    abi: RuntimeHelperAbi {
        params: FS_PATH_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_LINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readLine",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_ALL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readAll",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_READ_ALL_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readAllBytes",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "List OF Byte",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_WRITE_ALL_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeAllBytes",
    abi: RuntimeHelperAbi {
        params: FS_FILE_BYTE_LIST_PARAMS,
        returns: "Nothing",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_EOF_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.eof",
    abi: RuntimeHelperAbi {
        params: FS_FILE_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_CANONICAL_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.canonicalPath",
    abi: RuntimeHelperAbi {
        params: FS_PATH_PARAMS,
        returns: "String",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};

pub(crate) const FS_IS_WITHIN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.isWithin",
    abi: RuntimeHelperAbi {
        params: FS_TWO_PATH_PARAMS,
        returns: "Boolean",
        clobbers: abi::IO_PRINT_CLOBBERS,
    },
};
