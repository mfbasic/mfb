//! Shared codegen constants: the `Result`/`Error` calling protocol, the error
//! catalog, runtime-helper symbol names, and the byte layouts of every runtime
//! record (arena state, closures, resources/`File`, collections). Grouped by
//! concern; `const` items may reference one another regardless of order, so the
//! layout chains below are written in ascending-offset (dependency) order.

// --- codegen tier imports (migration) ---
use crate::codegen::engine::operand::*;
use crate::target::shared::abi;
// ===========================================================================
// Result / Error calling protocol
// ===========================================================================

pub(crate) const RESULT_OK_TAG: &str = "0";
pub(crate) const RESULT_ERR_TAG: &str = "1";
pub(crate) const RESULT_PROGRAM_EXIT_TAG: &str = "2";
/// Error result whose owned Error block is parked in the per-thread
/// `ARENA_CURRENT_ERROR_OFFSET` slot for the catcher to ADOPT rather than rebuild
/// (plan-error-block-in-slot / design "b"). Treated exactly like `RESULT_ERR_TAG`
/// by every "is it not Ok?" test (`tag != 0`); only the trap route distinguishes
/// it, adopting the parked block instead of calling `emit_build_error_inline`.
/// During migration, producers that still emit the legacy loose-register error use
/// `RESULT_ERR_TAG` and the trap route rebuilds as before — never a stale slot.
pub(crate) const RESULT_ERR_BLOCK_TAG: &str = "3";

pub(crate) const RESULT_TAG_REGISTER: Operand = abi::mfb_return(0);
pub(crate) const RESULT_VALUE_REGISTER: Operand = abi::mfb_return(1);
pub(crate) const RESULT_ERROR_MESSAGE_REGISTER: Operand = abi::mfb_return(2);
/// Fourth error-result register: pointer to the `ErrorLoc` recording where the
/// error originated. Carried alongside code (x1) and message (x2) so propagation
/// preserves the origin and trap materialization can build a 3-field `Error`.
pub(crate) const RESULT_ERROR_SOURCE_REGISTER: Operand = abi::mfb_return(3);

/// Byte size of an allocated `Error` record: code(+0), message(+8), source(+16).
pub(crate) const ERROR_OBJECT_SIZE: usize = 24;
/// Byte size of an allocated `ErrorLoc` record: filename(+0), line(+8), char(+16).
pub(crate) const ERROR_LOC_OBJECT_SIZE: usize = 24;

/// Out-of-line `ErrorLoc` builder (plan-16). `x0 = filename String*`,
/// `x1 = line`, `x2 = char`; returns `x0 = ErrorLoc*`, or `x0 = 0` on OOM. One
/// shared copy replaces the ~48-instruction block formerly inlined at every trap
/// site (`emit_build_error_loc`).
pub(crate) const BUILD_ERROR_LOC_SYMBOL: &str = "_mfb_build_error_loc";
/// Out-of-line error-Result assembler (plan-16 "option 2"). Takes the
/// `_mfb_build_error_loc` inputs plus the code/message and lands the standard
/// error `Result` in its return registers: `x0 = filename`, `x1 = line`,
/// `x2 = char`, `x3 = code`, `x4 = message String*` in; returns
/// `x0 = RESULT_ERR_TAG`, `x1 = code`, `x2 = message`, `x3 = ErrorLoc*`. Collapses
/// the per-trap-site register shuffle (`emit_error_register_return`) to a call.
pub(crate) const MAKE_ERROR_RESULT_SYMBOL: &str = "_mfb_make_error_result";

// ===========================================================================
// Error catalog — (code, message, symbol) triples, ascending by code
// ===========================================================================

// -- Memory (7701) ----------------------------------------------------------

// -- I/O (7702) -------------------------------------------------------------

// -- Filesystem / resource / native link (7703) -----------------------------
/// Reported instead of `ErrResourceClosed` when a guard finds `RESOURCE_MOVED_BIT`
/// set — the handle was `thread::transfer`red away, so "already closed" would be a
/// misleading account of why it is unusable.
/// sec-02: a native `LINK` `OUT CBuffer` callee wrote past the declared `SIZE`
/// bytes, caught by a post-call canary in the buffer's guard region. The overrun
/// already touched memory; the thunk traps deterministically instead of
/// continuing on corrupted arena state (converting a silent heap smash into an
/// abort). The author's `BUFFER … SIZE` must be >= the callee's maximum write.

// -- General runtime (7705) -------------------------------------------------
/// The single expiry error for every builtin that can wait. Under the timeout
/// convention (spec `language builtin-functions` → "Timeout convention"), a
/// producing call (`accept`/`connect`/`receive`/`transfer`/`send`, or a
/// read/write past its socket timeout) that reaches its deadline — including an
/// explicit `timeoutMs` of `0` when the event is not already available — raises
/// exactly this code. It replaced `ErrNotFound` for thread `receive`/`accept` at
/// `0` (plan-73-A) and the retired `ErrReadTimeout`/`ErrWriteTimeout` for net
/// read/write (plan-73-C).

/// Convention-level "wait unbounded" sentinel for the optional trailing
/// `timeoutMs AS Integer` shared by every waiting builtin (thread/net/tls/audio).
/// It is the `i64::MIN` bit pattern spelled as the unsigned decimal the immediate
/// encoder parses (the encoder reads `u64`, so a signed `-9223372036854775808`
/// cannot be spelled directly). The lowering pads an *omitted* `timeoutMs` with
/// this value; each family's wait helper routes it to the block-forever path and
/// rejects every *other* negative `timeoutMs` with `ErrInvalidArgument`, so no
/// user-supplied value (always `>= 0`) can collide with it. See the "Timeout
/// convention" spec section (`mfb spec language builtin-functions`).
pub(crate) const TIMEOUT_UNBOUNDED_SENTINEL: &str = "9223372036854775808";
// Audio (plan-33-A §7). Raised by the plan-33-B/C backend helper bodies; the
// registry rows in `02_error-codes.md` land with plan-33-A so `errorCode::`
// resolves. `77050016` is `ErrAuthenticationFailed` (crypto).
// Invalid context (plan-15 D1): a thread that has not called `thread::openStdIn`
// tried to read stdin (the compiler-inserted main subscription exempts a normal
// single-threaded program).
// plan-62-E: `term::*` and the console-reading `io::` calls require the `Console`
// presentation mode in an `--app` build; outside it they raise this trappable error.

// -- Network (7707) ---------------------------------------------------------
// plan-73-C: `ErrReadTimeout` (77070005) and `ErrWriteTimeout` (77070006) are
// RETIRED — every net read/write timeout now raises the single `ErrTimeout`
// (77050008), per the language timeout convention.

// -- Process (7708) ---------------------------------------------------------
// plan-90-A: `process::spawn`/`shell` raise `ErrSpawnFailed` when the child
// cannot be created or `execvp`'d (the child reports the failure to the parent
// over a close-on-exec self-pipe). Operating on a dropped `Process` raises the
// shared `ErrResourceClosed` (7703); an empty `args` list raises the shared
// `ErrInvalidArgument` (7705).
// plan-88: `ErrSpawnFailed` (77080001) lives in `ERRORCODE_CONSTANTS` (the single
// metadata source) like every other runtime error; no `ERR_SPAWN_FAILED_*` codegen
// consts — the process codegen sources code/message/symbol from the table.

// ===========================================================================
// Entry-point & cleanup-failure diagnostic strings
// ===========================================================================

// Untrapped-error / cleanup-failure banners share one shape (doc
// `diagnostics 02_error-codes.md`): `<label> <G-SSS-EEEE>\n<message>\n`. The code
// is printed on the label line (canonical hyphenated form) and the message stands
// alone on the next line, so there is no inline separator string — both paths emit
// `ENTRY_ERROR_NEWLINE` between the code and the message.
pub(crate) const ENTRY_ERROR_PREFIX: &str = "Error: ";
pub(crate) const ENTRY_ERROR_PREFIX_SYMBOL: &str = "_mfb_str_entry_error_prefix";
pub(crate) const ENTRY_ERROR_NEWLINE: &str = "\n";
pub(crate) const ENTRY_ERROR_NEWLINE_SYMBOL: &str = "_mfb_str_entry_error_newline";
pub(crate) const CLEANUP_FAILURE_PREFIX: &str = "Cleanup failure: ";
pub(crate) const CLEANUP_FAILURE_PREFIX_SYMBOL: &str = "_mfb_str_cleanup_failure_prefix";

// ===========================================================================
// Neutral register tokens
// ===========================================================================

/// The arena-state pointer as shared code names it — the neutral `arena_base`
/// token (plan-34-A). Each backend's selection realizes it to a physical
/// register (AArch64 x19 via `regmodel::ARENA_BASE_REGISTER`, RISC-V s11,
/// x86-64 r15); shared lowering never spells the AArch64 register number.
pub(crate) const ARENA_STATE_REGISTER: &str = crate::target::shared::abi::ARENA;
pub(crate) const CLOSURE_ENV_REGISTER: &str = crate::target::shared::abi::CLOSURE_ENV;

