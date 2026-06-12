pub(crate) mod fs;
pub(crate) mod general;
pub(crate) mod io;
pub(crate) mod math;
pub(crate) mod strings;
pub(crate) mod thread;

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(name, "fs" | "io" | "math" | "strings" | "thread")
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    fs::is_builtin_type(name) || io::is_builtin_type(name) || thread::is_builtin_type(name)
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    fs::resource_close_function(type_name)
}

pub(crate) fn is_resource_type(type_name: &str) -> bool {
    resource_close_function(type_name).is_some()
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    strings::call_return_type_name(name)
        .or_else(|| math::call_return_type_name(name))
        .or_else(|| fs::call_return_type_name(name))
        .or_else(|| io::call_return_type_name(name))
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    general::is_general_call(name)
        || strings::is_strings_call(name)
        || math::is_math_call(name)
        || fs::is_fs_call(name)
        || io::is_io_call(name)
        || thread::is_thread_call(name)
        || call_return_type_name(name).is_some()
}
