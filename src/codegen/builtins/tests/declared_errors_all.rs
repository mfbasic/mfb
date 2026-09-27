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
        &[],
        &["linux-riscv64"],
        &[],
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

/// `_mfb_rt_fail_status` (plan-158-A, `codegen/error/emission/fail_status.rs`)
/// raises the code its call site passes in, with an OS status appended to the
/// message, parked as an `Error` block. The call site's own code immediate is
/// the member's raise; the helper itself adds only `ErrOutOfMemory` (the park
/// cannot allocate the block).
const AUDIO_PROPAGATING: &[(&str, Propagating)] = &[(
    "_mfb_rt_fail_status",
    Propagating::Known(&["ErrOutOfMemory"]),
)];

#[test]
fn every_audio_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "audio",
        ARENA,
        AUDIO_PROPAGATING,
        NONE_U,
        AUDIO_UNREACHABLE_OWN_RAISE,
        &[],
        &[],
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

/// `collections`' shared map/collection-lifecycle runtime helpers, none of
/// which has a `raise_error_into`/`raise_error_bare` site in its own
/// definition (`lower_map_bucket_put_helper`/`lower_map_probe_helper` in
/// `codegen/engine/builder/mod.rs`, `lower_map_reclaim_helper` in
/// `codegen/collection/map/map_bucket_remove.rs`,
/// `lower_drop_owned_collection_helper` in
/// `codegen/error/emission/park_error_helper.rs`).
const COLLECTIONS_RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_make_error_result",
        "packages an error code the caller already loaded",
    ),
    (
        "_mfb_rt_park_error",
        "parks an error the caller already raised",
    ),
    (
        "_mfb_rt_map_probe",
        "FNV-1a bucket lookup; pure computation, no allocation",
    ),
    (
        "_mfb_rt_map_bucket_put",
        "maintains the incremental bucket index on set; no raise_error site",
    ),
    (
        "_mfb_rt_map_reclaim",
        "packs a map's live payloads in place, returning 0/1; no raise_error site",
    ),
    (
        "_mfb_rt_drop_owned_collection",
        "frees one owned collection slot; never fails",
    ),
];

/// `append`/`prepend` share `lower_collection_end_insert` with `insert`
/// (`gen_mutate.rs`), which range-checks the index before
/// `lower_list_insert_collection` inserts — one inlined instruction stream, no
/// callee boundary, so this is the direct-body counterpart of
/// `UNREACHABLE_VIA_CALLEE` (see `tcp`'s `TCP_UNREACHABLE_OWN_RAISE`). For
/// `append`/`prepend` the index is computed as the list's own current `count`
/// or `0` — both are always within `0..=count`, the exact valid insert range —
/// so the range-check can never fail for them; only `insert`'s caller-supplied
/// index can be out of range.
const COLLECTIONS_UNREACHABLE_OWN_RAISE: &[(&str, &str, &str)] = &[
    (
        "append",
        "ErrIndexOutOfRange",
        "the insertion index is the list's own count -- always == count, so the \
         shared range-check can never fail",
    ),
    (
        "prepend",
        "ErrIndexOutOfRange",
        "the insertion index is the constant 0 -- always within 0..=count, so \
         the shared range-check can never fail",
    ),
];

#[test]
fn every_collections_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "collections",
        COLLECTIONS_RAISES_NOTHING,
        NONE_P,
        NONE_U,
        COLLECTIONS_UNREACHABLE_OWN_RAISE,
        &[],
        &[],
        &[],
        1,
    );
}

/// `astrings::fromString`/`writeSpans` construct an `AttributedString` record,
/// which this sweep's bare `TypeModel::empty()` has no layout for -- the same
/// fixture gap as `canvas`'s `KeyPair`/`Paint`. Repro: run without the skip
/// and read the "native record type 'AttributedString' does not resolve"
/// refusal.
const ASTRINGS_SKIP_MEMBERS: &[&str] = &["fromString", "writeSpans"];

const ASTRINGS_RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_make_error_result",
        "packages an error code the caller already loaded",
    ),
    (
        "_mfb_rt_park_error",
        "parks an error the caller already raised",
    ),
];

