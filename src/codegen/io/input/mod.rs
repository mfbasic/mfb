//! The keyboard input buffer and decoder (bug-699).
//!
//! Every stdin subscriber gets one INPUT BUFFER: the raw bytes that have arrived
//! for it and that no reader has consumed yet, in arrival order ([`buffer`]).
//! Readers take bytes out of it through the DECODER ([`decode`]), which turns the
//! front of the buffer into one token — a text scalar or grapheme, or a key with
//! its event and modifiers — from any of the encodings a terminal or an `--app`
//! surface produces (the tables in [`keys`]). Decoding happens on read and a read
//! consumes only its own token, so readers can be mixed without losing or
//! reordering input.
//!
//! The mouse decoder (`io::mouse`) stays in front: SGR mouse reports are stripped
//! before a byte reaches the input buffer.

pub(crate) mod buffer;
pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod keys;

#[cfg(test)]
mod corpus;
#[cfg(test)]
mod encode_tests;
#[cfg(test)]
mod model;
/// The `.ncode` interpreter the `--app` handler tests use (`tests/common`),
/// shared so the emitted decoder runs on the same machine model. This crate
/// calls only the interpreter, not the registration scan the handler tests use.
#[cfg(test)]
#[allow(dead_code)]
#[path = "../../../../tests/common/ncode_emu.rs"]
mod ncode_emu;
#[cfg(test)]
mod reader_tests;
#[cfg(test)]
mod tests;

/// The one delay input decoding waits (bug-699 Decision 3): how long a prefix
/// that only more input can classify — a lone ESC (the Escape key, or the start
/// of a sequence?), a base character (the whole grapheme, or is a combining mark
/// coming?) — waits for that input on a keyboard before it is decided with what
/// has arrived. Terminals write an escape sequence in one burst, so its tail
/// follows within microseconds; the same trade every curses library makes with
/// its `ESCDELAY`. The mouse decoder's lone-Esc wait is this same delay.
pub(crate) const INPUT_DELAY_MS: u64 = 25;

/// The input-buffer region, per arena (per stdin subscriber — a worker thread
/// that calls `thread::openStdIn` reads through its own).
///
/// Byte offset of the count of bytes the buffer holds; the bytes follow it, the
/// oldest first. Zero is the resting state: nothing pulled ahead of a reader.
pub(crate) const INPUT_STATE_LEN_OFFSET: usize = 0;
/// Byte offset of the buffered bytes.
pub(crate) const INPUT_STATE_BUF_OFFSET: usize = 8;
/// Bytes the buffer holds at most. A token never needs more than a key
/// sequence (a kitty `CSI … u` with alternates and text is well under 64 bytes)
/// or one grapheme (capped at [`GRAPHEME_CAP`]), so a reader looking ahead for
/// one token cannot fill it.
pub(crate) const INPUT_BUFFER_CAPACITY: usize = 256;
/// Total reserved slots.
pub(crate) const INPUT_STATE_SLOTS: usize = (INPUT_STATE_BUF_OFFSET + INPUT_BUFFER_CAPACITY) / 8;

/// The longest grapheme a token carries; a cluster that would grow past it is
/// split there. A combining-mark flood is not a keystroke.
pub(crate) const GRAPHEME_CAP: usize = 128;
const _: () = assert!(GRAPHEME_CAP < INPUT_BUFFER_CAPACITY);

/// The longest parameter string a CSI may carry before it is treated as garbage
/// and dropped (no key sequence comes near it).
pub(crate) const CSI_PARAM_CAP: usize = 64;

/// The runtime symbols of the readers that decode through the input buffer and
/// may leave bytes in it (bug-699 Phase 4: `io::readRaw`, `readScalar`,
/// `readGrapheme`, `readKey`). A program that calls one reserves the region,
/// and every reader in it — the plain ones included — then takes buffered bytes
/// before new ones. The list is empty until those readers exist: without one,
/// nothing can put a byte in the buffer, so no program pays for it and every
/// existing program is byte-identical.
pub(crate) const INPUT_BUFFER_READERS: &[&str] = &[];

/// Whether a program with these runtime symbols reserves the input buffer.
pub(crate) fn program_uses_input_buffer(runtime_symbols: &[String]) -> bool {
    #[cfg(test)]
    if FORCE_INPUT_BUFFER.with(std::cell::Cell::get) {
        return true;
    }
    runtime_symbols
        .iter()
        .any(|symbol| INPUT_BUFFER_READERS.contains(&symbol.as_str()))
}

#[cfg(test)]
thread_local! {
    /// Test-only: reserve the input buffer in every program lowered on this
    /// thread, so the buffered read paths can be built and run before a reader
    /// that needs them exists.
    pub(crate) static FORCE_INPUT_BUFFER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Test-only: where the last program lowered on this thread put the region,
    /// so a test can fill it the way a look-ahead reader would.
    pub(crate) static RESERVED_INPUT_STATE_OFFSET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
