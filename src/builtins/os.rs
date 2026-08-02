use std::borrow::Cow;

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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

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

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    DefaultResolver::return_type_name(&OS, name)
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    DefaultResolver::resolve_call(&OS, name, arg_types).map(|return_type| ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
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

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    DefaultResolver::arity(&OS, name)
}

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
        assert_eq!(call_param_names("os.nope"), None);
        assert_eq!(call_return_type_name("os.nope"), None);
        assert_eq!(expected_arguments("os.nope"), None);
        assert_eq!(arity("os.nope"), None);
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
    fn arity_specific() {
        for name in [GET_ENV, HAS_ENV, UNSET_ENV] {
            assert_eq!(arity(name), Some((1, 1)), "{name}");
        }
        for name in [GET_ENV_OR, SET_ENV] {
            assert_eq!(arity(name), Some((2, 2)), "{name}");
        }
        assert_eq!(arity(ENVIRON), Some((0, 0)));
    }

    #[test]
    fn resolve_env_family() {
        assert_eq!(ret(GET_ENV, &["String"]), Some("String".to_string()));
        assert_eq!(ret(GET_ENV, &[]), None);
        assert_eq!(ret(GET_ENV, &["Integer"]), None);
        assert_eq!(ret(HAS_ENV, &["String"]), Some("Boolean".to_string()));
        assert_eq!(ret(UNSET_ENV, &["String"]), Some("Nothing".to_string()));
        assert_eq!(
            ret(GET_ENV_OR, &["String", "String"]),
            Some("String".to_string())
        );
        assert_eq!(ret(GET_ENV_OR, &["String"]), None);
        assert_eq!(
            ret(SET_ENV, &["String", "String"]),
            Some("Nothing".to_string())
        );
        assert_eq!(ret(SET_ENV, &["String", "Integer"]), None);
        assert_eq!(
            ret(ENVIRON, &[]),
            Some("Map OF String TO String".to_string())
        );
        assert_eq!(ret(ENVIRON, &["String"]), None);
    }

    #[test]
    fn resolve_introspection_family() {
        for name in [EXECUTABLE_PATH, NAME, ARCH, HOST_NAME, USER_NAME] {
            assert_eq!(ret(name, &[]), Some("String".to_string()), "{name}");
            assert_eq!(ret(name, &["String"]), None, "{name} arity");
        }
        assert_eq!(ret(ARGS, &[]), Some("List OF String".to_string()));
        assert_eq!(ret(ARGS, &["String"]), None);
        for name in [PID, CPU_COUNT] {
            assert_eq!(ret(name, &[]), Some("Integer".to_string()), "{name}");
            assert_eq!(ret(name, &["Integer"]), None, "{name} arity");
        }
    }

    #[test]
    fn resolve_resource_path() {
        // plan-55-B: the first unary `String -> String` os:: call.
        assert_eq!(ret(RESOURCE_PATH, &["String"]), Some("String".to_string()));
        assert_eq!(ret(RESOURCE_PATH, &[]), None);
        assert_eq!(ret(RESOURCE_PATH, &["Integer"]), None);
        assert_eq!(ret(RESOURCE_PATH, &["String", "String"]), None);
        assert_eq!(arity(RESOURCE_PATH), Some((1, 1)));
        assert_eq!(expected_arguments(RESOURCE_PATH), Some("String"));
        assert_eq!(
            call_param_names(RESOURCE_PATH),
            Some(&[&["relative"][..]][..])
        );
    }

    #[test]
    fn introspection_metadata() {
        for name in [
            ARGS,
            PID,
            EXECUTABLE_PATH,
            NAME,
            ARCH,
            HOST_NAME,
            USER_NAME,
            CPU_COUNT,
        ] {
            assert_eq!(arity(name), Some((0, 0)), "{name}");
            assert_eq!(expected_arguments(name), Some("no arguments"), "{name}");
            assert_eq!(call_param_names(name), Some(&[][..]), "{name}");
        }
    }

    #[test]
    fn resolve_rejects_unknown_name() {
        assert_eq!(ret("os.nope", &["String"]), None);
    }

    // plan-72-S migration gate: prove `OS` reproduces every legacy helper answer
    // for every `os.*` name (and an unknown name) — membership, arity, param
    // names, and return type — pinning the borrowed `call_param_names` static
    // equal to `OS`. `expected_arguments` renders the niladic calls as the bespoke
    // `"no arguments"` phrasing the descriptor cannot reproduce (opted out, kept
    // hand-authored and checked above); `resolve_call` is checked directly. Keep
    // until plan-72-BB deletes the legacy helpers.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::parity;

        let calls: Vec<&str> = OS_FUNCTIONS.iter().map(|f| f.name).collect();
        let legacy = parity::LegacySet {
            is_call: &is_os_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            return_type_name: &call_return_type_name,
            // os renders niladic calls as "no arguments", which the descriptor's
            // per-position rendering ("()") cannot reproduce; opt out and keep the
            // hand-authored strings.
            expected_arguments: None,
            param_name_overloads: None,
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: None,
        };
        let mut probe = calls.clone();
        probe.push("os.nope");
        parity::assert_parity(&OS, &probe, &legacy, &[]);

        // os contributes no builtin types and no source companion.
        assert!(OS.types.is_empty());
        assert!(OS.source.is_none());
    }
}
