//! The built-in `csv` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`). csv registers itself here: the package, its imports,
//! the two `EXPORT` records, the shared `__csv_*` helpers (`package.mfb`), and its
//! four members, each owning its `FUNC __csv_* … END FUNC` body in its `func_*.rs`
//! (`Body::mfb`, carrying the rewrite target — `readRow` → `__csv_next`, irregular).
//! [`RegistryPackage::get_mfb`] reassembles the injectable source; the pipeline
//! reaches csv's return type / rewrite / arg validation through the seams below,
//! which read the clean-room descriptor rather than a static `BuiltinModule`.

use crate::codegen::registry::{
    registry, Body, DefaultValue, Implementation, Lowering, Parameter, RecordProp, Registry,
    RegistryFunction, RegistryPackage,
};

mod func_parse;
mod func_parse_stream;
mod func_read_row;
mod func_stringify;

/// RFC-4180 dialect defaults, injected as raw String literals when an optional
/// dialect argument is omitted (the `expr` of a `Fill` is the literal value).
pub(super) const DEFAULT_DELIMITER: &str = ",";
pub(super) const DEFAULT_QUOTE: &str = "\"";
pub(super) const DEFAULT_NEWLINE: &str = "\n";

/// A required parameter (with optional keyword aliases).
pub(super) fn required(
    name: &'static str,
    aliases: &'static [&'static str],
    ty: &'static str,
) -> Parameter {
    Parameter {
        name,
        aliases,
        ty,
        default: DefaultValue::None,
    }
}

/// An optional String parameter, default-padded with `expr` when omitted.
pub(super) fn opt(name: &'static str, expr: &'static str) -> Parameter {
    Parameter {
        name,
        aliases: &[],
        ty: "String",
        default: DefaultValue::Fill {
            type_name: "String",
            expr,
        },
    }
}

/// A source-backed member: its `FUNC` body plus the internal symbol a call rewrites
/// to (the `FUNC` the body declares).
pub(super) fn mfb_impl(
    params: Vec<Parameter>,
    return_type: &'static str,
    body: &'static str,
    rewrite: &'static str,
) -> Implementation {
    Implementation {
        params,
        return_type,
        errors: vec![],
        lowering: Lowering::Helper,
        body: Body::mfb(body, rewrite),
    }
}

/// Register the `csv` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let pkg = r.add_package(
        "csv",
        "Parse and serialize CSV text as a grid of String cells",
        "The `csv` package converts between CSV text and a grid of rows of String \
         cells: `csv::parse` / `csv::stringify` for the whole document, and \
         `csv::parseStream` / `csv::readRow` for row-by-row streaming.",
    );
    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    // plan-77 C4: the streaming reader/row records (EXPORT so callers can name them).
    pkg.add_record(
        "CsvReader",
        true,
        vec![
            RecordProp {
                name: "chars",
                ty: "List OF Integer",
                description: "The decoded input as Unicode scalars.",
            },
            RecordProp {
                name: "count",
                ty: "Integer",
                description: "The scalar count.",
            },
            RecordProp {
                name: "index",
                ty: "Integer",
                description: "The scan cursor.",
            },
            RecordProp {
                name: "delimCode",
                ty: "Integer",
                description: "The field-delimiter scalar.",
            },
            RecordProp {
                name: "quoteCode",
                ty: "Integer",
                description: "The quote-character scalar.",
            },
        ],
    );
    pkg.add_record(
        "CsvRow",
        true,
        vec![
            RecordProp {
                name: "fields",
                ty: "List OF String",
                description: "The record's cells.",
            },
            RecordProp {
                name: "reader",
                ty: "CsvReader",
                description: "The reader advanced past this record.",
            },
            RecordProp {
                name: "done",
                ty: "Boolean",
                description: "TRUE when the reader was already at end of input.",
            },
        ],
    );

    // The shared private `__csv_*` helpers the member bodies call.
    pkg.add_helper_functions(vec![include_str!("package.mfb")]);

    func_parse::add(pkg);
    func_stringify::add(pkg);
    func_parse_stream::add(pkg);
    func_read_row::add(pkg);
}

// ---------------------------------------------------------------------------
// Pipeline seams — the frontend/IR-lowering hooks csv is reached through. Each
// reads the clean-room descriptor (the package is the single source of truth).
// ---------------------------------------------------------------------------