// ===========================================================================
// Closures
// ===========================================================================

pub(crate) const CLOSURE_OBJECT_SIZE: usize = 16;
pub(crate) const CLOSURE_OFFSET_CODE: usize = 0;
pub(crate) const CLOSURE_OFFSET_ENV: usize = 8;

/// The static closure-descriptor data symbol for a function referenced as a
/// no-capture function value. One `{code, env=0}` descriptor per function, in
/// BSS, its `code` word populated once at startup — so a `FunctionRef` loads this
/// address instead of arena-allocating a fresh descriptor on every evaluation
/// (bug-78). `func_symbol` is already a valid symbol, so concatenation is unique.
pub(crate) fn closure_descriptor_symbol(func_symbol: &str) -> String {
    format!("_mfb_closure_desc_{func_symbol}")
}

/// The startup function that populates every static closure descriptor's `code`
/// word (bug-78). Run once from the entry before `main`.
pub(crate) const CLOSURE_DESC_INIT_SYMBOL: &str = "_mfb_closure_desc_init";

// ===========================================================================
// Program-entry frame & process lifecycle
// ===========================================================================

/// One in-frame scratch word between the arena state (0..`ARENA_STATE_SIZE`)
/// and the globals (`ENTRY_STACK_SIZE`..): the RNG-seed block's `getentropy`
/// buffer.
pub(crate) const ENTRY_SEED_SCRATCH_OFFSET: usize = ARENA_STATE_SIZE;
/// Entry-frame prefix: the arena state plus the one seed-scratch word after it.
/// Derived from `ARENA_STATE_SIZE` so the frame tracks arena-state growth
/// (e.g. the allocator-01 quick bins) automatically.
pub(crate) const ENTRY_STACK_SIZE: usize = ENTRY_SEED_SCRATCH_OFFSET + 8;
pub(crate) const ENTRY_GLOBALS_OFFSET: usize = ENTRY_STACK_SIZE;
/// Size of the args region appended to the entry frame for an arg-accepting
/// entry: five 8-byte slots (argc, argv, args list, data length, saved count),
/// rounded up to the 16-byte frame granule. The region sits ABOVE the globals
/// (at `entry_stack_size - ENTRY_ARGS_REGION_SIZE`); the old fixed offsets at
/// 104..144 overlapped the first four global slots and, for a program with no
/// globals, spilled past the frame — silently-scratch memory on macOS, but the
/// OS argc/argv words themselves at a raw Linux ELF entry.
pub(crate) const ENTRY_ARGS_REGION_SIZE: usize = 48;

/// macOS app mode (plan-04-macos-app.md §6.6): the standard program-entry logic
/// (arena setup + language `main` + exit) is emitted under this symbol and runs
/// on the worker thread, while `_main` is the AppKit bootstrap.
pub(crate) const MACAPP_PROGRAM_SYMBOL: &str = "_mfb_macapp_program";

/// Shared process-teardown routine: restores the terminal (when `term::` is used)
/// and frees the main arena, then returns. Called both after the entry FUNC/SUB
/// finishes and from the SIGINT/SIGTERM handler, so the cleanup is identical on a
/// normal exit and a signal kill. It locates the arena through
/// `MAIN_ARENA_GLOBAL_SYMBOL` (not `x19`) so it works from a signal handler whose
/// `x19` belongs to the interrupted code.
pub(crate) const SHUTDOWN_SYMBOL: &str = "_mfb_shutdown";
/// `void handler(int signo)` installed for SIGINT/SIGTERM in console programs. It
/// runs `_mfb_shutdown` and then `_exit(128 + signo)`; it never returns.
pub(crate) const SIGNAL_HANDLER_SYMBOL: &str = "_mfb_rt_signal_handler";
/// One writable 8-byte global holding the main thread's arena-state address,
/// stored at program startup. The signal handler and `_mfb_shutdown` read it to
/// find the arena without relying on the pinned `x19` (which is unavailable on a
/// signal frame). Per-thread worker arenas are intentionally not tracked here —
/// they are never freed by us anyway (the entry only ever frees the main arena).
pub(crate) const MAIN_ARENA_GLOBAL_SYMBOL: &str = "_mfb_rt_main_arena";

/// plan-32-A: a writable process-global byte (padded to 8) recording whether the
/// CPU has the RISC-V Vector "V" extension, probed once at startup from the ELF
/// aux vector (`AT_HWCAP` bit 21). The dual-path `v128` lowering (plan-32-C)
/// branches on it: `1` selects the native-RVV arm, `0` the scalar arm — so a
/// single `linux-riscv64` binary runs correctly on both V and non-V chips.
/// Emitted (and scanned) only for a **`linux-riscv64` entry** module, so every
/// other target stays byte-identical.
pub(crate) const HAS_RVV_GLOBAL_SYMBOL: &str = "_mfb_rt_has_rvv";

/// plan-67-B: one writable 8-byte global holding the base pointer of the runtime
/// perf-tracking region (the `emit_arena_map`-mmap'd system memory that holds the
/// name-keyed timing tables). Mirrors `MAIN_ARENA_GLOBAL_SYMBOL`: a zeroed
/// `kind:"raw"` object emitted only for a **`--cfg perf` macOS entry** module
/// (gated by `perf_injection_enabled()`), so ordinary and non-macOS plans never
/// see it. `perf_init` stores the region base here; every other perf helper loads
/// it and treats a 0 base as "perf inert" (mmap failed / perf-free build).
pub(crate) const PERF_STATE_SYMBOL: &str = "_mfb_rt_perf_state";

/// plan-67-B: the `mfb.string.v1` object holding the perf-table header line
/// (`name count avg median min max sum`), written to stderr by `perf_done`.
/// Emitted under the same `--cfg perf`-macOS-entry gate as `PERF_STATE_SYMBOL`.
pub(crate) const PERF_HEADER_SYMBOL: &str = "_mfb_rt_perf_header";

/// plan-67-C: the `mfb.string.v1` name object for the whole-program span. The
/// entry loads its address as `perf_start`'s `namePtr`; because every injection of
/// a given name references the one symbol, table B can key on pointer identity.
/// (plan-67-F emits one such object per instrumented arena region.)
pub(crate) const PERF_NAME_PROGRAM_SYMBOL: &str = "_mfb_rt_perf_name_program";

/// plan-67-D: pseudo-name objects for the two diagnostic counters `perf_done`
/// prints (only when non-zero, so normal output stays clean): `mismatch` counts a
/// `perf_end` with no open `perf_start`, `overflow` counts samples dropped because
/// the 16 MiB region filled.
pub(crate) const PERF_NAME_MISMATCH_SYMBOL: &str = "_mfb_rt_perf_name_mismatch";
pub(crate) const PERF_NAME_OVERFLOW_SYMBOL: &str = "_mfb_rt_perf_name_overflow";

/// plan-67-F: name objects for the instrumented arena hot-path regions. Each
/// arena helper's body is bracketed with `perf_start`/`perf_end` on the debug
/// macOS backend, so the exit table shows a timing row per region.
pub(crate) const PERF_NAME_MFB_ALLOC_SYMBOL: &str = "_mfb_rt_perf_name_mfb_alloc";
pub(crate) const PERF_NAME_MFB_FREE_SYMBOL: &str = "_mfb_rt_perf_name_mfb_free";

// ===========================================================================
// term:: TUI state slots (reserved in the program-entry frame)
// ===========================================================================

/// `term::` TUI-mode state slots reserved in the program-entry frame just past
/// the program globals and `LINK` slots (plan-01-term.md §6.2). The first eight
/// `u64` slots hold: active, packed foreground, packed background, bold,
/// underline, cursor-visible, and two reserved for the app backend. The
/// remaining slots (bug-149) hold the console single-key-mode state: a flag that
/// records whether `term::on` put the console tty into raw/cbreak mode, plus two
/// persistent `termios` save buffers (the saved cooked line-discipline restored
/// by `term::off` and by `io::input`/`io::readLine` for their own read, and the
/// raw discipline re-applied afterward). Zero-initialized by the entry's
/// global-slot clear, which is the inert (TUI-off, raw-inactive) default.
/// `LineStyle` ordinal (0..=6, in `term_package.mfb` order: Light, Heavy,
/// LightDash, HeavyDash, LightDot, HeavyDot, Double) → the box-drawing code point
/// `term::drawHLine` (horizontal form) and `term::drawVLine` (vertical form)
/// stamp into a cell. Shared by the console backend (which packs the code point's
/// UTF-8 bytes into the grid `glyph`) and the macOS app backend (which stores the
/// code point directly as a `unichar`), so the two can never drift.
pub(crate) const TERM_HLINE_CODEPOINTS: [u32; 7] =
    [0x2500, 0x2501, 0x2504, 0x2505, 0x2508, 0x2509, 0x2550];
pub(crate) const TERM_VLINE_CODEPOINTS: [u32; 7] =
    [0x2502, 0x2503, 0x2506, 0x2507, 0x250A, 0x250B, 0x2551];

