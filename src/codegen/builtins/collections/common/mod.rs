//! Shared, collections-package-only codegen primitives ("A1" tier).
//!
//! Home for the target-generic `impl CodeBuilder` helpers whose only callers are
//! collection-domain lowerings — the `func_*.rs` builtin entries and their
//! sibling collection code in `src/target`. A caller census (recorded in the git
//! history of `planning/`) split the collection codegen in `src/target` into
//! three tiers; this module is the tier that is genuinely private to the
//! `collections` package, moved here so it lives beside the builtins that use it.
//!
//! Tiers that do NOT belong here (they are shared beyond the package and are
//! destined for the `src/codegen/memory` data layer instead): the in-place
//! mutation primitives (also used by the `list[i] = x` assignment operator),
//! the byte/value comparison branches (used by search/numeric/strings), the
//! buffer-growth helpers, and the packed-data loop scaffolding (used by
//! destructor cleanup). See `src/codegen/memory/readme.md`.
//!
//! Everything here stays an `impl CodeBuilder` method: only the defining module
//! moved, so call sites (`builder.lower_list_get(..)`, `self.emit_map_probe(..)`)
//! are unchanged. `CodeBuilder` itself still lives in `src/target` (its relocation
//! is deferred), which is why these files carry the accepted temporary
//! `codegen -> target` import edge.

pub(crate) mod list;
pub(crate) mod map;
