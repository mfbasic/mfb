//! Re-export of the one terminal sanitizer.
//!
//! The implementation lives in `mfb_wire::terminal_safe` (plan-126-C), which is
//! the crate both this one and `mfb` may depend on. One implementation and one
//! set of tests is what stops the two drifting — a sanitizer that escapes a
//! different set on each side of a crate boundary is worse than either alone.
//!
//! It used to live *here*, with bug-489's explanation that the registry client
//! is one of its callers and `mfb_repository` cannot depend on `mfb`. That was
//! true, but it left the registry crate owning a terminal sanitizer, which has
//! nothing to do with a package registry. `mfb_wire` removed the constraint, so
//! the shim now points the other way.
//!
//! See that module for what is escaped and why.

pub use mfb_wire::terminal_safe::{is_terminal_unsafe, safe};