/// `term::drawBox` corner code points, indexed by `LineStyle` ordinal (same order
/// as above). Dash/dot styles have no dashed corner glyphs, so they reuse the
/// Light (`Light`/`LightDash`/`LightDot`) or Heavy (`Heavy`/`HeavyDash`/`HeavyDot`)
/// corners; `Double` uses the double corners. One table per corner position so the
/// glyph is selected by the same ordinal chain the edge glyphs use.
///   weight per ordinal: 0,2,4 → Light   1,3,5 → Heavy   6 → Double
/// TL ┌┏╔  TR ┐┓╗  BL └┗╚  BR ┘┛╝
pub(crate) const TERM_CORNER_TL_CODEPOINTS: [u32; 7] =
    [0x250C, 0x250F, 0x250C, 0x250F, 0x250C, 0x250F, 0x2554];
pub(crate) const TERM_CORNER_TR_CODEPOINTS: [u32; 7] =
    [0x2510, 0x2513, 0x2510, 0x2513, 0x2510, 0x2513, 0x2557];
pub(crate) const TERM_CORNER_BL_CODEPOINTS: [u32; 7] =
    [0x2514, 0x2517, 0x2514, 0x2517, 0x2514, 0x2517, 0x255A];
pub(crate) const TERM_CORNER_BR_CODEPOINTS: [u32; 7] =
    [0x2518, 0x251B, 0x2518, 0x251B, 0x2518, 0x251B, 0x255D];

/// `term::fillRect` block/shade code points, indexed by `FillStyle` ordinal:
/// `Filled` █, `Light` ░, `Medium` ▒, `Dark` ▓, `Checker` ▚, `CheckerAlt` ▞.
pub(crate) const TERM_FILL_CODEPOINTS: [u32; 6] = [0x2588, 0x2591, 0x2592, 0x2593, 0x259A, 0x259E];

pub(crate) const TERM_STATE_ACTIVE_OFFSET: usize = 0;
pub(crate) const TERM_STATE_FG_OFFSET: usize = 8;
pub(crate) const TERM_STATE_BG_OFFSET: usize = 16;
pub(crate) const TERM_STATE_BOLD_OFFSET: usize = 24;
pub(crate) const TERM_STATE_UNDERLINE_OFFSET: usize = 32;
pub(crate) const TERM_STATE_CURSOR_VISIBLE_OFFSET: usize = 40;
/// Cached "terminal was resized" flag (planning/term.md #11). Set to 1 whenever a
/// genuine terminal/window size change is detected — the shared CLI reflow
/// (`term_grid::emit_grid_resize`) and each app backend's resize hook — and
/// cleared (read-and-cleared) by `term::didResize()`, so the flag latches until
/// the program observes it. Offset 56 is the free slot between the grid pointer
/// (48) and the raw-active flag (64). App backends that record resize on their own
/// surface state mirror it here so the shared getter stays correct.
pub(crate) const TERM_STATE_DID_RESIZE_OFFSET: usize = 56;
/// Console single-key (raw/cbreak) mode: set to 1 by `term::on` once it has put
/// stdin into `~ICANON`/`~ECHO`/`VMIN=1`/`VTIME=0` (bug-149); 0 while the tty is
/// in its saved line discipline (never a tty, or `term::off` already restored
/// it). `io::input`/`io::readLine` consult it to decide whether to bracket their
/// read with a cooked-mode restore. App backends do not use it.
pub(crate) const TERM_STATE_RAW_ACTIVE_OFFSET: usize = 64;
/// Persistent save buffer for the tty's cooked/line `termios` (captured by
/// `term::on`, restored by `term::off` and temporarily by `io::input`/
/// `io::readLine`). Sized for the largest supported `termios` (macOS = 72 bytes).
pub(crate) const TERM_STATE_COOKED_TERMIOS_OFFSET: usize = 72;
/// Persistent save buffer for the derived raw/cbreak `termios` (built by
/// `term::on`, re-applied by `io::input`/`io::readLine` after their line read).
pub(crate) const TERM_STATE_RAW_TERMIOS_OFFSET: usize = 144;
/// Total reserved slots: through the raw `termios` buffer (144 + 72 = 216 bytes).
pub(crate) const TERM_STATE_SLOTS: usize = (TERM_STATE_RAW_TERMIOS_OFFSET + 72) / 8;

/// plan-62-B: the per-arena presentation-mode word (`app::Mode` discriminant:
/// `0 = Console`, `1 = None`). Reserved as one 8-byte slot in the program-entry
/// frame just past the `term::` state region, addressed off the pinned arena-state
/// register — the same threading model as `term::` state. Only reserved in app
/// builds (a console build has no `app::Mode`), so a console binary is unchanged.
/// `_mfb_rt_app_get_mode` loads it and `_mfb_rt_app_set_mode` stores it.
pub(crate) const PRESENTATION_MODE_SLOTS: usize = 1;

// plan-98-B: the per-arena **canvas scene** region — the retained scene
// `canvas::present` publishes. Reserved just past the presentation-mode word, on the
// same pinned arena-state register, and only when the program uses `canvas::`.
//
// It lives in the arena rather than in a writable global because the arena region is
// per-execution-context and its slot offsets are already threaded to every runtime
// helper; a global would have to be declared per target.
//
// The scene must outlive the `present` call that publishes it — the renderer reads it
// on vsync/resize/damage until the next `present` — which the arena satisfies: it is
// a growing bump region owned by the execution context, not a call frame.
/// Byte offset (within the canvas region) of the publish counter, incremented on
/// every `present` that actually changes the scene. A `present` of an identical
/// scene leaves it alone, which is how "re-presenting unchanged content is free"
/// becomes observable.
pub(crate) const CANVAS_SCENE_REVISION_OFFSET: usize = 0;
/// Byte offset of the item count of the published scene.
pub(crate) const CANVAS_SCENE_COUNT_OFFSET: usize = 8;
/// Byte offset of the pointer to the published items — a deep copy of the caller's
/// `List OF DrawItem`, owned by the arena, pointing at nothing caller-owned.
pub(crate) const CANVAS_SCENE_ITEMS_OFFSET: usize = 16;
/// Byte offset of the pointer to the per-item content hashes (plan-98-B Phase 3).
pub(crate) const CANVAS_SCENE_HASHES_OFFSET: usize = 24;
/// Total reserved slots for the canvas scene region.
pub(crate) const CANVAS_SCENE_SLOTS: usize = (CANVAS_SCENE_HASHES_OFFSET + 8) / 8;

// ===========================================================================
// Arena state layout (ascending offset) & allocator
// ===========================================================================

/// Back-pointer from a thread's arena state to its thread control block (plan-99),
/// stored in the reserved arena-state word at offset 8. The worker trampoline
/// publishes it right after both pinned registers are live; the main thread never
/// writes it, so the whole-`ARENA_STATE_SIZE` zero-init leaves it `0` there. That
/// makes `[arena+8]` the reliable "am I a worker, and if so which one" test on
/// every thread — `os::sleep` reads it to pick between a plain `nanosleep` delay
/// and the cancellation-aware condvar wait, and the non-zero value it reads IS the
/// TCB that wait needs. The pinned current-thread register (`abi::CURRENT_THREAD`)
/// cannot serve: the entry stub reuses it as scratch on the main thread.
pub(crate) const ARENA_WORKER_THREAD_OFFSET: usize = 8;
pub(crate) const ARENA_CLEANUP_FAILURE_COUNT_OFFSET: usize = 64;
pub(crate) const ARENA_CLEANUP_FAILURE_CODE_OFFSET: usize = 72;
pub(crate) const ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET: usize = 80;
/// Dedicated per-arena memory-fill PCG64 state, reusing the two reserved
/// arena-state words at offsets 16/24. This stream is **separate** from the
/// language RNG at 88/96 (`math::rand`): it is seeded independently and its
/// output is never observable (filled bytes are always overwritten before any
/// read), so advancing it on every alloc/free never perturbs `math::rand`'s
/// reproducible sequence.
pub(crate) const ARENA_FILL_RNG_LO_OFFSET: usize = 16;
pub(crate) const ARENA_FILL_RNG_HI_OFFSET: usize = 24;
/// Arena start time in nanoseconds (reserved word at offset 40). Captured once at
/// arena init for lightweight diagnostics and mixed into the fill-RNG seed so two
/// arenas seeding in the same instant (or after a `getentropy` failure) still get
/// distinct poison streams.
pub(crate) const ARENA_START_TIME_OFFSET: usize = 40;
/// Per-arena address-ordered coalescing free-list head (lowest-address free
/// chunk, 0 when empty). Stored in the reserved arena-state word at offset 48
/// (`memory_layouts.md` Arenas §). The list subsumes the old bump pointer: a
/// freshly mapped block's usable region is inserted as one big free chunk and
/// `arena_alloc` carves allocations out of it (first-fit + split), while
/// `arena_free` returns chunks and coalesces with address-adjacent neighbors.
pub(crate) const ARENA_FREE_LIST_HEAD_OFFSET: usize = 48;
/// Per-arena (per-thread) Money rounding mode (plan-29-D): `0 = Commercial`
/// (round-half-away-from-zero, the default), `1 = Banker` (round-half-to-even).
/// Stored in the reserved arena-state word at offset 56 so the zero-init clear
/// gives the Commercial default with no extra init code; a child thread inherits
/// the parent's mode at spawn (copied beside the RNG-seed derivation).
pub(crate) const ARENA_ROUNDING_MODE_OFFSET: usize = 56;
/// Per-arena (per-thread) PCG64 random-number generator state. Each OS thread
/// owns its own arena, so storing the 128-bit RNG state in the arena gives every
/// thread an independent stream reachable through the pinned arena register
/// (`x19`) without a thread-local lookup. Appended past the cleanup-audit fields
/// so the historical 0..88 layout is unchanged for programs that never seed.
pub(crate) const ARENA_RNG_STATE_LO_OFFSET: usize = 88;
pub(crate) const ARENA_RNG_STATE_HI_OFFSET: usize = 96;
/// Per-size-class quick bins (allocator-01): `ARENA_QUICK_BIN_COUNT` singly
/// linked bin heads for exact chunk sizes 16, 32, …, `ARENA_QUICK_BIN_MAX`
/// (granule 16; class index `size/16 - 1`), appended to the arena state after
/// the historical 104 bytes. A freed chunk ≤ `ARENA_QUICK_BIN_MAX` parks on its
/// bin (O(1) push) and the next same-class allocation pops it (O(1)); bins
/// drain through the coalescing insert before the arena grows
/// (flush-before-grow), so parked memory never forces a map. Bin nodes reuse
/// the `FreeNode {next@0, size@8}` overlay.
pub(crate) const ARENA_QUICK_BIN_BASE_OFFSET: usize = 104;
pub(crate) const ARENA_QUICK_BIN_COUNT: usize = 128;
pub(crate) const ARENA_QUICK_BIN_MAX: u64 = 2048;
/// Designated-victim carve chunk (allocator-01): one active chunk that
/// bump-serves small bin misses (`ptr`/`size` pair). Splitting parked bin
/// inventory on every miss shaves it into sub-class crumbs (measured); the
/// DV concentrates all small-miss carving in one chunk, dlmalloc-style.
pub(crate) const ARENA_CARVE_PTR_OFFSET: usize =
    ARENA_QUICK_BIN_BASE_OFFSET + ARENA_QUICK_BIN_COUNT * 8;
