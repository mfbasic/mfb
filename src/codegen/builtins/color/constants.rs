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
mod tests;
