//! Target-generic value/data memory codegen (see `readme.md`).
//!
//! Current resident: the get-result **owning copy** (`owned`) — `LET x =
//! get(...)` owns its element through `materialize_owned_element`. It is a
//! value-materialization primitive on the raw memory representation, shared by
//! core codegen, not `collections::` package logic. (The packed-data loop-walk
//! scaffolding that used to sit beside it has moved to the shared collection tier
//! at `codegen::collection::collection_loop`.)
//!
//! It stays an `impl CodeBuilder` method (call sites unchanged). Because
//! `CodeBuilder` and the low-level emit helpers still live in `src/target`, it
//! calls *back* into target (the accepted temporary `codegen -> target` edge), and
//! `src/target` code that uses it now calls *forward* into codegen — a
//! transitional bidirectional edge that resolves when `CodeBuilder` itself moves.

pub(crate) mod arena;
pub(crate) mod data;
pub(crate) mod marshal;
pub(crate) mod owned;
pub(crate) mod value;