pub(crate) const ARENA_CARVE_SIZE_OFFSET: usize = ARENA_CARVE_PTR_OFFSET + 8;
/// Opt-in stdout output buffer (plan-14-A), three per-arena (per-thread) words
/// appended after the allocator carve chunk. `OUT_ENABLED` is 0 (off) by default
/// — the entry / thread-spawn arena-state zeroing clears all three, so a program
/// that never calls `io::setBuffered(TRUE)` sees `OUT_ENABLED = 0` and takes the
/// unbuffered direct-write path (byte-identical to pre-plan-14). `OUT_PTR` is the
/// lazily-allocated 4 KiB buffer (NULL until the first buffered write) and
/// `OUT_FILLED` counts the pending bytes held in it.
pub(crate) const ARENA_OUT_PTR_OFFSET: usize = ARENA_CARVE_SIZE_OFFSET + 8;
pub(crate) const ARENA_OUT_FILLED_OFFSET: usize = ARENA_OUT_PTR_OFFSET + 8;
pub(crate) const ARENA_OUT_ENABLED_OFFSET: usize = ARENA_OUT_FILLED_OFFSET + 8;
/// Segregated large-block bins (plan-25-A): `ARENA_LARGE_BIN_COUNT` singly linked
/// bin heads, hashed by the chunk's exact byte size, for chunks *larger* than
/// `ARENA_QUICK_BIN_MAX` (which the 128 direct-indexed quick bins cannot cover
/// without a bin per 16-byte class). A large free pushes its chunk onto
/// `large_bin[(size >> 4) & (COUNT - 1)]` in O(1); a same-size large alloc scans
/// that one short bin list for an *exact*-size node and pops it in O(1)
/// amortized — so repetitive large-list churn (the benchmark's poison: a
/// 1000-element `List` frees ~40 KB per op) never walks the address-ordered
/// free-list. Bin nodes reuse the `FreeNode {next@0, size@8}` overlay; the count
/// is a power of two so the index is a mask, not a modulo. Appended after the
/// stdout-buffer words so every historical offset is unchanged.
pub(crate) const ARENA_LARGE_BIN_COUNT: usize = 64;
pub(crate) const ARENA_LARGE_BIN_BASE_OFFSET: usize = ARENA_OUT_ENABLED_OFFSET + 8;
/// Per-thread rv64 `v128` scalarization slot region (bug-122). RV64GC has no
/// 128-bit registers, so the neutral `v128` ops (the transcendental/`vector::`
/// math kernels) stage their lanes in memory. That region was a single process
/// **global**, which two OS threads running v128 kernels concurrently corrupted.
/// Reserving it inside the per-thread arena state — addressed off the pinned
/// per-thread arena base (`s11`) — gives every thread its own slots. 127 slots ×
/// 16 bytes matches `arch::riscv64::v128::SLOT_COUNT`. The region is reserved
/// uniformly (all targets) so the arena-state layout stays target-independent;
/// only rv64 codegen addresses it. Placed last so `ARENA_V128_SLOTS_OFFSET`
/// stays within the rv64 12-bit `addi` immediate (±2047).
///
/// bug-381: the region held 128 slots; one was reclaimed for
/// [`ARENA_FLAG_RHS_OFFSET`] below, so `ARENA_STATE_SIZE` (and every target's
/// arena-clear) is byte-for-byte unchanged. `arch::riscv64::v128::SLOT_COUNT`
/// dropped to 127 in lockstep.
pub(crate) const ARENA_V128_SLOTS_OFFSET: usize =
    ARENA_LARGE_BIN_BASE_OFFSET + ARENA_LARGE_BIN_COUNT * 8;
pub(crate) const ARENA_V128_SLOTS_SIZE: usize = 127 * 16;
/// Per-thread rv64 flag-emulation rhs snapshot (bug-381). RISC-V has no condition
/// flags, so a bare (non-fused) integer compare whose flag-reading branch is not
/// adjacent must keep its *compared values* live across the span — but under
/// register pressure the allocator spills/reloads a compare operand between the
/// compare and its branch, stranding it. The left operand is snapshotted into the
/// reserved `gp`; the right is stored here, in this per-thread word, at the
/// compare and reloaded at the branch. A reserved memory word (rather than a
/// second reserved register) is used because rv64 has no free second register:
/// every candidate is either allocatable, ABI/TLS-reserved (`tp` faults a
/// dynamically-linked binary), or a hand-written-helper scratch; and *shrinking*
/// the allocatable pool to free one destabilizes the allocator. This word is
/// addressed off the same pinned per-thread `s11` the v128 slots use, so it is
/// thread-safe, and it is only ever touched inside a single call-free/label-free
/// compare→branch span, so it never races itself. Carved from the v128 region
/// (16 bytes reclaimed above) so `ARENA_STATE_SIZE` does not grow and no other
/// target's bytes change.
pub(crate) const ARENA_FLAG_RHS_OFFSET: usize = ARENA_V128_SLOTS_OFFSET + ARENA_V128_SLOTS_SIZE;
/// Per-thread "current error" slot (plan-error-block-in-slot / design "b"): holds
/// the block base of the single in-flight owned Error while it propagates, so the
/// catching trap route ADOPTS that block (freeing it once) instead of rebuilding a
/// fresh one and orphaning the source (bug-152). 0 when no error is in flight.
/// Appended past the V128 slots so those keep the small offset rv64's 12-bit `addi`
/// immediate needs; this slot sits beyond that range, so its (error-path-only)
/// accesses compute the address in a register rather than using a fixed offset.
/// Zero-initialized by the same whole-`ARENA_STATE_SIZE` clear the entry and
/// thread-spawn paths already run.
pub(crate) const ARENA_CURRENT_ERROR_OFFSET: usize = ARENA_FLAG_RHS_OFFSET + 16;
/// Per-thread stdin broadcast staging (plan-15 §4.2), four `u64` words appended
/// after the current-error slot. All zero-initialized by the whole-`ARENA_STATE_SIZE`
/// clear the entry and thread-spawn paths run, so NULL/zero is the correct "not set
/// up / not subscribed" default and a program that never touches stdin is byte-
/// identical. Like the current-error slot, these sit past rv64's 12-bit `addi`
/// immediate, so accesses compute the address in a register (see
/// `stdin_arena_field_address`) rather than using a fixed load/store displacement.
///
/// `STDIN_LOCAL_BUF`  — pointer to this thread's lazily-arena-allocated 4 KiB copy
///                      buffer (NULL until first stdin read).
/// `STDIN_LOCAL_FILLED`/`STDIN_LOCAL_POS` — valid bytes / read cursor in that buffer
///                      (the lock-free fast path of `_mfb_rt_stdin_next_byte`).
/// `STDIN_SUBSCRIBER`  — pointer to this thread's entry in the global broadcast-log
///                      subscriber registry (NULL ⇒ not subscribed).
pub(crate) const ARENA_STDIN_LOCAL_BUF_OFFSET: usize = ARENA_CURRENT_ERROR_OFFSET + 8;
pub(crate) const ARENA_STDIN_LOCAL_FILLED_OFFSET: usize = ARENA_STDIN_LOCAL_BUF_OFFSET + 8;
pub(crate) const ARENA_STDIN_LOCAL_POS_OFFSET: usize = ARENA_STDIN_LOCAL_FILLED_OFFSET + 8;
pub(crate) const ARENA_STDIN_SUBSCRIBER_OFFSET: usize = ARENA_STDIN_LOCAL_POS_OFFSET + 8;
pub(crate) const ARENA_STATE_SIZE: usize = ARENA_STDIN_SUBSCRIBER_OFFSET + 8;

