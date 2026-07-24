//! PE32+ executable linker (plan-47-C).
//!
//! `pe.rs` (Phase 2) is the pure header/section-table byte writer. Phase 3 adds
//! the `.idata`/IAT construction, the import thunks, and the relocation patcher
//! that binds an `EncodedImage` into a finished `.exe` (`write_executable`). Until
//! then this module only re-exports the header writer for its own tests; the
//! `dead_code` allow lives on the parent `windows` module (removed by 47-D).

mod pe;
