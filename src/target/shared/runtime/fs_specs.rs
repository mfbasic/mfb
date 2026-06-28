use super::*;

use crate::arch::aarch64::abi;

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