/// Capacity of the per-thread lazily-allocated stdin local copy buffer, in bytes.
pub(crate) const STDIN_LOCAL_BUFFER_CAPACITY: u64 = 4096;

// ===========================================================================
// Stdin broadcast log (plan-15) — one process-global structure
// ===========================================================================

/// The single process-global broadcast log (plan-15 §4.1): the runtime owns fd 0,
/// reads it in chunks into an append-only deque of fixed blocks, and every
/// subscribed thread reads its own cursor over that log. Zero-initialized in a
/// writable data section and lazily set up (mutex/cond init, self-pipe) on first
/// stdin use. This is the only new cross-thread shared mutable state; it is guarded
/// by its own mutex + condvar (the same primitives the transfer queues use).
pub(crate) const STDIN_LOG_SYMBOL: &str = "_mfb_rt_stdin_log";
/// pthread primitives reserve 64 bytes each (matching the transfer-queue reserve),
/// which fits both glibc and macOS `pthread_mutex_t`/`pthread_cond_t`.
// Reserved layout slot, not a deferred feature: the mutex is addressed
// *implicitly* — `stdin_broadcast` passes the bare log address as `ARG[0]`,
// which is this offset — so no code names the constant. It completes the
// documented block map (0, 64, 128, 136 … 208); deleting it renumbers nothing
// and erases the map. The old comment claimed plan-15 Phase 3 used it for the
// self-pipe, which was wrong twice over: that phase is unbuilt, and the
// self-pipe fds are the two slots at 192/200, not this one (bug-326-D1).
#[allow(dead_code)]
pub(crate) const STDIN_LOG_MUTEX_OFFSET: usize = 0;
pub(crate) const STDIN_LOG_CV_OFFSET: usize = 64;
/// 0 until the log has been lazily initialized (mutex/cond init + self-pipe), 1 after.
pub(crate) const STDIN_LOG_INITIALIZED_OFFSET: usize = 128;
pub(crate) const STDIN_LOG_HEAD_OFFSET: usize = 136;
pub(crate) const STDIN_LOG_TAIL_OFFSET: usize = 144;
/// Absolute stream offset of the head block's first live byte (`base == min(cursor)`).
pub(crate) const STDIN_LOG_BASE_OFFSET: usize = 152;
/// Absolute offset one past the last byte read from the OS.
pub(crate) const STDIN_LOG_FILL_OFFSET: usize = 160;
/// Absolute offset where `read()==0` occurred; `U64_MAX` until then.
pub(crate) const STDIN_LOG_EOF_OFFSET: usize = 168;
/// A subscriber is currently parked in `poll`/`read(0)` (one-reader-at-a-time rule).
pub(crate) const STDIN_LOG_READER_BUSY_OFFSET: usize = 176;
/// Set by `_mfb_shutdown` / the signal path; released cv-waiters and parked reader
/// return EOF.
pub(crate) const STDIN_LOG_SHUTTING_DOWN_OFFSET: usize = 184;
/// Self-pipe read / write fds (plan-15 D4): `_mfb_shutdown` writes the write end;
/// the reader `poll`s the read end beside fd 0 so an orderly shutdown wakes a parked
/// reader deterministically. `-1` until the log is initialized.
// Reserved layout slots completing the block map. The self-pipe itself is
// unbuilt — plan-15 D4 was deferred — so nothing reads these yet; they are
// kept so the two fd slots stay claimed and the map has no hole (bug-326-D1).
#[allow(dead_code)]
pub(crate) const STDIN_LOG_SELFPIPE_READ_OFFSET: usize = 192;
#[allow(dead_code)]
pub(crate) const STDIN_LOG_SELFPIPE_WRITE_OFFSET: usize = 200;
/// Fixed-capacity subscriber registry (kept inside the shared log so no registry
/// entry ever lives in a per-thread arena). Each entry is `{active u64, cursor u64}`;
/// `cursor` is the next unread absolute offset. A thread's `STDIN_SUBSCRIBER` arena
/// word points at its entry here.
pub(crate) const STDIN_LOG_REGISTRY_OFFSET: usize = 208;
pub(crate) const STDIN_SUBSCRIBER_ENTRY_SIZE: usize = 16;
pub(crate) const STDIN_SUBSCRIBER_ACTIVE_OFFSET: usize = 0;
pub(crate) const STDIN_SUBSCRIBER_CURSOR_OFFSET: usize = 8;
pub(crate) const STDIN_LOG_MAX_SUBSCRIBERS: usize = 128;
/// Total size of the process-global log structure.
pub(crate) const STDIN_LOG_SIZE: usize =
    STDIN_LOG_REGISTRY_OFFSET + STDIN_LOG_MAX_SUBSCRIBERS * STDIN_SUBSCRIBER_ENTRY_SIZE;

/// One log block: `{next ptr, baseOffset, data[STDIN_BLOCK_SIZE]}`. Blocks are
/// `malloc`/`free`d (never per-arena) so a block read on one thread and freed on
/// another never races an arena free-list. `baseOffset` is the absolute stream
/// offset of `data[0]`.
pub(crate) const STDIN_BLOCK_NEXT_OFFSET: usize = 0;
pub(crate) const STDIN_BLOCK_BASE_OFFSET: usize = 8;
pub(crate) const STDIN_BLOCK_DATA_OFFSET: usize = 16;
pub(crate) const STDIN_BLOCK_SIZE: u64 = 8192;
/// One OS `read(0, …)` chunk size (≤ `STDIN_BLOCK_SIZE`).
pub(crate) const STDIN_READ_CHUNK: u64 = 8192;

/// Cooperative per-thread stdin reader (plan-15 §4.3). Returns the next stdin byte
/// for the calling thread in the value register with an Ok result, an EOF error
/// result at end of stream, or traps `ErrInvalidContext` if the thread is not
/// subscribed. Fast path (bytes remain in the arena-local buffer) takes no lock.
pub(crate) const STDIN_NEXT_BYTE_SYMBOL: &str = "_mfb_rt_stdin_next_byte";
/// Recompute `base = min(cursor over active subscribers)` and free every log block
/// entirely before `base` (plan-15 §4.3 reclaim-at-min). Assumes the log mutex is
/// held; shared by `_mfb_rt_stdin_next_byte` and `_mfb_rt_stdin_unsubscribe`.
pub(crate) const STDIN_RECOMPUTE_BASE_SYMBOL: &str = "_mfb_rt_stdin_recompute_base";
/// Lazily initialize the global log (mutex/cond init + self-pipe) and subscribe the
/// calling thread at the current frontier. Idempotent per thread. Used both by the
/// compiler-inserted main-thread compat shim and by `thread::openStdIn`.
pub(crate) const STDIN_SUBSCRIBE_SYMBOL: &str = "_mfb_rt_stdin_subscribe";
/// Unsubscribe the calling thread (or, given a worker arena-state pointer, that
/// thread), release its registry entry, recompute `base`, and broadcast.
pub(crate) const STDIN_UNSUBSCRIBE_SYMBOL: &str = "_mfb_rt_stdin_unsubscribe";
/// Default stdin broadcast-log high-water backpressure cap, in bytes (plan-15 D3).
/// The reader refuses to advance `fill` past `base + cap` and blocks on the condvar
/// until a slow subscriber advances `base`. A fixed constant, not lag-relative; the
/// `project.json` `"config"` section can override the baked value at build time.
pub(crate) const STDIN_LOG_CAP_DEFAULT: u64 = 4 * 1024 * 1024;

pub(crate) const ARENA_DEFAULT_BLOCK_SIZE: u64 = 4096;
pub(crate) const ARENA_BLOCK_HEADER_SIZE: usize = 32;
/// Minimum allocation granule. A free chunk overlays a `FreeNode` ({next, size})
/// in its own dead bytes, so it must hold at least 16 bytes. Every request is
/// rounded up to this granule and every allocation is at least 16-byte aligned,
/// which keeps every chunk start 16-aligned and every chunk size a multiple of
/// 16 — so a split front/tail remainder is always either 0 or a valid (≥16)
/// node, never sub-granule slack.
pub(crate) const ARENA_MIN_CHUNK: u64 = 16;

