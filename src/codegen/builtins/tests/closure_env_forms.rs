//! Closure environments that carry more than their captures
//! (`engine/value/builder_values/lower_construct.rs:lower_closure_value`): a
//! non-escaping callback (`collections::forEach`) that self-updates a by-reference
//! captured `String` works on its OWNER's block, so the environment hands it the
//! owner's resources through one extra word each —
//!
//! * the creator's self-update scratch (plan-142-G), for an arm that builds its
//!   result in scratch (`strings::upper`);
//! * the owner's capacity shadow (plan-146-G), for an in-place append
//!   (`s = s & x`);
//! * the owner's `os::appResourcePath` base cache (plan-146-G).
//!
//! Each is observable as a stack object the CREATOR reserves for that purpose (the
//! callback could not reach it otherwise) and, for the shadow, one the callback
//! reserves to read it back.

use super::lowered::slots_on;
use crate::testutil::CodeTarget;

const LINUX: CodeTarget = CodeTarget::LinuxX86_64;

fn has(names: &[String], wanted: &str) -> bool {
    names.iter().any(|name| name == wanted)
}

fn callback_name(source: &str) -> String {
    let plan = crate::testutil::code_for_src_cached(
        source,
        LINUX,
        crate::target::NativeBuildMode::Console,
    );
    plan.functions
        .iter()
        .map(|function| function.name.clone())
        .find(|name| name.contains("lambda"))
        .expect("the callback lowers as its own function")
}

#[test]
fn a_callback_self_updating_a_captured_string_shares_its_owners_scratch() {
    let source = "\
IMPORT collections
IMPORT io
IMPORT strings

FUNC main() AS Integer
  MUT s AS String = \"abc\"
  collections::forEach([1, 2], LAMBDA(v AS Integer) -> s = strings::upper(s))
  io::print(s)
  RETURN 0
END FUNC
";
    let main = slots_on(source, "main", LINUX);
    assert!(
        has(&main, "su_scratch"),
        "the creator owns the scratch: {main:?}"
    );
    assert!(has(&main, "closure_env"), "{main:?}");
}

#[test]
fn a_callback_appending_to_a_captured_string_shares_its_owners_capacity_shadow() {
    let source = "\
IMPORT collections
IMPORT io

FUNC main() AS Integer
  MUT acc AS String = \"\"
  collections::forEach([\"a\", \"b\"], LAMBDA(x AS String) -> acc = acc & x)
  io::print(acc)
  RETURN 0
END FUNC
";
    let main = slots_on(source, "main", LINUX);
    assert!(
        has(&main, "strcap_acc"),
        "the owner keeps the capacity shadow the callback grows: {main:?}"
    );
    let callback = slots_on(source, &callback_name(source), LINUX);
    assert!(
        has(&callback, "su_ref_strcap_owner"),
        "the callback reads the shadow through its environment: {callback:?}"
    );
}

#[test]
fn a_callback_resolving_a_resource_path_shares_its_owners_base_cache() {
    let source = "\
IMPORT collections
IMPORT io
IMPORT os

FUNC main() AS Integer
  MUT path AS String = \"icon.png\"
  collections::forEach([1], LAMBDA(v AS Integer) -> path = os::appResourcePath(path))
  io::print(path)
  RETURN 0
END FUNC
";
    let main = slots_on(source, "main", LINUX);
    assert!(
        has(&main, "str_resource_base"),
        "the owner keeps the base cache the callback fills: {main:?}"
    );
}
