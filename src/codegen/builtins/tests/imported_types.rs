//! Types that arrive from a built `.mfp` package, not from source.
//!
//! A program that names a record, union or enum declared by an imported binary
//! package reaches `ir::shape` and `ir::verify` through a door no source program
//! opens: the type is declared nowhere in the AST, so both passes learn its
//! shape from the decoded package table instead. Every in-process caller passes
//! an empty table, so that door had never been opened in a unit test — and it is
//! the door a real `IMPORT` of a published package uses.
//!
//! What is checked here is that a program is held to the SAME rules whichever
//! side the type came from. An imported record's field must still be spelled
//! correctly, an imported union must still be matched exhaustively, and an
//! imported enum member must still exist. A checker that learned only a name and
//! not a shape would accept all three silently — and the program would then be
//! lowered against a layout it never verified.

use crate::ir::{ImportedTypeDef, ImportedTypeField, ImportedTypeKind, ImportedTypeVariant};
use crate::testutil::{check_src, check_src_with_imports};
use crate::types::ParameterType;

/// A package `shapes` exporting one record, one union and one enum.
fn imported() -> Vec<ImportedTypeDef> {
    vec![
        ImportedTypeDef {
            name: "shapes.Point".to_string(),
            kind: ImportedTypeKind::Record,
            fields: vec![
                ImportedTypeField {
                    name: "x".to_string(),
                    type_: ParameterType::Integer,
                },
                ImportedTypeField {
                    name: "y".to_string(),
                    type_: ParameterType::Integer,
                },
            ],
            variants: Vec::new(),
            members: Vec::new(),
        },
        ImportedTypeDef {
            name: "shapes.Shape".to_string(),
            kind: ImportedTypeKind::Union,
            fields: Vec::new(),
            variants: vec![
                ImportedTypeVariant {
                    name: "shapes.Circle".to_string(),
                    fields: vec![ImportedTypeField {
                        name: "radius".to_string(),
                        type_: ParameterType::Integer,
                    }],
                },
                ImportedTypeVariant {
                    name: "shapes.Square".to_string(),
                    fields: vec![ImportedTypeField {
                        name: "side".to_string(),
                        type_: ParameterType::Integer,
                    }],
                },
            ],
            members: Vec::new(),
        },
        ImportedTypeDef {
            name: "shapes.Corner".to_string(),
            kind: ImportedTypeKind::Enum,
            fields: Vec::new(),
            variants: Vec::new(),
            members: vec!["TopLeft".to_string(), "BottomRight".to_string()],
        },
    ]
}

/// The imported table is what makes the difference, and it is loaded.
///
/// The same program is checked twice — once with the table and once without —
/// so a table that was accepted but never consulted cannot pass: without it the
/// type is unknown and the checker must say so.
#[test]
fn an_imported_type_is_only_known_when_the_table_declares_it() {
    const SRC: &str = "\
FUNC main() AS Integer
  LET p AS shapes::Point = shapes::Point[x := 1, y := 2]
  RETURN p.x
END FUNC
";
    let with_table = check_src_with_imports(SRC, &imported());
    let without = check_src(SRC);
    assert!(
        !without.is_empty(),
        "without the package table `shapes::Point` is an unknown type and the \
         checker must reject the program; it accepted it"
    );
    assert!(
        with_table.len() < without.len(),
        "loading the package table must make the program MORE acceptable, not \
         less: with it the checker reported {with_table:?}, without it {without:?}"
    );
}

/// An imported record's fields are checked, not just its name.
///
/// The table carries `x` and `y`. A checker that recorded only the name would
/// accept `p.z` and hand lowering a field offset it invented.
///
/// The DIAGNOSTIC differs from the source-declared case, and that difference is
/// measured here rather than assumed: a source record reports
/// `TYPE_UNKNOWN_FIELD`, naming the field, while the imported one reports
/// `TYPE_UNKNOWN_VALUE`, naming only the expression. Both reject, which is what
/// matters for correctness; the imported message is the less useful of the two,
/// and pinning both makes that visible instead of folklore.
#[test]
fn an_imported_records_fields_are_checked() {
    const IMPORTED: &str = "\
FUNC main() AS Integer
  LET p AS shapes::Point = shapes::Point[x := 1, y := 2]
  RETURN p.z
END FUNC
";
    const DECLARED: &str = "\
TYPE Point
  x AS Integer
  y AS Integer
END TYPE

FUNC main() AS Integer
  LET p AS Point = Point[x := 1, y := 2]
  RETURN p.z
END FUNC
";
    let from_package = check_src_with_imports(IMPORTED, &imported());
    assert!(
        !from_package.is_empty(),
        "reading `p.z` off an imported record whose table declares only x and y \
         must be rejected; the checker accepted it"
    );
    assert!(
        from_package.contains(&"TYPE_UNKNOWN_VALUE".to_string()),
        "the imported path rejects with TYPE_UNKNOWN_VALUE today; it said \
         {from_package:?}"
    );
    let from_source = check_src(DECLARED);
    assert!(
        from_source.contains(&"TYPE_UNKNOWN_FIELD".to_string()),
        "the same mistake on a SOURCE-declared record names the field; it said \
         {from_source:?}"
    );
}

/// An imported enum's members are checked, not just its name.
///
/// As above, the imported path rejects with the less specific
/// `TYPE_UNKNOWN_VALUE` where a source-declared enum names the member. Both are
/// pinned so the pair can be compared at a glance.
#[test]
fn an_imported_enums_members_are_checked() {
    const IMPORTED: &str = "\
FUNC main() AS Integer
  LET c AS shapes::Corner = shapes::Corner.Sideways
  RETURN 0
END FUNC
";
    const DECLARED: &str = "\
ENUM Corner
  TopLeft
  BottomRight
END ENUM

FUNC main() AS Integer
  LET c AS Corner = Corner.Sideways
  RETURN 0
END FUNC
";
    let from_package = check_src_with_imports(IMPORTED, &imported());
    assert!(
        !from_package.is_empty(),
        "naming a member the imported table does not declare must be rejected; \
         the checker accepted it"
    );
    let from_source = check_src(DECLARED);
    assert!(
        from_source.contains(&"TYPE_UNKNOWN_ENUM_MEMBER".to_string()),
        "the same mistake on a SOURCE-declared enum names the member; it said \
         {from_source:?}"
    );
}

/// An imported union is matched exhaustively, on its own variant list.
///
/// Exhaustiveness is decided from the table's variants. A checker that saw the
/// union as opaque would accept a MATCH that handles one of two variants, and
/// the missing case would be a trap at run time on a value the caller has every
/// right to pass.
#[test]
fn an_imported_union_must_still_be_matched_exhaustively() {
    const PARTIAL: &str = "\
FUNC area(s AS shapes::Shape) AS Integer
  MATCH s
    CASE shapes::Circle(c)
      RETURN c.radius
  END MATCH
END FUNC

FUNC main() AS Integer
  RETURN 0
END FUNC
";
    let rules = check_src_with_imports(PARTIAL, &imported());
    assert!(
        rules.iter().any(|rule| rule == "TYPE_MATCH_NOT_EXHAUSTIVE"),
        "a MATCH over an imported two-variant union that handles one variant \
         must be TYPE_MATCH_NOT_EXHAUSTIVE; the checker said {rules:?}"
    );
}
