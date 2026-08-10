
use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, DefaultResolver, Implementation,
    Lowering, Parameter, ReturnType,
};

// Environment variables (plan-31-A).
const GET_ENV: &str = "os.getEnv";
const GET_ENV_OR: &str = "os.getEnvOr";
const HAS_ENV: &str = "os.hasEnv";
const SET_ENV: &str = "os.setEnv";
const UNSET_ENV: &str = "os.unsetEnv";
const ENVIRON: &str = "os.environ";
// Process & platform introspection (plan-31-B). All read-only, all nullary.
const ARGS: &str = "os.args";
const PID: &str = "os.pid";
const EXECUTABLE_PATH: &str = "os.executablePath";
// Resource locator (plan-55-B). The first `os::` call taking an argument that is
// not an env name: maps a build-relative resource path to its absolute on-disk
// location for the running build shape.
const RESOURCE_PATH: &str = "os.resourcePath";
const NAME: &str = "os.name";
const ARCH: &str = "os.arch";
const HOST_NAME: &str = "os.hostName";
const USER_NAME: &str = "os.userName";
const CPU_COUNT: &str = "os.cpuCount";

// plan-72-S: `OS` is the descriptor authority for this package. os is fully
// data-only (like io): every call has fixed positional argument types and a fixed
// return, lowers to a runtime helper with no implementation rewrite
// (`Implementation::Same`), and contributes no builtin types or source companion.
// `is_os_call`/`arity`/`call_return_type_name`/`resolve_call` derive from the
// descriptor. `call_param_names` (borrowed) is a static pinned by parity;
// `expected_arguments` renders the niladic calls as the bespoke `"no arguments"`
// phrasing the descriptor's per-position `"()"` cannot reproduce, so it stays
// hand-authored and the parity harness opts out of that row (as for io).
const fn ov(params: &'static [Parameter], ret: &'static str) -> BuiltinOverload {
    BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }
}

const fn os_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        doc_into: "",
        doc_desc: "",
        errors: &[],
        overloads,
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const P_NAME: &[Parameter] = &[Parameter::required("name", "String")];
const P_RELATIVE: &[Parameter] = &[Parameter::required("relative", "String")];
const P_NAME_FALLBACK: &[Parameter] = &[
    Parameter::required("name", "String"),
    Parameter::required("fallback", "String"),
];
const P_NAME_VALUE: &[Parameter] = &[
    Parameter::required("name", "String"),
    Parameter::required("value", "String"),
];

const OV_GET_ENV: &[BuiltinOverload] = &[ov(P_NAME, "String")];
const OV_HAS_ENV: &[BuiltinOverload] = &[ov(P_NAME, "Boolean")];
const OV_UNSET_ENV: &[BuiltinOverload] = &[ov(P_NAME, "Nothing")];
const OV_RESOURCE_PATH: &[BuiltinOverload] = &[ov(P_RELATIVE, "String")];
const OV_GET_ENV_OR: &[BuiltinOverload] = &[ov(P_NAME_FALLBACK, "String")];
const OV_SET_ENV: &[BuiltinOverload] = &[ov(P_NAME_VALUE, "Nothing")];
const OV_ENVIRON: &[BuiltinOverload] = &[ov(&[], "Map OF String TO String")];
const OV_ARGS: &[BuiltinOverload] = &[ov(&[], "List OF String")];
const OV_INTEGER: &[BuiltinOverload] = &[ov(&[], "Integer")];
const OV_STRING_NILADIC: &[BuiltinOverload] = &[ov(&[], "String")];

const OS_FUNCTIONS: &[BuiltinFunction] = &[
    os_fn(GET_ENV, "getEnv", OV_GET_ENV),
    os_fn(GET_ENV_OR, "getEnvOr", OV_GET_ENV_OR),
    os_fn(HAS_ENV, "hasEnv", OV_HAS_ENV),
    os_fn(SET_ENV, "setEnv", OV_SET_ENV),
    os_fn(UNSET_ENV, "unsetEnv", OV_UNSET_ENV),
    os_fn(ENVIRON, "environ", OV_ENVIRON),
    os_fn(ARGS, "args", OV_ARGS),
    os_fn(PID, "pid", OV_INTEGER),
    os_fn(EXECUTABLE_PATH, "executablePath", OV_STRING_NILADIC),
    os_fn(RESOURCE_PATH, "resourcePath", OV_RESOURCE_PATH),
    os_fn(NAME, "name", OV_STRING_NILADIC),
    os_fn(ARCH, "arch", OV_STRING_NILADIC),
    os_fn(HOST_NAME, "hostName", OV_STRING_NILADIC),
    os_fn(USER_NAME, "userName", OV_STRING_NILADIC),
    os_fn(CPU_COUNT, "cpuCount", OV_INTEGER),
];