/// `io`'s three shared stdin/stdout runtime helpers, none of which has a
/// `raise_error_into`/`raise_error_bare` site of its own -- each returns a
/// status/tag the CALLER (already declaring the corresponding code) raises
/// off, the same `_mfb_arena_alloc` shape `fs_declared_errors.rs` documents.
/// `_mfb_rt_stdin_next_byte`'s own doc comment says as much: "an EOF error
/// result ... or traps ErrInvalidContext" is the value it returns, not a
/// raise it performs.
const IO_RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_rt_io_stdout_drain",
        "drains buffered stdout bytes; no raise_error site of its own",
    ),
    (
        "_mfb_rt_input_decode",
        "decodes buffered bytes into a scalar/grapheme/key value; no raise_error site",
    ),
    (
        "_mfb_rt_stdin_next_byte",
        "returns an EOF/ErrInvalidContext status the caller raises off, per its own \
         doc comment; no raise_error site of its own",
    ),
];

/// `prepend_wrong_mode_gate` (`app/hook/app.rs`) only emits its
/// `ErrWrongMode` raise when `presentation_mode_offset` is `Some` AND the
/// caller's own Rust-level dispatch took the app-mode branch (an `if
/// ctx.build_mode.is_app() { .. }` in `gen_read_line_family.rs`/the reader
/// bodies) -- this sweep's harness fixes `build_mode` to `Console`, so that
/// branch's code is never even emitted, not merely unreached at runtime. Real
/// for an `--app` build; unmeasurable under this sweep's one fixed
/// `build_mode`, the mirror image of `unreachable_own_raise`.
const IO_DECLARED_EXTRA: &[(&str, &str, &str)] = &[
    (
        "input",
        "ErrWrongMode",
        "app-mode branch of gen_read_line_family.rs",
    ),
    (
        "readLine",
        "ErrWrongMode",
        "app-mode branch of gen_read_line_family.rs",
    ),
    ("readRaw", "ErrWrongMode", "app-mode wrong-mode gate"),
    ("readScalar", "ErrWrongMode", "app-mode wrong-mode gate"),
    ("readGrapheme", "ErrWrongMode", "app-mode wrong-mode gate"),
    ("readKey", "ErrWrongMode", "app-mode wrong-mode gate"),
];

#[test]
fn every_io_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "io",
        IO_RAISES_NOTHING,
        NONE_P,
        NONE_U,
        NONE_U,
        IO_DECLARED_EXTRA,
        &[],
        &[],
        1,
    );
}

/// `term::on` subscribes the calling thread to stdin (kitty-protocol probe),
/// reaching the same `_mfb_rt_stdin_next_byte` `io` classifies (an EOF/
/// ErrInvalidContext status the CALLER raises off, per its own doc comment --
/// no raise_error site of its own).
const TERM_RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_rt_stdin_next_byte",
        "returns an EOF/ErrInvalidContext status the caller raises off, per its own \
         doc comment; no raise_error site of its own",
    ),
];

/// `term::off`'s final present calls `term::sync`'s own runtime helper.
const TERM_PROPAGATING: &[(&str, Propagating)] =
    &[("_mfb_rt_term_term_sync", Propagating::Member("sync"))];

/// Every `term::` member goes through `gen_shared.rs::lower_term_helper`,
/// which prepends `prepend_wrong_mode_gate(.., ModeRequirement::Console)` --
/// but the gate's own raise only emits when the Rust-level dispatch takes the
/// app-mode branch, which this sweep's fixed `build_mode: Console` harness
/// never does (the same "real but unmeasured under one fixed build_mode"
/// shape as `IO_DECLARED_EXTRA`; bug-727's own evidence: `terminalSize` on a
/// pipe/non-terminal is `ErrUnsupported`, but every `term::` call in an
/// `--app` build outside `app::Mode.Console` is `ErrWrongMode`).
const TERM_DECLARED_EXTRA: &[(&str, &str, &str)] = &[
    (
        "clear",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "didResize",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "drawBox",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "drawGlyph",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "drawHLine",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "drawText",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "drawVLine",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "enableMouse",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "fillRect",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "getBackground",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "getBold",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "getForeground",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "getUnderline",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "hideCursor",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "isOn",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "moveTo",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "off",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "on",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "pollMouse",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "setBackground",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "setBold",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "setForeground",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "setUnderline",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "showCursor",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "sync",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
    (
        "terminalSize",
        "ErrWrongMode",
        "app-mode ModeRequirement::Console gate (gen_shared.rs)",
    ),
];

#[test]
fn every_term_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "term",
        TERM_RAISES_NOTHING,
        TERM_PROPAGATING,
        NONE_U,
        NONE_U,
        TERM_DECLARED_EXTRA,
        &[],
        &[],
        1,
    );
}

