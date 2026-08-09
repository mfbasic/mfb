use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinSource,
    DefaultResolver, DefaultValue, Implementation, InjectionRule, Lowering, Parameter,
    ParameterType, ReturnType,
};

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
// plan-70-A: the terminal column width of a string — the sum of its grapheme
// clusters' display widths (0 for zero-width, 2 for East Asian Wide/emoji, 1
// otherwise). Additive; does not change any scalar/grapheme/byte semantics.
const DISPLAY_WIDTH: &str = "strings.displayWidth";
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

// plan-72-V: `STRINGS` is the descriptor authority. Despite the census `custom 1`,
// `strings` needs no per-call resolver: `implementation_name` is a fixed per-name
// `Implementation::Rewrite(__strings_*)` for the seven scalar-seam members and
// `Same` for every native member, and `resolve_call`/`call_return_type_name`/
// `arity` all derive from `DefaultResolver`. Optional trailing arguments
// (`padLeft`/`padRight`'s `padChar`, `find`'s `start`) are `DefaultValue::Optional`
// — they widen arity but are never default-padded (strings has no
// `default_argument_padding`; the native/source bodies select by arg count). Only
// `expected_arguments` stays hand-authored (the optional-arg `[, T]` bracket
// phrasing the descriptor's per-position type list cannot render — the
// `collections` precedent). The module DOES carry a resolver, but solely for the
// scalar-seam source predicate: its companion injects `WhenUsed` (only when a
// program both `IMPORT strings` AND references a seam member), so `uses_source`
// delegates to the load-bearing `uses_package` walk below.
const P_VALUE: &[Parameter] = &[Parameter::required("value", "String")];
const P_PREFIX: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("prefix", "String"),
];
const P_SUFFIX: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("suffix", "String"),
];
const P_NEEDLE: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("needle", "String"),
];
const P_SPLIT: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter {
        name: "delimiter",
        aliases: &["separator"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
];
const P_JOIN: &[Parameter] = &[
    Parameter {
        name: "parts",
        aliases: &["values"],
        ty: ParameterType::Named("List OF String"),
        default: DefaultValue::None,
    },
    Parameter {
        name: "delimiter",
        aliases: &["separator"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
];
const P_PREFIXES: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("prefixes", "List OF String"),
];
const P_SUFFIXES: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("suffixes", "List OF String"),
];
const P_COUNT_INT: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("count", "Integer"),
];
const P_TIMES: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("times", "Integer"),
];
const P_INDEX: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("index", "Integer"),
];
const P_CHARS: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("chars", "String"),
];
// padLeft/padRight(value, width, [padChar]) — trailing `padChar` widens arity but
// is not default-padded.
const P_PAD: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("width", "Integer"),
    Parameter {
        name: "padChar",
        aliases: &[],
        ty: ParameterType::Named("String"),
        default: DefaultValue::Optional,
    },
];
// find(value, needle, [start]) — trailing `start` widens arity but is not padded.
const P_FIND: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("needle", "String"),
    Parameter {
        name: "start",
        aliases: &[],
        ty: ParameterType::Named("Integer"),
        default: DefaultValue::Optional,
    },
];
const P_MID: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter::required("start", "Integer"),
    Parameter::required("count", "Integer"),
];
const P_REPLACE: &[Parameter] = &[
    Parameter::required("value", "String"),
    Parameter {
        name: "old",
        aliases: &["needle"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
    Parameter {
        name: "new",
        aliases: &["replacement"],
        ty: ParameterType::Named("String"),
        default: DefaultValue::None,
    },
];
const P_FROM_SCALARS: &[Parameter] = &[Parameter::required("scalars", "List OF Scalar")];
const P_SCALAR: &[Parameter] = &[Parameter::required("scalar", "Scalar")];

const fn ov(params: &'static [Parameter], ret: &'static str) -> [BuiltinOverload; 1] {
    [BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }]
}

const OV_VALUE_STRING: &[BuiltinOverload] = &ov(P_VALUE, "String");
const OV_VALUE_LIST_STRING: &[BuiltinOverload] = &ov(P_VALUE, "List OF String");
const OV_VALUE_LIST_BYTE: &[BuiltinOverload] = &ov(P_VALUE, "List OF Byte");
const OV_VALUE_INTEGER: &[BuiltinOverload] = &ov(P_VALUE, "Integer");
const OV_VALUE_LIST_SCALAR: &[BuiltinOverload] = &ov(P_VALUE, "List OF Scalar");
const OV_PREFIX_BOOL: &[BuiltinOverload] = &ov(P_PREFIX, "Boolean");
const OV_SUFFIX_BOOL: &[BuiltinOverload] = &ov(P_SUFFIX, "Boolean");
const OV_NEEDLE_BOOL: &[BuiltinOverload] = &ov(P_NEEDLE, "Boolean");
const OV_NEEDLE_INT: &[BuiltinOverload] = &ov(P_NEEDLE, "Integer");
const OV_PREFIX_STRING: &[BuiltinOverload] = &ov(P_PREFIX, "String");
const OV_SUFFIX_STRING: &[BuiltinOverload] = &ov(P_SUFFIX, "String");
const OV_SPLIT: &[BuiltinOverload] = &ov(P_SPLIT, "List OF String");
const OV_JOIN: &[BuiltinOverload] = &ov(P_JOIN, "String");
const OV_PREFIXES_BOOL: &[BuiltinOverload] = &ov(P_PREFIXES, "Boolean");
const OV_SUFFIXES_BOOL: &[BuiltinOverload] = &ov(P_SUFFIXES, "Boolean");
const OV_COUNT_STRING: &[BuiltinOverload] = &ov(P_COUNT_INT, "String");
const OV_TIMES_STRING: &[BuiltinOverload] = &ov(P_TIMES, "String");
const OV_INDEX_STRING: &[BuiltinOverload] = &ov(P_INDEX, "String");
const OV_CHARS_STRING: &[BuiltinOverload] = &ov(P_CHARS, "String");
const OV_PAD: &[BuiltinOverload] = &ov(P_PAD, "String");
const OV_FIND: &[BuiltinOverload] = &ov(P_FIND, "Integer");
const OV_MID: &[BuiltinOverload] = &ov(P_MID, "String");
const OV_REPLACE: &[BuiltinOverload] = &ov(P_REPLACE, "String");
const OV_FROM_SCALARS: &[BuiltinOverload] = &ov(P_FROM_SCALARS, "String");
const OV_SCALAR_BOOL: &[BuiltinOverload] = &ov(P_SCALAR, "Boolean");

const fn strings_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
    implementation: Implementation,
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        implementation,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const STRINGS_FUNCTIONS: &[BuiltinFunction] = &[
    strings_fn(TRIM, "trim", OV_VALUE_STRING, Implementation::Same),
    strings_fn(
        TRIM_START,
        "trimStart",
        OV_VALUE_STRING,
        Implementation::Same,
    ),
    strings_fn(TRIM_END, "trimEnd", OV_VALUE_STRING, Implementation::Same),
    strings_fn(UPPER, "upper", OV_VALUE_STRING, Implementation::Same),
    strings_fn(LOWER, "lower", OV_VALUE_STRING, Implementation::Same),
    strings_fn(CASE_FOLD, "caseFold", OV_VALUE_STRING, Implementation::Same),
    strings_fn(
        NORMALIZE_NFC,
        "normalizeNfc",
        OV_VALUE_STRING,
        Implementation::Same,
    ),
    strings_fn(
        GRAPHEMES,
        "graphemes",
        OV_VALUE_LIST_STRING,
        Implementation::Same,
    ),
    strings_fn(
        STARTS_WITH,
        "startsWith",
        OV_PREFIX_BOOL,
        Implementation::Same,
    ),
    strings_fn(ENDS_WITH, "endsWith", OV_SUFFIX_BOOL, Implementation::Same),
    strings_fn(CONTAINS, "contains", OV_NEEDLE_BOOL, Implementation::Same),
    strings_fn(SPLIT, "split", OV_SPLIT, Implementation::Same),
    strings_fn(JOIN, "join", OV_JOIN, Implementation::Same),
    strings_fn(BYTE_LEN, "byteLen", OV_VALUE_INTEGER, Implementation::Same),
    strings_fn(
        STARTS_WITH_ANY,
        "startsWithAny",
        OV_PREFIXES_BOOL,
        Implementation::Same,
    ),
    strings_fn(
        ENDS_WITH_ANY,
        "endsWithAny",
        OV_SUFFIXES_BOOL,
        Implementation::Same,
    ),
    strings_fn(
        STRIP_PREFIX,
        "stripPrefix",
        OV_PREFIX_STRING,
        Implementation::Same,
    ),
    strings_fn(
        STRIP_SUFFIX,
        "stripSuffix",
        OV_SUFFIX_STRING,
        Implementation::Same,
    ),
    strings_fn(COUNT, "count", OV_NEEDLE_INT, Implementation::Same),
    strings_fn(LEFT, "left", OV_COUNT_STRING, Implementation::Same),
    strings_fn(RIGHT, "right", OV_COUNT_STRING, Implementation::Same),
    strings_fn(REPEAT, "repeat", OV_TIMES_STRING, Implementation::Same),
    strings_fn(PAD_LEFT, "padLeft", OV_PAD, Implementation::Same),
    strings_fn(PAD_RIGHT, "padRight", OV_PAD, Implementation::Same),
    strings_fn(
        GRAPHEME_AT,
        "graphemeAt",
        OV_INDEX_STRING,
        Implementation::Same,
    ),
    strings_fn(
        GRAPHEMES_COUNT,
        "graphemesCount",
        OV_VALUE_INTEGER,
        Implementation::Same,
    ),
    strings_fn(
        DISPLAY_WIDTH,
        "displayWidth",
        OV_VALUE_INTEGER,
        Implementation::Same,
    ),
    strings_fn(
        TRIM_CHARS,
        "trimChars",
        OV_CHARS_STRING,
        Implementation::Same,
    ),
    strings_fn(
        TO_BYTES,
        "toBytes",
        OV_VALUE_LIST_BYTE,
        Implementation::Same,
    ),
    strings_fn(FIND, "find", OV_FIND, Implementation::Same),
    strings_fn(MID, "mid", OV_MID, Implementation::Same),
    strings_fn(REPLACE, "replace", OV_REPLACE, Implementation::Same),
    // Scalar seam + classification predicates — source-companion rewrites.
    strings_fn(
        TO_SCALARS,
        "toScalars",
        OV_VALUE_LIST_SCALAR,
        Implementation::Rewrite("__strings_toScalars"),
    ),
    strings_fn(
        FROM_SCALARS,
        "fromScalars",
        OV_FROM_SCALARS,
        Implementation::Rewrite("__strings_fromScalars"),
    ),
    strings_fn(
        IS_LETTER,
        "isLetter",
        OV_SCALAR_BOOL,
        Implementation::Rewrite("__strings_isLetter"),
    ),
    strings_fn(
        IS_DIGIT,
        "isDigit",
        OV_SCALAR_BOOL,
        Implementation::Rewrite("__strings_isDigit"),
    ),
    strings_fn(
        IS_WHITESPACE,
        "isWhitespace",
        OV_SCALAR_BOOL,
        Implementation::Rewrite("__strings_isWhitespace"),
    ),
    strings_fn(
        IS_UPPER,
        "isUpper",
        OV_SCALAR_BOOL,
        Implementation::Rewrite("__strings_isUpper"),
    ),
    strings_fn(
        IS_LOWER,
        "isLower",
        OV_SCALAR_BOOL,
        Implementation::Rewrite("__strings_isLower"),
    ),
];

/// The scalar-seam source predicate — the ONLY resolver hook `strings` needs. The
/// companion is injected `WhenUsed`; `uses_source` reuses the load-bearing
/// `uses_package` walk (import of `strings` AND a reference to a seam member).
struct StringsResolver;
impl BuiltinResolver for StringsResolver {
    fn uses_source(
        &self,
        _module: &BuiltinModule,
        project: &crate::ast::AstProject,
    ) -> Option<bool> {
        Some(uses_package(project))
    }

    /// Return type, delegating to `resolve_call` (which is the exact-match
    /// `DefaultResolver` resolution). Exposed so plan-72-BB can drive `strings::`
    /// return types uniformly through the registry resolver for every
    /// resolver-backed package.
    ///
    /// plan-89-C: the Tier-A query members also accept an `AttributedString` at the
    /// text position, returning exactly what the `String` overload returns
    /// (computed on the visible text). Substituting `String` for a leading
    /// `AttributedString` reuses the `String` resolution unchanged; codegen rewrites
    /// the argument to `toString(a)` so the existing `String` lowering runs.
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        if is_tier_a_query(name) && arg_types.first().map(String::as_str) == Some("AttributedString")
        {
            let mut substituted = arg_types.to_vec();
            substituted[0] = "String".to_string();
            return resolve_call(name, &substituted).map(|resolved| resolved.return_type.into_owned());
        }
        resolve_call(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static STRINGS_RESOLVER: StringsResolver = StringsResolver;

/// The Tier-A `strings::` query members (plan-89-C): they *interrogate* the text
/// (returning a measurement, a position, or a decomposition into a collection)
/// rather than re-expressing it, so an `AttributedString` argument is answered on
/// its visible text and the result type matches the `String` overload. The
/// text-modifying members (Tier-B) return `AttributedString` and are handled in
/// plan-89-D. The frozen partition is recorded in plan-89-C §4.1.
pub(crate) fn is_tier_a_query(name: &str) -> bool {
    matches!(
        name,
        BYTE_LEN
            | CONTAINS
            | COUNT
            | DISPLAY_WIDTH
            | ENDS_WITH
            | ENDS_WITH_ANY
            | FIND
            | GRAPHEMES
            | GRAPHEMES_COUNT
            | SPLIT
            | STARTS_WITH
            | STARTS_WITH_ANY
            | TO_BYTES
            | TO_SCALARS
            | GRAPHEME_AT
    )
}

pub(crate) static STRINGS: BuiltinModule = BuiltinModule {
    name: "strings",
    functions: STRINGS_FUNCTIONS,
    types: &[],
    source: Some(BuiltinSource {
        rule: InjectionRule::WhenUsed,
        loader: source_file,
    }),
    resolver: Some(&STRINGS_RESOLVER),
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_strings_call(name: &str) -> bool {
    DefaultResolver::contains(&STRINGS, name)
}

// `call_param_names` returns a `&'static` borrowed shape the owned
// `DefaultResolver::param_names` cannot produce, so it stays a static table PINNED
// equal to `STRINGS` by the parity test until plan-72-BB. Aliases (`split`
// delimiter/separator, `join` parts/values, `replace` old/needle & new/replacement)
// come from each parameter's `aliases`.
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
        GRAPHEMES_COUNT | DISPLAY_WIDTH => Some(&[&["value"]]),
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

pub(crate) fn resolve_call<'a>(name: &str, arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    DefaultResolver::resolve_call(&STRINGS, name, arg_types).map(|return_type| ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

// Bespoke `[, T]` bracket phrasing for the optional `padChar`/`start`; the
// descriptor's per-position type list renders `String, Integer, String` etc., so
// this stays hand-authored (the `collections` precedent) and is NOT asserted
// against the descriptor by the parity test.
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
        GRAPHEMES_COUNT | DISPLAY_WIDTH => Some("String"),
        FIND => Some("String, String[, Integer]"),
        MID => Some("String, Integer, Integer"),
        REPLACE => Some("String, String, String"),
        TO_SCALARS => Some("String"),
        FROM_SCALARS => Some("List OF Scalar"),
        IS_LETTER | IS_DIGIT | IS_WHITESPACE | IS_UPPER | IS_LOWER => Some("Scalar"),
        _ => None,
    }
}

/// The source-companion implementation name (`__strings_*`) for the Scalar seam
/// and classification predicates (plan-41-D). Only these members are backed by
/// `strings_package.mfb`; every other `strings::` member is native codegen and
/// returns `None` here so it keeps its native lowering. Delegates to `STRINGS`'
/// per-name `Implementation::Rewrite`.
pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    DefaultResolver::implementation_name(&STRINGS, name)
}

/// The source companion backing the Scalar seam/predicates: the scalar helpers
/// plus the shared Unicode general-category table (`__regex_genCat`), appended
/// from `unicode_gencat.mfb`. Both are file-local, so this copy of the table never
/// collides with the regex companion's own copy when both packages are imported.
pub(crate) fn source_file() -> Result<crate::ast::AstFile, ()> {
    // The Unicode general-category table is the same generated source as the
    // regex companion (`unicode_gencat.mfb`, one source of truth), but its sole
    // function `__regex_genCat` is renamed to `__strings_genCat` so the two
    // companions never collide on a project-global symbol when both `regex` and
    // `strings` are imported.
    //
    // bug-339 B1: the SOURCE has a single source of truth (this one file). What is
    // embedded twice is the COMPILED table — once per package — when a program
    // imports both `regex` and `strings`. That is language-mandated, not an
    // oversight: an injected built-in source is one file whose FUNCs are
    // file-local (PACKAGE visibility is invalid in an executable — see
    // `regex::source_file`), so each package must carry its own file-local copy of
    // `genCat`. Holding ONE compiled copy would require exporting the table as a
    // project-global symbol from a shared source injected exactly once — a
    // restructure of the built-in injection/augmentation model that risks every
    // regex/strings program for a compile-time-only win, so it is deliberately not
    // done here. The `.replace` seam is the load-bearing part and must stay.
    let table = include_str!("unicode_gencat.mfb").replace("__regex_genCat", "__strings_genCat");
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
        // TCASE bodies are ordinary statements and can reference the scalar seam.
        // `lower_testing_blocks` desugars them into `Item::Function`s before this
        // gate runs today, so this arm is belt-and-braces — but relying on that
        // pass ordering left `__strings_*` undefined if it ever changed (bug-222).
        // Over-injection is harmless (the module's design note).
        Item::Testing(block) => block.groups.iter().any(group_references_seam),
        _ => false,
    }
}

