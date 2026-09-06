//! `codegen::resource` module wiring, plus the data-driven resource registry.
//!
//! Resources used to be recognized by a hardcoded set of type names spread
//! across the type checker, the binary-representation writer, and the backend.
//! This module replaces that with a single table keyed by resolved type name.
//! It is seeded with the standard built-ins (`File`, `Socket`, `Listener`) and
//! extended at type-check time from each imported package's `RESOURCE_TABLE`.
//!
//! Stages that operate on already-resolved types and therefore only ever see
//! built-in resources (the backend, the binary-representation writer) keep using
//! the free `is_builtin_*` helpers, which read from the same built-in table so
//! there is one source of truth.
//!
//! (Relocated from `src/builtins/resource.rs` into the codegen layer it fronts,
//! plan-103. Pure code motion.)

pub(crate) mod cleanup;

use crate::codegen::registry::{registry, ResolvedType};
use crate::types::ParameterType;

/// Where a resource descriptor came from. Only built-in resources carry a
/// descriptor (the clean-room registry's); a project's native `LINK` resources
/// and an imported package's `RESOURCE_TABLE` rows are read by `ir::shape` /
/// `ir::verify` directly (plan-107-D deleted the source checker's registry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ResourceKind {
    /// A standard built-in resource (`File`, `Socket`, `Listener`).
    Builtin,
}

/// The bare resource type name, with any `STATE T` suffix removed. A stateful
/// resource carries its `STATE` type in the type string (`File STATE FileState`)
/// once lowered to IR/NIR; recognition keys on the bare resource name.
///
/// plan-111-A deleted this module's own copy of the split. The grammar now
/// lives once, in `crate::types::split_state_clause`, which is also what
/// [`ParameterType::parse`](crate::types::ParameterType::parse) calls to build
/// a [`Stateful`](crate::types::ParameterType::Stateful) — so there is one rule
/// and it cannot drift. The two copies used to be pinned to each other by a
/// parity test, which is the lockstep-edit hazard
/// `planning/Compiler Pipeline.md:25` named.
///
/// Still `&str -> &str`: the result is borrowed from the input, which the owned
/// `ParameterType::split_state` cannot give. That signature dies in plan-111-E,
/// where the callers take a `&ParameterType` and ask `without_state()` directly.
pub(crate) fn base_resource_name(type_name: &str) -> &str {
    match crate::types::split_state_clause(type_name) {
        Some((base, _)) => base,
        None => type_name,
    }
}

/// The `STATE` record type carried by a resource type string, if any. See
/// [`base_resource_name`] for why the grammar is not restated here.
///
/// plan-111-G: every production caller now holds a `ParameterType` and asks
/// [`ParameterType::state`](crate::types::ParameterType::state) directly, so the
/// only reader left is the round-trip parity test in `src/types.rs` that pins
/// this `&str` adapter against the structural splitter. Gated to `cfg(test)`
/// rather than deleted, because deleting it deletes that parity check — the one
/// thing standing between the two spellings of the STATE grammar.
#[cfg(test)]
pub(crate) fn state_type_name(type_name: &str) -> Option<&str> {
    crate::types::split_state_clause(type_name).map(|(_, state)| state)
}

/// Whether `type_name` is a built-in resource type. Used by stages that only
/// ever see built-in resources (codegen, binary-representation writer).
pub(crate) fn is_builtin_resource_type(type_: &ParameterType) -> bool {
    matches!(
        registry().resolve_type(&type_.without_state().name()),
        Some(ResolvedType::Resource(_))
    )
}

