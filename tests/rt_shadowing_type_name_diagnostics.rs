//! A declared type may shadow a built-in type spelling — `TYPE Integer` is
//! legal and compiles — and every name-keyed type table therefore MERGED the
//! two: `records["Integer"]` was the record, and a field annotated
//! `AS Integer` matched it by name equality.
//!
//! plan-111 keys those tables by `ParameterType` instead, and that is exactly
//! where a merge can turn into a split. `ParameterType::named("Integer")` mints
//! a `Named` nominal, while an `AS Integer` annotation elaborates to the
//! `Integer` *variant* — so a table keyed with `named` no longer answers a
//! lookup the string table answered, silently. Measured against a pre-plan-111
//! binary, that dropped `TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION` from
//!
//! ```text
//! TYPE Integer
//!   a AS Integer
//! END TYPE
//! ```
//!
//! `ParameterType::declared` (`src/types.rs`) is the fix: it parses, so a
//! declaration keys as the type its own annotations denote. For every name that
//! is NOT a built-in spelling the two constructors agree, which is why the
//! divergence hides from the whole fixture corpus — no fixture shadows a
//! built-in name.
//!
//! These cases are the shapes that reach a re-keyed table by a different route:
//! the record-cycle walk, the constructor arity/field checks, comparability,
//! and the union and enum tables.

mod common;
use common::{mfb_exe, temp_project};
use std::process::Command;

/// Build `source` and return its diagnostic header lines, sorted. The build is
/// expected to FAIL for the rejecting cases, so the status is not asserted —
/// the diagnostics are the observation.
fn diagnostics(name: &str, source: &str) -> Vec<String> {
    let project = temp_project(name, source);
    let output = Command::new(mfb_exe())
        .arg("build")
        .arg("-q")
        .arg(&project)
        .output()
        .expect("run mfb build");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut codes: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let open = line.find("error[")?;
            let close = line[open..].find(']')? + open;
            Some(line[open + 6..close].to_string())
        })
        .collect();
    codes.sort();
    codes.dedup();
    codes
}

#[test]
fn a_record_shadowing_a_builtin_name_still_reports_its_self_cycle() {
    // The record's own field names the record, which the name-keyed tables saw
    // as a self-cycle. It must stay one.
    assert!(
        diagnostics(
            "shadow_cycle",
            "TYPE Integer\n  a AS Integer\nEND TYPE\n\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .iter()
        .any(|code| code.contains("TYPE_RECURSIVE_RECORD_REQUIRES_INDIRECTION")),
        "a record shadowing `Integer` whose field is `AS Integer` is a self-cycle"
    );
}

#[test]
fn a_record_shadowing_a_builtin_name_still_arity_checks_its_constructor() {
    // Reaches `record_field_lists` through the constructor path, which has no
    // primitive-name short circuit ahead of it.
    assert!(
        diagnostics(
            "shadow_arity",
            "TYPE String\n  a AS Integer\nEND TYPE\n\nFUNC main() AS Integer\n  LET v = String[1, 2]\n  RETURN 0\nEND FUNC\n",
        )
        .iter()
        .any(|code| code.contains("TYPE_CONSTRUCTOR_ARITY_MISMATCH")),
        "the constructor of a record shadowing `String` is still arity-checked"
    );
}

#[test]
fn a_record_shadowing_a_builtin_name_accepts_a_correct_constructor() {
    // The positive direction: a SPLIT key shows up as a spurious rejection.
    // A correct-arity constructor must produce no arity or record diagnostic.
    let codes = diagnostics(
        "shadow_ctor_ok",
        "TYPE Boolean\n  a AS Integer\nEND TYPE\n\nFUNC main() AS Integer\n  LET v = Boolean[7]\n  RETURN 0\nEND FUNC\n",
    );
    assert!(
        !codes
            .iter()
            .any(|code| code.contains("TYPE_CONSTRUCTOR_ARITY_MISMATCH")
                || code.contains("TYPE_CONSTRUCTOR_REQUIRES_RECORD")),
        "a correct constructor of a record shadowing `Boolean` must not be \
         rejected; got {codes:?}"
    );
}

/// Reading a FIELD off a record that shadows a built-in name is rejected —
/// `TYPE_FIELD_ACCESS_REQUIRES_RECORD` — because the primitive-name test runs
/// ahead of the record lookup. That is pre-existing (verified against a
/// pre-plan-111 binary, which reports the identical code) and is NOT what these
/// tests are about; it is pinned here so a future reader does not mistake it for
/// re-keying damage, and so that "fixing" it is a deliberate act.
#[test]
fn a_field_read_on_a_shadowing_record_is_rejected_by_the_primitive_test() {
    assert!(
        diagnostics(
            "shadow_field_primitive",
            "TYPE Boolean\n  a AS Integer\nEND TYPE\n\nFUNC main() AS Integer\n  LET v = Boolean[7]\n  RETURN v.a\nEND FUNC\n",
        )
        .iter()
        .any(|code| code.contains("TYPE_FIELD_ACCESS_REQUIRES_RECORD")),
        "pre-existing: the primitive-name test wins over the record lookup"
    );
}

#[test]
fn a_union_shadowing_a_builtin_name_still_registers_its_variants() {
    assert!(
        diagnostics(
            "shadow_union",
            "TYPE Leaf\n  n AS Integer\nEND TYPE\n\nUNION Float\n  Leaf\nEND UNION\n\nFUNC main() AS Integer\n  RETURN 0\nEND FUNC\n",
        )
        .is_empty(),
        "a union shadowing `Float` must still register `Leaf` as its variant"
    );
}

#[test]
fn an_enum_shadowing_a_builtin_name_still_checks_its_members() {
    assert!(
        diagnostics(
            "shadow_enum",
            "ENUM Money\n  A\n  B\nEND ENUM\n\nFUNC main() AS Integer\n  LET x = Money.C\n  RETURN 0\nEND FUNC\n",
        )
        .iter()
        .any(|code| code.contains("TYPE_UNKNOWN_ENUM_MEMBER")),
        "an enum shadowing `Money` must still reject an undeclared member"
    );
}