pub(crate) static OS: BuiltinModule = BuiltinModule {
    name: "os",
    functions: OS_FUNCTIONS,
    types: &[],
    source: None,
    resolver: None,
};

pub(crate) fn is_os_call(name: &str) -> bool {
    DefaultResolver::contains(&OS, name)
}

// `call_param_names` returns a `&'static` borrowed shape the owned
// `DefaultResolver` (which yields `Vec`) cannot produce, so it stays a static
// literal PINNED equal to `OS` by `parity_matches_descriptor`.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        GET_ENV | HAS_ENV | UNSET_ENV => Some(&[&["name"]]),
        GET_ENV_OR => Some(&[&["name"], &["fallback"]]),
        SET_ENV => Some(&[&["name"], &["value"]]),
        RESOURCE_PATH => Some(&[&["relative"]]),
        ENVIRON | ARGS | PID | EXECUTABLE_PATH | NAME | ARCH | HOST_NAME | USER_NAME
        | CPU_COUNT => Some(&[]),
        _ => None,
    }
}

// The niladic os calls render their expected arguments as `"no arguments"`, a
// bespoke phrasing the descriptor's per-position type rendering (`"()"`) cannot
// reproduce, so this stays a hand-authored static (not descriptor-derived) and the
// parity harness opts out of the `expected_arguments` row for os (as for io). BB
// removes it.
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        GET_ENV | HAS_ENV | UNSET_ENV | RESOURCE_PATH => Some("String"),
        GET_ENV_OR | SET_ENV => Some("String, String"),
        ENVIRON | ARGS | PID | EXECUTABLE_PATH | NAME | ARCH | HOST_NAME | USER_NAME
        | CPU_COUNT => Some("no arguments"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[&str] = &[
        GET_ENV,
        GET_ENV_OR,
        HAS_ENV,
        SET_ENV,
        UNSET_ENV,
        ENVIRON,
        ARGS,
        PID,
        EXECUTABLE_PATH,
        RESOURCE_PATH,
        NAME,
        ARCH,
        HOST_NAME,
        USER_NAME,
        CPU_COUNT,
    ];

    #[test]
    fn is_os_call_recognizes_all_and_rejects_others() {
        for name in ALL {
            assert!(is_os_call(name), "{name}");
        }
        assert!(!is_os_call("os.unknown"));
        assert!(!is_os_call("fs.exists"));
        assert!(!is_os_call(""));
    }

    #[test]
    fn param_names_specific() {
        assert_eq!(call_param_names(GET_ENV), Some(&[&["name"][..]][..]));
        assert_eq!(
            call_param_names(GET_ENV_OR),
            Some(&[&["name"][..], &["fallback"][..]][..])
        );
        assert_eq!(
            call_param_names(SET_ENV),
            Some(&[&["name"][..], &["value"][..]][..])
        );
        assert_eq!(call_param_names(ENVIRON), Some(&[][..]));
    }

    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `ov`/`os_fn` are const fns used only in const context; call them at
        // runtime so their bodies are exercised.
        let overload = ov(P_NAME_FALLBACK, "String");
        assert_eq!(overload.params.len(), 2);
        assert_eq!(overload.return_type, ReturnType::Fixed("String"));

        let func = os_fn(PID, "pid", OV_INTEGER);
        assert_eq!(func.name, PID);
        assert_eq!(func.doc_slug, "pid");
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Helper);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }

    #[test]
    fn expected_arguments_covers_every_arity_class() {
        assert_eq!(expected_arguments(GET_ENV), Some("String"));
        assert_eq!(expected_arguments(GET_ENV_OR), Some("String, String"));
        assert_eq!(expected_arguments(SET_ENV), Some("String, String"));
        assert_eq!(expected_arguments(PID), Some("no arguments"));
        assert_eq!(expected_arguments("os.unknown"), None);
    }
}