fn group_references_seam(group: &crate::ast::TestGroup) -> bool {
    use crate::ast::TestGroupMember;
    group.members.iter().any(|member| match member {
        TestGroupMember::Case(case) => case.body.iter().any(stmt_references_seam),
        TestGroupMember::Group(nested) => group_references_seam(nested),
    })
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
                || cases
                    .iter()
                    .any(|case| case.body.iter().any(stmt_references_seam))
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
        Statement::ForEach {
            iterable, body: b, ..
        } => expr_references_seam(iterable) || body(b),
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
            expr_references_seam(target) || updates.iter().any(|u| expr_references_seam(&u.value))
        }
        Expression::ListLiteral(values) => values.iter().any(expr_references_seam),
        Expression::SetLiteral { elements, .. } => elements.iter().any(expr_references_seam),
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

#[cfg(test)]
mod tests {
    use super::*;
    // `resolve_call` now derives from the descriptor, so the shared `exact` helper
    // is only referenced by the `exact_helper` regression test below.
    use crate::builtins::exact;

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
        DISPLAY_WIDTH,
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

    fn parse_file(src: &str) -> crate::ast::AstFile {
        crate::ast::parse_source(std::path::Path::new("t.mfb"), "t.mfb", src).unwrap()
    }

    fn project(files: Vec<crate::ast::AstFile>) -> crate::ast::AstProject {
        crate::ast::AstProject {
            name: "test".to_string(),
            files,
        }
    }

