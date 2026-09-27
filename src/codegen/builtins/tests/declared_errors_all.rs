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
    assert_package_declares_exactly_the_errors_its_lowering_raises,
    assert_package_declares_exactly_the_errors_its_lowering_raises_on, Propagating,
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

/// [`ARENA`] plus `_mfb_rt_validate_utf8` — the shared UTF-8 validation helper
/// (`fs_declared_errors.rs` classifies the same symbol the same way): it
/// returns 0/1 in `x0` and the caller raises `ErrEncoding` off that, so the
/// helper itself never raises.
const ARENA_UTF8: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_rt_validate_utf8",
        "returns 0/1 in x0; the caller raises ErrEncoding",
    ),
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

/// `_mfb_macapp_window_sync_marshal` (macOS `app::setTitle`/`setFullscreen`)
/// asks the main thread to re-sync the window (`performSelectorOnMainThread:`)
/// and returns nothing; it never loads an error code and its only call is
/// external (`_objc_msgSend`, filtered by `library.is_some()` already).
const APP_RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_macapp_window_sync_marshal",
        "asks the main thread to sync the window and returns; it makes no other \
         internal call and loads no error code",
    ),
    (
        "_mfb_macapp_reconcile_marshal",
        "boxes the mode and marshals mfbReconcile: onto the main thread; every \
         call it makes is external (_objc_msgSend) and it loads no error code",
    ),
];

#[test]
fn every_app_member_declares_exactly_the_errors_its_lowering_raises() {
    // `app`'s GTK-mode hooks hard-`unimplemented!()` on linux-riscv64
    // (`AppSupport::Unsupported`, plan-05: aarch64/x86-64 only) — a real,
    // documented platform gap, not something a test-harness hook can supply.
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "app",
        APP_RAISES_NOTHING,
        NONE_P,
        NONE_U,
        NONE_U,
        &["linux-riscv64"],
        1,
    );
}

/// `audio::xruns` shares its runtime body with `poll`/`available`/`pollTimeout`
/// (`dispatch_query`/`Query`), whose ALSA/Windows lowerings emit one shared
/// `<symbol>_unavailable: raise ErrAudioUnavailable` epilogue label — but
/// `Query::Xruns`'s own arm never branches to it
/// (`gen_alsa_io.rs::lower_query`'s `Query::Xruns` arm falls straight to
/// `clamp`, and the equivalent Windows/macOS arms never touch the device at
/// all), so the immediate is present in the emitted instructions yet
/// unreachable for this member. This is the direct-body counterpart of
/// `UNREACHABLE_VIA_CALLEE` in `fs_declared_errors.rs`, verified the same way:
/// read the source, not re-derived by the sweep.
const AUDIO_UNREACHABLE_OWN_RAISE: &[(&str, &str, &str)] = &[(
    "xruns",
    "ErrAudioUnavailable",
    "shares dispatch_query's shared epilogue label with poll/available, but its \
     own Query::Xruns arm never branches to it on any backend",
)];

#[test]
fn every_audio_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "audio",
        ARENA,
        NONE_P,
        NONE_U,
        AUDIO_UNREACHABLE_OWN_RAISE,
        &[],
        1,
    );
}

/// `tcp::listen` shares `lower_net_endpoint_helper` with `tcp::connect`
/// (`os/socket/shared.rs`, `listen: bool` parameter). The non-blocking-connect
/// + `poll` sequence that ends in the shared `connect_timeout` label (which
/// raises `ErrTimeout`) lives entirely in that function's `else` (connect) arm
/// — the `if listen { .. bind/listen syscalls .. }` arm never branches there —
/// so the immediate is present in `listen`'s emitted instructions (one
/// function body, one label) but structurally unreachable for it.
const TCP_UNREACHABLE_OWN_RAISE: &[(&str, &str, &str)] = &[(
    "listen",
    "ErrTimeout",
    "lower_net_endpoint_helper's `else` (connect) arm is the only one that ever \
     branches to the shared connect_timeout/ErrTimeout label; the `if listen` \
     arm (bind + listen(2)) does not",
)];

#[test]
fn every_big_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises("big", ARENA, NONE_P, NONE_U, 1);
}

#[test]
fn every_os_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "os", ARENA_UTF8, NONE_P, NONE_U, 1,
    );
}

#[test]
fn every_process_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "process", ARENA_UTF8, NONE_P, NONE_U, 1,
    );
}

#[test]
fn every_thread_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "thread", ARENA, NONE_P, NONE_U, 1,
    );
}

#[test]
fn every_canvas_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "canvas", ARENA, NONE_P, NONE_U, 1,
    );
}

#[test]
fn every_crypto_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "crypto", ARENA, NONE_P, NONE_U, 1,
    );
}

#[test]
fn every_tcp_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "tcp",
        ARENA,
        NONE_P,
        NONE_U,
        TCP_UNREACHABLE_OWN_RAISE,
        &[],
        1,
    );
}
