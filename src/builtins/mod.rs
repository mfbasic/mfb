pub(crate) mod collections;
pub(crate) mod fs;
pub(crate) mod general;
pub(crate) mod io;
pub(crate) mod json;
pub(crate) mod math;
pub(crate) mod net;
pub(crate) mod resource;
pub(crate) mod strings;
pub(crate) mod term;
pub(crate) mod thread;
pub(crate) mod tls;

pub(crate) use resource::{ResourceInfo, ResourceKind, ResourceRegistry};

pub(crate) fn is_builtin_import(name: &str) -> bool {
    matches!(
        name,
        "collections"
            | "fs"
            | "io"
            | "json"
            | "math"
            | "net"
            | "strings"
            | "term"
            | "thread"
            | "tls"
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    fs::is_builtin_type(name)
        || io::is_builtin_type(name)
        || json::is_builtin_type(name)
        || net::is_builtin_type(name)
        || term::is_builtin_type(name)
        || thread::is_builtin_type(name)
        || tls::is_builtin_type(name)
}

pub(crate) fn resource_close_function(type_name: &str) -> Option<&'static str> {
    resource::builtin_resource_close_function(type_name)
}

pub(crate) fn is_resource_type(type_name: &str) -> bool {
    resource::is_builtin_resource_type(type_name)
}

pub(crate) fn is_thread_sendable_resource_type(type_name: &str) -> bool {
    resource::is_builtin_sendable_resource_type(type_name)
}

/// The bare native lowering name for a migrated `collections::`/`strings::`
/// member (plan-01-functions.md §5). The native code generator stays keyed on the
/// original bare names (`get`, `transform`, `find`, `mid`, `replace`, ...), so the
/// IR call target for these members is dequalified back to the bare name. Returns
/// `None` for every other call (including the `collections::` source generics,
/// which the monomorphizer rewrites to `__collections_X` instead).
pub(crate) fn native_builtin_target(name: &str) -> Option<&'static str> {
    if let Some(member) = name.strip_prefix("strings.") {
        return match member {
            "find" => Some("find"),
            "mid" => Some("mid"),
            "replace" => Some("replace"),
            _ => None,
        };
    }
    match collections::native_member_bare(name)? {
        "get" => Some("get"),
        "getOr" => Some("getOr"),
        "set" => Some("set"),
        "append" => Some("append"),
        "prepend" => Some("prepend"),
        "insert" => Some("insert"),
        "removeAt" => Some("removeAt"),
        "removeKey" => Some("removeKey"),
        "keys" => Some("keys"),
        "values" => Some("values"),
        "hasKey" => Some("hasKey"),
        "contains" => Some("contains"),
        "forEach" => Some("forEach"),
        "transform" => Some("transform"),
        "filter" => Some("filter"),
        "reduce" => Some("reduce"),
        "sum" => Some("sum"),
        "find" => Some("find"),
        "mid" => Some("mid"),
        "replace" => Some("replace"),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    general::call_return_type_name(name)
        .or_else(|| collections::call_return_type_name(name))
        .or_else(|| strings::call_return_type_name(name))
        .or_else(|| math::call_return_type_name(name))
        .or_else(|| fs::call_return_type_name(name))
        .or_else(|| io::call_return_type_name(name))
        .or_else(|| json::call_return_type_name(name))
        .or_else(|| net::call_return_type_name(name))
        .or_else(|| term::call_return_type_name(name))
        .or_else(|| tls::call_return_type_name(name))
}

pub(crate) fn is_builtin_call(name: &str) -> bool {
    collections::is_collections_call(name)
        || general::is_general_call(name)
        || strings::is_strings_call(name)
        || math::is_math_call(name)
        || fs::is_fs_call(name)
        || io::is_io_call(name)
        || json::is_json_call(name)
        || net::is_net_call(name)
        || term::is_term_call(name)
        || thread::is_thread_call(name)
        || tls::is_tls_call(name)
        || call_return_type_name(name).is_some()
}

pub(crate) fn is_builtin_member(name: &str) -> bool {
    is_builtin_call(name) || math::is_math_constant(name)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    general::call_param_names(name)
        .or_else(|| collections::call_param_names(name))
        .or_else(|| strings::call_param_names(name))
        .or_else(|| math::call_param_names(name))
        .or_else(|| fs::call_param_names(name))
        .or_else(|| io::call_param_names(name))
        .or_else(|| json::call_param_names(name))
        .or_else(|| net::call_param_names(name))
        .or_else(|| term::call_param_names(name))
        .or_else(|| tls::call_param_names(name))
        .or_else(|| thread::call_param_names(name))
}