pub(crate) const ARENA_ALLOC_SYMBOL: &str = "_mfb_arena_alloc";
pub(crate) const ARENA_DESTROY_SYMBOL: &str = "_mfb_arena_destroy";
/// `arena_free(x0 = ptr, x1 = size)` — return a single compiler-sized allocation
/// to the per-arena free-list (entropy-scrub then coalescing insert). Never
/// unmaps; memory returns to the OS only at `arena_destroy`.
pub(crate) const ARENA_FREE_SYMBOL: &str = "_mfb_arena_free";
/// `arena_insert_free(x0 = ptr, x1 = size)` — the address-ordered coalescing
/// insert shared by `arena_free` and `arena_alloc`'s block-grow path. Pure
/// free-list surgery; does not scrub (callers fill first when required).
pub(crate) const ARENA_INSERT_FREE_SYMBOL: &str = "_mfb_arena_insert_free";
/// `arena_flush_coalesce()` — plan-64 A1. The flush-before-grow path: gather every
/// parked quick-bin chunk plus the address-ordered list into one chain, sort it by
/// address in a single in-place merge sort, coalesce physically-adjacent chunks in
/// one linear pass, then re-park (≤ QUICK_BIN_MAX → exact bin, larger → the rebuilt
/// address-ordered list). Replaces the old per-chunk `arena_insert_free` drain,
/// whose each-insert-is-O(list) walk made a flush of M parked chunks O(M²) — the
/// arena mixed-transient-churn quadratic. Coalescing is address-exact, so even a
/// sort defect can only under-coalesce (a memory shortfall), never overlap live
/// allocations. No args, operates on arena state; leaf, vreg-allocated.
pub(crate) const ARENA_FLUSH_COALESCE_SYMBOL: &str = "_mfb_arena_flush_coalesce";

/// Capacity of the lazily-allocated stdout output buffer, in bytes.
pub(crate) const OUT_BUFFER_CAPACITY: u64 = 4096;
/// Internal helper that drains the per-arena stdout buffer to fd 1 (plan-14-A):
/// no-op when `OUT_ENABLED == 0` or nothing is pending, otherwise a write-loop
/// that empties `OUT_PTR[0..OUT_FILLED]` and resets `OUT_FILLED = 0`. Returns
/// `x0 = 0` on success (or nothing-to-do) and `x0 = 1` on a write failure. Shared
/// by `io::flush`, the buffered-write overflow path, `io::setBuffered(FALSE)`,
/// every stdin read, and `_mfb_shutdown` — every point where held-back bytes
/// would otherwise be lost or misordered.
pub(crate) const STDOUT_DRAIN_SYMBOL: &str = "_mfb_rt_io_stdout_drain";

// ===========================================================================
// PCG64 random-number generation
// ===========================================================================

/// PCG64 (XSL-RR 128/64) default LCG multiplier, high and low 64-bit limbs.
pub(crate) const PCG_MULT_HI: u64 = 0x2360_ED05_1FC6_5DA4;
pub(crate) const PCG_MULT_LO: u64 = 0x4385_DF64_9FCC_F645;
/// PCG64 default stream increment, high and low 64-bit limbs.
pub(crate) const PCG_INC_HI: u64 = 0x5851_F42D_4C95_7F2D;
pub(crate) const PCG_INC_LO: u64 = 0x1405_7B7E_F767_814F;

/// Advance one PCG64 step and return the next 64-bit value in `x0`; reads/writes
/// the calling thread's arena RNG state via `x19`.
pub(crate) const RNG_NEXT_SYMBOL: &str = "_mfb_rng_next";
/// Seed the PCG64 state at `[x0 + ARENA_RNG_STATE_*]` from the 64-bit seed in
/// `x1`. Used both for the program-startup seed and to give each spawned thread
/// its own stream drawn from the parent's generator.
pub(crate) const RNG_SEED_SYMBOL: &str = "_mfb_rng_seed_at";
/// Fill `x1` bytes at `x0` with output from the dedicated per-arena fill RNG.
/// Used to scrub freed chunks and poison freshly mapped blocks. Clobbers
/// x0, x1, x9–x16.
pub(crate) const ARENA_FILL_RANDOM_SYMBOL: &str = "_mfb_arena_fill_random";
/// Seed the fill RNG at `[x0 + ARENA_FILL_RNG_*]` from the 64-bit seed in `x1`,
/// using the same canonical PCG64 seeding dance as the language RNG.
pub(crate) const ARENA_FILL_SEED_SYMBOL: &str = "_mfb_arena_fill_seed";
/// Advance the calling thread's fill RNG (`x19`) one step and return the next
/// 64-bit value in `x0`. Used to draw an independent child seed from the parent
/// at thread spawn (runs in the parent, so the draw is race-free).
pub(crate) const ARENA_FILL_NEXT_SYMBOL: &str = "_mfb_arena_fill_next";

// ===========================================================================
// SIMD
// ===========================================================================

/// Allocate a tight homogeneous numeric `List` (plan-01-simd §4.3). Input
/// `x0 = count`, `x1 = valueTypeCode`; returns `x0 = list base` (or `0` on OOM).
/// Writes the 40-byte header and `count` uniform 40-byte lookup entries so the
/// per-op SIMD lowerings only stream the data region. Confines the
/// `_mfb_arena_alloc` clobber discipline to one audited routine.
pub(crate) const SIMD_ALLOC_LIST_SYMBOL: &str = "_mfb_simd_alloc_list";

// ===========================================================================
// Shared string symbols
// ===========================================================================

pub(crate) const EMPTY_STRING_SYMBOL: &str = "_mfb_str_empty";

// ===========================================================================
// Filesystem mode bits
// ===========================================================================

pub(crate) const FS_MODE_TYPE_MASK: &str = "61440";
pub(crate) const FS_MODE_DIRECTORY: &str = "16384";
pub(crate) const FS_MODE_REGULAR: &str = "32768";

// ===========================================================================
// Resource / File record layout
// ===========================================================================

// --- The one canonical resource-record header (plan-80) --------------------
// Every built-in and package resource shares this header for offsets 0..32, so
// the generic `STATE` payload (plan-74) has a slot free in *every* layout — not
// just the File-layout ones. Before plan-80 the header diverged after offset 8
// and `STATE` lived at 16, which the TLS/audio backends already used for
// `SSL*`/dispatch-queue/`H_SAMPLE_RATE` — so union `STATE` over a `TlsSocket`
// SIGSEGV'd (plan-76-D Corrections D4). The header is now:
//   tag@0  handle@8  closed@16  STATE@24  |  type-specific@32+
/// Resource type id (plan-80). `0x00` = uninitialized/invalid (never a live
/// record); `< 0xFE` = a built-in resource keyed by `RESOURCE_TAG_*` below;
/// `>= 0xFE` = an Imported (`0xFE`) / Native (`0xFF`) resource. Written at
/// record construction so a record is self-describing.
pub(crate) const RESOURCE_OFFSET_TAG: usize = 0;
/// The polymorphic handle word: fd (File/Socket/…) / connection ptr (macOS
/// Network.framework TLS) / `H_KIND` (audio) / `CPtr` (imported/native).
pub(crate) const RESOURCE_OFFSET_HANDLE: usize = 8;

