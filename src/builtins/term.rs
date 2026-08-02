//! Built-in `term::` module surface (plan-01-term.md).
//!
//! `term::` gives MFBASIC programs a structured terminal surface: cursor
//! movement, colors, text attributes, screen clearing, and a full-screen "TUI
//! mode" toggle. This module declares the language-facing surface (call names,
//! arity, argument/return types, and the two built-in record types `TermColor`
//! and `TermSize`); the runtime behavior lives in the native code backends.
//!
//! `term::on()` is the gate: every `term::*` call other than `term::on()` and
//! `term::isOn()` is a no-op while TUI mode is off (plan §4.2.1). That rule is a
//! runtime concern (a `state.active` check in each helper), not a syntaxcheck one,
//! so typing and arity here are unconditional.

use std::borrow::Cow;

use super::descriptor::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinType, DefaultResolver,
    Implementation, Lowering, Parameter, ReturnType, TypeKind,
};

pub(crate) const TERM_COLOR_TYPE: &str = "TermColor";
pub(crate) const TERM_SIZE_TYPE: &str = "TermSize";
/// The `LineStyle` enum (`Light`…`Double`) selecting the box-drawing weight for
/// `term::drawHLine` / `term::drawVLine`. Unlike the two records above it is a
/// real enum, declared in `term_package.mfb` (native `builtin_type_fields` can
/// only declare records), so the resolver learns its members from the injected
/// package source while `is_builtin_type` accepts the bare type name here.
pub(crate) const LINE_STYLE_TYPE: &str = "LineStyle";
/// The `FillStyle` enum (`Filled`/`Light`/`Medium`/`Dark`/`Checker`/`CheckerAlt`)
/// selecting the block or shade glyph `term::fillRect` stamps. Declared alongside
/// `LineStyle` in `term_package.mfb`.
pub(crate) const FILL_STYLE_TYPE: &str = "FillStyle";

pub(crate) const ON: &str = "term.on";
pub(crate) const OFF: &str = "term.off";
pub(crate) const IS_ON: &str = "term.isOn";
pub(crate) const SET_FOREGROUND: &str = "term.setForeground";
pub(crate) const SET_BACKGROUND: &str = "term.setBackground";
pub(crate) const SET_BOLD: &str = "term.setBold";
pub(crate) const SET_UNDERLINE: &str = "term.setUnderline";
pub(crate) const SHOW_CURSOR: &str = "term.showCursor";
pub(crate) const HIDE_CURSOR: &str = "term.hideCursor";
pub(crate) const CLEAR: &str = "term.clear";
pub(crate) const SYNC: &str = "term.sync";
pub(crate) const MOVE_TO: &str = "term.moveTo";
pub(crate) const DRAW_HLINE: &str = "term.drawHLine";
pub(crate) const DRAW_VLINE: &str = "term.drawVLine";
pub(crate) const DRAW_BOX: &str = "term.drawBox";
pub(crate) const FILL_RECT: &str = "term.fillRect";
pub(crate) const DRAW_TEXT: &str = "term.drawText";
pub(crate) const DRAW_GLYPH: &str = "term.drawGlyph";
pub(crate) const GET_FOREGROUND: &str = "term.getForeground";
pub(crate) const GET_BACKGROUND: &str = "term.getBackground";
pub(crate) const GET_BOLD: &str = "term.getBold";
pub(crate) const GET_UNDERLINE: &str = "term.getUnderline";
pub(crate) const TERMINAL_SIZE: &str = "term.terminalSize";

