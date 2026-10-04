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
use crate::codegen::io::input::keys::{KeyEvent, KeyName};
use crate::codegen::registry::{
    EnumVariant, RecordProp, Registry, RegistryEnum, RegistryPackage, RegistryRecord,
};
use crate::types::ParameterType;

mod func_flush;
mod func_input;
mod func_is_buffered;
mod func_is_error_terminal;
mod func_is_input_terminal;
mod func_is_output_terminal;
mod func_print;
mod func_print_error;
mod func_read_grapheme;
mod func_read_key;
mod func_read_line;
mod func_read_raw;
mod func_read_scalar;
mod func_set_buffered;
mod func_write;
mod func_write_error;
mod gen_is_terminal;
pub(crate) mod gen_read_family;
pub(crate) mod gen_read_key_family;
mod gen_read_line_family;
pub(crate) mod gen_write_family;

const MODULE_INTRO: &str = r#"Standard stream input/output and terminal inspection"#;
const MODULE_DESC: &str = r#"The `io` package provides access to the three standard streams — standard
input, standard output, and standard error — together with helpers for reading
the keyboard and inspecting which streams are terminals. It is the console
counterpart to the `fs` package: where `fs` works through named files and `File`
handles, `io` works through the process standard streams. `io` is a built-in
package: `IMPORT io` needs no manifest dependency.

`io::print` and `io::write` accept a `String` or an `AttributedString`; the
error-output functions accept a `String`. None of them convert implicitly, so
convert other values with `toString` first. Text is treated as UTF-8
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

Input functions read from standard input, and each reads either what was typed or
the bytes that were sent. From a terminal or an `--app` window they read keys:
`io::readKey` returns one key as an `io::Key` — the key's name, its text, its
modifiers, and whether it was pressed, repeated or released — so the arrows,
Home, End, Escape, the function keys and Ctrl or Alt combinations come out the
same on every platform. `io::readGrapheme` and `io::readScalar` return only text,
one user-perceived character or one Unicode scalar at a time, and skip keys that
type nothing. From a pipe or a file the same calls read the data as it is: an
escape byte in the data is text, not a key. `io::readRaw` returns the next byte
exactly as it arrived, whatever the source.

`io::input` reads a whole line with normal terminal echo and an optional prompt;
`io::readLine` reads a line without a prompt and, on a terminal, suppresses echo
while it reads — which is what makes it the one to use for a passphrase. Line
and text reads reject ill-formed UTF-8 rather than substituting replacement
characters.

Every reader takes an optional `timeoutMs`: omitted, the call waits for as long
as it takes; `0` makes one immediate attempt, which is how a game loop asks
"was a key pressed?" without stopping; a positive value waits up to that many
milliseconds. A wait that runs out raises `ErrTimeout` and loses nothing — a
half-read line is still there for the next read. End of input is reported as an
error, not as an empty or sentinel result, once no input is left: a final line
with no terminator is still returned, and only the next read reports end of
input.

The terminal predicates `io::isInputTerminal`, `io::isOutputTerminal`, and
`io::isErrorTerminal` return a `Boolean` reporting whether the corresponding
standard stream is connected to an interactive terminal; they never block,
read input, or raise. Each call uses its own standard stream; in app mode all of
them are routed to the application transcript window, which is treated as an
interactive terminal.

Each thread that reads standard input has its own view of it: reading in one
thread does not take input away from another, and a thread sees the input that
arrives after it subscribes. The main thread can read without subscribing. A
worker thread must subscribe with `thread::openStdIn` before reading, or the read
raises `ErrInvalidContext`; at most 128 threads can be subscribed at once, and a
thread beyond that cannot read and gets the same error; `thread::closeStdIn`
unsubscribes."#;