pub(crate) const FILE_OFFSET_FD: usize = RESOURCE_OFFSET_HANDLE;
pub(crate) const FILE_OFFSET_CLOSED: usize = 16;
/// Offset of the optional `STATE` payload pointer in a resource record. A
/// resource value is a pointer to its arena record, so every copy of the pointer shares the same
/// record and therefore the same `STATE`. The slot is null until the owning
/// `RES` binding default-initializes it. Equal to `RESOURCE_OFFSET_STATE` — the
/// slot is free in *every* backend layout (plan-80), which is the plan-76-D D4 fix.
pub(crate) const FILE_OFFSET_STATE: usize = RESOURCE_OFFSET_STATE;
/// Opt-in per-`File` output buffer fields (plan-14-B), appended after the generic
/// resource header (which now ends at offset 32). Only `File` handles use them;
/// other resources (sockets, TLS, thread handles) carry the words inertly.
/// `FILE_OFFSET_BUF_ENABLED` is 0 (off)
/// on every freshly opened handle — the open helpers zero these three words after
/// the poisoned arena alloc, so a handle that never calls `fs::setBuffered(f, TRUE)`
/// takes the unbuffered direct-write path (byte-identical to pre-plan-14). The
/// thread-transfer copy also zeroes them so a moved handle starts unbuffered.
pub(crate) const FILE_OFFSET_BUF_PTR: usize = 32;
pub(crate) const FILE_OFFSET_BUF_FILLED: usize = 40;
pub(crate) const FILE_OFFSET_BUF_ENABLED: usize = 48;
/// Transparent per-`File` **read** buffer fields (plan-14-C), appended after the
/// write-buffer fields. Always-on (a read buffer can never lose or reorder data):
/// `fs::readLine` serves lines from `READ_PTR` and refills with one block `read()`,
/// turning an O(N²) line loop into O(N). `READ_PTR` is the lazily-allocated block
/// (NULL until the first incremental read), `READ_POS` the next unconsumed byte
/// offset, `READ_FILL` the valid bytes in the block, and `READ_AT_EOF` a flag set
/// once the underlying `read()` returns 0. The fd position runs *ahead* of the
/// logical read position by `READ_FILL - READ_POS` unconsumed bytes; whole-file
/// reads (`fs::readAll`/`readAllBytes`) and `fs::writeAll` reconcile that
/// (seek back + invalidate) before touching the fd. Zeroed at every File alloc
/// and in the thread-transfer copy, so a fresh/moved handle starts with an empty
/// cache at the fd's current position.
pub(crate) const FILE_OFFSET_READ_PTR: usize = 56;
pub(crate) const FILE_OFFSET_READ_POS: usize = 64;
pub(crate) const FILE_OFFSET_READ_FILL: usize = 72;
pub(crate) const FILE_OFFSET_READ_AT_EOF: usize = 80;
/// Size of a resource record: the canonical header (tag/handle/closed/STATE) plus
/// the widest type-specific tail (the File output-buffer fields ptr/filled/enabled
/// and read-buffer fields ptr/pos/fill/at_eof end at offset 88). All resource
/// kinds share the size so the generic thread-transfer copy and closed-default
/// stay uniform. Grown 80 → 96 by plan-80 (header +8 for the `tag`, rounded up to
/// a 16-byte multiple with one slot of headroom).
pub(crate) const RESOURCE_RECORD_SIZE: &str = "96";
/// `RESOURCE_RECORD_SIZE` as a `usize`, for compile-time layout checks (the
/// string form above is what the arena-alloc immediate needs). Every per-backend
/// resource record MUST fit inside this many zeroed bytes so the closed-default
/// (`lower_default_value`) covers each real layout — see the asserts in the
/// backend modules (`audio/mod.rs`, `tls/mod.rs`, `tls/macos.rs`).
pub(crate) const RESOURCE_RECORD_SIZE_BYTES: usize = 96;

/// Canonical byte offset of the `closed` flag in every built-in resource record
/// (moved 8 → 16 by plan-80 to make room for the `tag`/`handle` header words).
/// The closed-resource default (`lower_default_value`) sets exactly this byte;
/// every resource op's closed-guard reads it. All per-resource closed-offset
/// constants MUST equal this — enforced by the compile-time asserts here and in
/// `audio/mod.rs`, `tls/mod.rs`, and `tls/macos.rs` (plan-38). This turns the
/// de-facto convention into a compiler-enforced invariant: a future
/// resource whose closed flag drifts fails to compile.
pub(crate) const RESOURCE_OFFSET_CLOSED: usize = 16;

/// Canonical byte offset of the generic `STATE` payload pointer (plan-74),
/// **free in every backend layout** (plan-80). This is the plan-76-D D4 fix: a
/// `STATE`-carrying union over *any* resource variant — including `TlsSocket`,
/// whose record used offset 16 for `SSL*` — writes STATE here without clobbering
/// a live field. Each backend asserts its own STATE slot equals this.
pub(crate) const RESOURCE_OFFSET_STATE: usize = 24;

// The canonical header shape (plan-80): the File template must match it exactly.
const _: () = assert!(RESOURCE_OFFSET_TAG == 0);
const _: () = assert!(RESOURCE_OFFSET_HANDLE == 8);
const _: () = assert!(RESOURCE_OFFSET_CLOSED == 16);
const _: () = assert!(RESOURCE_OFFSET_STATE == 24);
const _: () = assert!(FILE_OFFSET_FD == RESOURCE_OFFSET_HANDLE);
const _: () = assert!(FILE_OFFSET_CLOSED == RESOURCE_OFFSET_CLOSED);
const _: () = assert!(FILE_OFFSET_STATE == RESOURCE_OFFSET_STATE);
// The header and the widest File tail live inside the zeroed default record.
const _: () = assert!(RESOURCE_OFFSET_STATE + 8 <= RESOURCE_RECORD_SIZE_BYTES);
const _: () = assert!(RESOURCE_OFFSET_CLOSED + 8 <= RESOURCE_RECORD_SIZE_BYTES);
const _: () = assert!(FILE_OFFSET_READ_AT_EOF + 8 <= RESOURCE_RECORD_SIZE_BYTES);

// Resource type tags written at `RESOURCE_OFFSET_TAG` at record construction
// (plan-80). Values match the layout table in `planning/plan-80-*`. They make a
// record self-describing; close dispatch itself stays compile-time-resolved by
// the static resource type (a concrete resource's type is known at its close
// site, and a resource union already dispatches on its own variant tag), so no
// runtime read of this tag is required today. Both Imported and Native resources
// are wrapped by the SAME native `return_resource` path in `link_thunk` (an
// imported package obtains its handle via a native LINK call), so there is no
// distinct imported construction site — both carry `RESOURCE_TAG_NATIVE`
// (plan-80 Corrections; the plan table's separate `0xFE` never materializes as a
// live record). Fed to `move_immediate` as decimal &str.
pub(crate) const RESOURCE_TAG_FILE: &str = "1";
pub(crate) const RESOURCE_TAG_SOCKET: &str = "2";
pub(crate) const RESOURCE_TAG_UDP_SOCKET: &str = "3";
pub(crate) const RESOURCE_TAG_LISTENER: &str = "4";
pub(crate) const RESOURCE_TAG_TLS_OPENSSL: &str = "5";
pub(crate) const RESOURCE_TAG_TLS_MACOS: &str = "6";
pub(crate) const RESOURCE_TAG_TLS_SCHANNEL: &str = "7";
pub(crate) const RESOURCE_TAG_TLS_LISTENER: &str = "8";
pub(crate) const RESOURCE_TAG_AUDIO: &str = "9";
pub(crate) const RESOURCE_TAG_PROCESS: &str = "10";
// plan-98-B: the `canvas::` drawing resources. `handle@8` is the backend's id for
// the object, which is also what an `ImageRef`/`FontRef` carries into a scene — the
// scene holds the id, never the record, so it has no opinion about the resource's
// lifetime.
pub(crate) const RESOURCE_TAG_IMAGE: &str = "11";
// `12` is reserved for `canvas::Font`, which lands with the text path (plan-98-G):
// a `Font` cannot be constructed without `canvas::loadFont`, and that needs the
// font parser G vendors.
pub(crate) const RESOURCE_TAG_NATIVE: &str = "255";

/// The word at `RESOURCE_OFFSET_CLOSED` is a u64 flag set, not a boolean: bit 0
/// is `closed`, bit 1 is `moved`, and 62 bits are spare. Storing `moved` here
/// costs no space and keeps plan-38's offset-8 invariant intact.
///
/// The payoff is that every existing guard is `load; compare 0; branch_ne` —
/// a *non-zero* test, not an equals-1 test — so a moved resource already refuses
/// every operation with no new code. Only a path that wants to *distinguish*
/// `ErrResourceMoved` from `ErrResourceClosed` reads the individual bits.
///
/// Bit 0 keeps meaning exactly what it meant before this existed: a record whose
/// word is `1` is closed-and-not-moved, which is what `lower_default_value`'s
/// closed-default record and every `CLOSED = 1` store already produce.
pub(crate) const RESOURCE_CLOSED_BIT: u64 = 0;
pub(crate) const RESOURCE_MOVED_BIT: u64 = 1;
/// `1 << RESOURCE_MOVED_BIT`, as the immediate a store needs. A moved record is
/// flagged `moved|closed` (= 3): moved implies the sender may no longer use the
/// handle, so the closed bit keeps every existing `!= 0` guard rejecting it even
/// on a path that never looks at bit 1.
pub(crate) const RESOURCE_MOVED_CLOSED_VALUE: &str = "3";

const _: () = assert!(RESOURCE_CLOSED_BIT == 0);
const _: () = assert!(RESOURCE_MOVED_BIT == 1);
/// Block size of the lazily-allocated per-`File` read buffer, in bytes.
pub(crate) const FILE_READ_BUFFER_CAPACITY: u64 = 16384;
/// Capacity of a lazily-allocated per-`File` output buffer, in bytes.
pub(crate) const FILE_BUFFER_CAPACITY: u64 = 4096;
/// Internal helper that drains one `File`'s output buffer to its fd (plan-14-B):
/// `x0 = File*`. No-op when the handle is unbuffered or nothing is pending;
/// otherwise a write-loop that empties `BUF_PTR[0..BUF_FILLED]` to `FILE_OFFSET_FD`
/// and resets `BUF_FILLED`. Returns `x0 = 0` on success (or nothing to do) and
/// `x0 = 1` on a write failure (buffer left intact for a retry). Shared by
/// `fs::flush`, buffered `fs::writeAll`/`writeAllBytes` overflow, the
/// `fs::setBuffered(FALSE)` transition, and the mandatory flush-on-close.
pub(crate) const FILE_DRAIN_SYMBOL: &str = "_mfb_rt_fs_file_drain";

