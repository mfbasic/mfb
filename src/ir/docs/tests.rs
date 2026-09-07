//! A `DOC` block whose header names something that is not there.
//!
//! `collect_project_docs` walks the file's DOC blocks and, for each, looks up
//! the item its header names — an overload for `DOC FUNC`, a type for
//! `DOC TYPE`/`UNION`/`ENUM`, a resource for `DOC RESOURCE`. Each lookup has a
//! `continue` for the miss, and none had ever been taken: every fixture in the
//! tree documents something that exists.
//!
//! **The `continue` is the whole contract.** Documentation is not a declaration:
//! a `DOC FUNC` for a function that was renamed, or one whose only overload was
//! made private, must leave the package's doc table without that entry and must
//! NOT stop the build. Both halves matter. Failing would make a stale comment a
//! compile error; persisting it would put a signature in the published `.mfp`
//! for a function importers cannot call — `mfb pkg doc` would list it and every
//! call to it would then fail to resolve.
//!
//! Each case is paired with the same DOC block over an item that IS exported, so
//! a collector that dropped everything would fail the positive half rather than
//! pass the negative one.

use super::collect_project_docs;
use crate::testutil::project_from_src;

/// The names `collect_project_docs` persisted, in order.
fn doc_names(source: &str) -> Vec<String> {
    collect_project_docs(&project_from_src(source))
        .decls
        .into_iter()
        .map(|decl| decl.name)
        .collect()
}

/// A `DOC FUNC` whose function is not there is dropped, not persisted.
#[test]
fn a_doc_header_naming_no_function_is_dropped() {
    let present = "\
DOC
  FUNC greet()
  DESC Say hello.
END DOC
EXPORT FUNC greet() AS String
  RETURN \"hi\"
END FUNC
SUB main
END SUB
";
    assert_eq!(
        doc_names(present),
        vec!["greet".to_string()],
        "the positive half: a DOC block over an exported function IS persisted, \
         so the assertion below is about the lookup and not about the collector \
         having stopped working"
    );

    let absent = "\
DOC
  FUNC greet()
  DESC Say hello.
END DOC
SUB main
END SUB
";
    assert!(
        doc_names(absent).is_empty(),
        "a DOC block for a function that is not there names nothing to publish; \
         persisting it would put a signature in the `.mfp` for a function no \
         importer can call"
    );
}

/// A `DOC FUNC` whose only overload is PRIVATE is dropped too.
///
/// The second gate on the same arm, and the one a rename does not cause: the
/// function exists, so the lookup succeeds, and it is the visibility check that
/// drops it. Only exported members are published.
#[test]
fn a_doc_header_naming_a_private_function_is_dropped() {
    let source = "\
DOC
  FUNC hidden()
  DESC Internal only.
END DOC
FUNC hidden() AS String
  RETURN \"x\"
END FUNC
SUB main
END SUB
";
    assert!(
        doc_names(source).is_empty(),
        "the function is there but not exported, so nothing outside the package \
         can call it and its documentation has no audience"
    );
}

/// The same, for a `DOC TYPE` and a `DOC RESOURCE`.
///
/// Separate arms in the same `match`, each with its own lookup and its own
/// `continue`, so a missing type and a missing resource are two claims rather
/// than one.
#[test]
fn a_doc_header_naming_no_type_or_resource_is_dropped() {
    let missing_type = "\
DOC
  TYPE Point
  DESC A point.
END DOC
SUB main
END SUB
";
    assert!(
        doc_names(missing_type).is_empty(),
        "a DOC TYPE for a type that is not declared publishes nothing"
    );

    let present_type = "\
DOC
  TYPE Point
  DESC A point.
END DOC
EXPORT TYPE Point
  x AS Integer
END TYPE
SUB main
END SUB
";
    assert_eq!(
        doc_names(present_type),
        vec!["Point".to_string()],
        "the positive half for the type arm"
    );

    let missing_resource = "\
DOC
  RESOURCE Handle
  DESC A handle.
END DOC
SUB main
END SUB
";
    assert!(
        doc_names(missing_resource).is_empty(),
        "a DOC RESOURCE for a resource that is not declared publishes nothing"
    );
}
