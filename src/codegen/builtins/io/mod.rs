//! The built-in `io` package.
//!
//! `io` provides access to the three process standard streams — standard input,
//! standard output, and standard error — together with helpers for reading the
//! keyboard, buffering standard output, and inspecting which streams are
//! terminals. It is the console counterpart to `fs`: where `fs` works through
//! named files and `File` handles, `io` works through the process standard
//! streams.
//!
//! Migrated onto the clean-room registry (`crate::codegen::registry`) as a native
//! OS-seam package. Every member is a per-function `Body::abi_function` clean-room
//! lowering (plan-101): each `func_*.rs` owns a `lower_*` adapter that branches
//! app-vs-console off the threaded [`AbiCtx`](crate::codegen::registry::AbiCtx),
//! calls the console OS-seam emitter it owns (shared emitters live with their
//! primary member — the write emitter in `func_write`, the stdin read machinery
//! in `func_read_line`, the terminal probe in `func_is_input_terminal`) or the
//! platform app hook (app mode), and hands the finalized helper body back through
//! the
//! `abi_function` pre-finalized hatch — so `io` keeps its `Io`-family
//! `_mfb_rt_io_io_*` symbols and the migration off `Body::native_os_seam` is
//! byte-identical. `io` owns **no** resource handle, contributes no builtin value
//! type, and has no source companion — the registry's generic overload/return
//! resolution answers arity/return/validation with no custom resolver.

use crate::codegen::engine::types::CodegenPlatform;
use crate::codegen::registry::{Registry, RegistryPackage};

mod func_flush;
mod func_input;
mod func_is_buffered;
mod func_is_error_terminal;
mod func_is_input_terminal;
mod func_is_output_terminal;
mod func_poll_input;
mod func_print;
mod func_print_error;
mod func_read_byte;
mod func_read_char;
mod func_read_line;
mod func_set_buffered;
mod func_write;
mod func_write_error;

const MODULE_INTRO: &str = r#"Standard stream input/output and terminal inspection"#;
const MODULE_DESC: &str = r#"The `io` package provides access to the three standard streams — standard
input, standard output, and standard error — together with helpers for reading
the keyboard and inspecting which streams are terminals. It is the console
counterpart to the `fs` package: where `fs` works through named files and `File`
handles, `io` works through the process standard streams. `io` is a built-in
package: `IMPORT io` needs no manifest dependency.

Output functions accept `String` values only and perform no implicit
conversion; convert other values with `toString` first. Text is treated as UTF-8
and emitted byte for byte, with no escaping or newline translation beyond the
trailing newline that `io::print` and `io::printError` add. `io::write` and
`io::print` target standard output, `io::writeError` and `io::printError` target
standard error, and `io::flush` drains standard output. Standard-output buffering
is opt-in and off by default: `io::setBuffered(TRUE)` holds output in a per-thread
buffer (drained on `io::flush`, before any read, when full, and at exit) and
`io::setBuffered(FALSE)` drains and disables it, while `io::isBuffered` reports the
current mode. With buffering on, written text is not guaranteed visible to an
external reader until flushed; flush before blocking on a read when a prompt must
appear first. Standard error is never buffered — it is written immediately, so it
has no flush.

Input functions read from standard input. `io::input` reads a whole line with
normal terminal echo and an optional prompt; `io::readLine` reads a line the same
way but never writes a prompt. `io::readChar` returns one whole Unicode scalar
value as a `String` and `io::readByte` returns one raw `Byte`, both reading a
single unit without waiting for a newline and, on a terminal, with echo and
canonical line mode suppressed for the read before the prior mode is restored.
Character and line reads decode input as UTF-8 and reject ill-formed byte
sequences rather than substituting replacement characters; `io::readByte`
transfers bytes verbatim with no decoding. End of input is reported as an error,
not as an empty or sentinel result. `io::pollInput` tests whether input is ready
to read, optionally waiting up to a timeout in milliseconds, without consuming
any input.

The terminal predicates `io::isInputTerminal`, `io::isOutputTerminal`, and
`io::isErrorTerminal` return a `Boolean` reporting whether the corresponding
standard stream is connected to an interactive terminal; they never block,
consume input, or raise. Output is directed to whichever destination is bound to
each standard stream: in a normal console program these are file descriptors 0,
1, and 2; in app mode the same calls are routed to the application transcript
window, which is treated as an interactive terminal.

Standard input is a per-thread broadcast. The runtime owns file descriptor 0 and
reads it in chunks into one process-global append-only log; each subscribed
thread reads its own cursor over that log, so every subscriber sees the whole
stdin stream from its subscription point and a byte read by one thread is never
consumed out from under another. The compiler subscribes the main thread at
program entry, so a single-threaded program is byte-identical to a direct per-byte
reader. A thread other than main must subscribe with `thread::openStdIn` before
reading, or the read raises `ErrInvalidContext`; `thread::closeStdIn`
unsubscribes."#;

/// Register the `io` package on the clean-room registry.
pub(crate) fn register(r: &mut Registry) {
    let mut pkg = RegistryPackage::new("io", MODULE_INTRO, MODULE_DESC);

    func_print::register(&mut pkg);
    func_write::register(&mut pkg);
    func_print_error::register(&mut pkg);
    func_write_error::register(&mut pkg);
    func_flush::register(&mut pkg);
    func_is_buffered::register(&mut pkg);
    func_set_buffered::register(&mut pkg);
    func_input::register(&mut pkg);
    func_read_line::register(&mut pkg);
    func_read_char::register(&mut pkg);
    func_read_byte::register(&mut pkg);
    func_poll_input::register(&mut pkg);
    func_is_input_terminal::register(&mut pkg);
    func_is_output_terminal::register(&mut pkg);
    func_is_error_terminal::register(&mut pkg);

    r.add_package(pkg);
}

// --- shared io abi_function glue (plan-101) ---

/// The error a migrated io abi_function body raises when the target lacks an
/// app-mode io hook (e.g. `io::is*Terminal` on a target with no app backend).
pub(crate) fn app_unsupported(platform: &dyn CodegenPlatform) -> String {
    format!(
        "native target '{}' does not support app-mode io helpers",
        platform.target()
    )
}
