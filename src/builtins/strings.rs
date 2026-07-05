use std::borrow::Cow;

const TRIM: &str = "strings.trim";
const TRIM_START: &str = "strings.trimStart";
const TRIM_END: &str = "strings.trimEnd";
const UPPER: &str = "strings.upper";
const LOWER: &str = "strings.lower";
const CASE_FOLD: &str = "strings.caseFold";
const NORMALIZE_NFC: &str = "strings.normalizeNfc";
const GRAPHEMES: &str = "strings.graphemes";
const STARTS_WITH: &str = "strings.startsWith";
const ENDS_WITH: &str = "strings.endsWith";
const CONTAINS: &str = "strings.contains";
const SPLIT: &str = "strings.split";
const JOIN: &str = "strings.join";
const BYTE_LEN: &str = "strings.byteLen";
const STARTS_WITH_ANY: &str = "strings.startsWithAny";
const ENDS_WITH_ANY: &str = "strings.endsWithAny";
const STRIP_PREFIX: &str = "strings.stripPrefix";
const STRIP_SUFFIX: &str = "strings.stripSuffix";
const COUNT: &str = "strings.count";
const LEFT: &str = "strings.left";
const RIGHT: &str = "strings.right";
const REPEAT: &str = "strings.repeat";
const PAD_LEFT: &str = "strings.padLeft";
const PAD_RIGHT: &str = "strings.padRight";
const GRAPHEME_AT: &str = "strings.graphemeAt";
const GRAPHEMES_COUNT: &str = "strings.graphemesCount";
const TRIM_CHARS: &str = "strings.trimChars";
// The raw UTF-8 bytes backing a String, one element per byte (the inverse of
// `toString(List OF Byte)`). The foundation the `encoding` package's Unicode
// codecs build on (plan-02-encoding.md).
const TO_BYTES: &str = "strings.toBytes";
// Migrated from the bare global namespace (plan-01-functions.md §5): the String
// overloads of `find`/`mid`/`replace`. The List overloads moved to
// `collections::`. The native code generator still lowers these by their bare
// names (`find`/`mid`/`replace`); `super::native_builtin_target` dequalifies the
// IR target accordingly.
const FIND: &str = "strings.find";
const MID: &str = "strings.mid";
const REPLACE: &str = "strings.replace";

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_strings_call(name: &str) -> bool {
    matches!(
        name,
        TRIM | TRIM_START
            | TRIM_END
            | UPPER
            | LOWER
            | CASE_FOLD
            | NORMALIZE_NFC
            | GRAPHEMES
            | STARTS_WITH
            | ENDS_WITH
            | CONTAINS
            | SPLIT
            | JOIN
            | BYTE_LEN
            | STARTS_WITH_ANY
            | ENDS_WITH_ANY
            | STRIP_PREFIX
            | STRIP_SUFFIX
            | COUNT
            | LEFT
            | RIGHT
            | REPEAT
            | PAD_LEFT
            | PAD_RIGHT
            | GRAPHEME_AT
            | GRAPHEMES_COUNT
            | TRIM_CHARS
            | TO_BYTES
            | FIND
            | MID
            | REPLACE
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | GRAPHEMES
        | BYTE_LEN | TO_BYTES => Some(&[&["value"]]),
        STARTS_WITH => Some(&[&["value"], &["prefix"]]),
        ENDS_WITH => Some(&[&["value"], &["suffix"]]),
        CONTAINS => Some(&[&["value"], &["needle"]]),
        SPLIT => Some(&[&["value"], &["delimiter", "separator"]]),
        JOIN => Some(&[&["parts", "values"], &["delimiter", "separator"]]),
        STARTS_WITH_ANY => Some(&[&["value"], &["prefixes"]]),
        ENDS_WITH_ANY => Some(&[&["value"], &["suffixes"]]),
        STRIP_PREFIX => Some(&[&["value"], &["prefix"]]),
        STRIP_SUFFIX => Some(&[&["value"], &["suffix"]]),
        COUNT => Some(&[&["value"], &["needle"]]),
        LEFT | RIGHT => Some(&[&["value"], &["count"]]),
        REPEAT => Some(&[&["value"], &["times"]]),
        PAD_LEFT | PAD_RIGHT => Some(&[&["value"], &["width"], &["padChar"]]),
        GRAPHEME_AT => Some(&[&["value"], &["index"]]),
        GRAPHEMES_COUNT => Some(&[&["value"]]),
        TRIM_CHARS => Some(&[&["value"], &["chars"]]),
        FIND => Some(&[&["value"], &["needle"], &["start"]]),
        MID => Some(&[&["value"], &["start"], &["count"]]),
        REPLACE => Some(&[&["value"], &["old", "needle"], &["new", "replacement"]]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | JOIN => {
            Some("String")
        }
        GRAPHEMES | SPLIT => Some("List OF String"),
        TO_BYTES => Some("List OF Byte"),
        STARTS_WITH | ENDS_WITH | CONTAINS | STARTS_WITH_ANY | ENDS_WITH_ANY => Some("Boolean"),
        BYTE_LEN | COUNT | GRAPHEMES_COUNT => Some("Integer"),
        STRIP_PREFIX | STRIP_SUFFIX | LEFT | RIGHT | REPEAT | PAD_LEFT | PAD_RIGHT
        | GRAPHEME_AT | TRIM_CHARS | MID | REPLACE => Some("String"),
        FIND => Some("Integer"),
        _ => None,
    }
}

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC
            if exact(arg_types, &["String"]) =>
        {
            Cow::Borrowed("String")
        }
        GRAPHEMES if exact(arg_types, &["String"]) => Cow::Borrowed("List OF String"),
        TO_BYTES if exact(arg_types, &["String"]) => Cow::Borrowed("List OF Byte"),
        STARTS_WITH | ENDS_WITH | CONTAINS if exact(arg_types, &["String", "String"]) => {
            Cow::Borrowed("Boolean")
        }
        SPLIT if exact(arg_types, &["String", "String"]) => Cow::Borrowed("List OF String"),
        JOIN if exact(arg_types, &["List OF String", "String"]) => Cow::Borrowed("String"),
        BYTE_LEN if exact(arg_types, &["String"]) => Cow::Borrowed("Integer"),
        STARTS_WITH_ANY | ENDS_WITH_ANY if exact(arg_types, &["String", "List OF String"]) => {
            Cow::Borrowed("Boolean")
        }
        STRIP_PREFIX | STRIP_SUFFIX | TRIM_CHARS if exact(arg_types, &["String", "String"]) => {
            Cow::Borrowed("String")
        }
        COUNT if exact(arg_types, &["String", "String"]) => Cow::Borrowed("Integer"),
        LEFT | RIGHT | REPEAT if exact(arg_types, &["String", "Integer"]) => {
            Cow::Borrowed("String")
        }
        PAD_LEFT | PAD_RIGHT
            if exact(arg_types, &["String", "Integer"])
                || exact(arg_types, &["String", "Integer", "String"]) =>
        {
            Cow::Borrowed("String")
        }
        GRAPHEME_AT if exact(arg_types, &["String", "Integer"]) => Cow::Borrowed("String"),
        GRAPHEMES_COUNT if exact(arg_types, &["String"]) => Cow::Borrowed("Integer"),
        FIND if exact(arg_types, &["String", "String"])
            || exact(arg_types, &["String", "String", "Integer"]) =>
        {
            Cow::Borrowed("Integer")
        }
        MID if exact(arg_types, &["String", "Integer", "Integer"]) => Cow::Borrowed("String"),
        REPLACE if exact(arg_types, &["String", "String", "String"]) => Cow::Borrowed("String"),
        _ => return None,
    };
    Some(ResolvedCall { return_type })
}

pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | GRAPHEMES
        | BYTE_LEN | TO_BYTES => Some("String"),
        STARTS_WITH | ENDS_WITH | CONTAINS | SPLIT => Some("String, String"),
        JOIN => Some("List OF String, String"),
        STARTS_WITH_ANY | ENDS_WITH_ANY => Some("String, List OF String"),
        STRIP_PREFIX | STRIP_SUFFIX | COUNT | TRIM_CHARS => Some("String, String"),
        LEFT | RIGHT | REPEAT | GRAPHEME_AT => Some("String, Integer"),
        PAD_LEFT | PAD_RIGHT => Some("String, Integer[, String]"),
        GRAPHEMES_COUNT => Some("String"),
        FIND => Some("String, String[, Integer]"),
        MID => Some("String, Integer, Integer"),
        REPLACE => Some("String, String, String"),
        _ => None,
    }
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | GRAPHEMES
        | BYTE_LEN | GRAPHEMES_COUNT | TO_BYTES => Some((1, 1)),
        STARTS_WITH | ENDS_WITH | CONTAINS | SPLIT | JOIN | STARTS_WITH_ANY | ENDS_WITH_ANY
        | STRIP_PREFIX | STRIP_SUFFIX | COUNT | LEFT | RIGHT | REPEAT | GRAPHEME_AT
        | TRIM_CHARS => Some((2, 2)),
        PAD_LEFT | PAD_RIGHT | FIND => Some((2, 3)),
        MID | REPLACE => Some((3, 3)),
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

    // Every builtin name in this module, for exhaustive iteration.
    const ALL: &[&str] = &[
        TRIM,
        TRIM_START,
        TRIM_END,
        UPPER,
        LOWER,
        CASE_FOLD,
        NORMALIZE_NFC,
        GRAPHEMES,
        STARTS_WITH,
        ENDS_WITH,
        CONTAINS,
        SPLIT,
        JOIN,
        BYTE_LEN,
        STARTS_WITH_ANY,
        ENDS_WITH_ANY,
        STRIP_PREFIX,
        STRIP_SUFFIX,
        COUNT,
        LEFT,
        RIGHT,
        REPEAT,
        PAD_LEFT,
        PAD_RIGHT,
        GRAPHEME_AT,
        GRAPHEMES_COUNT,
        TRIM_CHARS,
        TO_BYTES,
        FIND,
        MID,
        REPLACE,
    ];

    #[test]
    fn is_strings_call_recognizes_all_and_rejects_others() {
        for name in ALL {
            assert!(is_strings_call(name), "{name} should be a strings call");
        }
        assert!(!is_strings_call("strings.unknown"));
        assert!(!is_strings_call("collections.find"));
        assert!(!is_strings_call(""));
    }

    #[test]
    fn every_name_has_consistent_metadata() {
        for name in ALL {
            assert!(call_param_names(name).is_some(), "param_names {name}");
            assert!(call_return_type_name(name).is_some(), "return_type {name}");
            assert!(expected_arguments(name).is_some(), "expected_args {name}");
            assert!(arity(name).is_some(), "arity {name}");
            // Param-name group count must match the max arity.
            let (_, max) = arity(name).unwrap();
            assert_eq!(
                call_param_names(name).unwrap().len(),
                max,
                "param-name group count vs arity for {name}"
            );
        }
    }

    #[test]
    fn metadata_returns_none_for_unknown() {
        assert_eq!(call_param_names("nope"), None);
        assert_eq!(call_return_type_name("nope"), None);
        assert_eq!(expected_arguments("nope"), None);
        assert_eq!(arity("nope"), None);
        assert!(resolve_call("nope", &types(&["String"])).is_none());
    }

    #[test]
    fn param_names_specific() {
        assert_eq!(call_param_names(TRIM), Some(&[&["value"][..]][..]));
        assert_eq!(
            call_param_names(SPLIT),
            Some(&[&["value"][..], &["delimiter", "separator"][..]][..])
        );
        assert_eq!(
            call_param_names(JOIN),
            Some(&[&["parts", "values"][..], &["delimiter", "separator"][..]][..])
        );
        assert_eq!(
            call_param_names(PAD_LEFT),
            Some(&[&["value"][..], &["width"][..], &["padChar"][..]][..])
        );
        assert_eq!(
            call_param_names(REPLACE),
            Some(
                &[
                    &["value"][..],
                    &["old", "needle"][..],
                    &["new", "replacement"][..]
                ][..]
            )
        );
    }

    #[test]
    fn return_type_names_cover_all_categories() {
        assert_eq!(call_return_type_name(TRIM), Some("String"));
        assert_eq!(call_return_type_name(JOIN), Some("String"));
        assert_eq!(call_return_type_name(GRAPHEMES), Some("List OF String"));
        assert_eq!(call_return_type_name(SPLIT), Some("List OF String"));
        assert_eq!(call_return_type_name(TO_BYTES), Some("List OF Byte"));
        assert_eq!(call_return_type_name(STARTS_WITH), Some("Boolean"));
        assert_eq!(call_return_type_name(STARTS_WITH_ANY), Some("Boolean"));
        assert_eq!(call_return_type_name(BYTE_LEN), Some("Integer"));
        assert_eq!(call_return_type_name(COUNT), Some("Integer"));
        assert_eq!(call_return_type_name(GRAPHEMES_COUNT), Some("Integer"));
        assert_eq!(call_return_type_name(STRIP_PREFIX), Some("String"));
        assert_eq!(call_return_type_name(MID), Some("String"));
        assert_eq!(call_return_type_name(REPLACE), Some("String"));
        assert_eq!(call_return_type_name(FIND), Some("Integer"));
    }

    #[test]
    fn expected_arguments_specific() {
        assert_eq!(expected_arguments(TRIM), Some("String"));
        assert_eq!(expected_arguments(STARTS_WITH), Some("String, String"));
        assert_eq!(expected_arguments(JOIN), Some("List OF String, String"));
        assert_eq!(
            expected_arguments(STARTS_WITH_ANY),
            Some("String, List OF String")
        );
        assert_eq!(expected_arguments(STRIP_PREFIX), Some("String, String"));
        assert_eq!(expected_arguments(LEFT), Some("String, Integer"));
        assert_eq!(
            expected_arguments(PAD_LEFT),
            Some("String, Integer[, String]")
        );
        assert_eq!(expected_arguments(GRAPHEMES_COUNT), Some("String"));
        assert_eq!(expected_arguments(FIND), Some("String, String[, Integer]"));
        assert_eq!(expected_arguments(MID), Some("String, Integer, Integer"));
        assert_eq!(expected_arguments(REPLACE), Some("String, String, String"));
    }

    #[test]
    fn arity_specific() {
        assert_eq!(arity(TRIM), Some((1, 1)));
        assert_eq!(arity(GRAPHEMES_COUNT), Some((1, 1)));
        assert_eq!(arity(STARTS_WITH), Some((2, 2)));
        assert_eq!(arity(TRIM_CHARS), Some((2, 2)));
        assert_eq!(arity(PAD_LEFT), Some((2, 3)));
        assert_eq!(arity(FIND), Some((2, 3)));
        assert_eq!(arity(MID), Some((3, 3)));
        assert_eq!(arity(REPLACE), Some((3, 3)));
    }

    #[test]
    fn resolve_single_string_arg_family() {
        for name in [
            TRIM,
            TRIM_START,
            TRIM_END,
            UPPER,
            LOWER,
            CASE_FOLD,
            NORMALIZE_NFC,
        ] {
            assert_eq!(ret(name, &["String"]), Some("String".to_string()));
            assert_eq!(ret(name, &["Integer"]), None);
            assert_eq!(ret(name, &["String", "String"]), None);
            assert_eq!(ret(name, &[]), None);
        }
        assert_eq!(
            ret(GRAPHEMES, &["String"]),
            Some("List OF String".to_string())
        );
        assert_eq!(ret(GRAPHEMES, &["Integer"]), None);
        assert_eq!(ret(TO_BYTES, &["String"]), Some("List OF Byte".to_string()));
        assert_eq!(ret(TO_BYTES, &["Integer"]), None);
        assert_eq!(ret(BYTE_LEN, &["String"]), Some("Integer".to_string()));
        assert_eq!(
            ret(GRAPHEMES_COUNT, &["String"]),
            Some("Integer".to_string())
        );
        assert_eq!(ret(GRAPHEMES_COUNT, &["Integer"]), None);
    }

    #[test]
    fn resolve_two_string_families() {
        for name in [STARTS_WITH, ENDS_WITH, CONTAINS] {
            assert_eq!(
                ret(name, &["String", "String"]),
                Some("Boolean".to_string())
            );
            assert_eq!(ret(name, &["String", "Integer"]), None);
            assert_eq!(ret(name, &["String"]), None);
        }
        assert_eq!(
            ret(SPLIT, &["String", "String"]),
            Some("List OF String".to_string())
        );
        assert_eq!(ret(SPLIT, &["String", "Integer"]), None);
        assert_eq!(
            ret(JOIN, &["List OF String", "String"]),
            Some("String".to_string())
        );
        assert_eq!(ret(JOIN, &["String", "String"]), None);
        for name in [STARTS_WITH_ANY, ENDS_WITH_ANY] {
            assert_eq!(
                ret(name, &["String", "List OF String"]),
                Some("Boolean".to_string())
            );
            assert_eq!(ret(name, &["String", "String"]), None);
        }
        for name in [STRIP_PREFIX, STRIP_SUFFIX, TRIM_CHARS] {
            assert_eq!(ret(name, &["String", "String"]), Some("String".to_string()));
            assert_eq!(ret(name, &["String", "Integer"]), None);
        }
        assert_eq!(
            ret(COUNT, &["String", "String"]),
            Some("Integer".to_string())
        );
        assert_eq!(ret(COUNT, &["String", "Integer"]), None);
    }

    #[test]
    fn resolve_string_integer_families() {
        for name in [LEFT, RIGHT, REPEAT] {
            assert_eq!(
                ret(name, &["String", "Integer"]),
                Some("String".to_string())
            );
            assert_eq!(ret(name, &["String", "String"]), None);
        }
        assert_eq!(
            ret(GRAPHEME_AT, &["String", "Integer"]),
            Some("String".to_string())
        );
        assert_eq!(ret(GRAPHEME_AT, &["String", "String"]), None);
    }

    #[test]
    fn resolve_pad_overloads() {
        for name in [PAD_LEFT, PAD_RIGHT] {
            assert_eq!(
                ret(name, &["String", "Integer"]),
                Some("String".to_string())
            );
            assert_eq!(
                ret(name, &["String", "Integer", "String"]),
                Some("String".to_string())
            );
            assert_eq!(ret(name, &["String"]), None);
            assert_eq!(ret(name, &["String", "String"]), None);
            assert_eq!(ret(name, &["String", "Integer", "Integer"]), None);
        }
    }

    #[test]
    fn resolve_find_overloads() {
        assert_eq!(
            ret(FIND, &["String", "String"]),
            Some("Integer".to_string())
        );
        assert_eq!(
            ret(FIND, &["String", "String", "Integer"]),
            Some("Integer".to_string())
        );
        assert_eq!(ret(FIND, &["String", "Integer"]), None);
        assert_eq!(ret(FIND, &["String", "String", "String"]), None);
    }

    #[test]
    fn resolve_mid_and_replace() {
        assert_eq!(
            ret(MID, &["String", "Integer", "Integer"]),
            Some("String".to_string())
        );
        assert_eq!(ret(MID, &["String", "Integer"]), None);
        assert_eq!(
            ret(REPLACE, &["String", "String", "String"]),
            Some("String".to_string())
        );
        assert_eq!(ret(REPLACE, &["String", "String", "Integer"]), None);
    }

    #[test]
    fn resolve_rejects_unknown_name() {
        assert_eq!(ret("strings.bogus", &["String"]), None);
    }

    #[test]
    fn exact_helper() {
        assert!(exact(
            &types(&["String", "Integer"]),
            &["String", "Integer"]
        ));
        assert!(!exact(&types(&["String"]), &["String", "Integer"]));
        assert!(!exact(&types(&["Integer"]), &["String"]));
        assert!(exact(&types(&[]), &[]));
    }
}
