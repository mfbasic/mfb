use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinType, DefaultResolver,
    DefaultValue, Implementation, Lowering, Parameter, ParameterType, ReturnType, TypeKind,
};

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
const OPEN_WITHIN: &str = "fs.openWithin";
const CREATE_TEMP_FILE: &str = "fs.createTempFile";
const TEMP_DIRECTORY: &str = "fs.tempDirectory";
const READ_LINE: &str = "fs.readLine";
const READ_ALL: &str = "fs.readAll";
const READ_ALL_BYTES: &str = "fs.readAllBytes";
const WRITE_ALL: &str = "fs.writeAll";
const WRITE_ALL_BYTES: &str = "fs.writeAllBytes";
const SET_BUFFERED: &str = "fs.setBuffered";
const IS_BUFFERED: &str = "fs.isBuffered";
const FLUSH: &str = "fs.flush";
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

// plan-72-K: `FS` is the descriptor authority for this package. Every function has
// a single fixed-return overload and its `resolve_call` arms are pure per-position
// exact-type matches, so `is_fs_call`, `arity`, `call_return_type_name`, and
// `resolve_call` all derive from the descriptor via `DefaultResolver`. Optional
// trailing arguments (`open*`'s `mode`, `createTempFile`'s `directory`) are
// `DefaultValue::Optional` — they widen arity without default padding (fs has no
// default-padding helper). `File` is the one opaque builtin resource type.
// `call_param_names` (a `&'static` borrowed shape) and `expected_arguments`
// (bespoke `"String[, String]"` / `"no arguments"` phrasing) stay hand-authored
// statics pinned by parity where derivable. `resource_close_function` and
// `consumes_argument` are fs-specific (not descriptor-generic) and untouched.
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn ffn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const fn req(name: &'static str, ty: &'static str) -> Parameter {
    Parameter::required(name, ty)
}

// `bytes` accepts the alias `value` (writeBytes/appendBytes/writeAllBytes).
const BYTES_VALUE: Parameter = Parameter {
    name: "bytes",
    aliases: &["value"],
    ty: ParameterType::Named("List OF Byte"),
    default: DefaultValue::None,
};
const fn opt(name: &'static str, ty: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: ParameterType::Named(ty),
        default: DefaultValue::Optional,
    }
}

const P_PATH: &[Parameter] = &[req("path", "String")];
const P_FILE: &[Parameter] = &[req("file", "File")];
const P_NONE: &[Parameter] = &[];
const P_PATH_BYTES: &[Parameter] = &[req("path", "String"), BYTES_VALUE];
const P_PATH_VALUE: &[Parameter] = &[req("path", "String"), req("value", "String")];
const P_OPEN: &[Parameter] = &[req("path", "String"), req("mode", "String")];
const P_OPEN_FILE: &[Parameter] = &[req("path", "String"), opt("mode", "String")];
const P_OPEN_WITHIN: &[Parameter] =
    &[req("root", "String"), req("relPath", "String"), opt("mode", "String")];