// plan-72-X W: `TERM` is the descriptor authority for `term::`. Every call's
// return type is a function of the NAME alone (`resolve_call` ignores its argument
// types), so each function is one fixed-return overload and `resolve_call`
// delegates to `return_type_name`, NOT the exact-argument-match
// `DefaultResolver::resolve_call`. `term` owns all four builtin types: the records
// `TermColor`/`TermSize` (native `builtin_type_fields`) and the source-companion
// enums `LineStyle`/`FillStyle` (declared in `term_package.mfb`, no native
// fields). `arity`, `call_return_type_name`, membership, and type membership/
// fields derive from the descriptor; `param_types`, `call_param_names`, and
// `expected_arguments` keep their hand-authored tables (the descriptor's zero-arg
// conventions are `None`/`"()"`, but `term` uses `Some(&[])`/`"no arguments"`).
const TERM_TYPES: &[BuiltinType] = &[
    BuiltinType {
        name: TERM_COLOR_TYPE,
        kind: TypeKind::Record,
        fields: &[("r", "Byte"), ("g", "Byte"), ("b", "Byte")],
    },
    BuiltinType {
        name: TERM_SIZE_TYPE,
        kind: TypeKind::Record,
        fields: &[("columns", "Integer"), ("rows", "Integer")],
    },
    BuiltinType {
        name: LINE_STYLE_TYPE,
        kind: TypeKind::Enum,
        fields: &[],
    },
    BuiltinType {
        name: FILL_STYLE_TYPE,
        kind: TypeKind::Enum,
        fields: &[],
    },
];

const P_EMPTY: &[Parameter] = &[];
const P_RGB: &[Parameter] = &[
    Parameter::required("r", "Byte"),
    Parameter::required("g", "Byte"),
    Parameter::required("b", "Byte"),
];
const P_ENABLED: &[Parameter] = &[Parameter::required("enabled", "Boolean")];
const P_MOVE: &[Parameter] = &[
    Parameter::required("row", "Integer"),
    Parameter::required("column", "Integer"),
];
const P_HLINE: &[Parameter] = &[
    Parameter::required("line", LINE_STYLE_TYPE),
    Parameter::required("row", "Integer"),
    Parameter::required("colA", "Integer"),
    Parameter::required("colB", "Integer"),
];
const P_VLINE: &[Parameter] = &[
    Parameter::required("line", LINE_STYLE_TYPE),
    Parameter::required("col", "Integer"),
    Parameter::required("rowA", "Integer"),
    Parameter::required("rowB", "Integer"),
];
const P_BOX: &[Parameter] = &[
    Parameter::required("line", LINE_STYLE_TYPE),
    Parameter::required("x1", "Integer"),
    Parameter::required("y1", "Integer"),
    Parameter::required("x2", "Integer"),
    Parameter::required("y2", "Integer"),
];
const P_FILL: &[Parameter] = &[
    Parameter::required("fill", FILL_STYLE_TYPE),
    Parameter::required("x1", "Integer"),
    Parameter::required("y1", "Integer"),
    Parameter::required("x2", "Integer"),
    Parameter::required("y2", "Integer"),
];
const P_TEXT: &[Parameter] = &[
    Parameter::required("x", "Integer"),
    Parameter::required("y", "Integer"),
    Parameter::required("text", "String"),
];
const P_GLYPH: &[Parameter] = &[
    Parameter::required("x", "Integer"),
    Parameter::required("y", "Integer"),
    Parameter::required("codepoint", "Integer"),
];

const fn ov(params: &'static [Parameter], ret: &'static str) -> [BuiltinOverload; 1] {
    [BuiltinOverload {
        params,
        return_type: ReturnType::Fixed(ret),
    }]
}

const OV_NOTHING_EMPTY: &[BuiltinOverload] = &ov(P_EMPTY, "Nothing");
const OV_BOOL_EMPTY: &[BuiltinOverload] = &ov(P_EMPTY, "Boolean");
const OV_COLOR_EMPTY: &[BuiltinOverload] = &ov(P_EMPTY, TERM_COLOR_TYPE);
const OV_SIZE_EMPTY: &[BuiltinOverload] = &ov(P_EMPTY, TERM_SIZE_TYPE);
const OV_RGB: &[BuiltinOverload] = &ov(P_RGB, "Nothing");
const OV_ENABLED: &[BuiltinOverload] = &ov(P_ENABLED, "Nothing");
const OV_MOVE: &[BuiltinOverload] = &ov(P_MOVE, "Nothing");
const OV_HLINE: &[BuiltinOverload] = &ov(P_HLINE, "Nothing");
const OV_VLINE: &[BuiltinOverload] = &ov(P_VLINE, "Nothing");
const OV_BOX: &[BuiltinOverload] = &ov(P_BOX, "Nothing");
const OV_FILL: &[BuiltinOverload] = &ov(P_FILL, "Nothing");
const OV_TEXT: &[BuiltinOverload] = &ov(P_TEXT, "Nothing");
const OV_GLYPH: &[BuiltinOverload] = &ov(P_GLYPH, "Nothing");

