//! The built-in `astrings` package (clean-room registry migration).
//!
//! `astrings` provides construction, mutation, query, and rendering for the opaque,
//! value-semantic `AttributedString` type. That TYPE stays **hardcoded and
//! always-in-scope** (like `Error`) — spread across `ir/verify`,
//! `target/macos_aarch64`, `target/shared/registry`, and the code layer — and is NOT
//! migrated here; only `astrings`' FUNCTIONS and its source companion move.
//!
//! The public members split three ways by realization:
//!   - `fromString` is **native-direct** codegen — `Body::Native` `common`, a thin
//!     wrapper over the shared `AttributedString` carrier
//!     (`CodeBuilder::lower_astrings_package_call` in
//!     `src/target/shared/code/builder_astrings.rs`, kept in place like `vector`'s
//!     SIMD carrier and `strings`' string carrier).
//!   - the `Attribute`-model constructors (`bold`..`background`) and the Tier-C
//!     mutation/query members (`addAttribute`..`toMarkdown`) are **source-companion
//!     rewrites** — `Body::Rewrite("__astrings_*")` into `package.mfb`.
//!     `clearAttributes` overloads on arity: the whole form (1 arg) rewrites to
//!     `__astrings_clearAttributes`, the ranged form (3 args) to
//!     `__astrings_clearAttributesRange`, selected by the registry's overload-aware
//!     `rewrite_target`.
//!   - `readSpans`/`writeSpans`/`scalarLen` are **internal-only** native overlay-bridge
//!     primitives (`Body::Native` `common`, `internal_only: true`): they cross the
//!     opaque record boundary the `.mfb` companion cannot touch. Users can never call
//!     them (the `internal_only` flag, honored by `builtins::is_internal_only_call`).
//!
//! The companion `package.mfb` carries the open `Attribute` model
//! (`AttrTypeFlag`/`AttrText`/… enums, records, the `Attribute` union), the internal
//! `AttrSpan`/`MdState` records, and every `__astrings_*` body. It is injected on
//! IMPORT as an `Always` helper named exactly `"astrings"`, so its synthetic file
//! derives the legacy `<builtin-astrings>` label (byte-identical injection). The
//! companion `IMPORT strings` and calls the scalar seam (`strings::toScalars`/…); the
//! `strings` package rides that seam in whenever `astrings` is imported via its
//! landed `WhenImported("astrings")` gate.

use crate::codegen::registry::{Registry, RegistryHelper, RegistryPackage};

mod func_add_attribute;
mod func_background;
mod func_bold;
mod func_clear_attributes;
mod func_font;
mod func_font_size;
mod func_foreground;
mod func_from_string;
mod func_get_attributes;
mod func_italic;
mod func_overline;
mod func_read_spans;
mod func_remove_attribute;
mod func_scalar_len;
mod func_strike;
mod func_to_markdown;
mod func_underline;
mod func_write_spans;

/// One-line package intro (was `BuiltinModule::doc_intro`, historically empty).
const INTRO: &str = "";
/// Package-overview description (historically empty; the man page is the doc
/// authority for `astrings`).
const DESC: &str = "";

/// The source companion — the open `Attribute` model plus every `__astrings_*` body.
/// Injected verbatim on IMPORT (byte-exact with the legacy `package_source_glue!`
/// `include_str!`).
const COMPANION_SOURCE: &str = include_str!("package.mfb");

/// Register the `astrings` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("astrings", INTRO, DESC);

    // Native-direct constructor (shared codegen carrier).
    func_from_string::register(&mut pkg);

    // Source-companion `Attribute`-model constructors.
    func_bold::register(&mut pkg);
    func_italic::register(&mut pkg);
    func_underline::register(&mut pkg);
    func_strike::register(&mut pkg);
    func_overline::register(&mut pkg);
    func_font::register(&mut pkg);
    func_font_size::register(&mut pkg);
    func_foreground::register(&mut pkg);
    func_background::register(&mut pkg);

    // Source-companion Tier-C mutation/query members.
    func_add_attribute::register(&mut pkg);
    func_remove_attribute::register(&mut pkg);
    func_clear_attributes::register(&mut pkg);
    func_get_attributes::register(&mut pkg);
    func_to_markdown::register(&mut pkg);

    // Internal-only native overlay bridge (never user-callable).
    func_read_spans::register(&mut pkg);
    func_write_spans::register(&mut pkg);
    func_scalar_len::register(&mut pkg);

    // The source companion, injected on IMPORT. Named exactly `"astrings"` so its
    // synthetic file derives the legacy `<builtin-astrings>` label. `Always` renders
    // it inline in `get_mfb`, which `Registry::augment_project` emits as the package's
    // injected file whenever a program `IMPORT astrings` — reproducing the legacy
    // `astrings::augmented_project` `uses_package` late pass.
    pkg.add_helper(RegistryHelper::always("astrings", COMPANION_SOURCE));

    r.add_package(pkg);
}

pub(crate) mod builder_astrings;