#[test]
fn every_astrings_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "astrings",
        ASTRINGS_RAISES_NOTHING,
        NONE_P,
        NONE_U,
        NONE_U,
        &[],
        &[],
        ASTRINGS_SKIP_MEMBERS,
        1,
    );
}

#[test]
fn every_strings_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "strings",
        ASTRINGS_RAISES_NOTHING,
        NONE_P,
        NONE_U,
        1,
    );
}

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

/// `thread` callees that raise nothing of their own:
/// `_mfb_arena_fill_next`/`_mfb_arena_fill_seed` step/seed the per-arena fill
/// RNG (pure register arithmetic, no call, no error code —
/// `error_constants.rs`'s `ARENA_FILL_NEXT_SYMBOL`/`ARENA_FILL_SEED_SYMBOL`
/// doc comments); `_mfb_rt_thread_block_release` frees a block with no error
/// path (`control_block.rs::lower_thread_block_release` has no
/// `raise_error_into` call in its body); `_mfb_rt_stdin_subscribe`/
/// `_mfb_rt_stdin_unsubscribe` register/unregister the calling thread with the
/// stdin broadcaster (`stdin_broadcast.rs`, neither raises).
const THREAD_RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_arena_fill_next",
        "advances the per-arena fill RNG one step; pure register arithmetic, no call",
    ),
    (
        "_mfb_arena_fill_seed",
        "seeds the per-arena fill RNG from a 64-bit value; pure register arithmetic",
    ),
    (
        "_mfb_rt_thread_block_release",
        "frees a released block; control_block.rs::lower_thread_block_release has \
         no raise_error_into call",
    ),
    (
        "_mfb_rt_stdin_subscribe",
        "registers the calling thread with the stdin broadcaster; no error path",
    ),
    (
        "_mfb_rt_stdin_unsubscribe",
        "unregisters the calling thread from the stdin broadcaster; no error path",
    ),
];

/// `_mfb_rt_thread_table_acquire` and `_mfb_rt_thread_block_arena_create` each
/// take a `platform`/`platform_imports` of their own, so they cannot be
/// re-lowered standalone the way `Propagating::Helper` re-lowers a
/// platform-agnostic helper (fs's path-join helper). Both raise exactly
/// `ErrOutOfMemory`, read directly off their one `raise_error_into` call each
/// (`control_block.rs::lower_thread_table_acquire`,
/// `::lower_thread_block_arena_create`).
const THREAD_PROPAGATING: &[(&str, Propagating)] = &[
    (
        "_mfb_rt_thread_table_acquire",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_rt_thread_block_arena_create",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
];

#[test]
fn every_thread_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises(
        "thread",
        THREAD_RAISES_NOTHING,
        THREAD_PROPAGATING,
        NONE_U,
        1,
    );
}

