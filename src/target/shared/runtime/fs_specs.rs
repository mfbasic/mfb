use super::*;

pub(crate) const FS_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.exists",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const FS_FILE_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.fileExists",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const FS_DIRECTORY_EXISTS_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.directoryExists",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const FS_CURRENT_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.currentDirectory",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const FS_TEMP_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.tempDirectory",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const FS_SET_CURRENT_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.setCurrentDirectory",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_DELETE_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.deleteFile",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_CREATE_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createDirectory",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_CREATE_DIRECTORIES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createDirectories",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_DELETE_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.deleteDirectory",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_LIST_DIRECTORY_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.listDirectory",
    abi: RuntimeHelperAbi {
        returns: "List OF String",
    },
};

pub(crate) const FS_OPEN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.open",
    abi: RuntimeHelperAbi { returns: "File" },
};

pub(crate) const FS_OPEN_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openFile",
    abi: RuntimeHelperAbi { returns: "File" },
};

pub(crate) const FS_OPEN_FILE_NO_FOLLOW_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openFileNoFollow",
    abi: RuntimeHelperAbi { returns: "File" },
};

pub(crate) const FS_OPEN_WITHIN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.openWithin",
    abi: RuntimeHelperAbi { returns: "File" },
};

pub(crate) const FS_CREATE_TEMP_FILE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.createTempFile",
    abi: RuntimeHelperAbi { returns: "File" },
};

pub(crate) const FS_CLOSE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.close",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_SET_BUFFERED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.setBuffered",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_IS_BUFFERED_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.isBuffered",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const FS_FLUSH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.flush",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_WRITE_ALL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeAll",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_READ_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readText",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const FS_READ_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readBytes",
    abi: RuntimeHelperAbi {
        returns: "List OF Byte",
    },
};

pub(crate) const FS_WRITE_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeText",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_WRITE_TEXT_ATOMIC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeTextAtomic",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_WRITE_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeBytes",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_WRITE_BYTES_ATOMIC_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeBytesAtomic",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_APPEND_TEXT_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.appendText",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_APPEND_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.appendBytes",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_READ_LINE_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readLine",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const FS_READ_ALL_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readAll",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const FS_READ_ALL_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.readAllBytes",
    abi: RuntimeHelperAbi {
        returns: "List OF Byte",
    },
};

pub(crate) const FS_WRITE_ALL_BYTES_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.writeAllBytes",
    abi: RuntimeHelperAbi { returns: "Nothing" },
};

pub(crate) const FS_EOF_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.eof",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};

pub(crate) const FS_CANONICAL_PATH_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.canonicalPath",
    abi: RuntimeHelperAbi { returns: "String" },
};

pub(crate) const FS_IS_WITHIN_SPEC: RuntimeHelperSpec = RuntimeHelperSpec {
    helper: RuntimeHelper::Fs,
    call: "fs.isWithin",
    abi: RuntimeHelperAbi { returns: "Boolean" },
};
