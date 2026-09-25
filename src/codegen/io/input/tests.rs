//! The decoder corpus, run against the reference model.

use super::corpus::{cases, Case};
use super::model::{decode, Outcome, FINAL};

/// Decode `case` the way a reader would: token after token until the input is
/// used up (STREAM), or once (SINGLE).
pub(super) fn run(case: &Case, decode: &dyn Fn(&[u8], u64) -> Outcome) -> Vec<Outcome> {
    if !case.stream {
        return vec![decode(&case.input, case.flags)];
    }
    let mut out = Vec::new();
    let mut at = 0;
    while at < case.input.len() {
        let outcome = decode(&case.input[at..], case.flags | FINAL);
        let n = match &outcome {
            Outcome::Token(token) => token.consumed,
            Outcome::Skip(n) => *n,
            Outcome::NeedMore => {
                out.push(outcome);
                break;
            }
        };
        assert!(
            n > 0,
            "{}: a decode that consumes nothing never ends",
            case.name
        );
        out.push(outcome);
        at += n;
    }
    out
}

#[test]
fn the_model_decodes_the_whole_corpus() {
    let mut failures = Vec::new();
    for case in cases() {
        let got = run(&case, &decode);
        if got != case.expect {
            failures.push(format!(
                "{}: input {:?}\n    want {:?}\n    got  {:?}",
                case.name, case.input, case.expect, got
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} corpus case(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn the_corpus_covers_every_named_key() {
    use super::keys::KeyName;
    use super::model::Token;
    let mut seen = std::collections::HashSet::new();
    for case in cases() {
        for outcome in &case.expect {
            if let Outcome::Token(Token { key, .. }) = outcome {
                seen.insert(*key as u64);
            }
        }
    }
    for ordinal in 0..=KeyName::F12 as u64 {
        assert!(
            seen.contains(&ordinal),
            "no corpus case produces KeyName ordinal {ordinal}"
        );
    }
}

/// The control for the test above: a decoder that is wrong in one small way
/// (Ctrl+letters lose Ctrl) must fail the corpus, or the corpus pins nothing.
#[test]
fn the_corpus_rejects_a_slightly_wrong_decoder() {
    let wrong = |buf: &[u8], flags: u64| match decode(buf, flags) {
        Outcome::Token(mut token) => {
            token.mods &= !super::keys::MOD_CTRL;
            Outcome::Token(token)
        }
        other => other,
    };
    let failing = cases()
        .iter()
        .filter(|case| run(case, &wrong) != case.expect)
        .count();
    assert!(
        failing >= 10,
        "only {failing} case(s) notice Ctrl being dropped"
    );
}

// --- The emitted decoder -------------------------------------------------------
//
// `_mfb_rt_input_decode` as the compiler emits it for macos-aarch64, run in the
// `.ncode` interpreter: on the whole corpus, and against the model on random
// input.

use super::decode::*;
use super::keys::{KeyEvent, KeyName};
use super::model::{Text, Token, CLUSTER, EVENTS};
use super::ncode_emu::{Externals, Machine};
use std::sync::OnceLock;

/// The decoder calls nothing: any external call is a bug in it.
struct NoPlatform;
impl Externals for NoPlatform {
    fn call(&mut self, _: &mut Machine, _: &str) -> Option<u64> {
        None
    }
}

fn decoder_dump() -> &'static serde_json::Value {
    decoder_dump_for(crate::testutil::CodeTarget::MacosAarch64)
}

/// The decoder as lowered for `target`, built once per target.
fn decoder_dump_for(target: crate::testutil::CodeTarget) -> &'static serde_json::Value {
    use std::collections::HashMap;
    use std::sync::Mutex;
    static DUMPS: OnceLock<Mutex<HashMap<&'static str, &'static serde_json::Value>>> =
        OnceLock::new();
    let dumps = DUMPS.get_or_init(Default::default);
    let mut dumps = dumps
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    dumps.entry(target.name()).or_insert_with(|| {
        let (plan, _) = crate::testutil::code_for_src_with_input_buffer(
            "FUNC main AS Integer\n  RETURN 0\nEND FUNC\n",
            target,
            crate::target::NativeBuildMode::Console,
        );
        Box::leak(Box::new(
            serde_json::from_str(&plan.to_json()).expect("the ncode dump is JSON"),
        ))
    })
}

const KEY_NAMES: [KeyName; 27] = [
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
const KEY_EVENTS: [KeyEvent; 4] = [
    KeyEvent::Press,
    KeyEvent::Repeat,
    KeyEvent::Release,
    KeyEvent::NotSupported,
];

/// One machine per test thread, reused for every decode: the call is pure, so
/// nothing a decode leaves behind can affect the next.
struct Emitted {
    machine: Machine<'static>,
    buf: u64,
    out: u64,
}

impl Emitted {
    fn new() -> Emitted {
        Emitted::for_dump(decoder_dump())
    }

    fn for_dump(dump: &'static serde_json::Value) -> Emitted {
        let mut machine = Machine::new(dump);
        let buf = machine.alloc(1024);
        let out = machine.alloc(TOKEN_BYTES);
        Emitted { machine, buf, out }
    }

    fn decode(&mut self, bytes: &[u8], flags: u64) -> Outcome {
        assert!(bytes.len() <= 1024);
        let m = &mut self.machine;
        m.write_bytes(self.buf, bytes);
        m.write_bytes(self.out, &[0xAA; TOKEN_BYTES]);
        let status = m.call(
            INPUT_DECODE_SYMBOL,
            &[self.buf, bytes.len() as u64, flags, self.out],
            &mut NoPlatform,
        );
        let field = |offset: usize| m.read_u64(self.out + offset as u64);
        // The result register is the MFB one, which is not the C result register
        // on every target; the AArch64 PCS is where they coincide.
        if m.abi == super::ncode_emu::Abi::Aarch64 {
            assert_eq!(
                status,
                field(TOKEN_STATUS),
                "returned status = stored status"
            );
        }
        match field(TOKEN_STATUS) {
            STATUS_NEED_MORE => Outcome::NeedMore,
            STATUS_SKIP => Outcome::Skip(field(TOKEN_CONSUMED) as usize),
            STATUS_TOKEN => {
                let glen = field(TOKEN_GRAPHEME_LEN) as usize;
                Outcome::Token(Token {
                    consumed: field(TOKEN_CONSUMED) as usize,
                    key: KEY_NAMES[field(TOKEN_KEY) as usize],
                    event: KEY_EVENTS[field(TOKEN_EVENT) as usize],
                    mods: field(TOKEN_MODS),
                    grapheme: m.read_bytes(self.out + TOKEN_GRAPHEME as u64, glen),
                    text: match field(TOKEN_TEXT_KIND) {
                        TEXT_GRAPHEME => Text::Grapheme,
                        TEXT_SCALAR => Text::Scalar(field(TOKEN_TEXT_SCALAR) as u8),
                        TEXT_NONE => Text::None,
                        other => panic!("text kind {other}"),
                    },
                })
            }
            other => panic!("status {other}"),
        }
    }
}

#[test]
fn the_decoder_flags_are_the_models() {
    assert_eq!(
        (FLAG_FINAL, FLAG_EVENTS, FLAG_CLUSTER),
        (FINAL, EVENTS, CLUSTER)
    );
    for (i, key) in KEY_NAMES.iter().enumerate() {
        assert_eq!(*key as usize, i);
    }
    for (i, event) in KEY_EVENTS.iter().enumerate() {
        assert_eq!(*event as usize, i);
    }
}

#[test]
fn the_emitted_decoder_decodes_the_whole_corpus() {
    let mut emitted = Emitted::new();
    let mut failures = Vec::new();
    for case in cases() {
        let got = run(&case, &|bytes, flags| emitted_decode_shim(bytes, flags));
        if got != case.expect {
            failures.push(format!(
                "{}: input {:?}\n    want {:?}\n    got  {:?}",
                case.name, case.input, case.expect, got
            ));
        }
    }
    let _ = &mut emitted;
    assert!(
        failures.is_empty(),
        "{} corpus case(s) failed:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

thread_local! {
    static EMITTED: std::cell::RefCell<Option<Emitted>> = const { std::cell::RefCell::new(None) };
}

/// `run` takes a `Fn`; the machine is mutable, so it lives in a thread-local.
fn emitted_decode_shim(bytes: &[u8], flags: u64) -> Outcome {
    EMITTED.with(|cell| {
        let mut cell = cell.borrow_mut();
        cell.get_or_insert_with(Emitted::new).decode(bytes, flags)
    })
}

/// Random input, emitted decoder against the model. The alphabet is fragments
/// of everything the decoder parses — sequence introducers, parameters,
/// finals, UTF-8 of each length, invalid bytes, controls — so random strings
/// hit the parser's corners far more often than random bytes would. Clusters
/// are drawn only from characters whose segmentation both implementations
/// agree on by construction (combining acute, ZWJ, a pictograph, regional
/// indicators): the emitted decoder's Unicode tables and `unicode-segmentation`
/// are separate snapshots, and a version skew is not what this compares.
#[test]
fn the_emitted_decoder_agrees_with_the_model_on_random_input() {
    let fragments: &[&[u8]] = &[
        b"\x1b",
        b"\x1b",
        b"[",
        b"[",
        b"O",
        b"1",
        b"2",
        b"3",
        b"5",
        b"9",
        b"13",
        b"15",
        b"27",
        b"97",
        b"57399",
        b"57441",
        b";",
        b";",
        b":",
        b"~",
        b"u",
        b"A",
        b"D",
        b"H",
        b"P",
        b"R",
        b"Z",
        b"$",
        b"^",
        b"@",
        b"a",
        b"d",
        b"u",
        b"<",
        b"?",
        b" ",
        b"x",
        b"\r",
        b"\n",
        b"\x7f",
        b"\x08",
        b"\x01",
        b"\x00",
        b"\t",
        b"\xff",
        b"\x80",
        b"\xc3",
        b"\xa9",
        b"\xe2\x82\xac",
        b"\xe2\x82",
        b"\xf0\x9f\x98\x80",
        b"\xed\xa0\x80",
        "\u{301}".as_bytes(),
        "\u{200D}".as_bytes(),
        "\u{1F468}".as_bytes(),
        "\u{1F1EF}".as_bytes(),
        b"e",
    ];
    let mut state: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut emitted = Emitted::new();
    let mut disagreements = Vec::new();
    for _ in 0..4000 {
        let parts = 1 + (next() % 8) as usize;
        let mut input = Vec::new();
        for _ in 0..parts {
            input.extend_from_slice(fragments[(next() % fragments.len() as u64) as usize]);
        }
        let flags = next() % 8;
        let want = decode(&input, flags);
        let got = emitted.decode(&input, flags);
        if got != want && disagreements.len() < 20 {
            disagreements.push(format!(
                "{input:?} flags {flags}:\n    model   {want:?}\n    emitted {got:?}"
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the emitted decoder disagrees with the model:\n{}",
        disagreements.join("\n")
    );
}

/// The same corpus through every backend's lowering of the decoder: register
/// allocation, the x86 two-operand forms and RISC-V's emulated condition flags
/// all differ from AArch64's, and the decoder is long enough to exercise them.
#[test]
fn the_decoder_decodes_the_corpus_as_every_backend_lowers_it() {
    use crate::testutil::CodeTarget;
    for target in CodeTarget::ALL {
        if target == CodeTarget::MacosAarch64 {
            continue; // the case above
        }
        let mut emitted = Emitted::for_dump(decoder_dump_for(target));
        let mut failures = Vec::new();
        for case in cases() {
            let got = if case.stream {
                let mut out = Vec::new();
                let mut at = 0;
                while at < case.input.len() {
                    let outcome = emitted.decode(&case.input[at..], case.flags | FINAL);
                    let n = match &outcome {
                        Outcome::Token(token) => token.consumed,
                        Outcome::Skip(n) => *n,
                        Outcome::NeedMore => {
                            out.push(outcome);
                            break;
                        }
                    };
                    out.push(outcome);
                    at += n.max(1);
                }
                out
            } else {
                vec![emitted.decode(&case.input, case.flags)]
            };
            if got != case.expect {
                failures.push(format!("{}: got {:?}", case.name, got));
            }
        }
        assert!(
            failures.is_empty(),
            "{}: {} corpus case(s) failed:\n{}",
            target.name(),
            failures.len(),
            failures.join("\n")
        );
    }
}
