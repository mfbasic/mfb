//! An enum member (`Color.Blue`) as, or inside, the argument that binds a template's
//! type parameter.
//!
//! The monomorphizer infers each argument's type to bind `T`. It typed a member
//! access only as a record field read, so `Color.Blue` had no type, an inferred
//! `[Color.Blue, Color.Red]` was `List OF Unknown`, and the instantiation was
//! `T = Unknown`: `collections::distinct([Color.Blue, …])` — valid source — was
//! refused inside the template's body ("`item` does not have a known type"), and
//! `collections::sort([Color.Blue, …])` — invalid, enums are not orderable
//! (`mfb spec language types`) — slipped past the body's `<` check (an `Unknown`
//! operand is permissive) and died in the backend on "packed payload does not
//! support type 'Unknown'". The declared spelling (`LET xs AS List OF Color = …`)
//! always worked; these rows pin the inferred ones to the same answers.

use crate::testutil::check_src;

fn program(body: &str) -> String {
    format!(
        "IMPORT io\nIMPORT collections\n\nENUM Color\n  Red, Blue\nEND ENUM\n\n\
         FUNC main() AS Nothing\n{body}END FUNC\n"
    )
}

#[test]
fn distinct_over_an_enum_list_literal_is_accepted() {
    let rules = check_src(&program(
        "  LET s AS List OF Color = collections::distinct([Color.Blue, Color.Red, Color.Blue])\n\
         \x20 io::print(toString(len(s)))\n",
    ));
    assert!(rules.is_empty(), "{rules:?}");
}

#[test]
fn distinct_over_an_inferred_enum_list_binding_is_accepted() {
    let rules = check_src(&program(
        "  LET ys = [Color.Blue, Color.Red, Color.Blue]\n\
         \x20 LET s AS List OF Color = collections::distinct(ys)\n\
         \x20 io::print(toString(len(s)))\n",
    ));
    assert!(rules.is_empty(), "{rules:?}");
}

/// Enums are comparable but not orderable: `sort` over an enum list is refused by
/// the front end with the same rule as over a declared enum list (the template
/// body's `<`), never left to the backend.
#[test]
fn sort_over_an_enum_list_literal_is_refused_as_not_orderable() {
    let declared = check_src(&program(
        "  LET xs AS List OF Color = [Color.Blue, Color.Red]\n\
         \x20 LET s AS List OF Color = collections::sort(xs)\n\
         \x20 io::print(toString(len(s)))\n",
    ));
    let literal = check_src(&program(
        "  LET s AS List OF Color = collections::sort([Color.Blue, Color.Red])\n\
         \x20 io::print(toString(len(s)))\n",
    ));
    assert_eq!(declared, vec!["TYPE_BINARY_OPERATOR_MISMATCH".to_string()]);
    assert_eq!(
        literal, declared,
        "the literal spelling gets the declared spelling's answer"
    );
}
