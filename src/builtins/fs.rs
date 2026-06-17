use crate::bytecode::{
    BuiltinCallLowerer, ValueSlot, OPCODE_FS_APPEND_BYTES, OPCODE_FS_APPEND_TEXT,
    OPCODE_FS_CANONICAL_PATH, OPCODE_FS_CLOSE, OPCODE_FS_CREATE_DIRECTORIES,
    OPCODE_FS_CREATE_DIRECTORY, OPCODE_FS_CREATE_TEMP_FILE, OPCODE_FS_CURRENT_DIRECTORY,
    OPCODE_FS_DELETE_DIRECTORY, OPCODE_FS_DELETE_FILE, OPCODE_FS_DIRECTORY_EXISTS, OPCODE_FS_EOF,
    OPCODE_FS_EXISTS, OPCODE_FS_FILE_EXISTS, OPCODE_FS_IS_WITHIN, OPCODE_FS_LIST_DIRECTORY,
    OPCODE_FS_OPEN, OPCODE_FS_OPEN_NO_FOLLOW, OPCODE_FS_PATH_BASE_NAME, OPCODE_FS_PATH_DIR_NAME,
    OPCODE_FS_PATH_EXTENSION, OPCODE_FS_PATH_JOIN, OPCODE_FS_PATH_NORMALIZE, OPCODE_FS_READ_ALL,
    OPCODE_FS_READ_ALL_BYTES, OPCODE_FS_READ_BYTES, OPCODE_FS_READ_LINE, OPCODE_FS_READ_TEXT,
    OPCODE_FS_SET_CURRENT_DIRECTORY, OPCODE_FS_TEMP_DIRECTORY, OPCODE_FS_WRITE_ALL,
    OPCODE_FS_WRITE_ALL_BYTES, OPCODE_FS_WRITE_BYTES, OPCODE_FS_WRITE_BYTES_ATOMIC,
    OPCODE_FS_WRITE_TEXT, OPCODE_FS_WRITE_TEXT_ATOMIC, TYPE_BOOLEAN, TYPE_FILE_HANDLE,
    TYPE_NOTHING, TYPE_STRING,
};
use crate::ir::IrValue;
use std::borrow::Cow;
use std::collections::HashMap;

const PACKAGE: &str = "fs";

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

pub(crate) fn lower_bytecode_call(
    lowerer: &mut dyn BuiltinCallLowerer,
    name: &str,
    args: &[IrValue],
    locals: &HashMap<String, ValueSlot>,
) -> Result<ValueSlot, String> {
    let mut lowered = args
        .iter()
        .map(|arg| lowerer.lower_value(arg, locals))
        .collect::<Result<Vec<_>, _>>()?;

    match name {
        OPEN_FILE | OPEN_FILE_NO_FOLLOW if lowered.len() == 1 => {
            lowered.push(lowerer.push_string_const("read")?);
        }
        CREATE_TEMP_FILE if lowered.is_empty() => {
            let register = lowerer.add_register(TYPE_STRING, 0);
            lowerer.push(OPCODE_FS_TEMP_DIRECTORY, vec![register]);
            lowered.push(ValueSlot {
                register,
                type_name: "String".to_string(),
            });
        }
        _ => {}
    }

    let arg_types = lowered
        .iter()
        .map(|slot| slot.type_name.clone())
        .collect::<Vec<_>>();
    let resolved = resolve_call(name, &arg_types).ok_or_else(|| {
        format!(
            "built-in `{name}` does not accept ({})",
            arg_types.join(", ")
        )
    })?;

    let dst_type_id = primitive_type_id(&resolved.return_type)
        .unwrap_or_else(|| lowerer.type_id(&resolved.return_type));
    let dst = lowerer.add_register(dst_type_id, 0);
    let mut operands = vec![dst];
    operands.extend(lowered.iter().map(|slot| slot.register));
    lowerer.push(opcode_for(name)?, operands);
    Ok(ValueSlot {
        register: dst,
        type_name: resolved.return_type.into_owned(),
    })
}

