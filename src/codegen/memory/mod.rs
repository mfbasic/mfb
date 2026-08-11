//! Target-generic value/data memory codegen (see `readme.md`).
//!
//! First real residents (moved out of `src/target/shared/code`): the packed-data
//! **loop-walk scaffolding** (`collection_loop`) and the get-result **owning copy**
//! (`owned`). These are collection-*data* primitives — they operate on the raw
//! memory representation of a collection — but they are NOT collection-*package*
//! logic: core codegen shares them (destructor cleanup in `builder_owned_cleanup`
//! walks a `List` to free its elements through the loop scaffolding; `LET x =
//! get(...)` owns its element through `materialize_owned_element`; `builder_control`
//! materializes bound elements). So they belong in this shared data tier rather
//! than under `builtins/collections`.
//!
//! They stay `impl CodeBuilder` methods (call sites unchanged). Because
//! `CodeBuilder` and the low-level emit helpers still live in `src/target`, these
//! call *back* into target (the accepted temporary `codegen -> target` edge), and
//! `src/target` code that uses them now calls *forward* into codegen — a
//! transitional bidirectional edge that resolves when `CodeBuilder` itself moves.

pub(crate) mod collection_loop;
pub(crate) mod owned;