/// The `Key` value record `io::readKey` returns (bug-699 Decision 4).
pub(crate) const KEY_TYPE: &str = "Key";
/// Its package-qualified identity (bug-480 Phase 4b): what the record marshaller
/// and a consumer name.
pub(crate) const KEY_TYPE_ID: &str = "io.Key";
pub(crate) const KEY_NAME_TYPE: &str = "KeyName";
pub(crate) const KEY_EVENT_TYPE: &str = "KeyEvent";

/// A `KeyName` variant's man-page description. The variants are declared in
/// `keys::KeyName` order, because a variant's ordinal is what the decoder stores.
fn key_name_description(key: KeyName) -> &'static str {
    match key {
        KeyName::Unicode => {
            "A key that types text — a letter, a digit, a symbol, the space bar. \
             The text is in `grapheme`."
        }
        KeyName::Up => "The up arrow.",
        KeyName::Down => "The down arrow.",
        KeyName::Left => "The left arrow.",
        KeyName::Right => "The right arrow.",
        KeyName::Home => "Home.",
        KeyName::End => "End. (Spelled `EndKey`: `END` is a keyword.)",
        KeyName::PageUp => "Page Up.",
        KeyName::PageDown => "Page Down.",
        KeyName::Insert => "Insert.",
        KeyName::Delete => "Delete — the key that deletes forward. Backspace is its own key.",
        KeyName::Backspace => "Backspace. Its `grapheme` is U+007F.",
        KeyName::Enter => "Return or Enter, the keypad's too. Its `grapheme` is `\"\\n\"`.",
        KeyName::Tab => "Tab. Shift+Tab is `Tab` with `shift` set. Its `grapheme` is `\"\\t\"`.",
        KeyName::Escape => "Esc. Its `grapheme` is U+001B.",
        KeyName::F1 => "The function key F1.",
        KeyName::F2 => "The function key F2.",
        KeyName::F3 => "The function key F3.",
        KeyName::F4 => "The function key F4.",
        KeyName::F5 => "The function key F5.",
        KeyName::F6 => "The function key F6.",
        KeyName::F7 => "The function key F7.",
        KeyName::F8 => "The function key F8.",
        KeyName::F9 => "The function key F9.",
        KeyName::F10 => "The function key F10.",
        KeyName::F11 => "The function key F11.",
        KeyName::F12 => "The function key F12.",
    }
}

/// Every `KeyName`, in declaration (ordinal) order.
pub(crate) const KEY_NAMES: [KeyName; 27] = [
    KeyName::Unicode,
    KeyName::Up,
    KeyName::Down,
    KeyName::Left,
    KeyName::Right,
    KeyName::Home,
    KeyName::End,
    KeyName::PageUp,
    KeyName::PageDown,
    KeyName::Insert,
    KeyName::Delete,
    KeyName::Backspace,
    KeyName::Enter,
    KeyName::Tab,
    KeyName::Escape,
    KeyName::F1,
    KeyName::F2,
    KeyName::F3,
    KeyName::F4,
    KeyName::F5,
    KeyName::F6,
    KeyName::F7,
    KeyName::F8,
    KeyName::F9,
    KeyName::F10,
    KeyName::F11,
    KeyName::F12,
];

