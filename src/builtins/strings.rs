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
            | FIND
            | MID
            | REPLACE
    )
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        TRIM | TRIM_START | TRIM_END | UPPER | LOWER | CASE_FOLD | NORMALIZE_NFC | GRAPHEMES
        | BYTE_LEN => Some(&[&["value"]]),
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
        | BYTE_LEN => Some("String"),
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
        | BYTE_LEN | GRAPHEMES_COUNT => Some((1, 1)),
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