    // A body exercising the loop / lambda recursion arms of the seam walk without
    // referencing a seam function (so each arm executes and returns false), then a
    // TESTING block (nested TGROUP + TCASE) that DOES reference a seam member.
    const SEAM_SOURCE: &str = "\
IMPORT strings

FUNC noSeam() AS Nothing
  MUT total AS Integer = 0
  FOR i = 1 TO 3 STEP 1
    total = i
  NEXT
  DO
    total = total + 1
  LOOP UNTIL total >= 5
  LET f AS FUNC(Integer) AS Integer = LAMBDA(x AS Integer) -> x + 1
END FUNC

TESTING
  TGROUP \"outer\"
    TGROUP \"nested\"
      TCASE \"refs seam\"
        LET v AS List OF Scalar = strings::toScalars(\"hi\")
      END TCASE
    END TGROUP
  END TGROUP
END TESTING
";

    #[test]
    fn uses_package_true_through_nested_shapes() {
        let ast = project(vec![parse_file(SEAM_SOURCE)]);
        assert!(uses_package(&ast));
    }

    #[test]
    fn uses_package_false_without_seam_reference() {
        // Imports strings but never calls a seam member.
        let ast = project(vec![parse_file(
            "IMPORT strings\n\nFUNC plain() AS Nothing\n  LET x AS Integer = 1\nEND FUNC\n",
        )]);
        assert!(!uses_package(&ast));
        // Does not import strings at all.
        let ast2 = project(vec![parse_file(
            "FUNC plain() AS List OF Scalar\n  RETURN strings::toScalars(\"hi\")\nEND FUNC\n",
        )]);
        assert!(!uses_package(&ast2));
    }

    #[test]
    fn augmented_project_appends_companion_source() {
        let ast = project(vec![parse_file(SEAM_SOURCE)]);
        let before = ast.files.len();
        let augmented = augmented_project(&ast).unwrap();
        assert_eq!(augmented.files.len(), before + 1);
        // The companion parses (source_file succeeded) and carries the renamed
        // general-category function.
        let companion = augmented.files.last().unwrap();
        assert!(companion.path.contains("strings"));

        // A project that does not use the package is returned unchanged.
        let plain = project(vec![parse_file(
            "FUNC plain() AS Nothing\n  LET x AS Integer = 1\nEND FUNC\n",
        )]);
        assert_eq!(augmented_project(&plain).unwrap().files.len(), 1);
    }

    // The `ov` and `strings_fn` const-fn constructors are only evaluated in const
    // context (the `OV_*` tables and `STRINGS_FUNCTIONS`), so they show no runtime
    // coverage. Call them at runtime and assert their returned fields.
    #[test]
    fn descriptor_constructors_execute_at_runtime() {
        // `ov` returns a single-element `[BuiltinOverload; 1]`.
        let overload = ov(P_VALUE, "String");
        assert_eq!(overload.len(), 1);
        assert_eq!(overload[0].params.len(), 1);
        assert_eq!(overload[0].params[0].name, "value");
        assert_eq!(overload[0].return_type, ReturnType::Fixed("String"));

        // `strings_fn` builds a `BuiltinFunction`. `OV_VALUE_STRING` is already a
        // `&'static [BuiltinOverload]`, so it passes without the E0716 temporary.
        let func = strings_fn(TRIM, "trim", OV_VALUE_STRING, Implementation::Same);
        assert_eq!(func.name, TRIM);
        assert_eq!(func.doc_slug, "trim");
        assert_eq!(func.overloads.len(), 1);
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Helper);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);