/// `canvas` internal-only members whose lowering needs a fixture this sweep
/// does not build: a `DrawItem` union tag for the `Text` variant
/// (`carriedHashes`/`itemHash`/`sceneHashes`), a `Paint` record layout
/// (`geoBuild`), and the injected companion source's own globals
/// (`__CANVAS_GEO_DATA`/`__CANVAS_SNAP_ITEMS`/`__CANVAS_SNAP_LAYERS`/
/// `__CANVAS_SNAP_HASHES`/`__CANVAS_TOP_OFFSETS` for `geoFind`/`geoInsert`/
/// `geoForget`/`geoBeginFrame`/`sceneResolve`/`sceneSnapshot`/`snapshotItems`/
/// `snapshotLayers`/`snapshotHashes`/`sceneDrawsFlat`/`sceneLayout`). These
/// members are never called from user source directly (`internal_only:
/// true`, injected-only plumbing for the canvas companion `.mfb`), so no man
/// page renders an Errors table for them — bug-698's Errors-table motivation
/// does not reach them, unlike `publishScene`/`publishLayers` below, which
/// share the same `internal_only: true` flag but DO lower cleanly. Follow-up:
/// building the type-model/global fixtures these fifteen need is tracked as
/// remaining scope, not silently dropped — repro: run this test without this
/// list and read the "the global ... is not declared" / "has no union tag" /
/// "has no layout in the type model" refusals.
const CANVAS_SKIP_MEMBERS: &[&str] = &[
    "carriedHashes",
    "itemHash",
    "sceneHashes",
    "geoBuild",
    "geoFind",
    "geoInsert",
    "geoForget",
    "geoBeginFrame",
    "sceneResolve",
    "sceneSnapshot",
    "snapshotItems",
    "snapshotLayers",
    "snapshotHashes",
    "sceneDrawsFlat",
    "sceneLayout",
];

/// `_mfb_make_error_result`/`_mfb_rt_park_error` are the same shared helpers
/// `fs_declared_errors.rs` classifies (package an already-loaded error code /
/// park an already-raised one — never raise their own). The five platform
/// blit/Metal helpers (`_mfb_gtkapp_canvas_blit`, `_mfb_macapp_canvas_blit`,
/// `_mfb_winapp_canvas_blit`, `_mfb_macapp_metal_draw`,
/// `_mfb_macapp_metal_init`) load no error code and raise nothing in any of
/// their `emit_*` definitions (`linux_gtk/bootstrap.rs`, `win_x86_64/app/
/// mod.rs`, `macos_aarch64/app/{bootstrap,metal}.rs`).
const CANVAS_RAISES_NOTHING: &[(&str, &str)] = &[
    (
        "_mfb_arena_alloc",
        "returns a null pointer; the caller raises ErrOutOfMemory",
    ),
    ("_mfb_arena_free", "releases a block; never fails"),
    (
        "_mfb_make_error_result",
        "packages an error code the caller already loaded",
    ),
    (
        "_mfb_rt_park_error",
        "parks an error the caller already raised",
    ),
    (
        "_mfb_gtkapp_canvas_blit",
        "blits the published scene to the GTK window; no raise_error site",
    ),
    (
        "_mfb_macapp_canvas_blit",
        "blits the published scene to the Cocoa window; no raise_error site",
    ),
    (
        "_mfb_winapp_canvas_blit",
        "blits the published scene to the Win32 window; no raise_error site",
    ),
    (
        "_mfb_macapp_metal_draw",
        "draws the published scene through Metal; no raise_error site",
    ),
    (
        "_mfb_macapp_metal_init",
        "initializes the Metal layer; no raise_error site",
    ),
];

#[test]
fn every_canvas_member_declares_exactly_the_errors_its_lowering_raises() {
    // `canvas`'s GTK-mode hooks hard-`unimplemented!()` on linux-riscv64, the
    // same real, documented plan-05 gap as `app` (see that test).
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "canvas",
        CANVAS_RAISES_NOTHING,
        NONE_P,
        NONE_U,
        NONE_U,
        &[],
        &["linux-riscv64"],
        CANVAS_SKIP_MEMBERS,
        1,
    );
}

/// `crypto::generate`'s native lowering constructs a `KeyPair` record, which
/// this sweep's harness (a bare `TypeModel::empty()`) has no layout for — a
/// fixture gap like `CANVAS_SKIP_MEMBERS`'s, not a real ambiguity: `generate`
/// already declares `errors: vec!["ErrOutOfMemory", "ErrUnknown"]` and is
/// covered by `every_crypto_member_is_admitted_on_every_backend`-style
/// registry-wide tests elsewhere. Repro: run this test without the skip and
/// read the "native record type 'KeyPair' does not resolve" refusal.
const CRYPTO_SKIP_MEMBERS: &[&str] = &["generate"];

