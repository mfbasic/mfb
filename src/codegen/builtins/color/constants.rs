//! The basic-colour record constants (`color::black`, `color::white`, …).
//!
//! A record constant inlines its four per-field literals into a `Color`
//! constructor at the call site (`RegistryConstant::components`,
//! `vector::zeroFloat3` is the shipped precedent), so `color::black` needs no call
//! and no string lookup — it is not a lookup into the CSS name table and does not
//! depend on it.

use crate::codegen::registry::{RegistryConstant, RegistryPackage};

/// The sixteen colours a program reaches for without thinking, as CSS defines
/// them. Every one is fully opaque.
///
/// **`green` is `#008000`, not `#00ff00`.** The CSS keyword `green` is a dark
/// green; the vivid colour most people picture is `lime`. The constant follows CSS
/// because `color::fromName("green")` must agree with it — two spellings of the
/// same name disagreeing would be far worse than the surprise. `color::fromName`
/// reaches `lime` for the vivid one; there is deliberately no `color::lime`
/// constant, because the sixteen here are the classic basic set and adding a
/// seventeenth to paper over the surprise would just move it.
///
/// Values taken from the CSS Color Level 4 `<named-color>` table — the same source
/// as `helper_name_table`, so the constant and the lookup cannot drift.
const BASIC: &[(&str, &[&str])] = &[
    ("black", &["0", "0", "0", "255"]),
    ("white", &["255", "255", "255", "255"]),
    ("red", &["255", "0", "0", "255"]),
    ("green", &["0", "128", "0", "255"]),
    ("blue", &["0", "0", "255", "255"]),
    ("yellow", &["255", "255", "0", "255"]),
    ("cyan", &["0", "255", "255", "255"]),
    ("magenta", &["255", "0", "255", "255"]),
    ("gray", &["128", "128", "128", "255"]),
    ("silver", &["192", "192", "192", "255"]),
    ("maroon", &["128", "0", "0", "255"]),
    ("olive", &["128", "128", "0", "255"]),
    ("navy", &["0", "0", "128", "255"]),
    ("teal", &["0", "128", "128", "255"]),
    ("purple", &["128", "0", "128", "255"]),
    ("orange", &["255", "165", "0", "255"]),
];

pub(crate) fn register(pkg: &mut RegistryPackage) {
    for (name, components) in BASIC {
        pkg.add_constant(RegistryConstant {
            name,
            type_name: super::COLOR_TYPE,
            value: None,
            components: Some(components),
            message: None,
            symbol: None,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::BASIC;

    /// Every constant is four components, and every one is a `Byte`-range integer.
    /// A component out of range would inline a constructor argument the `Color`
    /// record cannot hold.
    #[test]
    fn basic_constants_are_four_byte_components() {
        for (name, components) in BASIC {
            assert_eq!(components.len(), 4, "{name} needs r, g, b, a");
            for c in components.iter() {
                let value: i64 = c.parse().unwrap_or_else(|_| panic!("{name}: {c}"));
                assert!((0..=255).contains(&value), "{name}: {c} out of Byte range");
            }
        }
    }

    /// Every constant is fully opaque, matching the CSS named colours they mirror.
    #[test]
    fn basic_constants_are_opaque() {
        for (name, components) in BASIC {
            assert_eq!(components[3], "255", "{name} must be opaque");
        }
    }

    /// The values most often misremembered, against CSS Color Level 4.
    ///
    /// `green` is the one that matters: CSS `green` is `#008000`, and a constant
    /// set to `#00ff00` would contradict `color::fromName("green")` — the two must
    /// agree or the package says different things about the same name.
    #[test]
    fn basic_constants_match_the_css_values() {
        let get = |want: &str| {
            BASIC
                .iter()
                .find(|(name, _)| *name == want)
                .unwrap_or_else(|| panic!("missing {want}"))
                .1
        };
        assert_eq!(
            get("green"),
            &["0", "128", "0", "255"],
            "CSS green is #008000"
        );
        assert_eq!(get("gray"), &["128", "128", "128", "255"]);
        assert_eq!(get("purple"), &["128", "0", "128", "255"]);
        assert_eq!(get("silver"), &["192", "192", "192", "255"]);
        assert_eq!(get("orange"), &["255", "165", "0", "255"]);
        assert_eq!(get("teal"), &["0", "128", "128", "255"]);
    }

    /// No duplicate names.
    #[test]
    fn basic_constant_names_are_unique() {
        let mut names: Vec<&str> = BASIC.iter().map(|(n, _)| *n).collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(names.len(), before, "duplicate constant name");
    }
}
