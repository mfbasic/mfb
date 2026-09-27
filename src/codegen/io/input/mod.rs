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
pub(crate) mod reader;

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
/// Byte offset of the count of bytes the buffer holds; the bytes are at
/// [`INPUT_STATE_BUF_OFFSET`], the oldest first. Zero is the resting state:
/// nothing pulled ahead of a reader.
pub(crate) const INPUT_STATE_LEN_OFFSET: usize = 0;
/// The monotonic nanosecond at which the bytes at the front of the buffer
/// started waiting to be classified (a lone ESC, a base character that a
/// combining mark may follow), or 0 when nothing is waiting. Kept across reads
/// so that a program polling with `timeoutMs = 0` still sees the Escape key
/// once [`INPUT_DELAY_MS`] has passed. Any consume from the front clears it.
pub(crate) const INPUT_STATE_PENDING_OFFSET: usize = 8;
/// What standard input is, decided once per thread: 0 not yet asked,
/// [`SOURCE_KEYBOARD`] or [`SOURCE_DATA`].
pub(crate) const INPUT_STATE_SOURCE_OFFSET: usize = 16;
/// The decoder's token record ([`decode::TOKEN_BYTES`]), where every look-ahead
/// reader's last token lands.
pub(crate) const INPUT_STATE_TOKEN_OFFSET: usize = 24;
/// Byte offset of the buffered bytes. One spare word follows the token record,
/// so a NUL can always be written after its grapheme — which makes the record's
/// `{length, bytes}` pair a complete `String` block.
pub(crate) const INPUT_STATE_BUF_OFFSET: usize = INPUT_STATE_TOKEN_OFFSET + decode::TOKEN_BYTES + 8;
/// Bytes the buffer holds at most. A decoded token never needs more than a key
/// sequence (a kitty `CSI … u` with alternates and text is well under 64 bytes)
/// or one grapheme (capped at [`GRAPHEME_CAP`]); the rest is room for a line a
/// timed `io::readLine` gives back when its time runs out.
pub(crate) const INPUT_BUFFER_CAPACITY: usize = 4096;
/// Total reserved slots.
pub(crate) const INPUT_STATE_SLOTS: usize = (INPUT_STATE_BUF_OFFSET + INPUT_BUFFER_CAPACITY) / 8;
const _: () = assert!(INPUT_STATE_BUF_OFFSET % 8 == 0);

/// Standard input is a terminal (console) or an `--app` window: a prefix that
/// only more input can classify waits at most [`INPUT_DELAY_MS`].
pub(crate) const SOURCE_KEYBOARD: u64 = 1;
/// Standard input is a pipe or a file: a prefix waits for the input that
/// completes it, or for its end, so the same file always splits the same way.
pub(crate) const SOURCE_DATA: u64 = 2;

/// The longest grapheme a token carries; a cluster that would grow past it is
/// split there. A combining-mark flood is not a keystroke.
pub(crate) const GRAPHEME_CAP: usize = 128;
const _: () = assert!(GRAPHEME_CAP < INPUT_BUFFER_CAPACITY);

/// The longest parameter string a CSI may carry before it is treated as garbage
/// and dropped (no key sequence comes near it).
pub(crate) const CSI_PARAM_CAP: usize = 64;

/// The runtime symbols of the readers that read through the input buffer
/// (bug-699 Phase 4): the key and text readers look ahead through the decoder,
/// and the two line readers give a half-read line back to the buffer when their
/// time runs out. A program that calls one reserves the region, and every
/// reader in it — the plain ones included — then takes buffered bytes before new
/// ones.
pub(crate) const INPUT_BUFFER_READERS: &[&str] = &[
    "_mfb_rt_io_io_readRaw",
    "_mfb_rt_io_io_readScalar",
    "_mfb_rt_io_io_readGrapheme",
    "_mfb_rt_io_io_readKey",
    "_mfb_rt_io_io_readLine",
    "_mfb_rt_io_io_input",
];

/// Whether a program with these runtime symbols reserves the input buffer.
pub(crate) fn program_uses_input_buffer(runtime_symbols: &[String]) -> bool {
    runtime_symbols
        .iter()
        .any(|symbol| INPUT_BUFFER_READERS.contains(&symbol.as_str()))
}

/// The readers that ask the decoder for whole grapheme clusters.
pub(crate) const CLUSTER_READERS: &[&str] =
    &["_mfb_rt_io_io_readGrapheme", "_mfb_rt_io_io_readKey"];

/// Whether a program with these runtime symbols needs the decoder's grapheme
/// clusters (and so the Unicode property tables).
pub(crate) fn program_uses_grapheme_clusters(runtime_symbols: &[String]) -> bool {
    runtime_symbols
        .iter()
        .any(|symbol| CLUSTER_READERS.contains(&symbol.as_str()))
}

#[cfg(test)]
thread_local! {
    /// Test-only: where the last program lowered on this thread put the region,
    /// so a test can fill it the way a look-ahead reader would.
    pub(crate) static RESERVED_INPUT_STATE_OFFSET: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}
