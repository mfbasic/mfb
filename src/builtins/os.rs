use std::borrow::Cow;

// Environment variables (plan-31-A).
const GET_ENV: &str = "os.getEnv";
const GET_ENV_OR: &str = "os.getEnvOr";
const HAS_ENV: &str = "os.hasEnv";
const SET_ENV: &str = "os.setEnv";
const UNSET_ENV: &str = "os.unsetEnv";
const ENVIRON: &str = "os.environ";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_os_call(name: &str) -> bool {
    matches!(
        name,
        GET_ENV | GET_ENV_OR | HAS_ENV | SET_ENV | UNSET_ENV | ENVIRON
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        GET_ENV | HAS_ENV | UNSET_ENV => Some(&[&["name"]]),
        GET_ENV_OR => Some(&[&["name"], &["fallback"]]),
        SET_ENV => Some(&[&["name"], &["value"]]),
        ENVIRON => Some(&[]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        GET_ENV | GET_ENV_OR => Some("String"),
        HAS_ENV => Some("Boolean"),
        SET_ENV | UNSET_ENV => Some("Nothing"),
        ENVIRON => Some("Map OF String TO String"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        GET_ENV | HAS_ENV | UNSET_ENV if exact(arg_types, &["String"]) => {
            Cow::Borrowed(call_return_type_name(name)?)
        }
        GET_ENV_OR | SET_ENV if exact(arg_types, &["String", "String"]) => {
            Cow::Borrowed(call_return_type_name(name)?)
        }
        ENVIRON if arg_types.is_empty() => Cow::Borrowed("Map OF String TO String"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        GET_ENV | HAS_ENV | UNSET_ENV => Some("String"),
        GET_ENV_OR | SET_ENV => Some("String, String"),
        ENVIRON => Some("no arguments"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        GET_ENV | HAS_ENV | UNSET_ENV => Some((1, 1)),
        GET_ENV_OR | SET_ENV => Some((2, 2)),
        ENVIRON => Some((0, 0)),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn types(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn ret(name: &str, args: &[&str]) -> Option<String> {
        resolve_call(name, &types(args)).map(|r| r.return_type.into_owned())
    }

    const ALL: &[&str] = &[GET_ENV, GET_ENV_OR, HAS_ENV, SET_ENV, UNSET_ENV, ENVIRON];

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
        assert_eq!(ret(ENVIRON, &[]), Some("Map OF String TO String".to_string()));
        assert_eq!(ret(ENVIRON, &["String"]), None);
    }

    #[test]
    fn resolve_rejects_unknown_name() {
        assert_eq!(ret("os.nope", &["String"]), None);
    }
}
