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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

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

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    DefaultResolver::return_type_name(&FS, name)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    DefaultResolver::resolve_call(&FS, name, arg_types).map(|return_type| ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
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

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    DefaultResolver::arity(&FS, name)
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

    fn ret(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &types(args)).map(|r| r.return_type.into_owned())
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
    fn every_name_has_consistent_metadata() {
        for name in ALL {
            assert!(call_param_names(name).is_some(), "param_names {name}");
            assert!(call_return_type_name(name).is_some(), "return_type {name}");
            assert!(expected_arguments(name).is_some(), "expected_args {name}");
            assert!(arity(name).is_some(), "arity {name}");
        }
    }

    #[test]
    fn metadata_returns_none_for_unknown() {
        assert_eq!(call_param_names("fs.nope"), None);
        assert_eq!(call_return_type_name("fs.nope"), None);
        assert_eq!(expected_arguments("fs.nope"), None);
        assert_eq!(arity("fs.nope"), None);
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
    fn return_type_names_cover_categories() {
        for name in [FILE_EXISTS, DIRECTORY_EXISTS, EXISTS, EOF, IS_WITHIN] {
            assert_eq!(call_return_type_name(name), Some("Boolean"), "{name}");
        }
        for name in [READ_BYTES, READ_ALL_BYTES] {
            assert_eq!(call_return_type_name(name), Some("List OF Byte"), "{name}");
        }
        for name in [
            READ_TEXT,
            READ_LINE,
            READ_ALL,
            CANONICAL_PATH,
            PATH_JOIN,
            PATH_DIR_NAME,
            PATH_BASE_NAME,
            PATH_EXTENSION,
            PATH_NORMALIZE,
            CURRENT_DIRECTORY,
            TEMP_DIRECTORY,
        ] {
            assert_eq!(call_return_type_name(name), Some("String"), "{name}");
        }
        for name in [
            WRITE_BYTES,
            WRITE_TEXT,
            WRITE_BYTES_ATOMIC,
            WRITE_TEXT_ATOMIC,
            APPEND_BYTES,
            APPEND_TEXT,
            WRITE_ALL,
            WRITE_ALL_BYTES,
            CLOSE,
            DELETE_FILE,
            CREATE_DIRECTORY,
            CREATE_DIRECTORIES,
            DELETE_DIRECTORY,
            SET_CURRENT_DIRECTORY,
        ] {
            assert_eq!(call_return_type_name(name), Some("Nothing"), "{name}");
        }
        for name in [OPEN, OPEN_FILE, OPEN_FILE_NO_FOLLOW, CREATE_TEMP_FILE] {
            assert_eq!(call_return_type_name(name), Some(FILE_TYPE), "{name}");
        }
        assert_eq!(
            call_return_type_name(LIST_DIRECTORY),
            Some("List OF String")
        );
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
    fn arity_specific() {
        for name in [
            FILE_EXISTS,
            READ_TEXT,
            DELETE_FILE,
            LIST_DIRECTORY,
            SET_CURRENT_DIRECTORY,
        ] {
            assert_eq!(arity(name), Some((1, 1)), "{name}");
        }
        for name in [
            WRITE_BYTES,
            WRITE_TEXT,
            OPEN,
            WRITE_ALL,
            WRITE_ALL_BYTES,
            IS_WITHIN,
        ] {
            assert_eq!(arity(name), Some((2, 2)), "{name}");
        }
        assert_eq!(arity(OPEN_FILE), Some((1, 2)));
        assert_eq!(arity(OPEN_FILE_NO_FOLLOW), Some((1, 2)));
        assert_eq!(arity(CREATE_TEMP_FILE), Some((0, 1)));
        for name in [READ_LINE, READ_ALL, READ_ALL_BYTES, CLOSE, EOF] {
            assert_eq!(arity(name), Some((1, 1)), "{name}");
        }
        assert_eq!(arity(PATH_JOIN), Some((1, 1)));
        assert_eq!(arity(CURRENT_DIRECTORY), Some((0, 0)));
        assert_eq!(arity(TEMP_DIRECTORY), Some((0, 0)));
    }

    #[test]
    fn resolve_single_string_path_family() {
        for name in [
            FILE_EXISTS,
            DIRECTORY_EXISTS,
            EXISTS,
            READ_BYTES,
            READ_TEXT,
            CANONICAL_PATH,
            PATH_DIR_NAME,
            PATH_BASE_NAME,
            PATH_EXTENSION,
            PATH_NORMALIZE,
            DELETE_FILE,
            CREATE_DIRECTORY,
            CREATE_DIRECTORIES,
            DELETE_DIRECTORY,
            LIST_DIRECTORY,
            SET_CURRENT_DIRECTORY,
        ] {
            let expected = call_return_type_name(name).unwrap().to_string();
            assert_eq!(ret(name, &["String"]), Some(expected), "{name}");
            assert_eq!(ret(name, &["Integer"]), None, "{name} wrong type");
            assert_eq!(ret(name, &[]), None, "{name} zero arg");
            assert_eq!(ret(name, &["String", "String"]), None, "{name} two arg");
        }
    }

    #[test]
    fn resolve_write_bytes_family() {
        for name in [WRITE_BYTES, WRITE_BYTES_ATOMIC, APPEND_BYTES] {
            assert_eq!(
                ret(name, &["String", "List OF Byte"]),
                Some("Nothing".to_string()),
                "{name}"
            );
            assert_eq!(ret(name, &["String", "String"]), None, "{name}");
            assert_eq!(ret(name, &["String"]), None, "{name}");
        }
    }

    #[test]
    fn resolve_write_text_family() {
        for name in [WRITE_TEXT, WRITE_TEXT_ATOMIC, APPEND_TEXT] {
            assert_eq!(
                ret(name, &["String", "String"]),
                Some("Nothing".to_string()),
                "{name}"
            );
            assert_eq!(ret(name, &["String", "List OF Byte"]), None, "{name}");
        }
    }

    #[test]
    fn resolve_open_variants() {
        assert_eq!(
            ret(OPEN, &["String", "String"]),
            Some(FILE_TYPE.to_string())
        );
        assert_eq!(ret(OPEN, &["String"]), None);
        for name in [OPEN_FILE, OPEN_FILE_NO_FOLLOW] {
            assert_eq!(
                ret(name, &["String"]),
                Some(FILE_TYPE.to_string()),
                "{name}"
            );
            assert_eq!(
                ret(name, &["String", "String"]),
                Some(FILE_TYPE.to_string()),
                "{name}"
            );
            assert_eq!(ret(name, &[]), None, "{name}");
            assert_eq!(ret(name, &["Integer"]), None, "{name}");
        }
    }

    #[test]
    fn resolve_create_temp_and_dirs() {
        assert_eq!(ret(CREATE_TEMP_FILE, &[]), Some(FILE_TYPE.to_string()));
        assert_eq!(
            ret(CREATE_TEMP_FILE, &["String"]),
            Some(FILE_TYPE.to_string())
        );
        assert_eq!(ret(CREATE_TEMP_FILE, &["Integer"]), None);
        assert_eq!(ret(CREATE_TEMP_FILE, &["String", "String"]), None);
        assert_eq!(ret(TEMP_DIRECTORY, &[]), Some("String".to_string()));
        assert_eq!(ret(TEMP_DIRECTORY, &["String"]), None);
        assert_eq!(ret(CURRENT_DIRECTORY, &[]), Some("String".to_string()));
        assert_eq!(ret(CURRENT_DIRECTORY, &["String"]), None);
    }

    #[test]
    fn resolve_file_handle_family() {
        for name in [READ_LINE, READ_ALL] {
            assert_eq!(
                ret(name, &[FILE_TYPE]),
                Some("String".to_string()),
                "{name}"
            );
            assert_eq!(ret(name, &["String"]), None, "{name}");
        }
        assert_eq!(
            ret(READ_ALL_BYTES, &[FILE_TYPE]),
            Some("List OF Byte".to_string())
        );
        assert_eq!(ret(READ_ALL_BYTES, &["String"]), None);
        assert_eq!(
            ret(WRITE_ALL, &[FILE_TYPE, "String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(ret(WRITE_ALL, &[FILE_TYPE, "List OF Byte"]), None);
        assert_eq!(
            ret(WRITE_ALL_BYTES, &[FILE_TYPE, "List OF Byte"]),
            Some("Nothing".to_string())
        );
        assert_eq!(ret(WRITE_ALL_BYTES, &[FILE_TYPE, "String"]), None);
        assert_eq!(ret(CLOSE, &[FILE_TYPE]), Some("Nothing".to_string()));
        assert_eq!(ret(CLOSE, &["String"]), None);
        assert_eq!(ret(EOF, &[FILE_TYPE]), Some("Boolean".to_string()));
        assert_eq!(ret(EOF, &["String"]), None);
    }

    #[test]
    fn resolve_is_within_and_path_join() {
        assert_eq!(
            ret(IS_WITHIN, &["String", "String"]),
            Some("Boolean".to_string())
        );
        assert_eq!(ret(IS_WITHIN, &["String"]), None);
        assert_eq!(
            ret(PATH_JOIN, &["List OF String"]),
            Some("String".to_string())
        );
        assert_eq!(ret(PATH_JOIN, &["String"]), None);
    }

    #[test]
    fn resolve_rejects_unknown_name() {
        assert_eq!(ret("fs.nope", &["String"]), None);
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

    // plan-72-K migration gate: prove `FS` reproduces every legacy helper answer
    // for every `fs.*` name (and an unknown name) — membership, arity, param
    // names, return type — pins the borrowed `call_param_names` static equal to
    // `FS`, and confirms `resolve_call` derives byte-identically across the whole
    // surface (checked exhaustively by the resolve_* tests above). `File` is the
    // one opaque builtin type. `expected_arguments` is opted out (bespoke
    // `"String[, String]"` / `"no arguments"` phrasing). Keep until plan-72-BB.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let calls: Vec<&str> = FS_FUNCTIONS.iter().map(|f| f.name).collect();
        let legacy = parity::LegacySet {
            is_call: &is_fs_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            return_type_name: &call_return_type_name,
            expected_arguments: None, // bespoke optional-argument / "no arguments" phrasing
            param_name_overloads: None,
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            // `File` is opaque (no record fields) and fs has no builtin_type_fields
            // helper; membership is asserted below.
            builtin_type_fields: None,
        };
        let mut probe = calls.clone();
        probe.push("fs.nope");
        parity::assert_parity(&FS, &probe, &legacy, &[]);

        // The one opaque builtin resource type and its close function.
        assert!(is_builtin_type(FILE_TYPE));
        assert!(!is_builtin_type("Directory"));
        assert_eq!(FS.types.iter().map(|ty| ty.name).collect::<Vec<_>>(), vec![FILE_TYPE]);
        assert_eq!(resource_close_function(FILE_TYPE), Some(CLOSE));

        // resolve_call parity spot-checks across the overloaded / optional forms.
        assert_eq!(ret(OPEN_FILE, &["String"]), Some(FILE_TYPE.to_string()));
        assert_eq!(ret(OPEN_FILE, &["String", "String"]), Some(FILE_TYPE.to_string()));
        assert!(ret(OPEN_FILE, &["Integer"]).is_none());
        assert_eq!(ret(CREATE_TEMP_FILE, &[]), Some(FILE_TYPE.to_string()));
        assert!(ret(CREATE_TEMP_FILE, &["String", "String"]).is_none());
        assert_eq!(ret(WRITE_BYTES, &["String", "List OF Byte"]), Some("Nothing".to_string()));
        assert!(ret(WRITE_BYTES, &["String", "String"]).is_none());
    }
}