// ===========================================================================
// Collections (List / Map) record layout
// ===========================================================================

pub(crate) const COLLECTION_KIND_LIST: usize = 0;
pub(crate) const COLLECTION_KIND_MAP: usize = 1;
/// A `List` whose element type is a fixed-width scalar: **no `LookupEntry`
/// array**, payloads packed at `HEADER + i * payloadSize` in index order
/// (plan-57-D). `List OF Byte` costs `40 + N` bytes rather than `40 + 41N`.
///
/// This is a representation, not a type. Source-level `List OF Byte` is one
/// type; the emitter picks the block shape from the element type via
/// `list_element_is_fixed_width`. `kind` is written for self-description only —
/// dispatch is static, and no generated code loads this field to branch on.
///
/// The ordering invariant that makes an entry-free layout addressable is
/// established by plan-57-C and machine-checked by
/// `tests/rt-behavior/collections/list-order-invariant-rt`.
pub(crate) const COLLECTION_KIND_LIST_FIXED: usize = 2;
/// A `Set OF T` (plan-63): a Map-shaped block (LookupEntry array + data region +
/// FNV-1a bucket index) whose entries are key-only — each element is stored as a
/// key with `valueType = COLLECTION_TYPE_NONE` and `valueLength = 0`. Like every
/// other `kind` byte this is written for self-description only; dispatch is
/// static, so tagging a Set `3` branches no generated code (it does select the
/// bucket region via `collection_has_buckets`).
pub(crate) const COLLECTION_KIND_SET: usize = 3;
pub(crate) const COLLECTION_HEADER_SIZE: usize = 40;
pub(crate) const COLLECTION_OFFSET_KIND: usize = 0;
pub(crate) const COLLECTION_OFFSET_KEY_TYPE: usize = 1;
pub(crate) const COLLECTION_OFFSET_VALUE_TYPE: usize = 2;
pub(crate) const COLLECTION_OFFSET_FLAGS_VERSION: usize = 3;
pub(crate) const COLLECTION_OFFSET_COUNT: usize = 8;
pub(crate) const COLLECTION_OFFSET_CAPACITY: usize = 16;
pub(crate) const COLLECTION_OFFSET_DATA_LENGTH: usize = 24;
pub(crate) const COLLECTION_OFFSET_DATA_CAPACITY: usize = 32;
pub(crate) const COLLECTION_ENTRY_SIZE: usize = 40;
pub(crate) const COLLECTION_ENTRY_OFFSET_FLAGS: usize = 0;
pub(crate) const COLLECTION_ENTRY_OFFSET_KEY_OFFSET: usize = 8;
pub(crate) const COLLECTION_ENTRY_OFFSET_KEY_LENGTH: usize = 16;
pub(crate) const COLLECTION_ENTRY_OFFSET_VALUE_OFFSET: usize = 24;
pub(crate) const COLLECTION_ENTRY_OFFSET_VALUE_LENGTH: usize = 32;
pub(crate) const COLLECTION_ENTRY_FLAG_USED: usize = 1;

// Map hash index (plan-02 Phase 6). A `Map` reserves a bucket array of
// `2*capacity` u64 entries **after** the data region (so the capacity-based data
// base is unchanged); each bucket holds `entryIndex + 1` (0 = empty) addressed by
// FNV-1a(key) mod bucketCount with linear probing. The bucket region is derived
// metadata: a 1-byte "ready" flag in the header's free padding (offset 4) is 0 on
// every fresh/copied/grown map and set to 1 once `_mfb_rt_map_build_buckets` fills
// it lazily on the first probe — so copy/transfer just reserve space + mark
// not-ready and the next probe recomputes, with no stale offsets. `set` maintains
// the index incrementally (`_mfb_rt_map_bucket_put`) so building a map via repeated
// `set` stays O(n). Lists never probe; their bucket region is empty (`2*0`-sized
// for a tight list is 0, and the field stays 0).
pub(crate) const COLLECTION_OFFSET_BUCKETS_READY: usize = 4;
pub(crate) const MAP_BUCKET_SIZE: usize = 8;
pub(crate) const MAP_BUILD_BUCKETS_SYMBOL: &str = "_mfb_rt_map_build_buckets";
pub(crate) const MAP_BUCKET_PUT_SYMBOL: &str = "_mfb_rt_map_bucket_put";
pub(crate) const MAP_PROBE_SYMBOL: &str = "_mfb_rt_map_probe";
/// FNV-1a 64-bit offset basis / prime (decimal) for the map key hash.
pub(crate) const FNV1A_BASIS: &str = "14695981039346656037";
pub(crate) const FNV1A_PRIME: &str = "1099511628211";

// Geometric growth shape for the append grow path (plan-01 §5): start small,
// double until a taper threshold, then ×1.5. Lookup slots and data bytes grow
// independently. Literals and known-size builders ignore these (exact alloc).
pub(crate) const COLLECTION_GROW_LOOKUP_INIT: usize = 4;
pub(crate) const COLLECTION_GROW_LOOKUP_TAPER: usize = 1024;
pub(crate) const COLLECTION_GROW_DATA_INIT: usize = 32;
pub(crate) const COLLECTION_GROW_DATA_TAPER: usize = 65536;

pub(crate) const COLLECTION_TYPE_NONE: usize = 0;
pub(crate) const COLLECTION_TYPE_BOOLEAN: usize = 2;
pub(crate) const COLLECTION_TYPE_INTEGER: usize = 3;
pub(crate) const COLLECTION_TYPE_FLOAT: usize = 4;
pub(crate) const COLLECTION_TYPE_FIXED: usize = 5;
pub(crate) const COLLECTION_TYPE_STRING: usize = 6;
pub(crate) const COLLECTION_TYPE_BYTE: usize = 7;
/// IEEE-754 binary64 bit patterns as the unsigned-decimal `move_immediate`
/// strings the kernels load (bug-332 G3). `F64_SIGN_BIT` doubles as the
/// `i64::MIN` pattern; `TIMEOUT_UNBOUNDED_SENTINEL` is the same bits with an
/// unrelated meaning and is deliberately kept separate.
pub(crate) const F64_SIGN_BIT: &str = "9223372036854775808";
pub(crate) const F64_MANTISSA_MASK: &str = "4503599627370495";
pub(crate) const F64_POSITIVE_INF_BITS: &str = "9218868437227405312";
/// `Money` collection element (plan-29-C): an 8-byte signed-i64 lane, compared
/// as a signed integer (same scale ⇒ raw order = value order). Takes the free
/// tag 8 between `Byte` (7) and `List` (20).
pub(crate) const COLLECTION_TYPE_MONEY: usize = 8;
/// `Scalar` collection element (plan-41-C): a 4-byte 32-bit Unicode codepoint
/// lane, compared as an unsigned integer (codepoint order = value order). Takes
/// the free tag 9 between `Money` (8) and `List` (20).
pub(crate) const COLLECTION_TYPE_SCALAR: usize = 9;
pub(crate) const COLLECTION_TYPE_LIST: usize = 20;
pub(crate) const COLLECTION_TYPE_MAP: usize = 21;
pub(crate) const COLLECTION_TYPE_OBJECT: usize = 22;

// ===========================================================================
// Unicode data-table symbols
// ===========================================================================

pub(crate) const UNICODE_STAGE1_SYMBOL: &str = "_mfb_unicode_stage1";
pub(crate) const UNICODE_STAGE2_SYMBOL: &str = "_mfb_unicode_stage2";
pub(crate) const UNICODE_PROPERTIES_SYMBOL: &str = "_mfb_unicode_properties";
pub(crate) const UNICODE_COMBINATIONS_SECOND_SYMBOL: &str = "_mfb_unicode_combinations_second";
pub(crate) const UNICODE_COMBINATIONS_COMBINED_SYMBOL: &str = "_mfb_unicode_combinations_combined";
pub(crate) const UNICODE_NFD_ENTRIES_SYMBOL: &str = "_mfb_unicode_nfd_entries";
pub(crate) const UNICODE_NFD_SEQUENCES_SYMBOL: &str = "_mfb_unicode_nfd_sequences";
pub(crate) const UNICODE_UPPERCASE_ENTRIES_SYMBOL: &str = "_mfb_unicode_uppercase_entries";
pub(crate) const UNICODE_UPPERCASE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_uppercase_sequences";
pub(crate) const UNICODE_LOWERCASE_ENTRIES_SYMBOL: &str = "_mfb_unicode_lowercase_entries";
pub(crate) const UNICODE_LOWERCASE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_lowercase_sequences";
pub(crate) const UNICODE_CASEFOLD_ENTRIES_SYMBOL: &str = "_mfb_unicode_casefold_entries";
pub(crate) const UNICODE_CASEFOLD_SEQUENCES_SYMBOL: &str = "_mfb_unicode_casefold_sequences";

// ===========================================================================
// Threads
// ===========================================================================

pub(crate) const THREAD_TRAMPOLINE_SYMBOL: &str = "_mfb_rt_thread_trampoline";