/// `hash`/`seal`/`open`/`sign`/`verify` each dispatch at runtime to a fixed
/// `_mfb_ifn_crypto_*` symbol — a companion MFBASIC helper (`add_helper`, not
/// a registry member), so this native-only sweep cannot lower it directly
/// (the general `Body::Mfb` guard bug-698 Phase 1 describes is separate,
/// future scope). Verified directly instead, per symbol below: `grep -n
/// "FAIL " src/codegen/builtins/crypto/helper_*.rs` (note the space — `FAIL
/// error(code, msg)` is a function call, not `FAIL(`) finds every raise site;
/// every helper's own `List OF Byte`/`List OF Integer` result can also raise
/// `ErrOutOfMemory` regardless.
const CRYPTO_IFN_PROPAGATING: &[(&str, Propagating)] = &[
    // The nine SHA cores (`grep -n 'FAIL ' src/codegen/builtins/crypto/helper_sha*.rs
    // src/codegen/builtins/crypto/gen_hash.rs` finds none): only the List OF Byte
    // result can fail, with ErrOutOfMemory.
    (
        "_mfb_ifn_crypto_5Fsha1_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha224_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha256_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha384_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha3_5F224_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha3_5F256_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha3_5F384_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha3_5F512_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fsha512_5Fbytes",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    // `helper_aes256_gcm_seal.rs`: two `FAIL error(77050002, …)` (bad key/nonce
    // length) = ErrInvalidArgument, plus ErrOutOfMemory building the result.
    (
        "_mfb_ifn_crypto_5Faes256GcmSeal",
        Propagating::Known(&["ErrInvalidArgument", "ErrOutOfMemory"]),
    ),
    // `helper_chacha20_poly1305_seal.rs`: same shape, two ErrInvalidArgument FAILs.
    (
        "_mfb_ifn_crypto_5Fchacha20Poly1305Seal",
        Propagating::Known(&["ErrInvalidArgument", "ErrOutOfMemory"]),
    ),
    // `helper_aes256_gcm_open.rs`: two ErrInvalidArgument FAILs (key/nonce length)
    // plus `FAIL error(77050016, …)` (tag mismatch) = ErrAuthenticationFailed.
    (
        "_mfb_ifn_crypto_5Faes256GcmOpen",
        Propagating::Known(&[
            "ErrAuthenticationFailed",
            "ErrInvalidArgument",
            "ErrOutOfMemory",
        ]),
    ),
    // `helper_chacha20_poly1305_open.rs`: same shape.
    (
        "_mfb_ifn_crypto_5Fchacha20Poly1305Open",
        Propagating::Known(&[
            "ErrAuthenticationFailed",
            "ErrInvalidArgument",
            "ErrOutOfMemory",
        ]),
    ),
    // `helper_ed25519_sign.rs`/`helper_ed448_sign.rs`: one ErrInvalidArgument FAIL
    // each (wrong private-key length), plus ErrOutOfMemory building the signature.
    (
        "_mfb_ifn_crypto_5Fed25519Sign",
        Propagating::Known(&["ErrInvalidArgument", "ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fed448Sign",
        Propagating::Known(&["ErrInvalidArgument", "ErrOutOfMemory"]),
    ),
    // `helper_ed25519_verify.rs`/`helper_ed448_verify.rs`: NO `FAIL` — a
    // wrong-length key/signature is `RETURN FALSE`, "never an error" per the
    // ed448 helper's own doc comment. Only ErrOutOfMemory (the working `List OF
    // Integer`/`List OF Byte` values these build).
    (
        "_mfb_ifn_crypto_5Fed25519Verify",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
    (
        "_mfb_ifn_crypto_5Fed448Verify",
        Propagating::Known(&["ErrOutOfMemory"]),
    ),
];

#[test]
fn every_crypto_member_declares_exactly_the_errors_its_lowering_raises() {
    assert_package_declares_exactly_the_errors_its_lowering_raises_on(
        "crypto",
        ARENA,
        CRYPTO_IFN_PROPAGATING,
        NONE_U,
        NONE_U,
        &[],
        &[],
        CRYPTO_SKIP_MEMBERS,
        1,
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
        &[],
        &[],
        1,
    );
}