/// Whether `type_name` names — or is the **bare base** of — a built-in resource
/// (`File` matches the package-qualified `fs.File`). Used ONLY on the package-import
/// path (`collect_package_resources` / `imported_resource_closers`): an imported
/// `.mfp` may reference a builtin resource by its bare base name even though the
/// builtin's own identity is package-qualified (plan-97). Deliberately more lenient
/// than [`is_builtin_resource_type`] — it must NOT be used for user-type resolution,
/// where a bare `File` is a distinct user type.
pub(crate) fn is_builtin_backed_resource(type_: &ParameterType) -> bool {
    if is_builtin_resource_type(type_) {
        return true;
    }
    // The BARE base (`File` for `fs.File`): a package qualifier is a name-domain
    // prefix, not a type constructor, so it is stripped from the rendered base.
    let base = type_.without_state().name().into_owned();
    let bare = base.rsplit('.').next().unwrap_or(&base);
    registry()
        .packages()
        .iter()
        .any(|p| p.resources().iter().any(|r| r.name == bare))
}

/// The built-in close op for `type_name`, if it is a built-in resource.
pub(crate) fn builtin_resource_close_function(type_: &ParameterType) -> Option<&'static str> {
    match registry().resolve_type(&type_.without_state().name()) {
        Some(ResolvedType::Resource(r)) => Some(r.close_function),
        _ => None,
    }
}

/// The **argument index** of `target`'s consuming parameter, if it has one — the
/// parameter that consumes every resource reachable from its argument (plan-116-J).
///
/// `target` is the qualified member name as it appears in an `IrValue::Call`, e.g.
/// `"canvas.setGroup"`. Returns `None` for everything else, which is the overwhelming
/// majority: a member with no consuming parameter and an unknown member are the same
/// answer here, and that is why `the_consuming_parameters_name_real_members` exists —
/// a typo in an `add_consuming_parameter` call would otherwise be silent.
///
/// **The index is derived from the parameter NAME, not stored.** Storing an index would
/// go stale the first time a parameter is inserted ahead of it, and silently: the lookup
/// would still succeed and point at the wrong argument.
pub(crate) fn builtin_consuming_parameter_index(target: &str) -> Option<usize> {
    let (pkg_name, func_name) = target.split_once('.')?;
    let package = registry()
        .packages()
        .iter()
        .find(|p| p.import_name() == pkg_name)?;
    let entry = package
        .consuming_params()
        .iter()
        .find(|c| c.function == func_name)?;
    // Every overload that HAS the parameter must agree on where it sits; one that does
    // not have it contributes nothing. `the_consuming_parameter_index_agrees_across_overloads`
    // is what holds that, so this may take the first match.
    package
        .function(func_name)?
        .implementations
        .iter()
        .find_map(|imp| imp.params.iter().position(|p| p.name == entry.parameter))
}

/// Whether `type_name` is a built-in resource that may cross a thread boundary.
pub(crate) fn is_builtin_sendable_resource_type(type_: &ParameterType) -> bool {
    match registry().resolve_type(&type_.without_state().name()) {
        Some(ResolvedType::Resource(r)) => r.sendable,
        _ => false,
    }
}

/// Whether `type_`'s built-in close op can fail. Mirrors the registry row; used
/// by the `.mfp` `RESOURCE_TABLE` writer, which hardcoded `true` while its table
/// only ever held three types that all shared it (bug-464 fallout).
pub(crate) fn builtin_resource_close_may_fail(type_: &ParameterType) -> bool {
    match registry().resolve_type(&type_.without_state().name()) {
        Some(ResolvedType::Resource(r)) => r.close_may_fail,
        _ => false,
    }
}

/// The live words in `type_`'s record past the canonical header, which the
/// thread-transfer copy must carry (bug-464). Empty for a resource whose record
/// is the header alone, and for anything that is not a built-in resource.
///
/// Note this is deliberately independent of [`is_builtin_sendable_resource_type`]:
/// a non-sendable resource still declares its slots, so that opting it in later
/// is a one-line change that cannot silently truncate its record.
pub(crate) fn builtin_resource_live_slots(
    type_: &ParameterType,
) -> &'static [crate::codegen::registry::ResourceLiveSlot] {
    match registry().resolve_type(&type_.without_state().name()) {
        Some(ResolvedType::Resource(r)) => r.live_slots,
        _ => &[],
    }
}

#[cfg(test)]
mod tests;
