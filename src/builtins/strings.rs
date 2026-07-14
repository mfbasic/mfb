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
// The Scalar seam + classification predicates (plan-41-D). These are backed by
// the source companion `strings_package.mfb` (dispatched via `implementation_name`
// to the `__strings_*` helpers), not native codegen.
const TO_SCALARS: &str = "strings.toScalars";
const FROM_SCALARS: &str = "strings.fromScalars";
const IS_LETTER: &str = "strings.isLetter";
const IS_DIGIT: &str = "strings.isDigit";
const IS_WHITESPACE: &str = "strings.isWhitespace";
const IS_UPPER: &str = "strings.isUpper";
const IS_LOWER: &str = "strings.isLower";

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
            | TO_SCALARS
            | FROM_SCALARS
            | IS_LETTER
            | IS_DIGIT
            | IS_WHITESPACE
            | IS_UPPER
            | IS_LOWER
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
        TO_SCALARS => Some(&[&["value"]]),
        FROM_SCALARS => Some(&[&["scalars"]]),
        IS_LETTER | IS_DIGIT | IS_WHITESPACE | IS_UPPER | IS_LOWER => Some(&[&["scalar"]]),
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
        TO_SCALARS => Some("List OF Scalar"),
        FROM_SCALARS => Some("String"),
        IS_LETTER | IS_DIGIT | IS_WHITESPACE | IS_UPPER | IS_LOWER => Some("Boolean"),
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
        TO_SCALARS if exact(arg_types, &["String"]) => Cow::Borrowed("List OF Scalar"),
        FROM_SCALARS if exact(arg_types, &["List OF Scalar"]) => Cow::Borrowed("String"),
        IS_LETTER | IS_DIGIT | IS_WHITESPACE | IS_UPPER | IS_LOWER
            if exact(arg_types, &["Scalar"]) =>
        {
            Cow::Borrowed("Boolean")
        }
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
        TO_SCALARS => Some("String"),
        FROM_SCALARS => Some("List OF Scalar"),
        IS_LETTER | IS_DIGIT | IS_WHITESPACE | IS_UPPER | IS_LOWER => Some("Scalar"),
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
        TO_SCALARS | FROM_SCALARS | IS_LETTER | IS_DIGIT | IS_WHITESPACE | IS_UPPER | IS_LOWER => {
            Some((1, 1))
        }
        _ => None,
    }
}

/// The source-companion implementation name (`__strings_*`) for the Scalar seam
/// and classification predicates (plan-41-D). Only these members are backed by
/// `strings_package.mfb`; every other `strings::` member is native codegen and
/// returns `None` here so it keeps its native lowering.
pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    match name {
        TO_SCALARS => Some("__strings_toScalars"),
        FROM_SCALARS => Some("__strings_fromScalars"),
        IS_LETTER => Some("__strings_isLetter"),
        IS_DIGIT => Some("__strings_isDigit"),
        IS_WHITESPACE => Some("__strings_isWhitespace"),
        IS_UPPER => Some("__strings_isUpper"),
        IS_LOWER => Some("__strings_isLower"),
        _ => None,
    }
}

/// The source companion backing the Scalar seam/predicates: the scalar helpers
/// plus the shared Unicode general-category table (`__regex_genCat`), appended
/// from `regex_unicode.mfb`. Both are file-local, so this copy of the table never
/// collides with the regex companion's own copy when both packages are imported.
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    // The Unicode general-category table is the same generated source as the
    // regex companion (`regex_unicode.mfb`, one source of truth), but its sole
    // function `__regex_genCat` is renamed to `__strings_genCat` so the two
    // companions never collide on a project-global symbol when both `regex` and
    // `strings` are imported.
    let table = include_str!("regex_unicode.mfb").replace("__regex_genCat", "__strings_genCat");
    let combined = format!("{}\n{}", include_str!("strings_package.mfb"), table);
    crate::ast::parse_source_internal(
        std::path::Path::new("<builtin-strings>"),
        "builtins/strings.mfb",
        &combined,
    )
}

/// The seven scalar-seam members backed by the source companion. Their short
/// (unqualified) names, used to gate injection on actual usage.
const SEAM_SHORT_NAMES: &[&str] = &[
    "toScalars",
    "fromScalars",
    "isLetter",
    "isDigit",
    "isWhitespace",
    "isUpper",
    "isLower",
];