const fn term_fn(
    name: &'static str,
    slug: &'static str,
    overloads: &'static [BuiltinOverload],
) -> BuiltinFunction {
    BuiltinFunction {
        name,
        doc_slug: slug,
        overloads,
        // `term::` calls lower directly by name to the native backend — no rewrite.
        implementation: Implementation::Same,
        lowering: Lowering::Helper,
        flags: BuiltinFlags {
            internal_only: false,
            return_type_overloaded: false,
        },
    }
}

const TERM_FUNCTIONS: &[BuiltinFunction] = &[
    term_fn(ON, "on", OV_NOTHING_EMPTY),
    term_fn(OFF, "off", OV_NOTHING_EMPTY),
    term_fn(IS_ON, "isOn", OV_BOOL_EMPTY),
    term_fn(SET_FOREGROUND, "setForeground", OV_RGB),
    term_fn(SET_BACKGROUND, "setBackground", OV_RGB),
    term_fn(SET_BOLD, "setBold", OV_ENABLED),
    term_fn(SET_UNDERLINE, "setUnderline", OV_ENABLED),
    term_fn(SHOW_CURSOR, "showCursor", OV_NOTHING_EMPTY),
    term_fn(HIDE_CURSOR, "hideCursor", OV_NOTHING_EMPTY),
    term_fn(CLEAR, "clear", OV_NOTHING_EMPTY),
    term_fn(SYNC, "sync", OV_NOTHING_EMPTY),
    term_fn(MOVE_TO, "moveTo", OV_MOVE),
    term_fn(DRAW_HLINE, "drawHLine", OV_HLINE),
    term_fn(DRAW_VLINE, "drawVLine", OV_VLINE),
    term_fn(DRAW_BOX, "drawBox", OV_BOX),
    term_fn(FILL_RECT, "fillRect", OV_FILL),
    term_fn(DRAW_TEXT, "drawText", OV_TEXT),
    term_fn(DRAW_GLYPH, "drawGlyph", OV_GLYPH),
    term_fn(GET_FOREGROUND, "getForeground", OV_COLOR_EMPTY),
    term_fn(GET_BACKGROUND, "getBackground", OV_COLOR_EMPTY),
    term_fn(GET_BOLD, "getBold", OV_BOOL_EMPTY),
    term_fn(GET_UNDERLINE, "getUnderline", OV_BOOL_EMPTY),
    term_fn(TERMINAL_SIZE, "terminalSize", OV_SIZE_EMPTY),
];

pub(crate) static TERM: BuiltinModule = BuiltinModule {
    name: "term",
    functions: TERM_FUNCTIONS,
    types: TERM_TYPES,
    source: Some(super::descriptor::BuiltinSource {
        rule: super::descriptor::InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: None,
};

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_term_call(name: &str) -> bool {
    DefaultResolver::contains(&TERM, name)
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    TERM.types.iter().any(|ty| ty.name == name)
}

pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    // A record contributes its `(field, type)` layout; the enums carry no native
    // fields (empty slice → `None`), matching the legacy table.
    TERM.types
        .iter()
        .find(|ty| ty.name == name)
        .filter(|ty| !ty.fields.is_empty())
        .map(|ty| ty.fields)
}

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        ON | OFF | IS_ON | SHOW_CURSOR | HIDE_CURSOR | CLEAR | SYNC | GET_FOREGROUND
        | GET_BACKGROUND | GET_BOLD | GET_UNDERLINE | TERMINAL_SIZE => Some(&[]),
        SET_FOREGROUND | SET_BACKGROUND => Some(&[&["r"], &["g"], &["b"]]),
        SET_BOLD | SET_UNDERLINE => Some(&[&["enabled"]]),
        MOVE_TO => Some(&[&["row"], &["column"]]),
        DRAW_HLINE => Some(&[&["line"], &["row"], &["colA"], &["colB"]]),
        DRAW_VLINE => Some(&[&["line"], &["col"], &["rowA"], &["rowB"]]),
        DRAW_BOX => Some(&[&["line"], &["x1"], &["y1"], &["x2"], &["y2"]]),
        FILL_RECT => Some(&[&["fill"], &["x1"], &["y1"], &["x2"], &["y2"]]),
        DRAW_TEXT => Some(&[&["x"], &["y"], &["text"]]),
        DRAW_GLYPH => Some(&[&["x"], &["y"], &["codepoint"]]),
        _ => None,
    }
}

