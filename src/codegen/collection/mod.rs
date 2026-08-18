//! Target-generic collection-data codegen — the shared **collection tier**.
//!
//! This is the shared *data* layer for `List`/`Map`/`Set` payloads (block layout,
//! buffer growth, in-place mutation, comparison, loop-walk scaffolding), NOT the
//! `collections::` builtin *package* (that lives under `builtins/collections`).
//! Core codegen depends on these primitives directly — e.g. destructor cleanup
//! walks a `List` to free its elements through the loop scaffolding, and
//! `builder_control` materializes bound elements through it — so they belong in
//! this shared tier rather than under any one package.
//!
//! First resident: `collection_loop` (the packed-data loop-walk scaffolding),
//! relocated here from `codegen::memory`. It stays an `impl CodeBuilder` method
//! block, so call sites are unchanged. The remaining `src/target/shared/code`
//! collection files (`builder_collection_layout`, `collection_buffer`,
//! `list_mutate`, `map_mutate`, `builder_inplace_assign`,
//! `builder_collection_compare`) land here as they migrate (see
//! `planning/helper.md`).

pub(crate) mod assign;
pub(crate) mod buffer;
pub(crate) mod collection_loop;
pub(crate) mod compare;
pub(crate) mod layout;
pub(crate) mod list;
pub(crate) mod map;
pub(crate) mod search;
pub(crate) mod sort;