const P_CREATE_TEMP: &[Parameter] = &[opt("directory", "String")];
const P_FILE_VALUE: &[Parameter] = &[req("file", "File"), req("value", "String")];
const P_FILE_BYTES: &[Parameter] = &[req("file", "File"), BYTES_VALUE];
const P_FILE_ENABLED: &[Parameter] = &[req("file", "File"), req("enabled", "Boolean")];
// `base`/`child` accept the aliases `path`/`parent`.
const P_IS_WITHIN: &[Parameter] = &[
    Parameter {
        name: "base",
        aliases: &["path"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
    Parameter {
        name: "child",
        aliases: &["parent"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
];
const P_PARTS: &[Parameter] = &[req("parts", "List OF String")];

const FS_FUNCTIONS: &[BuiltinFunction] = &[
    ffn(FILE_EXISTS, "fileExists", &[ov(P_PATH, "Boolean")]),
    ffn(DIRECTORY_EXISTS, "directoryExists", &[ov(P_PATH, "Boolean")]),
    ffn(EXISTS, "exists", &[ov(P_PATH, "Boolean")]),
    ffn(READ_BYTES, "readBytes", &[ov(P_PATH, "List OF Byte")]),
    ffn(READ_TEXT, "readText", &[ov(P_PATH, "String")]),
    ffn(WRITE_BYTES, "writeBytes", &[ov(P_PATH_BYTES, "Nothing")]),
    ffn(WRITE_TEXT, "writeText", &[ov(P_PATH_VALUE, "Nothing")]),
    ffn(WRITE_BYTES_ATOMIC, "writeBytesAtomic", &[ov(P_PATH_BYTES, "Nothing")]),
    ffn(WRITE_TEXT_ATOMIC, "writeTextAtomic", &[ov(P_PATH_VALUE, "Nothing")]),
    ffn(APPEND_BYTES, "appendBytes", &[ov(P_PATH_BYTES, "Nothing")]),
    ffn(APPEND_TEXT, "appendText", &[ov(P_PATH_VALUE, "Nothing")]),
    ffn(OPEN, "open", &[ov(P_OPEN, FILE_TYPE)]),
    ffn(OPEN_FILE, "openFile", &[ov(P_OPEN_FILE, FILE_TYPE)]),
    ffn(OPEN_FILE_NO_FOLLOW, "openFileNoFollow", &[ov(P_OPEN_FILE, FILE_TYPE)]),
    ffn(OPEN_WITHIN, "openWithin", &[ov(P_OPEN_WITHIN, FILE_TYPE)]),
    ffn(CREATE_TEMP_FILE, "createTempFile", &[ov(P_CREATE_TEMP, FILE_TYPE)]),
    ffn(TEMP_DIRECTORY, "tempDirectory", &[ov(P_NONE, "String")]),
    ffn(READ_LINE, "readLine", &[ov(P_FILE, "String")]),
    ffn(READ_ALL, "readAll", &[ov(P_FILE, "String")]),
    ffn(READ_ALL_BYTES, "readAllBytes", &[ov(P_FILE, "List OF Byte")]),
    ffn(WRITE_ALL, "writeAll", &[ov(P_FILE_VALUE, "Nothing")]),
    ffn(WRITE_ALL_BYTES, "writeAllBytes", &[ov(P_FILE_BYTES, "Nothing")]),
    ffn(SET_BUFFERED, "setBuffered", &[ov(P_FILE_ENABLED, "Nothing")]),
    ffn(IS_BUFFERED, "isBuffered", &[ov(P_FILE, "Boolean")]),
    ffn(FLUSH, "flush", &[ov(P_FILE, "Nothing")]),
    ffn(CLOSE, "close", &[ov(P_FILE, "Nothing")]),
    ffn(EOF, "eof", &[ov(P_FILE, "Boolean")]),
    ffn(CANONICAL_PATH, "canonicalPath", &[ov(P_PATH, "String")]),
    ffn(IS_WITHIN, "isWithin", &[ov(P_IS_WITHIN, "Boolean")]),
    ffn(PATH_JOIN, "pathJoin", &[ov(P_PARTS, "String")]),
    ffn(PATH_DIR_NAME, "pathDirName", &[ov(P_PATH, "String")]),
    ffn(PATH_BASE_NAME, "pathBaseName", &[ov(P_PATH, "String")]),
    ffn(PATH_EXTENSION, "pathExtension", &[ov(P_PATH, "String")]),
    ffn(PATH_NORMALIZE, "pathNormalize", &[ov(P_PATH, "String")]),
    ffn(DELETE_FILE, "deleteFile", &[ov(P_PATH, "Nothing")]),
    ffn(CREATE_DIRECTORY, "createDirectory", &[ov(P_PATH, "Nothing")]),
    ffn(CREATE_DIRECTORIES, "createDirectories", &[ov(P_PATH, "Nothing")]),
    ffn(DELETE_DIRECTORY, "deleteDirectory", &[ov(P_PATH, "Nothing")]),
    ffn(LIST_DIRECTORY, "listDirectory", &[ov(P_PATH, "List OF String")]),
    ffn(CURRENT_DIRECTORY, "currentDirectory", &[ov(P_NONE, "String")]),
    ffn(SET_CURRENT_DIRECTORY, "setCurrentDirectory", &[ov(P_PATH, "Nothing")]),
];

const FS_TYPES: &[BuiltinType] = &[BuiltinType {
    name: FILE_TYPE,
    kind: TypeKind::Opaque,
    fields: &[],
}];

pub(crate) static FS: BuiltinModule = BuiltinModule {
    name: "fs",
    functions: FS_FUNCTIONS,
    types: FS_TYPES,
    source: None,
    resolver: None,
};

pub(crate) fn is_fs_call(name: &str) -> bool {
    DefaultResolver::contains(&FS, name)
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    FS.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    match type_name {
        FILE_TYPE => Some(CLOSE),
        _ => None,
    }
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
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
        | SET_CURRENT_DIRECTORY => Some(&[&["path"]]),
        WRITE_BYTES | WRITE_BYTES_ATOMIC | APPEND_BYTES => Some(&[&["path"], &["bytes", "value"]]),
        WRITE_TEXT | WRITE_TEXT_ATOMIC | APPEND_TEXT => Some(&[&["path"], &["value"]]),
        OPEN | OPEN_FILE | OPEN_FILE_NO_FOLLOW => Some(&[&["path"], &["mode"]]),
        OPEN_WITHIN => Some(&[&["root"], &["relPath"], &["mode"]]),
        CREATE_TEMP_FILE => Some(&[&["directory"]]),
        TEMP_DIRECTORY | CURRENT_DIRECTORY => Some(&[]),
        READ_LINE | READ_ALL | READ_ALL_BYTES | CLOSE | EOF | IS_BUFFERED | FLUSH => {
            Some(&[&["file"]])
        }
        WRITE_ALL => Some(&[&["file"], &["value"]]),
        WRITE_ALL_BYTES => Some(&[&["file"], &["bytes", "value"]]),
        SET_BUFFERED => Some(&[&["file"], &["enabled"]]),
        IS_WITHIN => Some(&[&["base", "path"], &["child", "parent"]]),
        PATH_JOIN => Some(&[&["parts"]]),
        _ => None,
    }
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
        // `mode` is optional (arity 1..=2), so spell it as such rather than
        // advertising only the maximal form (bug-213).
        OPEN_FILE | OPEN_FILE_NO_FOLLOW => Some("String[, String]"),
        OPEN_WITHIN => Some("String, String[, String]"),
        CREATE_TEMP_FILE => Some("String"),
        READ_LINE | READ_ALL | READ_ALL_BYTES | CLOSE | EOF | IS_BUFFERED | FLUSH => {
            Some(FILE_TYPE)
        }
        WRITE_ALL => Some("File, String"),
        WRITE_ALL_BYTES => Some("File, List OF Byte"),
        SET_BUFFERED => Some("File, Boolean"),
        IS_WITHIN => Some("String, String"),
        PATH_JOIN => Some("List OF String"),
        CURRENT_DIRECTORY | TEMP_DIRECTORY => Some("no arguments"),
        _ => None,
    }
}

/// Whether argument `index` of `name` consumes (moves) its resource operand.
/// `fs.close` consumes the `RES File` it closes; every other call only uses the
/// file, which stays open.
pub(crate) fn consumes_argument(name: &str, index: usize) -> bool {
    matches!((name, index), (CLOSE, 0))
}

// `exact` is no longer used by production `fs` code (resolve_call now derives from
// the descriptor); only the `exact_helper` unit test exercises the shared helper.
#[cfg(test)]
use super::exact;

#[cfg(test)]
mod tests {
    use super::*;

    fn types(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    const ALL: &[&str] = &[
        FILE_EXISTS,
        DIRECTORY_EXISTS,
        EXISTS,
        READ_BYTES,
        READ_TEXT,
        WRITE_BYTES,
        WRITE_TEXT,
        WRITE_BYTES_ATOMIC,
        WRITE_TEXT_ATOMIC,
        APPEND_BYTES,
        APPEND_TEXT,
        OPEN,
        OPEN_FILE,
        OPEN_FILE_NO_FOLLOW,
        OPEN_WITHIN,
        CREATE_TEMP_FILE,
        TEMP_DIRECTORY,
        READ_LINE,
        READ_ALL,
        READ_ALL_BYTES,
        WRITE_ALL,
        WRITE_ALL_BYTES,
        SET_BUFFERED,
        IS_BUFFERED,
        FLUSH,
        CLOSE,
        EOF,
        CANONICAL_PATH,
        IS_WITHIN,
        PATH_JOIN,
        PATH_DIR_NAME,
        PATH_BASE_NAME,
        PATH_EXTENSION,
        PATH_NORMALIZE,
        DELETE_FILE,
        CREATE_DIRECTORY,
        CREATE_DIRECTORIES,
        DELETE_DIRECTORY,
        LIST_DIRECTORY,
        CURRENT_DIRECTORY,
        SET_CURRENT_DIRECTORY,
    ];

    #[test]
    fn is_fs_call_recognizes_all_and_rejects_others() {
        for name in ALL {
            assert!(is_fs_call(name), "{name}");
        }
        assert!(!is_fs_call("fs.unknown"));
        assert!(!is_fs_call("strings.trim"));
        assert!(!is_fs_call(""));
    }

    #[test]
    fn builtin_type_and_resource_close() {
        assert!(is_builtin_type(FILE_TYPE));
        assert!(!is_builtin_type("String"));
        assert!(!is_builtin_type("Directory"));
        assert_eq!(resource_close_function(FILE_TYPE), Some(CLOSE));
        assert_eq!(resource_close_function("String"), None);
        assert_eq!(resource_close_function("Socket"), None);
    }

    #[test]
    fn param_names_specific() {
        assert_eq!(call_param_names(FILE_EXISTS), Some(&[&["path"][..]][..]));
        assert_eq!(
            call_param_names(WRITE_BYTES),
            Some(&[&["path"][..], &["bytes", "value"][..]][..])
        );
        assert_eq!(
            call_param_names(WRITE_TEXT),
            Some(&[&["path"][..], &["value"][..]][..])
        );
        assert_eq!(
            call_param_names(OPEN),
            Some(&[&["path"][..], &["mode"][..]][..])
        );
        assert_eq!(
            call_param_names(CREATE_TEMP_FILE),
            Some(&[&["directory"][..]][..])
        );
        assert_eq!(call_param_names(TEMP_DIRECTORY), Some(&[][..]));
        assert_eq!(call_param_names(CURRENT_DIRECTORY), Some(&[][..]));
        assert_eq!(call_param_names(READ_LINE), Some(&[&["file"][..]][..]));
        assert_eq!(
            call_param_names(WRITE_ALL),
            Some(&[&["file"][..], &["value"][..]][..])
        );
        assert_eq!(
            call_param_names(WRITE_ALL_BYTES),
            Some(&[&["file"][..], &["bytes", "value"][..]][..])
        );
        assert_eq!(
            call_param_names(IS_WITHIN),
            Some(&[&["base", "path"][..], &["child", "parent"][..]][..])
        );
        assert_eq!(call_param_names(PATH_JOIN), Some(&[&["parts"][..]][..]));
    }

    #[test]
    fn expected_arguments_specific() {
        assert_eq!(expected_arguments(FILE_EXISTS), Some("String"));
        assert_eq!(
            expected_arguments(WRITE_BYTES),
            Some("String, List OF Byte")
        );
        assert_eq!(expected_arguments(WRITE_TEXT), Some("String, String"));
        assert_eq!(expected_arguments(OPEN), Some("String, String"));
        // bug-213: `mode` is optional (arity 1..=2), so it is spelled as optional.
        assert_eq!(expected_arguments(OPEN_FILE), Some("String[, String]"));
        assert_eq!(
            expected_arguments(OPEN_FILE_NO_FOLLOW),
            Some("String[, String]")
        );
        assert_eq!(expected_arguments(CREATE_TEMP_FILE), Some("String"));
        assert_eq!(expected_arguments(READ_LINE), Some(FILE_TYPE));
        assert_eq!(expected_arguments(WRITE_ALL), Some("File, String"));
        assert_eq!(
            expected_arguments(WRITE_ALL_BYTES),
            Some("File, List OF Byte")
        );
        assert_eq!(expected_arguments(IS_WITHIN), Some("String, String"));
        assert_eq!(expected_arguments(PATH_JOIN), Some("List OF String"));
        assert_eq!(expected_arguments(CURRENT_DIRECTORY), Some("no arguments"));
        assert_eq!(expected_arguments(TEMP_DIRECTORY), Some("no arguments"));
    }

    #[test]
    fn exact_helper() {
        assert!(exact(
            &types(&["String", "List OF Byte"]),
            &["String", "List OF Byte"]
        ));
        assert!(!exact(&types(&["String"]), &["String", "String"]));
        assert!(!exact(&types(&["Integer"]), &["String"]));
        assert!(exact(&types(&[]), &[]));
    }

}