/// The key record and its two enums (bug-699 Decision 4).
fn register_key_types(pkg: &mut RegistryPackage) {
    pkg.add_record(RegistryRecord {
        name: KEY_TYPE,
        export: true,
        description: "One key event, as `io::readKey` reports it: which key, what \
                      it typed, whether it was pressed, repeated or released, and \
                      which modifiers were held.",
        props: vec![
            RecordProp {
                name: "key",
                ty: ParameterType::named(KEY_NAME_TYPE),
                description: "Which key. `io::KeyName.Unicode` for a key that types \
                              text; the key's own name otherwise.",
            },
            RecordProp {
                name: "grapheme",
                ty: ParameterType::String,
                description: "What the keystroke typed — what `io::readGrapheme` \
                              would return for it — or `\"\"` for a key with no \
                              Unicode value. For a Ctrl chord it is the key's own \
                              character (`\"a\"` for Ctrl+A).",
            },
            RecordProp {
                name: "event",
                ty: ParameterType::named(KEY_EVENT_TYPE),
                description: "Press, repeat or release — or `NotSupported` when the \
                              input cannot tell them apart.",
            },
            RecordProp {
                name: "shift",
                ty: ParameterType::Boolean,
                description: "`TRUE` when Shift was held.",
            },
            RecordProp {
                name: "ctrl",
                ty: ParameterType::Boolean,
                description: "`TRUE` when Control was held.",
            },
            RecordProp {
                name: "alt",
                ty: ParameterType::Boolean,
                description: "`TRUE` when Alt was held — Option on macOS.",
            },
            RecordProp {
                name: "meta",
                ty: ParameterType::Boolean,
                description: "`TRUE` when Meta was held — Command on macOS, the \
                              Windows or Super key elsewhere.",
            },
        ],
    });
    pkg.add_enum(RegistryEnum {
        name: KEY_NAME_TYPE,
        export: true,
        variants: KEY_NAMES
            .iter()
            .map(|key| EnumVariant {
                name: key_name_spelling(*key),
                description: key_name_description(*key),
                advisory: None,
            })
            .collect(),
    });
    pkg.add_enum(RegistryEnum {
        name: KEY_EVENT_TYPE,
        export: true,
        variants: [
            (KeyEvent::Press, "The key went down."),
            (KeyEvent::Repeat, "The key is held down and repeating."),
            (
                KeyEvent::Release,
                "The key came up. Its `grapheme` is still the key's character; \
                 a release types nothing.",
            ),
            (
                KeyEvent::NotSupported,
                "The input cannot tell a press from a repeat or a release — a \
                 terminal outside the kitty keyboard protocol, a pipe, a file. \
                 Every key is reported once, as it is typed, and no release \
                 arrives.",
            ),
        ]
        .into_iter()
        .map(|(event, description)| EnumVariant {
            name: key_event_spelling(event),
            description,
            advisory: None,
        })
        .collect(),
    });
}

fn key_name_spelling(key: KeyName) -> &'static str {
    match key {
        KeyName::Unicode => "Unicode",
        KeyName::Up => "Up",
        KeyName::Down => "Down",
        KeyName::Left => "Left",
        KeyName::Right => "Right",
        KeyName::Home => "Home",
        // `END` is a reserved keyword (case-insensitive), so a variant cannot be
        // spelled `End`: neither the ENUM body nor `io::KeyName.End` would parse.
        // The noun goes with it, as `endIndex` does for a range's end.
        KeyName::End => "EndKey",
        KeyName::PageUp => "PageUp",
        KeyName::PageDown => "PageDown",
        KeyName::Insert => "Insert",
        KeyName::Delete => "Delete",
        KeyName::Backspace => "Backspace",
        KeyName::Enter => "Enter",
        KeyName::Tab => "Tab",
        KeyName::Escape => "Escape",
        KeyName::F1 => "F1",
        KeyName::F2 => "F2",
        KeyName::F3 => "F3",
        KeyName::F4 => "F4",
        KeyName::F5 => "F5",
        KeyName::F6 => "F6",
        KeyName::F7 => "F7",
        KeyName::F8 => "F8",
        KeyName::F9 => "F9",
        KeyName::F10 => "F10",
        KeyName::F11 => "F11",
        KeyName::F12 => "F12",
    }
}

fn key_event_spelling(event: KeyEvent) -> &'static str {
    match event {
        KeyEvent::Press => "Press",
        KeyEvent::Repeat => "Repeat",
        KeyEvent::Release => "Release",
        KeyEvent::NotSupported => "NotSupported",
    }
}

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
    func_read_raw::register(&mut pkg);
    func_read_scalar::register(&mut pkg);
    func_read_grapheme::register(&mut pkg);
    func_read_key::register(&mut pkg);
    register_key_types(&mut pkg);
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
