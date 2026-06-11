pub(crate) mod fs;
pub(crate) mod general;
pub(crate) mod io;
pub(crate) mod strings;

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(name, "fs" | "io" | "strings")
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    fs::is_builtin_type(name) || io::is_builtin_type(name)
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    strings::call_return_type_name(name)
        .or_else(|| fs::call_return_type_name(name))
        .or_else(|| io::call_return_type_name(name))
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    general::is_general_call(name)
        || strings::is_strings_call(name)
        || fs::is_fs_call(name)
        || io::is_io_call(name)
        || call_return_type_name(name).is_some()
}