/// The clean-room `RegistryFunction` a `csv.<member>` call names, or `None`.
fn csv_function(name: &str) -> Option<&'static RegistryFunction> {
    let member = name.strip_prefix("csv.")?;
    registry().get_package("csv")?.function(member)
}

/// Whether `name` is a `csv.<member>` call.
pub(crate) fn is_csv_call(name: &str) -> bool {
    csv_function(name).is_some()
}

/// The internal `__csv_*` symbol a `csv.<member>` call rewrites to at IR lowering.
pub(crate) fn implementation_name(name: &str) -> Option<&'static str> {
    csv_function(name)?
        .implementations()
        .first()?
        .body
        .rewrite_target()
}

/// The primary expected argument type of a `csv.<member>` call (its first param).
pub(crate) fn expected_arguments(name: &str) -> Option<&'static str> {
    Some(
        csv_function(name)?
            .implementations()
            .first()?
            .params
            .first()?
            .ty,
    )
}

/// Whether `name` is one of csv's `EXPORT` record types (`CsvReader`/`CsvRow`).
pub(crate) fn is_builtin_type(name: &str) -> bool {
    registry()
        .get_package("csv")
        .is_some_and(|pkg| pkg.records().iter().any(|record| record.name() == name))
}

/// The per-position `[name, alias…]` lists for keyword-argument matching. Kept as a
/// static table: this is the one seam whose return is a `&'static [&'static […]]`
/// shape the runtime-built descriptor cannot borrow without allocating.
pub(crate) fn call_param_names(name: &str) -> Option<&'static [&'static [&'static str]]> {
    match name {
        "csv.parse" => Some(&[&["value", "text"], &["delimiter"], &["quote"]]),
        "csv.stringify" => Some(&[&["value"], &["delimiter"], &["quote"], &["newline"]]),
        "csv.parseStream" => Some(&[&["value", "text"], &["delimiter"], &["quote"]]),
        "csv.readRow" => Some(&[&["reader"]]),
        _ => None,
    }
}

/// The `(type, expr)` constants to append after `provided` real arguments so the
/// injected `__csv_*` body always receives its full arity. Kept as a static table
/// for the same `&'static`-slice reason as [`call_param_names`]; the exprs mirror the
/// members' `Fill` defaults.
pub(crate) fn default_argument_padding(
    name: &str,
    provided: usize,
) -> &'static [(&'static str, &'static str)] {
    const PARSE_DEFAULTS: &[(&str, &str)] =
        &[("String", DEFAULT_DELIMITER), ("String", DEFAULT_QUOTE)];
    const STRINGIFY_DEFAULTS: &[(&str, &str)] = &[
        ("String", DEFAULT_DELIMITER),
        ("String", DEFAULT_QUOTE),
        ("String", DEFAULT_NEWLINE),
    ];
    match name {
        "csv.parse" | "csv.parseStream" => {
            &PARSE_DEFAULTS[provided.saturating_sub(1).min(PARSE_DEFAULTS.len())..]
        }
        "csv.stringify" => {
            &STRINGIFY_DEFAULTS[provided.saturating_sub(1).min(STRINGIFY_DEFAULTS.len())..]
        }
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_registered_on_the_clean_room_registry() {
        let pkg = registry().get_package("csv").expect("csv package");
        assert_eq!(pkg.functions().len(), 4);
        assert!(is_builtin_type("CsvReader"));
        assert!(is_builtin_type("CsvRow"));
        assert!(!is_builtin_type("Nope"));
    }

    #[test]
    fn seams_read_the_clean_room_descriptor() {
        assert!(is_csv_call("csv.parse"));
        assert!(!is_csv_call("csv.nope"));
        assert_eq!(implementation_name("csv.parse"), Some("__csv_parse"));
        assert_eq!(implementation_name("csv.readRow"), Some("__csv_next"));
        assert_eq!(expected_arguments("csv.parse"), Some("String"));
        assert_eq!(
            expected_arguments("csv.stringify"),
            Some("List OF List OF String")
        );
        assert_eq!(expected_arguments("csv.readRow"), Some("CsvReader"));
    }

    #[test]
    fn reassembled_source_parses() {
        let source = registry().get_package("csv").expect("csv").get_mfb();
        crate::ast::parse_source_internal(
            std::path::Path::new("<builtin-csv>"),
            "builtins/csv.mfb",
            &source,
        )
        .expect("reassembled csv source parses");
    }
}
