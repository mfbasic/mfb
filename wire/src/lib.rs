//! Wire formats shared by the compiler and the registry (plan-126-B).
//!
//! # The dependency rule that justifies this crate
//!
//! ```text
//!            mfb_wire            (this crate — sha2 only)
//!              ↑        ↑
//!              |        |
//!      mfb_repository   mfb      (registry server + client, compiler)
//! ```
//!
//! `mfb` depends on `mfb_repository` — the compiler genuinely *is* a registry
//! client, so that arrow is correct and stays. What could not exist was the
//! arrow back: `mfb_repository` cannot depend on `mfb`, so anything the two
//! both need had to be **restated** on the registry side. Three places said so
//! in prose and apologised for it (`repository/src/abi.rs`'s header,
//! `repository/src/package.rs`'s private byte readers, and the bug-489
//! `terminal_safe` shim, which put a *terminal sanitizer* in the registry crate
//! because there was nowhere else to put it).
//!
//! This crate is that missing home. Both siblings depend on it; it depends on
//! neither.
//!
//! **Keep it a pure computation over bytes.** `sha2` is the only dependency,
//! and the reason is concrete rather than aesthetic: `repository/Dockerfile`
//! builds `mfb-repo` *without* compiling the compiler, so every dependency
//! added here lands in the deploy image. No I/O, no HTTP, no database, no
//! async, and nothing that reaches a filesystem — a caller that needs those
//! reads the bytes itself and hands them over.
//!
//! # What belongs here, and what does not
//!
//! Here: byte-level decode/encode primitives, and the framing of the `.mfp`
//! container and its MFPC payload — the knowledge that was duplicated.
//!
//! Not here: `crypto`, `MfpPackage`, the server's wire DTOs, `client` and
//! `local`. Those are registry code that only the registry and its clients use;
//! moving them would be a re-layering, not a de-duplication.
//!
//! # Sharing primitives is not merging policies
//!
//! The `.mfp` fixed prefix is decoded by three readers with deliberately
//! *different* guard sets — see bug-340 B8, recorded at the top of
//! `src/binary_repr/mod.rs`. The manifest reader enforces per-field byte
//! limits, UTF-8 and `validate_package_name`; the registry's requires a
//! non-empty `ident` and applies no name charset guard. Folding them into one
//! decoder would drop trust-boundary guards. They share the primitives in this
//! crate and remain separate policies.

pub mod bytes;
pub mod docpage;
pub mod docs;
pub mod mfp;
pub mod mfpc;
pub mod terminal_safe;
pub mod validation;