/// Declared argument types per call, used by syntaxcheck to validate each argument
/// (with the usual integer-literal-to-`Byte` coercion).
pub(crate) fn param_types(name: &str) -> Option<&'static [&'static str]> {
    match name {
        ON | OFF | IS_ON | SHOW_CURSOR | HIDE_CURSOR | CLEAR | SYNC | GET_FOREGROUND
        | GET_BACKGROUND | GET_BOLD | GET_UNDERLINE | TERMINAL_SIZE => Some(&[]),
        SET_FOREGROUND | SET_BACKGROUND => Some(&["Byte", "Byte", "Byte"]),
        SET_BOLD | SET_UNDERLINE => Some(&["Boolean"]),
        MOVE_TO => Some(&["Integer", "Integer"]),
        DRAW_HLINE | DRAW_VLINE => Some(&[LINE_STYLE_TYPE, "Integer", "Integer", "Integer"]),
        DRAW_BOX => Some(&[LINE_STYLE_TYPE, "Integer", "Integer", "Integer", "Integer"]),
        FILL_RECT => Some(&[FILL_STYLE_TYPE, "Integer", "Integer", "Integer", "Integer"]),
        DRAW_TEXT => Some(&["Integer", "Integer", "String"]),
        DRAW_GLYPH => Some(&["Integer", "Integer", "Integer"]),
        _ => None,
    }
}

pub(crate) fn call_return_type_name(name: &str) -> Option<&'static str> {
    DefaultResolver::return_type_name(&TERM, name)
}

/// `arg_types` is accepted for signature parity with every other package's
/// `resolve_call` (so the `mod.rs` dispatch is uniform — bug-340 A7) but is
/// unused: a `term::` call's return type is a function of the name alone. This is
/// why the wrapper delegates to `return_type_name`, not the exact-argument-match
/// `DefaultResolver::resolve_call` (which would reject a name-only lookup).
pub(crate) fn resolve_call<'a>(name: &str, _arg_types: &'a [String]) -> Option<ResolvedCall<'a>> {
    let return_type = call_return_type_name(name)?;
    Some(ResolvedCall {
        return_type: Cow::Borrowed(return_type),
    })
}

pub(crate) fn expected_arguments(name: &str) -> Option<String> {
    let types = param_types(name)?;
    Some(if types.is_empty() {
        "no arguments".to_string()
    } else {
        types.join(", ")
    })
}

pub(crate) fn arity(name: &str) -> Option<(usize, usize)> {
    // Every `term::` parameter is required, so the descriptor's (min, max) is
    // (count, count) — identical to `param_types(name).len()`.
    DefaultResolver::arity(&TERM, name)
}

