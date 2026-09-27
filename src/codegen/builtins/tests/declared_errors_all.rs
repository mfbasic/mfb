//! Every package's native (`AbiFunction`/`AbiInline`) member declares exactly
//! the error codes its lowering can raise (bug-698, generalizing bug-694's
//! `fs`-only guard in `fs_declared_errors.rs`).
//!
//! `io` and `term` are deferred to bug-699's reader/`on`/`off` rewrite
//! (`bugs/bug-722-io-members-raise-errors-while-declaring-none.md`,
//! `bugs/bug-727-term-members-raise-errors-while-declaring-none.md`); they get
//! their own guard once that lands. `fs` already has its own file
//! (`fs_declared_errors.rs`), unchanged by this bug.

use super::declared_errors_sweep::{
    assert_package_declares_exactly_the_errors_its_lowering_raises, Propagating,
};

const NONE_P: &[(&str, Propagating)] = &[];
const NONE_U: &[(&str, &str, &str)] = &[];

/// The two arena helpers every package's allocating members call, common to
/// every native lowering (`fs_declared_errors.rs` classifies the same pair).
const ARENA: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
];

#[test]
fn every_datetime_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "datetime", ARENA, NONE_P, NONE_U, 1,
    );
}

#[test]
fn every_net_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises("net", ARENA, NONE_P, NONE_U, 1);
}

/// `tls::poll` (scalar overload) wraps `tls::read`'s own runtime helper
/// (`_mfb_rt_tls_tls_poll`, plan-76-B: a non-consuming peek at already-buffered
/// app bytes) — a self-referential runtime-helper wrap, the same shape as
/// `fs::readBytes` calling `_mfb_rt_fs_fs_readAllBytes`.
const TLS_PROPAGATING: &[(&str, Propagating)] =
    &[("_mfb_rt_tls_tls_poll", Propagating::Member("poll"))];

#[test]
fn every_tls_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "tls",
        ARENA,
        TLS_PROPAGATING,
        NONE_U,
        1,
    );
}

#[test]
fn every_udp_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises("udp", ARENA, NONE_P, NONE_U, 1);
}
