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
