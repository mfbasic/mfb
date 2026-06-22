use std::borrow::Cow;

pub(crate) const FILE_TYPE: &str = "File";

const FILE_EXISTS: &str = "fs.fileExists";
const DIRECTORY_EXISTS: &str = "fs.directoryExists";
const EXISTS: &str = "fs.exists";
const READ_BYTES: &str = "fs.readBytes";
const READ_TEXT: &str = "fs.readText";
const WRITE_BYTES: &str = "fs.writeBytes";
const WRITE_TEXT: &str = "fs.writeText";
const WRITE_BYTES_ATOMIC: &str = "fs.writeBytesAtomic";
const WRITE_TEXT_ATOMIC: &str = "fs.writeTextAtomic";
const APPEND_BYTES: &str = "fs.appendBytes";
const APPEND_TEXT: &str = "fs.appendText";
const OPEN: &str = "fs.open";
const OPEN_FILE: &str = "fs.openFile";
const OPEN_FILE_NO_FOLLOW: &str = "fs.openFileNoFollow";
const CREATE_TEMP_FILE: &str = "fs.createTempFile";
const TEMP_DIRECTORY: &str = "fs.tempDirectory";
const READ_LINE: &str = "fs.readLine";
const READ_ALL: &str = "fs.readAll";
const READ_ALL_BYTES: &str = "fs.readAllBytes";
const WRITE_ALL: &str = "fs.writeAll";
const WRITE_ALL_BYTES: &str = "fs.writeAllBytes";
const CLOSE: &str = "fs.close";
const EOF: &str = "fs.eof";
const CANONICAL_PATH: &str = "fs.canonicalPath";
const IS_WITHIN: &str = "fs.isWithin";
const PATH_JOIN: &str = "fs.pathJoin";
const PATH_DIR_NAME: &str = "fs.pathDirName";
const PATH_BASE_NAME: &str = "fs.pathBaseName";
const PATH_EXTENSION: &str = "fs.pathExtension";
const PATH_NORMALIZE: &str = "fs.pathNormalize";
const DELETE_FILE: &str = "fs.deleteFile";
const CREATE_DIRECTORY: &str = "fs.createDirectory";
const CREATE_DIRECTORIES: &str = "fs.createDirectories";
const DELETE_DIRECTORY: &str = "fs.deleteDirectory";
const LIST_DIRECTORY: &str = "fs.listDirectory";
const CURRENT_DIRECTORY: &str = "fs.currentDirectory";
const SET_CURRENT_DIRECTORY: &str = "fs.setCurrentDirectory";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_fs_call(name: &str) -> bool {
    matches!(
        name,
        FILE_EXISTS
            | DIRECTORY_EXISTS
            | EXISTS
            | READ_BYTES
            | READ_TEXT
            | WRITE_BYTES
            | WRITE_TEXT
            | WRITE_BYTES_ATOMIC
            | WRITE_TEXT_ATOMIC
            | APPEND_BYTES
            | APPEND_TEXT
            | OPEN
            | OPEN_FILE
            | OPEN_FILE_NO_FOLLOW
            | CREATE_TEMP_FILE
            | TEMP_DIRECTORY
            | READ_LINE
            | READ_ALL
            | READ_ALL_BYTES
            | WRITE_ALL
            | WRITE_ALL_BYTES
            | CLOSE
            | EOF
            | CANONICAL_PATH
            | IS_WITHIN
            | PATH_JOIN
            | PATH_DIR_NAME
            | PATH_BASE_NAME
            | PATH_EXTENSION
            | PATH_NORMALIZE
            | DELETE_FILE
            | CREATE_DIRECTORY
            | CREATE_DIRECTORIES
            | DELETE_DIRECTORY
            | LIST_DIRECTORY
            | CURRENT_DIRECTORY
            | SET_CURRENT_DIRECTORY
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == FILE_TYPE
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        FILE_TYPE => Some(CLOSE),
        _ => None,
    }
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        FILE_EXISTS | DIRECTORY_EXISTS | EXISTS | READ_BYTES | READ_TEXT | CANONICAL_PATH
        | PATH_DIR_NAME | PATH_BASE_NAME | PATH_EXTENSION | PATH_NORMALIZE | DELETE_FILE
        | CREATE_DIRECTORY | CREATE_DIRECTORIES | DELETE_DIRECTORY | LIST_DIRECTORY
        | SET_CURRENT_DIRECTORY => Some(&[&["path"]]),
        WRITE_BYTES | WRITE_BYTES_ATOMIC | APPEND_BYTES => Some(&[&["path"], &["bytes", "value"]]),
        WRITE_TEXT | WRITE_TEXT_ATOMIC | APPEND_TEXT => Some(&[&["path"], &["value"]]),
        OPEN | OPEN_FILE | OPEN_FILE_NO_FOLLOW => Some(&[&["path"], &["mode"]]),
        CREATE_TEMP_FILE => Some(&[&["directory"]]),
        TEMP_DIRECTORY | CURRENT_DIRECTORY => Some(&[]),
        READ_LINE | READ_ALL | READ_ALL_BYTES | CLOSE | EOF => Some(&[&["file"]]),
        WRITE_ALL => Some(&[&["file"], &["value"]]),
        WRITE_ALL_BYTES => Some(&[&["file"], &["bytes", "value"]]),
        IS_WITHIN => Some(&[&["base", "path"], &["child", "parent"]]),
        PATH_JOIN => Some(&[&["parts"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        FILE_EXISTS | DIRECTORY_EXISTS | EXISTS | EOF | IS_WITHIN => Some("Boolean"),
        READ_BYTES | READ_ALL_BYTES => Some("List OF Byte"),
        READ_TEXT | READ_LINE | READ_ALL | CANONICAL_PATH | PATH_JOIN | PATH_DIR_NAME
        | PATH_BASE_NAME | PATH_EXTENSION | PATH_NORMALIZE | CURRENT_DIRECTORY | TEMP_DIRECTORY => {
            Some("String")
        }
        WRITE_BYTES
        | WRITE_TEXT
        | WRITE_BYTES_ATOMIC
        | WRITE_TEXT_ATOMIC
        | APPEND_BYTES
        | APPEND_TEXT
        | WRITE_ALL
        | WRITE_ALL_BYTES
        | CLOSE
        | DELETE_FILE
        | CREATE_DIRECTORY
        | CREATE_DIRECTORIES
        | DELETE_DIRECTORY
        | SET_CURRENT_DIRECTORY => Some("Nothing"),
        OPEN | OPEN_FILE | OPEN_FILE_NO_FOLLOW | CREATE_TEMP_FILE => Some(FILE_TYPE),
        LIST_DIRECTORY => Some("List OF String"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        FILE_EXISTS
        | DIRECTORY_EXISTS
        | EXISTS
        | READ_BYTES
        | READ_TEXT
        | CANONICAL_PATH
        | PATH_DIR_NAME
        | PATH_BASE_NAME
        | PATH_EXTENSION
        | PATH_NORMALIZE
        | DELETE_FILE
        | CREATE_DIRECTORY
        | CREATE_DIRECTORIES
        | DELETE_DIRECTORY
        | LIST_DIRECTORY
        | SET_CURRENT_DIRECTORY
            if exact(arg_types, &["String"]) =>
        {
            Cow::Borrowed(call_return_type_name(name)?)
        }
        WRITE_BYTES | WRITE_BYTES_ATOMIC | APPEND_BYTES
            if exact(arg_types, &["String", "List OF Byte"]) =>
        {
            Cow::Borrowed("Nothing")
        }
        WRITE_TEXT | WRITE_TEXT_ATOMIC | APPEND_TEXT if exact(arg_types, &["String", "String"]) => {
            Cow::Borrowed("Nothing")
        }
        OPEN if exact(arg_types, &["String", "String"]) => Cow::Borrowed(FILE_TYPE),
        OPEN_FILE | OPEN_FILE_NO_FOLLOW
            if exact(arg_types, &["String"]) || exact(arg_types, &["String", "String"]) =>
        {
            Cow::Borrowed(FILE_TYPE)
        }
        CREATE_TEMP_FILE if arg_types.is_empty() || exact(arg_types, &["String"]) => {
            Cow::Borrowed(FILE_TYPE)
        }
        TEMP_DIRECTORY if arg_types.is_empty() => Cow::Borrowed("String"),
        READ_LINE | READ_ALL if exact(arg_types, &[FILE_TYPE]) => Cow::Borrowed("String"),
        READ_ALL_BYTES if exact(arg_types, &[FILE_TYPE]) => Cow::Borrowed("List OF Byte"),
        WRITE_ALL if exact(arg_types, &[FILE_TYPE, "String"]) => Cow::Borrowed("Nothing"),
        WRITE_ALL_BYTES if exact(arg_types, &[FILE_TYPE, "List OF Byte"]) => {
            Cow::Borrowed("Nothing")
        }
        CLOSE if exact(arg_types, &[FILE_TYPE]) => Cow::Borrowed("Nothing"),
        EOF if exact(arg_types, &[FILE_TYPE]) => Cow::Borrowed("Boolean"),
        IS_WITHIN if exact(arg_types, &["String", "String"]) => Cow::Borrowed("Boolean"),
        PATH_JOIN if exact(arg_types, &["List OF String"]) => Cow::Borrowed("String"),
        CURRENT_DIRECTORY if arg_types.is_empty() => Cow::Borrowed("String"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        FILE_EXISTS
        | DIRECTORY_EXISTS
        | EXISTS
        | READ_BYTES
        | READ_TEXT
        | CANONICAL_PATH
        | PATH_DIR_NAME
        | PATH_BASE_NAME
        | PATH_EXTENSION
        | PATH_NORMALIZE
        | DELETE_FILE
        | CREATE_DIRECTORY
        | CREATE_DIRECTORIES
        | DELETE_DIRECTORY
        | LIST_DIRECTORY
        | SET_CURRENT_DIRECTORY => Some("String"),
        WRITE_BYTES | WRITE_BYTES_ATOMIC | APPEND_BYTES => Some("String, List OF Byte"),
        WRITE_TEXT | WRITE_TEXT_ATOMIC | APPEND_TEXT => Some("String, String"),
        OPEN => Some("String, String"),
        OPEN_FILE | OPEN_FILE_NO_FOLLOW => Some("String, String"),
        CREATE_TEMP_FILE => Some("String"),
        READ_LINE | READ_ALL | READ_ALL_BYTES | CLOSE | EOF => Some(FILE_TYPE),
        WRITE_ALL => Some("File, String"),
        WRITE_ALL_BYTES => Some("File, List OF Byte"),
        IS_WITHIN => Some("String, String"),
        PATH_JOIN => Some("List OF String"),
        CURRENT_DIRECTORY | TEMP_DIRECTORY => Some("no arguments"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        FILE_EXISTS
        | DIRECTORY_EXISTS
        | EXISTS
        | READ_BYTES
        | READ_TEXT
        | CANONICAL_PATH
        | PATH_DIR_NAME
        | PATH_BASE_NAME
        | PATH_EXTENSION
        | PATH_NORMALIZE
        | DELETE_FILE
        | CREATE_DIRECTORY
        | CREATE_DIRECTORIES
        | DELETE_DIRECTORY
        | LIST_DIRECTORY
        | SET_CURRENT_DIRECTORY => Some((1, 1)),
        WRITE_BYTES | WRITE_BYTES_ATOMIC | APPEND_BYTES | WRITE_TEXT | WRITE_TEXT_ATOMIC
        | APPEND_TEXT | OPEN | WRITE_ALL | WRITE_ALL_BYTES | IS_WITHIN => Some((2, 2)),
        OPEN_FILE | OPEN_FILE_NO_FOLLOW => Some((1, 2)),
        CREATE_TEMP_FILE => Some((0, 1)),
        READ_LINE | READ_ALL | READ_ALL_BYTES | CLOSE | EOF => Some((1, 1)),
        PATH_JOIN => Some((1, 1)),
        CURRENT_DIRECTORY | TEMP_DIRECTORY => Some((0, 0)),
        _ => None,
    }
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}
