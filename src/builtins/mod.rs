pub(crate) mod general;
pub(crate) mod io;

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(name, "io")
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    io::call_return_type_name(name)
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    general::is_general_call(name) || call_return_type_name(name).is_some()
}
