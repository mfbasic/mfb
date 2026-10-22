// ---------------------------------------------------------------------------
// builder.rs — an importer's view of every literal parameter default a
// package compiled from source records (plan-136-B), kind by kind.
// ---------------------------------------------------------------------------

use super::*;

/// Build a package whose one export takes a parameter of each scalar type,
/// each with a literal default, and answer it decoded.
fn defaults_package() -> PackageBinaryRepr {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("project.json"),
        "{\"name\": \"defs\", \"version\": \"1.0.0\", \"mfb\": \"1.0\", \"kind\": \"package\", \
         \"description\": \"defaults\", \"sources\": [{ \"root\": \"src\", \"role\": \"lib\", \
         \"include\": [\"**/*.mfb\"] }]}",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/lib.mfb"),
        "EXPORT FUNC f(b AS Boolean = TRUE, i AS Integer = 3, x AS Float = 1.5, \
         d AS Fixed = 2.25F, s AS String = \"hi\", y AS Byte = 7, m AS Money = 1.5m, \
         c AS Scalar = `z`) AS Integer\n  RETURN i\nEND FUNC\n",
    )
    .unwrap();
    let options =
        crate::cli::build::parse_build_options(vec![dir.path().to_str().unwrap().to_string()])
            .unwrap();
    crate::cli::build::build_project(&options).expect("the package builds");
    read_package_binary_repr(&dir.path().join("defs.mfp")).expect("decodes")
}

/// The package's one export record.
fn defaults_export() -> BinaryReprExport {
    let mut exports = package_exports(&defaults_package()).expect("exports");
    assert_eq!(exports.len(), 1);
    exports.remove(0)
}

#[test]
fn every_literal_default_kind_decodes_to_its_type_and_spelling() {
    let export = defaults_export();
    let defaults: Vec<(String, String)> = export
        .params
        .iter()
        .map(|param| match &param.default {
            BinaryReprExportDefault::Literal { type_, value } => {
                (type_.name().into_owned(), value.clone())
            }
            other => panic!("{} has no literal default: {other:?}", param.name),
        })
        .collect();
    assert_eq!(
        defaults,
        [
            ("Boolean", "true"),
            ("Integer", "3"),
            ("Float", "1.5"),
            ("Fixed", "2.25"),
            ("String", "hi"),
            ("Byte", "7"),
            ("Money", "1.5"),
            ("Scalar", "122"),
        ]
        .map(|(type_, value)| (type_.to_string(), value.to_string()))
    );
}

/// A package whose records point past their tables, as a corrupted or
/// hand-edited `.mfp` would, is refused with what it points at.
#[test]
fn a_default_or_type_that_points_past_its_table_is_refused() {
    let exported = |package: &PackageBinaryRepr| package.exports[0].function_id as usize;
    let refusal = |edit: &dyn Fn(&mut PackageBinaryRepr)| {
        let mut package = defaults_package();
        edit(&mut package);
        package_exports(&package).map(|_| ()).unwrap_err()
    };
    assert_eq!(
        refusal(&|package| {
            let at = exported(package);
            package.project.functions[at].params[1].default_const = 9999;
        }),
        "parameter default references missing constant 9999"
    );
    assert_eq!(
        refusal(&|package| {
            let at = exported(package);
            let param = &mut package.project.functions[at].params[1];
            param.flags |= PARAM_FLAG_DEFAULT_FUNCTION;
            param.default_const = 9999;
        }),
        "parameter default references missing function 9999"
    );
    assert_eq!(
        refusal(&|package| {
            let at = exported(package);
            let constant = package.project.functions[at].params[1].default_const as usize;
            package.project.constants.entries[constant].kind = 99;
        }),
        "parameter default references a constant of unknown kind 99"
    );
    assert!(refusal(&|package| {
        let at = exported(package);
        package.project.functions[at].params[1].type_id = 99_999;
    })
    .contains("99999"));
    assert!(refusal(&|package| {
        let at = exported(package);
        package.project.functions[at].return_type = 99_999;
    })
    .contains("99999"));
}
