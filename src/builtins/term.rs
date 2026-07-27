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

#[derive(Clone)]
pub(crate) struct ResolvedCall<'a> {
    pub(crate) return_type: Cow<'a, str>,
}

pub(crate) fn is_term_call(name: &str) -> bool {
    matches!(
        name,
        ON | OFF
            | IS_ON
            | SET_FOREGROUND
            | SET_BACKGROUND
            | SET_BOLD
            | SET_UNDERLINE
            | SHOW_CURSOR
            | HIDE_CURSOR
            | CLEAR
            | SYNC
            | MOVE_TO
            | DRAW_HLINE
            | DRAW_VLINE
            | DRAW_BOX
            | FILL_RECT
            | DRAW_TEXT
            | DRAW_GLYPH
            | GET_FOREGROUND
            | GET_BACKGROUND
            | GET_BOLD
            | GET_UNDERLINE
            | TERMINAL_SIZE
    )
}

pub(crate) fn is_builtin_type(name: &str) -> bool {
    name == TERM_COLOR_TYPE
        || name == TERM_SIZE_TYPE
        || name == LINE_STYLE_TYPE
        || name == FILL_STYLE_TYPE
}

pub(crate) fn builtin_type_fields(name: &str) -> Option<&'static [(&'static str, &'static str)]> {
    match name {
        TERM_COLOR_TYPE => Some(&[("r", "Byte"), ("g", "Byte"), ("b", "Byte")]),
        TERM_SIZE_TYPE => Some(&[("columns", "Integer"), ("rows", "Integer")]),
        _ => None,
    }
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
    match name {
        ON | OFF | SET_FOREGROUND | SET_BACKGROUND | SET_BOLD | SET_UNDERLINE | SHOW_CURSOR
        | HIDE_CURSOR | CLEAR | SYNC | MOVE_TO | DRAW_HLINE | DRAW_VLINE | DRAW_BOX
        | FILL_RECT | DRAW_TEXT | DRAW_GLYPH => Some("Nothing"),
        IS_ON | GET_BOLD | GET_UNDERLINE => Some("Boolean"),
        GET_FOREGROUND | GET_BACKGROUND => Some(TERM_COLOR_TYPE),
        TERMINAL_SIZE => Some(TERM_SIZE_TYPE),
        _ => None,
    }
}

/// `arg_types` is accepted for signature parity with every other package's
/// `resolve_call` (so the `mod.rs` dispatch is uniform — bug-340 A7) but is
/// unused: a `term::` call's return type is a function of the name alone.
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
    let count = param_types(name)?.len();
    Some((count, count))
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
}