fn callee_is_seam(callee: &str) -> bool {
    // The callee may be source-qualified (`strings::toScalars`), aliased
    // (`s::toScalars`), or canonicalized to the dotted form (`strings.toScalars`)
    // depending on which pass runs the gate; reduce to the final segment across
    // both separators. Over-matching (a user's own `toScalars`) only injects the
    // companion unnecessarily, never wrongly.
    let short = callee
        .rsplit("::")
        .next()
        .unwrap_or(callee)
        .rsplit('.')
        .next()
        .unwrap_or(callee);
    SEAM_SHORT_NAMES.contains(&short)
}

/// Whether the project uses `strings` AND references at least one scalar-seam
/// member. The companion carries the full ~4k-line Unicode general-category
/// table, so injecting it for every `IMPORT strings` would tax the common case;
/// gating on actual usage keeps a plain strings program cheap (plan-41-D).
pub(crate) fn uses_package(ast: &crate::ast::AstProject) -> bool {
    let imports_strings = ast.files.iter().any(|file| {
        file.imports
            .iter()
            .any(|import| import.package_name() == "strings")
    });
    imports_strings
        && ast
            .files
            .iter()
            .any(|file| file.items.iter().any(item_references_seam))
}

fn item_references_seam(item: &crate::ast::Item) -> bool {
    use crate::ast::Item;
    match item {
        Item::Function(f) => f.body.iter().any(stmt_references_seam),
        Item::Binding(b) => b.value.as_ref().is_some_and(expr_references_seam),
        _ => false,
    }
}

fn stmt_references_seam(stmt: &crate::ast::Statement) -> bool {
    use crate::ast::Statement;
    let body = |stmts: &[Statement]| stmts.iter().any(stmt_references_seam);
    match stmt {
        Statement::Let { value, .. }
        | Statement::Return { value, .. }
        | Statement::Recover { value, .. }
        | Statement::Exit { code: value, .. } => value.as_ref().is_some_and(expr_references_seam),
        Statement::Fail { error, .. } => expr_references_seam(error),
        Statement::Assign { value, .. } | Statement::StateAssign { value, .. } => {
            expr_references_seam(value)
        }
        Statement::Expression { expression, .. } => expr_references_seam(expression),
        Statement::If {
            condition,
            then_body,
            else_body,
            ..
        } => expr_references_seam(condition) || body(then_body) || body(else_body),
        Statement::Match {
            expression, cases, ..
        } => {
            expr_references_seam(expression)
                || cases.iter().any(|case| case.body.iter().any(stmt_references_seam))
        }
        Statement::For {
            start,
            end,
            step,
            body: b,
            ..
        } => {
            expr_references_seam(start)
                || expr_references_seam(end)
                || step.as_ref().is_some_and(expr_references_seam)
                || body(b)
        }
        Statement::ForEach { iterable, body: b, .. } => {
            expr_references_seam(iterable) || body(b)
        }
        Statement::While {
            condition, body: b, ..
        }
        | Statement::DoUntil {
            condition, body: b, ..
        } => expr_references_seam(condition) || body(b),
        Statement::Continue { .. } | Statement::Propagate { .. } => false,
    }
}

fn expr_references_seam(expr: &crate::ast::Expression) -> bool {
    use crate::ast::{CallArg, ConstructorArg, Expression};
    let arg = |a: &CallArg| match a {
        CallArg::Positional(v) | CallArg::Named { value: v, .. } => expr_references_seam(v),
    };
    match expr {
        Expression::Call {
            callee, arguments, ..
        } => callee_is_seam(callee) || arguments.iter().any(arg),
        Expression::Binary { left, right, .. } => {
            expr_references_seam(left) || expr_references_seam(right)
        }
        Expression::Unary { operand, .. } => expr_references_seam(operand),
        Expression::Lambda { body, .. } => expr_references_seam(body),
        Expression::Constructor { arguments, .. } => arguments.iter().any(|a| match a {
            ConstructorArg::Positional(v) | ConstructorArg::Named { value: v, .. } => {
                expr_references_seam(v)
            }
        }),
        Expression::WithUpdate { target, updates } => {
            expr_references_seam(target)
                || updates.iter().any(|u| expr_references_seam(&u.value))
        }
        Expression::ListLiteral(values) => values.iter().any(expr_references_seam),
        Expression::MapLiteral { entries, .. } => entries
            .iter()
            .any(|(k, v)| expr_references_seam(k) || expr_references_seam(v)),
        Expression::MemberAccess { target, .. } => expr_references_seam(target),
        Expression::Trapped {
            expression,
            handler,
            ..
        } => expr_references_seam(expression) || handler.iter().any(stmt_references_seam),
        Expression::String(_)
        | Expression::Number(_)
        | Expression::Scalar(_)
        | Expression::Boolean(_)
        | Expression::Identifier(_) => false,
    }
}

pub(crate) fn augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(source_file()?);
    Ok(augmented)
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