        // A `Rewrite` implementation exercises the same builder with the seam arm.
        let seam = strings_fn(
            TO_SCALARS,
            "toScalars",
            OV_VALUE_LIST_SCALAR,
            Implementation::Rewrite("__strings_toScalars"),
        );
        assert_eq!(
            seam.implementation,
            Implementation::Rewrite("__strings_toScalars")
        );
    }

    // The scalar-seam arms of `expected_arguments` (their bespoke phrasing) are not
    // hit by `expected_arguments_specific`, which covers only native members.
    #[test]
    fn expected_arguments_seam_members() {
        assert_eq!(expected_arguments(TO_SCALARS), Some("String"));
        assert_eq!(expected_arguments(FROM_SCALARS), Some("List OF Scalar"));
        for name in [IS_LETTER, IS_DIGIT, IS_WHITESPACE, IS_UPPER, IS_LOWER] {
            assert_eq!(expected_arguments(name), Some("Scalar"));
        }
        assert_eq!(expected_arguments("strings.bogus"), None);
    }

    // The `StringsResolver::uses_source` hook (the only resolver method the module
    // needs) delegates to `uses_package`.
    #[test]
    fn uses_source_resolver_hook() {
        let seam = project(vec![parse_file(SEAM_SOURCE)]);
        assert_eq!(STRINGS_RESOLVER.uses_source(&STRINGS, &seam), Some(true));
        let plain = project(vec![parse_file(
            "IMPORT strings\n\nFUNC f() AS Nothing\n  LET x AS Integer = 1\nEND FUNC\n",
        )]);
        assert_eq!(STRINGS_RESOLVER.uses_source(&STRINGS, &plain), Some(false));
    }

    // The `Fail`, `Constructor`, and `MapLiteral` arms of the seam walk are not
    // reached by `SEAM_SOURCE`; drive each with a seam reference in that position.
    #[test]
    fn seam_walk_fail_constructor_map_arms() {
        // Statement::Fail — the error expression references a seam member.
        let fail_src =
            "IMPORT strings\n\nFUNC f() AS Nothing\n  FAIL strings::toScalars(\"hi\")\nEND FUNC\n";
        assert!(uses_package(&project(vec![parse_file(fail_src)])));

        // Expression::Constructor — a positional constructor argument references it.
        let ctor_src = "IMPORT strings\n\nFUNC f() AS Nothing\n  \
LET p AS Thing = Thing[strings::toScalars(\"hi\")]\nEND FUNC\n";
        assert!(uses_package(&project(vec![parse_file(ctor_src)])));

        // Expression::MapLiteral — a map entry value references it.
        let map_src = "IMPORT strings\n\nFUNC f() AS Nothing\n  \
LET m AS Map OF String TO String = Map OF String TO String { \"k\" := strings::toScalars(\"hi\") }\nEND FUNC\n";
        assert!(uses_package(&project(vec![parse_file(map_src)])));
    }
}
