use super::*;

#[test]
fn doc_table_round_trips() {
    let docs = PackageDocs {
        package: Some(PackageDocEntry {
            name: "mathx".to_string(),
            desc: vec![
                (0, "First paragraph.".to_string()),
                (0, "Second.".to_string()),
            ],
            deprecated: Some(String::new()),
        }),
        decls: vec![
            DeclDocEntry {
                kind: "func".to_string(),
                name: "addUp".to_string(),
                signature: "EXPORT FUNC addUp(a AS Integer, b AS Integer) AS Integer".to_string(),
                group: "Math".to_string(),
                desc: vec![(0, "Adds.".to_string()), (1, "Overflows.".to_string())],
                args: vec![
                    ("a".to_string(), "first".to_string()),
                    ("b".to_string(), "second".to_string()),
                ],
                props: vec![],
                ret: "the sum".to_string(),
                errors: vec![("5001".to_string(), "overflow".to_string())],
                example: "LET x AS Integer = addUp(1, 2)".to_string(),
                internal: false,
                deprecated: None,
            },
            DeclDocEntry {
                kind: "type".to_string(),
                name: "Point".to_string(),
                signature: "EXPORT TYPE Point".to_string(),
                group: String::new(),
                desc: vec![],
                args: vec![],
                props: vec![("x".to_string(), "the x".to_string())],
                ret: String::new(),
                errors: vec![],
                example: String::new(),
                internal: true,
                deprecated: Some("use Coord".to_string()),
            },
        ],
    };

    let bytes = encode_doc_table(&docs);
    let decoded = read_doc_table(&bytes).expect("doc table decodes");

    let package = decoded.package.expect("package entry");
    assert_eq!(package.name, "mathx");
    assert_eq!(package.desc, docs.package.as_ref().unwrap().desc);
    assert_eq!(package.deprecated, Some(String::new()));

    assert_eq!(decoded.decls.len(), 2);
    let add = &decoded.decls[0];
    assert_eq!(add.kind, "func");
    assert_eq!(add.name, "addUp");
    assert_eq!(add.group, "Math");
    assert_eq!(
        add.desc,
        vec![(0, "Adds.".to_string()), (1, "Overflows.".to_string())]
    );
    assert_eq!(add.args, docs.decls[0].args);
    assert_eq!(add.errors, docs.decls[0].errors);
    assert_eq!(add.ret, "the sum");
    assert!(!add.internal);
    assert_eq!(add.deprecated, None);

    let point = &decoded.decls[1];
    assert_eq!(point.kind, "type");
    assert_eq!(point.props, docs.decls[1].props);
    assert!(point.internal);
    assert_eq!(point.deprecated, Some("use Coord".to_string()));
}
