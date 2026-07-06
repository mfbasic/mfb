use super::*;

pub(crate) const RESULT_OK_TAG: &str = "0";
pub(crate) const RESULT_ERR_TAG: &str = "1";
pub(crate) const RESULT_PROGRAM_EXIT_TAG: &str = "2";
pub(crate) const ERR_OVERFLOW_CODE: &str = "77050010";
pub(crate) const ERR_OVERFLOW_MESSAGE: &str = "numeric overflow";
pub(crate) const ERR_OVERFLOW_SYMBOL: &str = "_mfb_str_error_overflow";
pub(crate) const ERR_UNDERFLOW_CODE: &str = "77050011";
pub(crate) const ERR_UNDERFLOW_MESSAGE: &str = "numeric underflow";
pub(crate) const ERR_UNDERFLOW_SYMBOL: &str = "_mfb_str_error_underflow";
pub(crate) const ERR_FLOAT_DOMAIN_CODE: &str = "77050012";
pub(crate) const ERR_FLOAT_DOMAIN_MESSAGE: &str = "float domain error";
pub(crate) const ERR_FLOAT_DOMAIN_SYMBOL: &str = "_mfb_str_error_float_domain";
pub(crate) const ERR_FLOAT_NAN_CODE: &str = "77050013";
pub(crate) const ERR_FLOAT_NAN_MESSAGE: &str = "float NaN result";
pub(crate) const ERR_FLOAT_NAN_SYMBOL: &str = "_mfb_str_error_float_nan";
pub(crate) const ERR_FLOAT_INF_CODE: &str = "77050014";
pub(crate) const ERR_FLOAT_INF_MESSAGE: &str = "float infinity result";
pub(crate) const ERR_FLOAT_INF_SYMBOL: &str = "_mfb_str_error_float_inf";
pub(crate) const ERR_FLOAT_OVERFLOW_CODE: &str = "77050015";
pub(crate) const ERR_FLOAT_OVERFLOW_MESSAGE: &str = "float overflow";
pub(crate) const ERR_FLOAT_OVERFLOW_SYMBOL: &str = "_mfb_str_error_float_overflow";
pub(crate) const ERR_ALLOCATION_MESSAGE: &str = "allocation failed";
pub(crate) const ERR_ALLOCATION_SYMBOL: &str = "_mfb_str_error_allocation";
pub(crate) const ERR_INDEX_OUT_OF_RANGE_CODE: &str = "77050001";
pub(crate) const ERR_INDEX_OUT_OF_RANGE_MESSAGE: &str = "index out of range";
pub(crate) const ERR_INDEX_OUT_OF_RANGE_SYMBOL: &str = "_mfb_str_error_index_out_of_range";
pub(crate) const ERR_NOT_FOUND_CODE: &str = "77050004";
pub(crate) const ERR_NOT_FOUND_MESSAGE: &str = "not found";
pub(crate) const ERR_NOT_FOUND_SYMBOL: &str = "_mfb_str_error_not_found";
pub(crate) const ERR_TIMEOUT_CODE: &str = "77050008";
pub(crate) const ERR_TIMEOUT_MESSAGE: &str = "timeout";
pub(crate) const ERR_TIMEOUT_SYMBOL: &str = "_mfb_str_error_timeout";
pub(crate) const ERR_INTERRUPTED_CODE: &str = "77050009";
pub(crate) const ERR_INTERRUPTED_MESSAGE: &str = "interrupted";
pub(crate) const ERR_INTERRUPTED_SYMBOL: &str = "_mfb_str_error_interrupted";
pub(crate) const ERR_UNKNOWN_CODE: &str = "77050000";
pub(crate) const ERR_UNKNOWN_MESSAGE: &str = "unknown error";
pub(crate) const ERR_UNKNOWN_SYMBOL: &str = "_mfb_str_error_unknown";
pub(crate) const ERR_READ_CODE: &str = "77020001";
pub(crate) const ERR_READ_MESSAGE: &str = "read failure";
pub(crate) const ERR_READ_SYMBOL: &str = "_mfb_str_error_read";
pub(crate) const ERR_OUTPUT_CODE: &str = "77020002";
pub(crate) const ERR_OUTPUT_MESSAGE: &str = "output failure";
pub(crate) const ERR_OUTPUT_SYMBOL: &str = "_mfb_str_error_output";
/// macOS app mode (plan-04-macos-app.md §6.6): the standard program-entry logic
/// (arena setup + language `main` + exit) is emitted under this symbol and runs
/// on the worker thread, while `_main` is the AppKit bootstrap.
pub(crate) const MACAPP_PROGRAM_SYMBOL: &str = "_mfb_macapp_program";
pub(crate) const ERR_UNSUPPORTED_CODE: &str = "77050007";
pub(crate) const ERR_UNSUPPORTED_MESSAGE: &str = "unsupported operation";
pub(crate) const ERR_UNSUPPORTED_SYMBOL: &str = "_mfb_str_error_unsupported";
pub(crate) const ERR_EOF_CODE: &str = "77020003";
pub(crate) const ERR_EOF_MESSAGE: &str = "end of file";
pub(crate) const ERR_EOF_SYMBOL: &str = "_mfb_str_error_eof";
pub(crate) const ERR_RESOURCE_CLOSED_CODE: &str = "77030004";
pub(crate) const ERR_RESOURCE_CLOSED_MESSAGE: &str = "resource closed";
pub(crate) const ERR_RESOURCE_CLOSED_SYMBOL: &str = "_mfb_str_error_resource_closed";
pub(crate) const ERR_NATIVE_LINK_LOAD_CODE: &str = "77030007";
pub(crate) const ERR_NATIVE_LINK_LOAD_MESSAGE: &str = "native binding library unavailable";
pub(crate) const ERR_NATIVE_LINK_LOAD_SYMBOL: &str = "_mfb_str_error_native_link_load";
pub(crate) const ERR_NATIVE_LINK_CALL_CODE: &str = "77030008";
pub(crate) const ERR_NATIVE_LINK_CALL_MESSAGE: &str = "native binding call failed";
pub(crate) const ERR_NATIVE_LINK_CALL_SYMBOL: &str = "_mfb_str_error_native_link_call";
pub(crate) const ERR_ENCODING_CODE: &str = "77020004";
pub(crate) const ERR_ENCODING_MESSAGE: &str = "invalid encoding";
pub(crate) const ERR_ENCODING_SYMBOL: &str = "_mfb_str_error_encoding";
pub(crate) const ERR_INPUT_CODE: &str = "77020005";
pub(crate) const ERR_INPUT_MESSAGE: &str = "input failure";
pub(crate) const ERR_INPUT_SYMBOL: &str = "_mfb_str_error_input";
pub(crate) const ENTRY_ERROR_PREFIX: &str = "Code: ";
pub(crate) const ENTRY_ERROR_PREFIX_SYMBOL: &str = "_mfb_str_entry_error_prefix";
pub(crate) const ENTRY_ERROR_SEPARATOR: &str = " Message: ";
pub(crate) const ENTRY_ERROR_SEPARATOR_SYMBOL: &str = "_mfb_str_entry_error_separator";
pub(crate) const ENTRY_ERROR_NEWLINE: &str = "\n";
pub(crate) const ENTRY_ERROR_NEWLINE_SYMBOL: &str = "_mfb_str_entry_error_newline";
pub(crate) const CLEANUP_FAILURE_PREFIX: &str = "Cleanup failure: Code: ";
pub(crate) const CLEANUP_FAILURE_PREFIX_SYMBOL: &str = "_mfb_str_cleanup_failure_prefix";
pub(crate) const CLEANUP_FAILURE_SEPARATOR: &str = " Message: ";
pub(crate) const CLEANUP_FAILURE_SEPARATOR_SYMBOL: &str = ENTRY_ERROR_SEPARATOR_SYMBOL;
pub(crate) const RESULT_TAG_REGISTER: &str = abi::RETURN_REGISTER;
pub(crate) const RESULT_VALUE_REGISTER: &str = "x1";
pub(crate) const RESULT_ERROR_MESSAGE_REGISTER: &str = "x2";
/// Fourth error-result register: pointer to the `ErrorLoc` recording where the
/// error originated. Carried alongside code (x1) and message (x2) so propagation
/// preserves the origin and trap materialization can build a 3-field `Error`.
pub(crate) const RESULT_ERROR_SOURCE_REGISTER: &str = "x3";
/// Byte size of an allocated `Error` record: code(+0), message(+8), source(+16).
pub(crate) const ERROR_OBJECT_SIZE: usize = 24;
/// Byte size of an allocated `ErrorLoc` record: filename(+0), line(+8), char(+16).
pub(crate) const ERROR_LOC_OBJECT_SIZE: usize = 24;
pub(crate) const ARENA_ALLOC_SYMBOL: &str = "_mfb_arena_alloc";
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
pub(crate) const ARENA_DESTROY_SYMBOL: &str = "_mfb_arena_destroy";
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
pub(crate) const ARENA_STATE_REGISTER: &str = "x19";
pub(crate) const CLOSURE_ENV_REGISTER: &str = "x28";
pub(crate) const CLOSURE_OBJECT_SIZE: usize = 16;
pub(crate) const CLOSURE_OFFSET_CODE: usize = 0;
pub(crate) const CLOSURE_OFFSET_ENV: usize = 8;
/// Entry-frame prefix: the arena state plus the one seed-scratch word after it.
/// Derived from `ARENA_STATE_SIZE` so the frame tracks arena-state growth
/// (e.g. the allocator-01 quick bins) automatically.
pub(crate) const ENTRY_STACK_SIZE: usize = ENTRY_SEED_SCRATCH_OFFSET + 8;
pub(crate) const ENTRY_GLOBALS_OFFSET: usize = ENTRY_STACK_SIZE;
/// `term::` TUI-mode state slots reserved in the program-entry frame just past
/// the program globals and `LINK` slots (plan-01-term.md §6.2). Eight `u64`
/// slots: active, packed foreground, packed background, bold, underline,
/// cursor-visible, and two reserved for the app backend. Zero-initialized by the
/// entry's global-slot clear, which is the inert (TUI-off) default.
pub(crate) const TERM_STATE_SLOTS: usize = 8;
pub(crate) const TERM_STATE_ACTIVE_OFFSET: usize = 0;
pub(crate) const TERM_STATE_FG_OFFSET: usize = 8;
pub(crate) const TERM_STATE_BG_OFFSET: usize = 16;
pub(crate) const TERM_STATE_BOLD_OFFSET: usize = 24;
pub(crate) const TERM_STATE_UNDERLINE_OFFSET: usize = 32;
pub(crate) const TERM_STATE_CURSOR_VISIBLE_OFFSET: usize = 40;
pub(crate) const ARENA_CLEANUP_FAILURE_COUNT_OFFSET: usize = 64;
pub(crate) const ARENA_CLEANUP_FAILURE_CODE_OFFSET: usize = 72;
pub(crate) const ARENA_CLEANUP_FAILURE_MESSAGE_OFFSET: usize = 80;
/// Per-arena (per-thread) PCG64 random-number generator state. Each OS thread
/// owns its own arena, so storing the 128-bit RNG state in the arena gives every
/// thread an independent stream reachable through the pinned arena register
/// (`x19`) without a thread-local lookup. Appended past the cleanup-audit fields
/// so the historical 0..88 layout is unchanged for programs that never seed.
pub(crate) const ARENA_RNG_STATE_LO_OFFSET: usize = 88;
pub(crate) const ARENA_RNG_STATE_HI_OFFSET: usize = 96;
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
pub(crate) const ARENA_STATE_SIZE: usize = ARENA_OUT_ENABLED_OFFSET + 8;
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
/// Advance one PCG64 step and return the next 64-bit value in `x0`; reads/writes
/// the calling thread's arena RNG state via `x19`.
pub(crate) const RNG_NEXT_SYMBOL: &str = "_mfb_rng_next";
/// Allocate a tight homogeneous numeric `List` (plan-01-simd §4.3). Input
/// `x0 = count`, `x1 = valueTypeCode`; returns `x0 = list base` (or `0` on OOM).
/// Writes the 40-byte header and `count` uniform 40-byte lookup entries so the
/// per-op SIMD lowerings only stream the data region. Confines the
/// `_mfb_arena_alloc` clobber discipline to one audited routine.
pub(crate) const SIMD_ALLOC_LIST_SYMBOL: &str = "_mfb_simd_alloc_list";
/// Seed the PCG64 state at `[x0 + ARENA_RNG_STATE_*]` from the 64-bit seed in
/// `x1`. Used both for the program-startup seed and to give each spawned thread
/// its own stream drawn from the parent's generator.
pub(crate) const RNG_SEED_SYMBOL: &str = "_mfb_rng_seed_at";
/// PCG64 (XSL-RR 128/64) default LCG multiplier, high and low 64-bit limbs.
pub(crate) const PCG_MULT_HI: u64 = 0x2360_ED05_1FC6_5DA4;
pub(crate) const PCG_MULT_LO: u64 = 0x4385_DF64_9FCC_F645;
/// PCG64 default stream increment, high and low 64-bit limbs.
pub(crate) const PCG_INC_HI: u64 = 0x5851_F42D_4C95_7F2D;
pub(crate) const PCG_INC_LO: u64 = 0x1405_7B7E_F767_814F;
/// One in-frame scratch word between the arena state (0..`ARENA_STATE_SIZE`)
/// and the globals (`ENTRY_STACK_SIZE`..): the RNG-seed block's `getentropy`
/// buffer.
pub(crate) const ENTRY_SEED_SCRATCH_OFFSET: usize = ARENA_STATE_SIZE;
/// Size of the args region appended to the entry frame for an arg-accepting
/// entry: five 8-byte slots (argc, argv, args list, data length, saved count),
/// rounded up to the 16-byte frame granule. The region sits ABOVE the globals
/// (at `entry_stack_size - ENTRY_ARGS_REGION_SIZE`); the old fixed offsets at
/// 104..144 overlapped the first four global slots and, for a program with no
/// globals, spilled past the frame — silently-scratch memory on macOS, but the
/// OS argc/argv words themselves at a raw Linux ELF entry.
pub(crate) const ENTRY_ARGS_REGION_SIZE: usize = 48;
pub(crate) const ARENA_DEFAULT_BLOCK_SIZE: u64 = 4096;
pub(crate) const ARENA_BLOCK_HEADER_SIZE: usize = 32;
/// Per-arena address-ordered coalescing free-list head (lowest-address free
/// chunk, 0 when empty). Stored in the reserved arena-state word at offset 48
/// (`memory_layouts.md` Arenas §). The list subsumes the old bump pointer: a
/// freshly mapped block's usable region is inserted as one big free chunk and
/// `arena_alloc` carves allocations out of it (first-fit + split), while
/// `arena_free` returns chunks and coalesces with address-adjacent neighbors.
pub(crate) const ARENA_FREE_LIST_HEAD_OFFSET: usize = 48;
/// Minimum allocation granule. A free chunk overlays a `FreeNode` ({next, size})
/// in its own dead bytes, so it must hold at least 16 bytes. Every request is
/// rounded up to this granule and every allocation is at least 16-byte aligned,
/// which keeps every chunk start 16-aligned and every chunk size a multiple of
/// 16 — so a split front/tail remainder is always either 0 or a valid (≥16)
/// node, never sub-granule slack.
pub(crate) const ARENA_MIN_CHUNK: u64 = 16;
/// `arena_free(x0 = ptr, x1 = size)` — return a single compiler-sized allocation
/// to the per-arena free-list (entropy-scrub then coalescing insert). Never
/// unmaps; memory returns to the OS only at `arena_destroy`.
pub(crate) const ARENA_FREE_SYMBOL: &str = "_mfb_arena_free";
/// `arena_insert_free(x0 = ptr, x1 = size)` — the address-ordered coalescing
/// insert shared by `arena_free` and `arena_alloc`'s block-grow path. Pure
/// free-list surgery; does not scrub (callers fill first when required).
pub(crate) const ARENA_INSERT_FREE_SYMBOL: &str = "_mfb_arena_insert_free";
pub(crate) const ERR_INVALID_ARGUMENT_CODE: &str = "77050002";
pub(crate) const ERR_INVALID_ARGUMENT_MESSAGE: &str = "invalid argument";
pub(crate) const ERR_INVALID_ARGUMENT_SYMBOL: &str = "_mfb_str_error_invalid_argument";
pub(crate) const ERR_INVALID_FORMAT_CODE: &str = "77050003";
pub(crate) const ERR_INVALID_FORMAT_MESSAGE: &str = "invalid format";
pub(crate) const ERR_INVALID_FORMAT_SYMBOL: &str = "_mfb_str_error_invalid_format";
pub(crate) const ERR_OUT_OF_MEMORY_CODE: &str = "77010001";
pub(crate) const ERR_ALREADY_EXISTS_CODE: &str = "77050005";
pub(crate) const ERR_ALREADY_EXISTS_MESSAGE: &str = "already exists";
pub(crate) const ERR_ALREADY_EXISTS_SYMBOL: &str = "_mfb_str_error_already_exists";
pub(crate) const ERR_ACCESS_DENIED_CODE: &str = "77030003";
pub(crate) const ERR_ACCESS_DENIED_MESSAGE: &str = "access denied";
pub(crate) const ERR_ACCESS_DENIED_SYMBOL: &str = "_mfb_str_error_access_denied";
pub(crate) const ERR_DIRECTORY_NOT_EMPTY_CODE: &str = "77030005";
pub(crate) const ERR_DIRECTORY_NOT_EMPTY_MESSAGE: &str = "directory not empty";
pub(crate) const ERR_DIRECTORY_NOT_EMPTY_SYMBOL: &str = "_mfb_str_error_directory_not_empty";
pub(crate) const ERR_CLOSE_FAILED_CODE: &str = "77030006";
pub(crate) const ERR_CLOSE_FAILED_MESSAGE: &str = "close failed";
pub(crate) const ERR_CLOSE_FAILED_SYMBOL: &str = "_mfb_str_error_close_failed";
pub(crate) const ERR_PATH_NOT_FOUND_CODE: &str = "77030001";
pub(crate) const ERR_PATH_NOT_FOUND_MESSAGE: &str = "path not found";
pub(crate) const ERR_PATH_NOT_FOUND_SYMBOL: &str = "_mfb_str_error_path_not_found";
pub(crate) const ERR_INVALID_PATH_CODE: &str = "77030002";
pub(crate) const ERR_INVALID_PATH_MESSAGE: &str = "invalid path";
pub(crate) const ERR_INVALID_PATH_SYMBOL: &str = "_mfb_str_error_invalid_path";
pub(crate) const ERR_ADDRESS_INVALID_CODE: &str = "77070001";
pub(crate) const ERR_ADDRESS_INVALID_MESSAGE: &str = "address invalid";
pub(crate) const ERR_ADDRESS_INVALID_SYMBOL: &str = "_mfb_str_error_address_invalid";
pub(crate) const ERR_ADDRESS_NOT_FOUND_CODE: &str = "77070002";
pub(crate) const ERR_ADDRESS_NOT_FOUND_MESSAGE: &str = "address not found";
pub(crate) const ERR_ADDRESS_NOT_FOUND_SYMBOL: &str = "_mfb_str_error_address_not_found";
pub(crate) const ERR_NETWORK_FAILED_CODE: &str = "77070003";
pub(crate) const ERR_NETWORK_FAILED_MESSAGE: &str = "network failed";
pub(crate) const ERR_NETWORK_FAILED_SYMBOL: &str = "_mfb_str_error_network_failed";
pub(crate) const ERR_CONNECTION_CLOSED_CODE: &str = "77070004";
pub(crate) const ERR_CONNECTION_CLOSED_MESSAGE: &str = "connection closed";
pub(crate) const ERR_CONNECTION_CLOSED_SYMBOL: &str = "_mfb_str_error_connection_closed";
pub(crate) const ERR_READ_TIMEOUT_CODE: &str = "77070005";
pub(crate) const ERR_READ_TIMEOUT_MESSAGE: &str = "read timeout";
pub(crate) const ERR_READ_TIMEOUT_SYMBOL: &str = "_mfb_str_error_read_timeout";
pub(crate) const ERR_WRITE_TIMEOUT_CODE: &str = "77070006";
pub(crate) const ERR_WRITE_TIMEOUT_MESSAGE: &str = "write timeout";
pub(crate) const ERR_WRITE_TIMEOUT_SYMBOL: &str = "_mfb_str_error_write_timeout";
pub(crate) const ERR_MESSAGE_TOO_LARGE_CODE: &str = "77070007";
pub(crate) const ERR_MESSAGE_TOO_LARGE_MESSAGE: &str = "message too large";
pub(crate) const ERR_MESSAGE_TOO_LARGE_SYMBOL: &str = "_mfb_str_error_message_too_large";
pub(crate) const ERR_TLS_FAILED_CODE: &str = "77070008";
pub(crate) const ERR_TLS_FAILED_MESSAGE: &str = "TLS failed";
pub(crate) const ERR_TLS_FAILED_SYMBOL: &str = "_mfb_str_error_tls_failed";
pub(crate) const EMPTY_STRING_SYMBOL: &str = "_mfb_str_empty";
pub(crate) const FS_MODE_TYPE_MASK: &str = "61440";
pub(crate) const FS_MODE_DIRECTORY: &str = "16384";
pub(crate) const FS_MODE_REGULAR: &str = "32768";
pub(crate) const FILE_OFFSET_FD: usize = 0;
pub(crate) const FILE_OFFSET_CLOSED: usize = 8;
/// Offset of the optional `STATE` payload pointer in a resource record. A
/// resource value is a pointer to its arena record, so a borrow shares the same
/// record and therefore the same `STATE`. The slot is null until the owning
/// `RES` binding default-initializes it.
pub(crate) const FILE_OFFSET_STATE: usize = 16;
/// Opt-in per-`File` output buffer fields (plan-14-B), appended after the generic
/// resource header. Only `File` handles use them; other resources (sockets, TLS,
/// thread handles) carry the words inertly. `FILE_OFFSET_BUF_ENABLED` is 0 (off)
/// on every freshly opened handle — the open helpers zero these three words after
/// the poisoned arena alloc, so a handle that never calls `fs::setBuffered(f, TRUE)`
/// takes the unbuffered direct-write path (byte-identical to pre-plan-14). The
/// thread-transfer copy also zeroes them so a moved handle starts unbuffered.
pub(crate) const FILE_OFFSET_BUF_PTR: usize = 24;
pub(crate) const FILE_OFFSET_BUF_FILLED: usize = 32;
pub(crate) const FILE_OFFSET_BUF_ENABLED: usize = 40;
/// Transparent per-`File` **read** buffer fields (plan-14-C), appended after the
/// write-buffer fields. Always-on (a read buffer can never lose or reorder data):
/// `fs::readLine` serves lines from `READ_PTR` and refills with one block `read()`,
/// turning an O(N²) line loop into O(N). `READ_PTR` is the lazily-allocated block
/// (NULL until the first incremental read), `READ_POS` the next unconsumed byte
/// offset, `READ_FILL` the valid bytes in the block, and `READ_AT_EOF` a flag set
/// once the underlying `read()` returns 0. The fd position runs *ahead* of the
/// logical read position by `READ_FILL - READ_POS` unconsumed bytes; whole-file
/// reads (`fs::readAll`/`readAllBytes`) and `fs::writeAll` reconcile that (seek back
/// + invalidate) before touching the fd. Zeroed at every File alloc and in the
/// thread-transfer copy, so a fresh/moved handle starts with an empty cache at the
/// fd's current position.
pub(crate) const FILE_OFFSET_READ_PTR: usize = 48;
pub(crate) const FILE_OFFSET_READ_POS: usize = 56;
pub(crate) const FILE_OFFSET_READ_FILL: usize = 64;
pub(crate) const FILE_OFFSET_READ_AT_EOF: usize = 72;
/// Size of a resource record: fd, closed flag, the `STATE` pointer, the per-`File`
/// output-buffer fields (ptr/filled/enabled), and the read-buffer fields
/// (ptr/pos/fill/at_eof). All resource kinds share the size so the generic
/// thread-transfer copy stays uniform.
pub(crate) const RESOURCE_RECORD_SIZE: &str = "80";
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
pub(crate) const COLLECTION_KIND_LIST: usize = 0;
pub(crate) const COLLECTION_KIND_MAP: usize = 1;
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
pub(crate) const COLLECTION_TYPE_LIST: usize = 20;
pub(crate) const COLLECTION_TYPE_MAP: usize = 21;
pub(crate) const COLLECTION_TYPE_OBJECT: usize = 22;
pub(crate) const UNICODE_STAGE1_SYMBOL: &str = "_mfb_unicode_stage1";
pub(crate) const UNICODE_STAGE2_SYMBOL: &str = "_mfb_unicode_stage2";
pub(crate) const UNICODE_PROPERTIES_SYMBOL: &str = "_mfb_unicode_properties";
pub(crate) const UNICODE_SEQUENCES_SYMBOL: &str = "_mfb_unicode_sequences";
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
pub(crate) const THREAD_TRAMPOLINE_SYMBOL: &str = "_mfb_rt_thread_trampoline";
