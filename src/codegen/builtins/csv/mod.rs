//! The built-in `csv` package — migrated onto the clean-room registry
//! (`crate::codegen::registry`). csv registers itself here: the package, its imports,
//! the two `EXPORT` records, the shared `__csv_*` helpers (`package.mfb`), and its
//! four members, each owning its `FUNC __csv_* … END FUNC` body in its `func_*.rs`
//! (`Body::mfb`, carrying the rewrite target — `readRow` → `__csv_next`, irregular).
//! [`RegistryPackage::get_mfb`] reassembles the injectable source; the frontend and
//! IR lowering reach csv through the *generic* clean-room dispatch (the
//! `get_package_by_func_name` dual-path in `builtins::` / `ir::lower`), so this module
//! is pure registration — no per-package `is_csv_call`/`implementation_name` seams.

use crate::codegen::registry::{RecordProp, Registry, RegistryPackage, RegistryRecord};

mod func_parse;
mod func_parse_stream;
mod func_read_row;
mod func_stringify;

/// RFC-4180 dialect defaults, injected as raw String literals when an optional
/// dialect argument is omitted (the `expr` of a `Fill` is the literal value).
pub(super) const DEFAULT_DELIMITER: &str = ",";
pub(super) const DEFAULT_QUOTE: &str = "\"";
pub(super) const DEFAULT_NEWLINE: &str = "\n";

/// Register the `csv` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new(
        "csv",
        "Parse and serialize CSV text as a grid of String cells",
        "The `csv` package converts between CSV text and a grid of rows of String \
         cells: `csv::parse` / `csv::stringify` for the whole document, and \
         `csv::parseStream` / `csv::readRow` for row-by-row streaming.",
    );
    pkg.add_imports(vec!["collections", "strings", "encoding"]);

    // plan-77 C4: the streaming reader/row records (EXPORT so callers can name them).
    pkg.add_record(RegistryRecord {
        name: "CsvReader",
        export: true,
        props: vec![
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
    });
    pkg.add_record(RegistryRecord {
        name: "CsvRow",
        export: true,
        props: vec![
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
    });

    // The shared private `__csv_*` helpers the member bodies call.
    pkg.add_helper_functions(vec![include_str!("package.mfb")]);

    func_parse::add(&mut pkg);
    func_stringify::add(&mut pkg);
    func_parse_stream::add(&mut pkg);
    func_read_row::add(&mut pkg);

    r.add_package(pkg);
}

#[cfg(test)]
mod tests {
    use crate::codegen::registry::{self, registry};

    #[test]
    fn csv_registered_on_the_clean_room_registry() {
        let pkg = registry().get_package("csv").expect("csv package");
        assert_eq!(pkg.functions().len(), 4);
        // The two EXPORT records are visible to the generic type query.
        assert!(registry::is_builtin_type("CsvReader"));
        assert!(registry::is_builtin_type("CsvRow"));
    }

    #[test]
    fn generic_dispatch_reaches_csv() {
        assert!(registry::is_member("csv.parse"));
        assert!(!registry::is_member("csv.nope"));
        assert_eq!(
            registry::call_return_type("csv.parse"),
            Some("List OF List OF String")
        );
        assert_eq!(registry::rewrite_target("csv.parse"), Some("__csv_parse"));
        // The irregular pairing the public name cannot derive.
        assert_eq!(registry::rewrite_target("csv.readRow"), Some("__csv_next"));
        assert_eq!(registry::arity("csv.parse"), Some((1, 3)));
        assert_eq!(registry::arity("csv.readRow"), Some((1, 1)));
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
