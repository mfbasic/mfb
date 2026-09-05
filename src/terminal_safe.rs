//! Re-export of the one terminal sanitizer.
//!
//! The implementation lives in `mfb_repository::terminal_safe` (bug-489): the
//! registry client has to sanitize the server-authored error strings it returns,
//! and `mfb_repository` cannot depend on `mfb`. Keeping one implementation there
//! rather than a copy here is what stops the two drifting — a sanitizer that
//! escapes a different set on each side of the crate boundary is worse than
//! either alone.
//!
//! See that module for what is escaped and why.

pub(crate) use mfb_repository::terminal_safe::{is_terminal_unsafe, safe};
