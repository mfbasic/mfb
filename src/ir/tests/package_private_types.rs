//! bug-624: a package's PRIVATE types get an identity-scoped name when the
//! package is prefixed for merging (`prefix_package_symbols` ->
//! `scope_private_types`), and every reference to them is rewritten with it.
//!
//! Left bare, a package-private `TYPE Frame` lost `merge_package`'s
//! first-wins de-duplication to an importer's own `Frame`, and the package's
//! code was then verified and lowered against the importer's fields. These
//! tests lower a package-shaped source and check the three classes the pass
//! distinguishes: the exported surface keeps its spelling, a referenced private
//! type is renamed everywhere it is named, and an unreferenced one is left alone.

use crate::internal_name::{file_scope_hash, mangle_private};
use crate::ir::{prefix_package_symbols, IrMatchPattern, IrOp, IrProject, IrValue};
use crate::testutil::lower_src;
use crate::types::ParameterType;

const PACKAGE: &str = r#"
EXPORT TYPE Shown
  n AS Integer
  inner AS Carried
END TYPE

TYPE Carried
  c AS Integer
END TYPE

TYPE Frame
  w AS Integer
END TYPE

TYPE Other
  o AS Integer
END TYPE

UNION Hidden
  Frame
  Other
END UNION

TYPE Holder
  frames AS List OF Frame
END TYPE

TYPE Dead
  z AS Integer
END TYPE

FUNC score(h AS Hidden) AS Integer
  MATCH h
    CASE Frame(f)
      RETURN f.w
    CASE Other(o)
      RETURN o.o
  END MATCH
END FUNC

EXPORT FUNC area(s AS Shown) AS Integer
  LET f AS Frame = Frame[s.n]
  LET held AS Holder = Holder[[f]]
  LET h AS Hidden = f
  RETURN score(h) + len(held.frames) + s.inner.c
END FUNC
"#;

const ID: &str = "pkgid42";

fn prefixed() -> IrProject {
    let mut pir = lower_src(PACKAGE);
    pir.name = "geo".to_string();
    prefix_package_symbols(&mut pir, ID, &Default::default());
    pir
}

fn type_names(pir: &IrProject) -> Vec<String> {
    pir.types.iter().map(|ty| ty.name.clone()).collect()
}

fn scoped(name: &str) -> String {
    format!("geo.{}", mangle_private(ID, name))
}

#[test]
fn a_referenced_private_type_is_renamed_to_its_identity_scoped_spelling() {
    let pir = prefixed();
    let names = type_names(&pir);
    for private in ["Frame", "Other", "Hidden", "Holder"] {
        assert!(
            names.contains(&scoped(private)),
            "private `{private}` must be scoped as `{}`; types: {names:?}",
            scoped(private)
        );
        assert!(
            !names.contains(&private.to_string()),
            "private `{private}` kept its bare spelling; types: {names:?}"
        );
    }
}

#[test]
fn the_exported_surface_and_an_unreferenced_type_keep_their_spelling() {
    let names = type_names(&prefixed());
    // `Shown` is EXPORT; `Carried` is reached through an exported record's
    // field, so the importer receives values of it and it is on the surface.
    for kept in ["Shown", "Carried", "Dead"] {
        assert!(
            names.contains(&format!("geo.{kept}")),
            "`{kept}` must keep its spelling; types: {names:?}"
        );
    }
}

#[test]
fn every_reference_to_a_scoped_type_is_rewritten() {
    let pir = prefixed();
    let hidden = pir
        .types
        .iter()
        .find(|ty| ty.name == scoped("Hidden"))
        .expect("scoped union");
    let variants: Vec<&str> = hidden.variants.iter().map(|v| v.name.as_str()).collect();
    assert_eq!(variants, [scoped("Frame"), scoped("Other")]);

    let holder = pir
        .types
        .iter()
        .find(|ty| ty.name == scoped("Holder"))
        .expect("scoped record");
    assert_eq!(
        holder.fields[0].type_,
        ParameterType::list_of(ParameterType::named(&scoped("Frame")))
    );

    // The private function's parameter is the scoped union.
    let score = pir
        .functions
        .iter()
        .find(|f| f.name.ends_with(".score"))
        .expect("score");
    assert_eq!(
        score.params[0].type_,
        ParameterType::named(&scoped("Hidden"))
    );

    // A local's annotation in the exported function's body is rewritten too.
    let area = pir
        .functions
        .iter()
        .find(|f| f.name.ends_with(".area"))
        .expect("area");
    let bound: Vec<(&str, &ParameterType)> = area
        .body
        .iter()
        .filter_map(|op| match op {
            IrOp::Bind { name, type_, .. } => Some((name.as_str(), type_)),
            _ => None,
        })
        .collect();
    let frame = ParameterType::named(&scoped("Frame"));
    let hidden_type = ParameterType::named(&scoped("Hidden"));
    assert!(
        bound.contains(&("f", &frame)) && bound.contains(&("h", &hidden_type)),
        "the body's bindings must carry the scoped types: {:?}",
        bound
            .iter()
            .map(|(n, t)| format!("{n}: {}", t.name()))
            .collect::<Vec<_>>()
    );
    // The MATCH case's variant pattern is a type name in value position.
    let matches_scoped = score.body.iter().any(|op| match op {
        IrOp::Match { cases, .. } => cases.iter().any(|case| {
            matches!(&case.pattern,
                IrMatchPattern::Value(IrValue::Local(name)) if *name == scoped("Frame"))
        }),
        _ => false,
    });
    assert!(
        matches_scoped,
        "the MATCH pattern still names the bare variant"
    );
}

#[test]
fn an_already_file_private_type_is_rehashed_with_the_identity() {
    // A file-PRIVATE type arrives as `#<file hash>$Frame`; the scoped spelling
    // keeps the one-hash shape, re-hashed with the package identity.
    let file_hash = file_scope_hash("src/main.mfb");
    let private = mangle_private(&file_hash, "Frame");
    let mut pir = lower_src(PACKAGE);
    pir.name = "geo".to_string();
    for ty in &mut pir.types {
        if ty.name == "Frame" {
            ty.name = private.clone();
        }
    }
    for ty in &mut pir.types {
        for variant in &mut ty.variants {
            if variant.name == "Frame" {
                variant.name = private.clone();
            }
        }
    }
    prefix_package_symbols(&mut pir, ID, &Default::default());
    let want = mangle_private(&file_scope_hash(&format!("{ID}${file_hash}")), "Frame");
    let names = type_names(&pir);
    assert!(
        names.contains(&format!("geo.{want}")),
        "a file-private type must be re-hashed as `{want}`; types: {names:?}"
    );
    assert_eq!(crate::internal_name::display_name(&want), "Frame");
}
