//! plan-176-M: package-qualifying the type-name leaves of a spelled type. `Registry` and
//! `build()` live in `codegen/registry/mod.rs`.

use super::*;

/// Construct the registry by registering every migrated package.
///
/// The `example` package is an illustrative entry that exercises the shape (a
/// single-implementation function and a two-implementation overload); `csv` is the
/// first real package migrated off `target::shared::registry` — it registers itself
/// Map every NOMINAL leaf of `ty` from its bare spelling to the package-qualified
/// identity, per the owner rule in
/// [`Registry::qualify_value_type_references`]. Container shapes
/// (`List OF`, `Map OF … TO …`, `Result OF`, a user generic's arguments) are
/// descended into, so `List OF Json` becomes `List OF json.Json`.
///
/// An already-qualified leaf is left alone: resources have carried their package
/// since plan-97, and a descriptor that spells one out is already correct.
pub(super) fn qualify_type_leaves(
    ty: &ParameterType,
    package: &str,
    owners: &std::collections::HashMap<String, Vec<String>>,
) -> ParameterType {
    qualify_type_leaves_inner(ty, package, owners, true)
}

/// The governing rule's two cases, for a type reference that is RENDERED BACK INTO
/// SOURCE (a record field): a name defined **locally** needs no prefix, a name that
/// is **imported** requires one. So the own-package arm leaves the leaf bare here,
/// where `qualify_type_leaves` (used for signatures, which are type-system
/// identities and never round-trip through source) qualifies it.
///
/// Getting this backwards is not cosmetic: `regex`'s companion declares PRIVATE
/// types like `#regex_Cont` locally, and prefixing those made the descriptor's
/// field type `regex.#regex_Cont` disagree with the `#regex_Cont` the parser
/// produces for the local declaration — `TYPE_CONSTRUCTOR_ARGUMENT_MISMATCH` on
/// the package's own source (bug-484).
pub(super) fn qualify_type_leaves_for_source(
    ty: &ParameterType,
    package: &str,
    owners: &std::collections::HashMap<String, Vec<String>>,
) -> ParameterType {
    qualify_type_leaves_inner(ty, package, owners, false)
}

pub(super) fn qualify_type_leaves_inner(
    ty: &ParameterType,
    package: &str,
    owners: &std::collections::HashMap<String, Vec<String>>,
    qualify_own: bool,
) -> ParameterType {
    let qualify_leaf = |leaf: &str| -> Option<String> {
        if leaf.contains('.') {
            return None; // already qualified (a resource id, or a spelled-out reference)
        }
        // Not a registry-declared value type at all: a scalar nominal (`Scalar`,
        // `Error`, `ErrorLoc`, `AttributedString`), a C ABI spelling, or a generic
        // parameter. Nothing to qualify, and not an error.
        let declaring = owners.get(leaf)?;
        if declaring.iter().any(|owner| owner == package) {
            // Declared by THIS package, so it is local. A signature carries the
            // qualified identity; a rendered field keeps the bare local name.
            return qualify_own.then(|| format!("{package}.{leaf}"));
        }
        // Not declared by this package, so the bare leaf denotes nothing: under the
        // governing rule a bare name is a LOCAL name, and this package has no such
        // declaration. How many OTHER packages happen to export the leaf is
        // irrelevant — one is not more resolvable than three, it is just a guess
        // that happens to have one candidate. The pass used to make exactly that
        // guess for a single owner and leave the reference bare for several, both
        // silently. A descriptor must spell a cross-package reference out
        // (`net::ADDRESS_TYPE_ID`, not `net::ADDRESS_TYPE`), so reaching here is an
        // authoring error, not a user one (bug-484).
        panic!(
            "registry: `{package}` references the bare type name `{leaf}`, but \
             declares no such type. A bare name is a LOCAL name; `{leaf}` is \
             declared by {declaring:?}. Spell the reference out as \
             `<package>.{leaf}`."
        )
    };
    match ty {
        ParameterType::Named(sym) => match qualify_leaf(sym.resolve()) {
            Some(qualified) => ParameterType::named(&qualified),
            None => ty.clone(),
        },
        ParameterType::UserOf(head, args) => {
            let args = args
                .iter()
                .map(|arg| qualify_type_leaves_inner(arg, package, owners, qualify_own))
                .collect::<Vec<_>>();
            match qualify_leaf(head.resolve()) {
                Some(qualified) => ParameterType::user_of(&qualified, args),
                None => ParameterType::user_of(head.resolve(), args),
            }
        }
        ParameterType::ListOf(inner) => ParameterType::list_of(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        )),
        ParameterType::SetOf(inner) => ParameterType::set_of(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        )),
        ParameterType::MapOf(key, value) => ParameterType::map_of(
            qualify_type_leaves_inner(key, package, owners, qualify_own),
            qualify_type_leaves_inner(value, package, owners, qualify_own),
        ),
        ParameterType::ResultOf(inner) => ParameterType::result_of(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        )),
        // A resource's STATE clause and a `RES` wrapper both hold a nominal that has
        // to be qualified too: `http::startRead` returns `Stream STATE PendingState`,
        // and leaving the state bare made the initializer disagree with the binding
        // (`declares STATE http.PendingState but its initializer carries STATE
        // PendingState`).
        ParameterType::Stateful { base, state } => ParameterType::Stateful {
            base: Box::new(qualify_type_leaves_inner(
                base,
                package,
                owners,
                qualify_own,
            )),
            state: Box::new(qualify_type_leaves_inner(
                state,
                package,
                owners,
                qualify_own,
            )),
        },
        ParameterType::Res(inner) => ParameterType::Res(Box::new(qualify_type_leaves_inner(
            inner,
            package,
            owners,
            qualify_own,
        ))),
        // A callback's signature names types too (`json::parse`'s reviver is
        // `FUNC(String, Json) AS Json`). Left bare, its leaves denote no registry type,
        // so strict matching could not hold a callback argument to them (bug-638).
        ParameterType::Func(params, ret, isolated) => ParameterType::Func(
            params
                .iter()
                .map(|param| qualify_type_leaves_inner(param, package, owners, qualify_own))
                .collect(),
            Box::new(qualify_type_leaves_inner(ret, package, owners, qualify_own)),
            *isolated,
        ),
        other => other.clone(),
    }
}