// The `term::` package source declares the `LineStyle` enum (the drawHLine /
// drawVLine weight selector). It is injected into the user's project only when a
// program `IMPORT term`s (the shared glue's `uses_package` gate); the records
// `TermColor` / `TermSize` stay native (`builtin_type_fields`) since they predate
// this source companion.
super::package_source_glue!(
    "term",
    "<builtin-term>",
    "builtins/term.mfb",
    include_str!("term_package.mfb")
);

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: &[&str] = &[
        ON,
        OFF,
        IS_ON,
        SET_FOREGROUND,
        SET_BACKGROUND,
        SET_BOLD,
        SET_UNDERLINE,
        SHOW_CURSOR,
        HIDE_CURSOR,
        CLEAR,
        SYNC,
        MOVE_TO,
        DRAW_HLINE,
        DRAW_VLINE,
        DRAW_BOX,
        FILL_RECT,
        DRAW_TEXT,
        DRAW_GLYPH,
        GET_FOREGROUND,
        GET_BACKGROUND,
        GET_BOLD,
        GET_UNDERLINE,
        TERMINAL_SIZE,
    ];

    const NO_ARG: &[&str] = &[
        ON,
        OFF,
        IS_ON,
        SHOW_CURSOR,
        HIDE_CURSOR,
        CLEAR,
        SYNC,
        GET_FOREGROUND,
        GET_BACKGROUND,
        GET_BOLD,
        GET_UNDERLINE,
        TERMINAL_SIZE,
    ];

    #[test]
    fn is_term_call_recognizes_all_and_rejects_others() {
        for name in ALL {
            assert!(is_term_call(name), "{name}");
        }
        assert!(!is_term_call("term.unknown"));
        assert!(!is_term_call("strings.trim"));
        assert!(!is_term_call(""));
    }

    #[test]
    fn builtin_types() {
        assert!(is_builtin_type(TERM_COLOR_TYPE));
        assert!(is_builtin_type(TERM_SIZE_TYPE));
        assert!(is_builtin_type(LINE_STYLE_TYPE));
        assert!(is_builtin_type(FILL_STYLE_TYPE));
        assert!(!is_builtin_type("String"));
        assert!(!is_builtin_type("File"));
        // The enums are not records, so they have no native field layout.
        assert_eq!(builtin_type_fields(LINE_STYLE_TYPE), None);
        assert_eq!(builtin_type_fields(FILL_STYLE_TYPE), None);
        assert_eq!(
            builtin_type_fields(TERM_COLOR_TYPE),
            Some(&[("r", "Byte"), ("g", "Byte"), ("b", "Byte")][..])
        );
        assert_eq!(
            builtin_type_fields(TERM_SIZE_TYPE),
            Some(&[("columns", "Integer"), ("rows", "Integer")][..])
        );
        assert_eq!(builtin_type_fields("String"), None);
    }

    #[test]
    fn every_name_has_consistent_metadata() {
        for name in ALL {
            assert!(call_param_names(name).is_some(), "param_names {name}");
            assert!(param_types(name).is_some(), "param_types {name}");
            assert!(call_return_type_name(name).is_some(), "return_type {name}");
            assert!(resolve_call(name, &[]).is_some(), "resolve {name}");
            assert!(expected_arguments(name).is_some(), "expected {name}");
            assert!(arity(name).is_some(), "arity {name}");
            let (min, max) = arity(name).unwrap();
            assert_eq!(min, max, "term arities are fixed for {name}");
            assert_eq!(
                param_types(name).unwrap().len(),
                min,
                "arity vs types {name}"
            );
        }
    }

    #[test]
    fn metadata_returns_none_for_unknown() {
        assert_eq!(call_param_names("term.nope"), None);
        assert_eq!(param_types("term.nope"), None);
        assert_eq!(call_return_type_name("term.nope"), None);
        assert!(resolve_call("term.nope", &[]).is_none());
        assert_eq!(expected_arguments("term.nope"), None);
        assert_eq!(arity("term.nope"), None);
    }

    #[test]
    fn param_names_and_types_by_group() {
        for name in NO_ARG {
            assert_eq!(call_param_names(name), Some(&[][..]), "{name}");
            assert_eq!(param_types(name), Some(&[][..]), "{name}");
        }
        for name in [SET_FOREGROUND, SET_BACKGROUND] {
            assert_eq!(
                call_param_names(name),
                Some(&[&["r"][..], &["g"][..], &["b"][..]][..]),
                "{name}"
            );
            assert_eq!(
                param_types(name),
                Some(&["Byte", "Byte", "Byte"][..]),
                "{name}"
            );
        }
        for name in [SET_BOLD, SET_UNDERLINE] {
            assert_eq!(
                call_param_names(name),
                Some(&[&["enabled"][..]][..]),
                "{name}"
            );
            assert_eq!(param_types(name), Some(&["Boolean"][..]), "{name}");
        }
        assert_eq!(
            call_param_names(MOVE_TO),
            Some(&[&["row"][..], &["column"][..]][..])
        );
        assert_eq!(param_types(MOVE_TO), Some(&["Integer", "Integer"][..]));
        assert_eq!(
            call_param_names(DRAW_HLINE),
            Some(&[&["line"][..], &["row"][..], &["colA"][..], &["colB"][..]][..])
        );
        assert_eq!(
            call_param_names(DRAW_VLINE),
            Some(&[&["line"][..], &["col"][..], &["rowA"][..], &["rowB"][..]][..])
        );
        for name in [DRAW_HLINE, DRAW_VLINE] {
            assert_eq!(
                param_types(name),
                Some(&["LineStyle", "Integer", "Integer", "Integer"][..]),
                "{name}"
            );
        }
        assert_eq!(
            call_param_names(DRAW_BOX),
            Some(&[&["line"][..], &["x1"][..], &["y1"][..], &["x2"][..], &["y2"][..]][..])
        );
        assert_eq!(
            param_types(DRAW_BOX),
            Some(&["LineStyle", "Integer", "Integer", "Integer", "Integer"][..])
        );
        assert_eq!(
            call_param_names(FILL_RECT),
            Some(&[&["fill"][..], &["x1"][..], &["y1"][..], &["x2"][..], &["y2"][..]][..])
        );
        assert_eq!(
            param_types(FILL_RECT),
            Some(&["FillStyle", "Integer", "Integer", "Integer", "Integer"][..])
        );
        assert_eq!(
            call_param_names(DRAW_TEXT),
            Some(&[&["x"][..], &["y"][..], &["text"][..]][..])
        );
        assert_eq!(
            param_types(DRAW_TEXT),
            Some(&["Integer", "Integer", "String"][..])
        );
        assert_eq!(
            call_param_names(DRAW_GLYPH),
            Some(&[&["x"][..], &["y"][..], &["codepoint"][..]][..])
        );
        assert_eq!(
            param_types(DRAW_GLYPH),
            Some(&["Integer", "Integer", "Integer"][..])
        );
    }

    #[test]
    fn return_types_by_group() {
        for name in [
            ON,
            OFF,
            SET_FOREGROUND,
            SET_BACKGROUND,
            SET_BOLD,
            SET_UNDERLINE,
            SHOW_CURSOR,
            HIDE_CURSOR,
            CLEAR,
            SYNC,
            MOVE_TO,
            DRAW_HLINE,
            DRAW_VLINE,
            DRAW_BOX,
            FILL_RECT,
            DRAW_TEXT,
            DRAW_GLYPH,
        ] {
            assert_eq!(call_return_type_name(name), Some("Nothing"), "{name}");
        }
        for name in [IS_ON, GET_BOLD, GET_UNDERLINE] {
            assert_eq!(call_return_type_name(name), Some("Boolean"), "{name}");
        }
        for name in [GET_FOREGROUND, GET_BACKGROUND] {
            assert_eq!(call_return_type_name(name), Some(TERM_COLOR_TYPE), "{name}");
        }
        assert_eq!(call_return_type_name(TERMINAL_SIZE), Some(TERM_SIZE_TYPE));
    }

    #[test]
    fn resolve_call_mirrors_return_type() {
        for name in ALL {
            let resolved = resolve_call(name, &[]).unwrap();
            assert_eq!(
                resolved.return_type.into_owned(),
                call_return_type_name(name).unwrap().to_string(),
                "{name}"
            );
        }
    }

    #[test]
    fn expected_arguments_formatting() {
        for name in NO_ARG {
            assert_eq!(
                expected_arguments(name).as_deref(),
                Some("no arguments"),
                "{name}"
            );
        }
        assert_eq!(
            expected_arguments(SET_FOREGROUND).as_deref(),
            Some("Byte, Byte, Byte")
        );
        assert_eq!(expected_arguments(SET_BOLD).as_deref(), Some("Boolean"));
        assert_eq!(
            expected_arguments(MOVE_TO).as_deref(),
            Some("Integer, Integer")
        );
        assert_eq!(
            expected_arguments(DRAW_HLINE).as_deref(),
            Some("LineStyle, Integer, Integer, Integer")
        );
        assert_eq!(
            expected_arguments(DRAW_VLINE).as_deref(),
            Some("LineStyle, Integer, Integer, Integer")
        );
        assert_eq!(
            expected_arguments(DRAW_BOX).as_deref(),
            Some("LineStyle, Integer, Integer, Integer, Integer")
        );
        assert_eq!(
            expected_arguments(FILL_RECT).as_deref(),
            Some("FillStyle, Integer, Integer, Integer, Integer")
        );
        assert_eq!(
            expected_arguments(DRAW_TEXT).as_deref(),
            Some("Integer, Integer, String")
        );
        assert_eq!(
            expected_arguments(DRAW_GLYPH).as_deref(),
            Some("Integer, Integer, Integer")
        );
    }

    #[test]
    fn arity_by_group() {
        for name in NO_ARG {
            assert_eq!(arity(name), Some((0, 0)), "{name}");
        }
        for name in [SET_FOREGROUND, SET_BACKGROUND] {
            assert_eq!(arity(name), Some((3, 3)), "{name}");
        }
        for name in [SET_BOLD, SET_UNDERLINE] {
            assert_eq!(arity(name), Some((1, 1)), "{name}");
        }
        assert_eq!(arity(MOVE_TO), Some((2, 2)));
        assert_eq!(arity(DRAW_HLINE), Some((4, 4)));
        assert_eq!(arity(DRAW_VLINE), Some((4, 4)));
        assert_eq!(arity(DRAW_BOX), Some((5, 5)));
        assert_eq!(arity(FILL_RECT), Some((5, 5)));
        assert_eq!(arity(DRAW_TEXT), Some((3, 3)));
        assert_eq!(arity(DRAW_GLYPH), Some((3, 3)));
    }

    // plan-72-W migration gate: prove `TERM` reproduces every legacy answer
    // (membership, arity, param names, return type, and all four builtin types +
    // record fields) for every `term.*` name + a non-member, and that the
    // descriptor's per-position types equal `param_types` (the descriptor's
    // zero-arg `argument_types` is `None`, but `term::param_types` uses `Some(&[])`
    // — the one convention divergence, asserted explicitly). `expected_arguments`
    // is bespoke (`"no arguments"`) and is NOT asserted against the descriptor.
    // Kept until plan-72-BB.
    #[test]
    fn parity_matches_descriptor() {
        use crate::builtins::descriptor::{parity, DefaultResolver, InjectionRule, REGISTRY};

        assert_eq!(TERM.functions.len(), ALL.len());

        let legacy = parity::LegacySet {
            is_call: &is_term_call,
            arity: &arity,
            param_names: &|name| {
                call_param_names(name).map(|rows| rows.iter().map(|row| row.to_vec()).collect())
            },
            return_type_name: &call_return_type_name,
            // Bespoke "no arguments" phrasing — not descriptor-derivable.
            expected_arguments: None,
            param_name_overloads: None,
            // `param_types` diverges from `argument_types` only on the zero-arg
            // convention; asserted separately below.
            argument_types: None,
            implementation_name: None,
            default_padding: None,
            builtin_type_fields: Some(&builtin_type_fields),
        };
        let mut probe = ALL.to_vec();
        probe.push("term.nope");
        parity::assert_parity(&TERM, &probe, &legacy, &[]);

        // Per-position argument types match `param_types` for every call; the only
        // divergence is the zero-arg convention (descriptor `None` vs `Some(&[])`).
        for &name in ALL {
            let types = param_types(name).expect("term param_types");
            let descriptor = DefaultResolver::argument_types(&TERM, name);
            if types.is_empty() {
                assert_eq!(descriptor, None, "zero-arg argument_types for {name}");
            } else {
                assert_eq!(
                    descriptor.as_deref(),
                    Some(types),
                    "argument_types == param_types for {name}"
                );
            }
        }

        // All four builtin types are present with the right kind; records keep
        // their fields, enums carry none.
        assert!(is_builtin_type(TERM_COLOR_TYPE));
        assert!(is_builtin_type(LINE_STYLE_TYPE));
        assert!(!is_builtin_type("String"));
        assert_eq!(builtin_type_fields(LINE_STYLE_TYPE), None);
        assert_eq!(
            builtin_type_fields(TERM_COLOR_TYPE),
            Some(&[("r", "Byte"), ("g", "Byte"), ("b", "Byte")][..])
        );

        // Source companion: `WhenImported`, loader parses.
        let source = TERM.source.expect("term has a source companion");
        assert_eq!(source.rule, InjectionRule::WhenImported);
        assert!((source.loader)().is_ok());

        // Registered and well-formed alongside every other package.
        assert!(REGISTRY.module("term").is_some());
        assert!(REGISTRY.function(TERMINAL_SIZE).is_some());
        assert_eq!(REGISTRY.duplicate_module_name(), None);
        assert_eq!(REGISTRY.duplicate_function_name(), None);
    }
}