fn opcode_for(name: &str) -> Result<u16, String> {
    match name {
        FILE_EXISTS => Ok(OPCODE_FS_FILE_EXISTS),
        DIRECTORY_EXISTS => Ok(OPCODE_FS_DIRECTORY_EXISTS),
        EXISTS => Ok(OPCODE_FS_EXISTS),
        READ_BYTES => Ok(OPCODE_FS_READ_BYTES),
        READ_TEXT => Ok(OPCODE_FS_READ_TEXT),
        WRITE_BYTES => Ok(OPCODE_FS_WRITE_BYTES),
        WRITE_TEXT => Ok(OPCODE_FS_WRITE_TEXT),
        WRITE_BYTES_ATOMIC => Ok(OPCODE_FS_WRITE_BYTES_ATOMIC),
        WRITE_TEXT_ATOMIC => Ok(OPCODE_FS_WRITE_TEXT_ATOMIC),
        APPEND_BYTES => Ok(OPCODE_FS_APPEND_BYTES),
        APPEND_TEXT => Ok(OPCODE_FS_APPEND_TEXT),
        OPEN | OPEN_FILE => Ok(OPCODE_FS_OPEN),
        OPEN_FILE_NO_FOLLOW => Ok(OPCODE_FS_OPEN_NO_FOLLOW),
        CREATE_TEMP_FILE => Ok(OPCODE_FS_CREATE_TEMP_FILE),
        READ_LINE => Ok(OPCODE_FS_READ_LINE),
        READ_ALL => Ok(OPCODE_FS_READ_ALL),
        READ_ALL_BYTES => Ok(OPCODE_FS_READ_ALL_BYTES),
        WRITE_ALL => Ok(OPCODE_FS_WRITE_ALL),
        WRITE_ALL_BYTES => Ok(OPCODE_FS_WRITE_ALL_BYTES),
        CLOSE => Ok(OPCODE_FS_CLOSE),
        EOF => Ok(OPCODE_FS_EOF),
        CANONICAL_PATH => Ok(OPCODE_FS_CANONICAL_PATH),
        IS_WITHIN => Ok(OPCODE_FS_IS_WITHIN),
        PATH_JOIN => Ok(OPCODE_FS_PATH_JOIN),
        PATH_DIR_NAME => Ok(OPCODE_FS_PATH_DIR_NAME),
        PATH_BASE_NAME => Ok(OPCODE_FS_PATH_BASE_NAME),
        PATH_EXTENSION => Ok(OPCODE_FS_PATH_EXTENSION),
        PATH_NORMALIZE => Ok(OPCODE_FS_PATH_NORMALIZE),
        DELETE_FILE => Ok(OPCODE_FS_DELETE_FILE),
        CREATE_DIRECTORY => Ok(OPCODE_FS_CREATE_DIRECTORY),
        CREATE_DIRECTORIES => Ok(OPCODE_FS_CREATE_DIRECTORIES),
        DELETE_DIRECTORY => Ok(OPCODE_FS_DELETE_DIRECTORY),
        LIST_DIRECTORY => Ok(OPCODE_FS_LIST_DIRECTORY),
        CURRENT_DIRECTORY => Ok(OPCODE_FS_CURRENT_DIRECTORY),
        TEMP_DIRECTORY => Ok(OPCODE_FS_TEMP_DIRECTORY),
        SET_CURRENT_DIRECTORY => Ok(OPCODE_FS_SET_CURRENT_DIRECTORY),
        _ => Err(format!("unsupported {PACKAGE} built-in `{name}`")),
    }
}

fn exact(arg_types: &[String], expected: &[&str]) -> bool {
    arg_types.len() == expected.len()
        && arg_types
            .iter()
            .zip(expected.iter())
            .all(|(actual, expected)| actual == expected)
}

fn primitive_type_id(type_name: &str) -> Option<u32> {
    match type_name {
        "Nothing" => Some(TYPE_NOTHING),
        "Boolean" => Some(TYPE_BOOLEAN),
        "String" => Some(TYPE_STRING),
        FILE_TYPE => Some(TYPE_FILE_HANDLE),
        _ => None,
    }
}
