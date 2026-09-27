//! `io::readKey` — descriptor entry + authored docs (bug-699).
//!
//! Per-member file; the body is the shared keystroke-reader lowering in
//! [`super::gen_read_key_family`].

use super::func_read_raw::TIMEOUT_DESC;
use super::gen_read_key_family::{lower_key_reader, KeyReader};
use crate::codegen::engine::builder::{CodeBuilder, ValueResult};
use crate::codegen::registry::{
    AbiCtx, Body, DefaultValue, Implementation, Parameter, RegistryFunction, RegistryPackage,
};
use crate::codegen::error::constants::TIMEOUT_UNBOUNDED_SENTINEL;
use crate::types::ParameterType;

pub(crate) fn lower_read_key(
    builder: &mut CodeBuilder,
    args: &[ValueResult],
    ctx: &AbiCtx,
) -> Result<ValueResult, String> {
    lower_key_reader(builder, args, ctx, KeyReader::Key)
}

const INTRO: &str = r#"Read the next key event — which key, what it typed, and which modifiers were held"#;
const DESC: &str = r#"`io::readKey` returns the next key event from standard input as an `io::Key`.
It is the reader for keyboard-driven programs — games, editors, menus — and it
answers the same way for the same keystroke on every platform: a console on any
terminal, and an `--app` window on macOS, Linux and Windows.

The fields:

- `key` names the key: `io::KeyName.Unicode` for a key that types text, or
  `Up`, `Down`, `Left`, `Right`, `Home`, `EndKey` (the End key — `END` is a
  keyword), `PageUp`, `PageDown`, `Insert`, `Delete`, `Backspace`, `Enter`,
  `Tab`, `Escape`, `F1` … `F12`.
- `grapheme` is what `io::readGrapheme` would return for the keystroke: `"a"`,
  `"A"` for Shift+A, `"\n"` for Enter, `"\t"` for Tab, U+007F for Backspace,
  U+001B for Escape. It is `""` for a key with no Unicode value. For Ctrl+A it is
  `"a"`, with `ctrl` set.
- `event` is `Press`, `Repeat` (the key is held down and repeating) or `Release`
  — or `NotSupported` when the input cannot tell them apart.
- `shift`, `ctrl`, `alt` and `meta` report the modifiers. `alt` is Option on
  macOS; `meta` is Command on macOS and the Windows or Super key elsewhere.

**Which events you see depends on the input.** An `--app` window always reports
`Press`, `Repeat` and `Release`, with every modifier. So does a terminal that
speaks the kitty keyboard protocol (the kitty terminal, for one) while
`term::on` is active. Everywhere else — another terminal,
a console outside `term::on`, a pipe, a file — every key arrives once, as it is
typed, with `event` = `io::KeyEvent.NotSupported`, and no release ever arrives.
There, the modifiers are the ones a plain terminal can express:

| Keystroke | What a plain terminal reports |
| --- | --- |
| Ctrl+letter | the letter, with `ctrl` |
| Alt+key | the key, with `alt` (the terminal sends Esc first) |
| Shift, Ctrl or Alt with an arrow, Home, End, Page Up/Down, Insert, Delete, F-key | the key, with those modifiers |
| Shift+letter | the capital letter; `shift` is not set |
| any key with Meta | the key; `meta` is never set |

**Escape needs a moment.** From a keyboard, the Escape key and the start of an
arrow key's sequence begin with the same byte, so a lone Escape is reported 25
milliseconds after it arrives — or at once, when the next key shows it was not a
sequence. From a pipe or a file there is no timer: a key is complete once the
input after it has arrived or the input has ended, so a timed call on a pipe can
time out while the last key to arrive is still undecided.

Keys `io::KeyName` has no name for — a modifier key on its own, F13 and above,
media keys — are read and skipped, as are bytes that are not valid UTF-8.

`timeoutMs` bounds the wait: omitted, the call waits for as long as it takes;
`0` makes one immediate attempt, which is how a game loop checks for input
without stopping; a positive value waits up to that many milliseconds. When the
time runs out the call raises `ErrTimeout` and nothing is read. A negative
`timeoutMs` raises `ErrInvalidArgument`. End of input raises `ErrEndOfFile`.

On a terminal each key is read as it is pressed, with nothing echoed, and the
previous mode is restored before the call returns. The `io` readers share one
queue of input, so they can be mixed without losing or reordering anything. A
thread other than the main thread must subscribe with `thread::openStdIn` before
reading, or the call raises `ErrInvalidContext`."#;
const EX: &str = r#"Report one key:

```
IMPORT io

SUB main()
  LET k AS io::Key = io::readKey()
  IF k.key = io::KeyName.Unicode THEN
    io::print("typed " & k.grapheme)
  ELSE
    io::print("a key with no text")
  END IF
END SUB
```

Move with the arrow keys; stop on Escape or `q`:

```
IMPORT io

SUB main()
  MUT x AS Integer = 0
  MUT y AS Integer = 0
  MUT running AS Boolean = TRUE
  WHILE running
    LET k AS io::Key = io::readKey()
    IF k.event <> io::KeyEvent.Release THEN
      MATCH k.key
        CASE io::KeyName.Left
          x = x - 1
        CASE io::KeyName.Right
          x = x + 1
        CASE io::KeyName.Up
          y = y - 1
        CASE io::KeyName.Down
          y = y + 1
        CASE io::KeyName.Escape
          running = FALSE
        CASE ELSE
          IF k.grapheme = "q" THEN
            running = FALSE
          END IF
      END MATCH
    END IF
  END WHILE
  io::print(toString(x) & "," & toString(y))
END SUB
```

Poll for a key once per frame without waiting:

```
IMPORT errorCode
IMPORT io

SUB main()
  LET k AS io::Key = io::readKey(0) TRAP(e)
    IF e.code = errorCode::ErrTimeout THEN
      io::print("no key this frame")
      EXIT SUB
    END IF
    FAIL e
  END TRAP
  io::print("key " & k.grapheme)
END SUB
```"#;

pub(crate) fn register(pkg: &mut RegistryPackage) {
    pkg.add_function(RegistryFunction {
        name: "readKey",
        intro: INTRO,
        desc: DESC,
        example: EX,
        expected_arguments: Some("Integer"),
        internal_only: false,
        implementations: vec![Implementation {
            params: vec![Parameter {
                name: "timeoutMs",
                desc: TIMEOUT_DESC,
                aliases: &[],
                ty: ParameterType::Integer,
                // An omitted timeout waits without bound: the convention's
                // sentinel, padded at the call.
                default: DefaultValue::Fill {
                    type_name: ParameterType::Integer,
                    expr: TIMEOUT_UNBOUNDED_SENTINEL,
                },
            }],
            return_type: ParameterType::named(super::KEY_TYPE),
            errors: vec![
                "ErrTimeout",
                "ErrInvalidArgument",
                "ErrEndOfFile",
                "ErrInputFailed",
                "ErrInvalidContext",
                "ErrOutOfMemory",
            ],
            body: Body::abi_function(lower_read_key),
        }],
    });
}
