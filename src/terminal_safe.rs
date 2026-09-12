//! Re-export of the one terminal sanitizer.
//!
//! The implementation lives in `mfb_wire::terminal_safe` (plan-126-C), which is
//! the crate both this one and `mfb_repository` may depend on. One
//! implementation and one set of tests is what stops the two drifting — a
//! sanitizer that escapes a different set on each side of a crate boundary is
//! worse than either alone.
//!
//! This shim used to point at `mfb_repository::terminal_safe`, because bug-489
//! had put the implementation there: the registry client is one of its callers
//! and `mfb_repository` could not depend on `mfb`. That left a *terminal
//! sanitizer* owned by the package-registry crate. `mfb_wire` removed the
//! constraint; the rationale is obsolete and should not be restored.
//!
//! See that module for what is escaped and why.

pub(crate) use mfb_wire::terminal_safe::{is_terminal_unsafe, safe};
