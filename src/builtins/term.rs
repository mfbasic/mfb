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

use crate::codegen::registry::{
    BuiltinFlags, BuiltinFunction, BuiltinModule, BuiltinOverload, BuiltinResolver, BuiltinType,
    DefaultResolver, Implementation, Lowering, Parameter, ReturnType, TypeKind,
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
pub(crate) const DID_RESIZE: &str = "term.didResize";

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
        doc_intro: "",
        doc_desc: "",
        errors: &[],
        overloads,
        doc_example: "",
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
    term_fn(DID_RESIZE, "didResize", OV_BOOL_EMPTY),
];

/// Return-type resolution for the term calls, delegating to the hand-authored
/// `resolve_call`. Exposed through the descriptor so plan-72-BB can drive `term::`
/// return types from the registry.
struct TermResolver;
impl BuiltinResolver for TermResolver {
    fn resolve_return_type(
        &self,
        _module: &BuiltinModule,
        name: &str,
        arg_types: &[String],
    ) -> Option<String> {
        resolve_call(name, arg_types).map(|resolved| resolved.return_type.into_owned())
    }
}
static TERM_RESOLVER: TermResolver = TermResolver;

pub(crate) static TERM: BuiltinModule = BuiltinModule {
    name: "term",
    doc_intro: "",
    doc_desc: "",
    functions: TERM_FUNCTIONS,
    types: TERM_TYPES,
    source: Some(crate::codegen::registry::BuiltinSource {
        rule: crate::codegen::registry::InjectionRule::WhenImported,
        loader: source_file,
    }),
    resolver: Some(&TERM_RESOLVER),
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

pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        ON | OFF | IS_ON | SHOW_CURSOR | HIDE_CURSOR | CLEAR | SYNC | GET_FOREGROUND
        | GET_BACKGROUND | GET_BOLD | GET_UNDERLINE | TERMINAL_SIZE | DID_RESIZE => Some(&[]),
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
        | GET_BACKGROUND | GET_BOLD | GET_UNDERLINE | TERMINAL_SIZE | DID_RESIZE => Some(&[]),
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

// The `term`↔`astrings` bridge source (`term_astrings_bridge.mfb`) carries the
// `term::drawText(x, y, AttributedString)` overload body `__term_drawTextAttr`
// (routed to `#term_drawTextAttr` in `ir::lower`). It is a SEPARATE source from
// `term_package.mfb` and gated on importing BOTH `term` and `astrings`, so a plain
// `IMPORT term` program never drags in the `astrings`/`strings` companions the
// bridge needs. The bridge itself imports term/astrings/strings, so it is injected
// before all three (their `uses_package` then sees the dependency).
pub(crate) fn bridge_source_file() -> Result<crate::ast::AstFile, ()> {
    crate::ast::parse_source_internal(
        std::path::Path::new("<builtin-term-astrings-bridge>"),
        "builtins/term_astrings_bridge.mfb",
        include_str!("term_astrings_bridge.mfb"),
    )
}

/// The bridge is used when a program imports both `term` and `astrings` — the pair
/// any `term::drawText(AttributedString)` call must have in scope. Over-injection
/// (a program that imports both but never draws attributed text) is harmless: it
/// adds only the two small resolver helpers plus `__term_drawTextAttr`.
pub(crate) fn bridge_uses_package(ast: &crate::ast::AstProject) -> bool {
    let imports = |name: &str| {
        ast.files.iter().any(|file| {
            file.imports
                .iter()
                .any(|import| import.package_name() == name)
        })
    };
    imports("term") && imports("astrings")
}

pub(crate) fn bridge_augmented_project(
    ast: &crate::ast::AstProject,
) -> Result<crate::ast::AstProject, ()> {
    if !bridge_uses_package(ast) {
        return Ok(ast.clone());
    }
    let mut augmented = ast.clone();
    augmented.files.push(bridge_source_file()?);
    Ok(augmented)
}

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
        DID_RESIZE,
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
        DID_RESIZE,
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
            Some(
                &[
                    &["line"][..],
                    &["x1"][..],
                    &["y1"][..],
                    &["x2"][..],
                    &["y2"][..]
                ][..]
            )
        );
        assert_eq!(
            param_types(DRAW_BOX),
            Some(&["LineStyle", "Integer", "Integer", "Integer", "Integer"][..])
        );
        assert_eq!(
            call_param_names(FILL_RECT),
            Some(
                &[
                    &["fill"][..],
                    &["x1"][..],
                    &["y1"][..],
                    &["x2"][..],
                    &["y2"][..]
                ][..]
            )
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
        for name in [IS_ON, GET_BOLD, GET_UNDERLINE, DID_RESIZE] {
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
    fn descriptor_constructors_execute_at_runtime() {
        // `ov` and `term_fn` are const fns invoked only in const context
        // (`OV_*` / `TERM_FUNCTIONS`), so their bodies never run at runtime and
        // show as uncovered. Call them at runtime to exercise (and pin the shape
        // of) both constructors.
        let overloads = ov(P_RGB, "Nothing");
        assert_eq!(overloads.len(), 1);
        assert_eq!(overloads[0].return_type, ReturnType::Fixed("Nothing"));
        assert_eq!(overloads[0].params.len(), 3);
        assert_eq!(overloads[0].params[0].name, "r");

        let niladic = ov(P_EMPTY, "Boolean");
        assert!(niladic[0].params.is_empty());
        assert_eq!(niladic[0].return_type, ReturnType::Fixed("Boolean"));

        let func = term_fn(ON, "on", OV_NOTHING_EMPTY);
        assert_eq!(func.name, ON);
        assert_eq!(func.doc_slug, "on");
        assert_eq!(func.overloads.len(), 1);
        // `term::` calls lower directly by name to the native backend — no rewrite.
        assert_eq!(func.implementation, Implementation::Same);
        assert_eq!(func.lowering, Lowering::Helper);
        assert!(!func.flags.internal_only);
        assert!(!func.flags.return_type_overloaded);
    }

    #[test]
    fn resolve_call_rejects_unknown_name() {
        // The `?` on `call_return_type_name` short-circuits to `None` for a name
        // that is not a `term::` call.
        assert!(resolve_call("term.unknown", &[]).is_none());
        assert!(resolve_call("strings.trim", &[]).is_none());
    }
}
