//! The decoder test corpus (bug-699 Phase 1).
//!
//! Every expectation here is written out by hand from the protocol documents —
//! the kitty keyboard protocol (<https://sw.kovidgoyal.net/kitty/keyboard-protocol/>),
//! xterm's "Control Sequences", the rxvt/VT220/Linux-console spellings in the bug
//! doc's Goal table, UTF-8 (RFC 3629, and Unicode §3.9's "maximal subpart" rule
//! for how much of an ill-formed sequence one error covers) and UAX #29 for
//! clusters. None of it is computed from the decoder's own tables, so a wrong
//! table row fails a case instead of being echoed by it.
//!
//! Two shapes of case:
//! - a STREAM is decoded token by token until the input is used up, with
//!   `FINAL` set (the whole input has arrived, nothing more is coming), and must
//!   produce exactly the listed outcomes;
//! - a SINGLE is one decode of an input that may be incomplete, with the flags
//!   given — how "wait for more" and "the delay expired" are pinned.

use super::keys::{KeyEvent, KeyName, MOD_ALT, MOD_CTRL, MOD_META, MOD_SHIFT};
use super::model::{Outcome, Text, Token, CLUSTER, EVENTS, FINAL};

pub(crate) struct Case {
    pub(crate) name: String,
    pub(crate) input: Vec<u8>,
    pub(crate) flags: u64,
    /// `true`: decode to the end with `FINAL`; `false`: one decode with `flags`.
    pub(crate) stream: bool,
    pub(crate) expect: Vec<Outcome>,
}

use KeyEvent::{NotSupported as NS, Press, Release, Repeat};
use KeyName::*;

/// A named key. Its grapheme and text are Decision 2's: Enter `"\n"`, Tab
/// `"\t"`, Backspace `"\u{7F}"`, Escape `"\u{1B}"`, every other named key `""`
/// with no text; a release carries no text.
fn k(consumed: usize, key: KeyName, event: KeyEvent, mods: u64) -> Outcome {
    let grapheme: &[u8] = match key {
        Enter => b"\n",
        Tab => b"\t",
        Backspace => b"\x7f",
        Escape => b"\x1b",
        _ => b"",
    };
    let text = if grapheme.is_empty() || event == Release {
        Text::None
    } else {
        Text::Grapheme
    };
    Outcome::Token(Token {
        consumed,
        key,
        event,
        mods,
        grapheme: grapheme.to_vec(),
        text,
    })
}

/// A Unicode key typing `grapheme`, whose text is the grapheme.
fn u(consumed: usize, grapheme: &str, event: KeyEvent, mods: u64) -> Outcome {
    Outcome::Token(Token {
        consumed,
        key: Unicode,
        event,
        mods,
        grapheme: grapheme.as_bytes().to_vec(),
        text: if event == Release {
            Text::None
        } else {
            Text::Grapheme
        },
    })
}

/// A Unicode key whose text is one scalar other than its grapheme (Ctrl+A).
fn uc(consumed: usize, grapheme: &str, event: KeyEvent, mods: u64, scalar: u8) -> Outcome {
    Outcome::Token(Token {
        consumed,
        key: Unicode,
        event,
        mods,
        grapheme: grapheme.as_bytes().to_vec(),
        text: Text::Scalar(scalar),
    })
}

fn skip(n: usize) -> Outcome {
    Outcome::Skip(n)
}

fn stream(name: &str, input: &[u8], flags: u64, expect: Vec<Outcome>) -> Case {
    Case {
        name: name.to_string(),
        input: input.to_vec(),
        flags,
        stream: true,
        expect,
    }
}

fn single(name: &str, input: &[u8], flags: u64, expect: Outcome) -> Case {
    Case {
        name: name.to_string(),
        input: input.to_vec(),
        flags,
        stream: false,
        expect: vec![expect],
    }
}

/// Both event regimes for a legacy sequence: no event field means Press when
/// the source reports events (an `--app` surface, kitty on) and NotSupported
/// when it cannot (a plain terminal, a pipe).
fn legacy(name: &str, input: &[u8], key: KeyName, mods: u64, out: &mut Vec<Case>) {
    let n = input.len();
    out.push(stream(
        &format!("{name} (plain)"),
        input,
        0,
        vec![k(n, key, NS, mods)],
    ));
    out.push(stream(
        &format!("{name} (events)"),
        input,
        EVENTS,
        vec![k(n, key, Press, mods)],
    ));
}

pub(crate) fn cases() -> Vec<Case> {
    let mut c = Vec::new();

    // --- The Goal table, canonical spellings ---------------------------------
    legacy("Up CSI A", b"\x1b[A", Up, 0, &mut c);
    legacy("Down CSI B", b"\x1b[B", Down, 0, &mut c);
    legacy("Right CSI C", b"\x1b[C", Right, 0, &mut c);
    legacy("Left CSI D", b"\x1b[D", Left, 0, &mut c);
    legacy("Home CSI H", b"\x1b[H", Home, 0, &mut c);
    legacy("End CSI F", b"\x1b[F", End, 0, &mut c);
    legacy("Insert CSI 2~", b"\x1b[2~", Insert, 0, &mut c);
    legacy("Delete CSI 3~", b"\x1b[3~", Delete, 0, &mut c);
    legacy("PageUp CSI 5~", b"\x1b[5~", PageUp, 0, &mut c);
    legacy("PageDown CSI 6~", b"\x1b[6~", PageDown, 0, &mut c);
    // F1–F4 SS3, F5–F12 CSI n~ — SS3 is legacy-only, so it never reports events.
    for (seq, key) in [
        (&b"\x1bOP"[..], F1),
        (b"\x1bOQ", F2),
        (b"\x1bOR", F3),
        (b"\x1bOS", F4),
    ] {
        c.push(stream(
            &format!("{key:?} SS3"),
            seq,
            EVENTS,
            vec![k(3, key, NS, 0)],
        ));
    }
    for (seq, key) in [
        (&b"\x1b[15~"[..], F5),
        (b"\x1b[17~", F6),
        (b"\x1b[18~", F7),
        (b"\x1b[19~", F8),
        (b"\x1b[20~", F9),
        (b"\x1b[21~", F10),
        (b"\x1b[23~", F11),
        (b"\x1b[24~", F12),
    ] {
        legacy(&format!("{key:?} CSI n~"), seq, key, 0, &mut c);
    }
    // Modified keys: CSI 1 ; m X, m = 1 + (shift 1 | alt 2 | ctrl 4 | meta 8).
    legacy("Shift+Up", b"\x1b[1;2A", Up, MOD_SHIFT, &mut c);
    legacy("Alt+Right", b"\x1b[1;3C", Right, MOD_ALT, &mut c);
    legacy("Ctrl+Left", b"\x1b[1;5D", Left, MOD_CTRL, &mut c);
    legacy(
        "Meta+Down (xterm meta / kitty super)",
        b"\x1b[1;9B",
        Down,
        MOD_META,
        &mut c,
    );
    legacy(
        "Ctrl+Shift+Home",
        b"\x1b[1;6H",
        Home,
        MOD_CTRL | MOD_SHIFT,
        &mut c,
    );
    legacy("Ctrl+Delete", b"\x1b[3;5~", Delete, MOD_CTRL, &mut c);
    legacy("Shift+F5", b"\x1b[15;2~", F5, MOD_SHIFT, &mut c);
    legacy("Shift+F1 CSI 1;2P", b"\x1b[1;2P", F1, MOD_SHIFT, &mut c);
    legacy(
        "Ctrl+F3 CSI 1;5R (xterm)",
        b"\x1b[1;5R",
        F3,
        MOD_CTRL,
        &mut c,
    );
    c.push(stream(
        "Shift+F1 SS3 2P (old xterm)",
        b"\x1bO2P",
        0,
        vec![k(4, F1, NS, MOD_SHIFT)],
    ));
    legacy("Shift+Tab CSI Z", b"\x1b[Z", Tab, MOD_SHIFT, &mut c);

    // --- Emulator variants ----------------------------------------------------
    for (name, seq, key) in [
        ("Up SS3 A", &b"\x1bOA"[..], Up),
        ("Down SS3 B", b"\x1bOB", Down),
        ("Right SS3 C", b"\x1bOC", Right),
        ("Left SS3 D", b"\x1bOD", Left),
        ("Home SS3 H", b"\x1bOH", Home),
        ("End SS3 F", b"\x1bOF", End),
        ("Enter keypad SS3 M", b"\x1bOM", Enter),
    ] {
        c.push(stream(name, seq, 0, vec![k(seq.len(), key, NS, 0)]));
    }
    legacy("Home VT220 CSI 1~", b"\x1b[1~", Home, 0, &mut c);
    legacy("End VT220 CSI 4~", b"\x1b[4~", End, 0, &mut c);
    legacy("Home rxvt CSI 7~", b"\x1b[7~", Home, 0, &mut c);
    legacy("End rxvt CSI 8~", b"\x1b[8~", End, 0, &mut c);
    legacy("F1 VT220 CSI 11~", b"\x1b[11~", F1, 0, &mut c);
    legacy("F2 VT220 CSI 12~", b"\x1b[12~", F2, 0, &mut c);
    legacy("F3 CSI 13~ (kitty's only F3)", b"\x1b[13~", F3, 0, &mut c);
    legacy("F4 VT220 CSI 14~", b"\x1b[14~", F4, 0, &mut c);
    legacy("F1 kitty CSI P", b"\x1b[P", F1, 0, &mut c);
    legacy("F2 kitty CSI Q", b"\x1b[Q", F2, 0, &mut c);
    legacy("F4 kitty CSI S", b"\x1b[S", F4, 0, &mut c);
    for (seq, key) in [
        (&b"\x1b[[A"[..], F1),
        (b"\x1b[[B", F2),
        (b"\x1b[[C", F3),
        (b"\x1b[[D", F4),
        (b"\x1b[[E", F5),
    ] {
        c.push(stream(
            &format!("{key:?} Linux console"),
            seq,
            0,
            vec![k(4, key, NS, 0)],
        ));
    }
    c.push(stream(
        "Shift+Up rxvt CSI a",
        b"\x1b[a",
        0,
        vec![k(3, Up, NS, MOD_SHIFT)],
    ));
    c.push(stream(
        "Shift+Left rxvt CSI d",
        b"\x1b[d",
        0,
        vec![k(3, Left, NS, MOD_SHIFT)],
    ));
    c.push(stream(
        "Ctrl+Right rxvt SS3 c",
        b"\x1bOc",
        0,
        vec![k(3, Right, NS, MOD_CTRL)],
    ));
    c.push(stream(
        "Shift+Delete rxvt CSI 3$",
        b"\x1b[3$",
        0,
        vec![k(4, Delete, NS, MOD_SHIFT)],
    ));
    c.push(stream(
        "Ctrl+PageUp rxvt CSI 5^",
        b"\x1b[5^",
        0,
        vec![k(4, PageUp, NS, MOD_CTRL)],
    ));
    c.push(stream(
        "Ctrl+Shift+Insert rxvt CSI 2@",
        b"\x1b[2@",
        0,
        vec![k(4, Insert, NS, MOD_CTRL | MOD_SHIFT)],
    ));
    c.push(stream(
        "keypad 5 SS3 u",
        b"\x1bOu",
        0,
        vec![u(3, "5", NS, 0)],
    ));
    c.push(stream(
        "Ctrl+a xterm modifyOtherKeys CSI 27;5;97~",
        b"\x1b[27;5;97~",
        0,
        vec![uc(10, "a", NS, MOD_CTRL, 0x01)],
    ));

    // --- C0 keys: Backspace, Enter, Tab, Ctrl+letter --------------------------
    c.push(stream(
        "Backspace DEL",
        b"\x7f",
        0,
        vec![k(1, Backspace, NS, 0)],
    ));
    c.push(stream(
        "Backspace BS normalised",
        b"\x08",
        0,
        vec![k(1, Backspace, NS, 0)],
    ));
    c.push(stream("Tab", b"\t", 0, vec![k(1, Tab, NS, 0)]));
    c.push(stream("Enter LF", b"\n", 0, vec![k(1, Enter, NS, 0)]));
    c.push(stream(
        "Enter CR normalised",
        b"\r",
        0,
        vec![k(1, Enter, NS, 0)],
    ));
    c.push(stream(
        "CR LF is one Enter",
        b"\r\n",
        0,
        vec![k(2, Enter, NS, 0)],
    ));
    c.push(stream(
        "CR CR LF is two Enters",
        b"\r\r\n",
        0,
        vec![k(1, Enter, NS, 0), k(2, Enter, NS, 0)],
    ));
    c.push(stream(
        "LF CR is two Enters",
        b"\n\r",
        0,
        vec![k(1, Enter, NS, 0), k(1, Enter, NS, 0)],
    ));
    c.push(stream(
        "line with CR LF",
        b"ok\r\n",
        0,
        vec![u(1, "o", NS, 0), u(1, "k", NS, 0), k(2, Enter, NS, 0)],
    ));
    c.push(single(
        "CR waits for a possible LF",
        b"\r",
        0,
        Outcome::NeedMore,
    ));
    c.push(stream(
        "Ctrl+A",
        b"\x01",
        0,
        vec![uc(1, "a", NS, MOD_CTRL, 0x01)],
    ));
    c.push(stream(
        "Ctrl+Z",
        b"\x1a",
        0,
        vec![uc(1, "z", NS, MOD_CTRL, 0x1a)],
    ));
    c.push(stream(
        "Ctrl+Space NUL",
        b"\x00",
        0,
        vec![uc(1, " ", NS, MOD_CTRL, 0x00)],
    ));
    c.push(stream(
        "Ctrl+\\",
        b"\x1c",
        0,
        vec![uc(1, "\\", NS, MOD_CTRL, 0x1c)],
    ));
    c.push(stream(
        "Ctrl+_",
        b"\x1f",
        0,
        vec![uc(1, "_", NS, MOD_CTRL, 0x1f)],
    ));

    // --- Escape: lone, Alt-prefix -----------------------------------------
    c.push(stream("lone Esc", b"\x1b", 0, vec![k(1, Escape, NS, 0)]));
    c.push(single(
        "Esc waits for the delay",
        b"\x1b",
        0,
        Outcome::NeedMore,
    ));
    c.push(single(
        "Esc after the delay",
        b"\x1b",
        FINAL,
        k(1, Escape, NS, 0),
    ));
    c.push(stream(
        "Esc+letter is Alt+a",
        b"\x1ba",
        0,
        vec![u(2, "a", NS, MOD_ALT)],
    ));
    c.push(stream(
        "Esc+capital is Alt+A",
        b"\x1bA",
        0,
        vec![u(2, "A", NS, MOD_ALT)],
    ));
    c.push(stream(
        "Alt+é",
        "\x1bé".as_bytes(),
        0,
        vec![u(3, "é", NS, MOD_ALT)],
    ));
    c.push(stream(
        "Alt+Backspace",
        b"\x1b\x7f",
        0,
        vec![k(2, Backspace, NS, MOD_ALT)],
    ));
    c.push(stream(
        "Alt+Enter",
        b"\x1b\r",
        0,
        vec![k(2, Enter, NS, MOD_ALT)],
    ));
    c.push(stream(
        "Alt+Ctrl+A",
        b"\x1b\x01",
        0,
        vec![uc(2, "a", NS, MOD_ALT | MOD_CTRL, 0x01)],
    ));
    c.push(stream(
        "Alt+Left rxvt ESC ESC [ D",
        b"\x1b\x1b[D",
        0,
        vec![k(4, Left, NS, MOD_ALT)],
    ));
    c.push(stream(
        "Alt+Esc ESC ESC",
        b"\x1b\x1b",
        0,
        vec![k(2, Escape, NS, MOD_ALT)],
    ));
    c.push(stream(
        "Esc then invalid byte",
        b"\x1b\xff",
        0,
        vec![k(1, Escape, NS, 0), skip(1)],
    ));
    c.push(stream(
        "Alt+[ after the delay",
        b"\x1b[",
        0,
        vec![u(2, "[", NS, MOD_ALT)],
    ));
    c.push(stream(
        "Alt+O after the delay",
        b"\x1bO",
        0,
        vec![u(2, "O", NS, MOD_ALT)],
    ));

    // --- Split sequences: wait, then complete --------------------------------
    for prefix in [
        &b"\x1b["[..],
        b"\x1b[1",
        b"\x1b[1;",
        b"\x1b[1;5",
        b"\x1b[15",
        b"\x1bO",
        b"\x1b[[",
        b"\x1b[97;1:",
        b"\x1b\x1b",
        b"\x1b\x1b[",
    ] {
        c.push(single(
            &format!("split {prefix:?} waits"),
            prefix,
            0,
            Outcome::NeedMore,
        ));
        c.push(single(
            &format!("split {prefix:?} waits (events)"),
            prefix,
            EVENTS,
            Outcome::NeedMore,
        ));
    }
    c.push(single(
        "split CSI 1;5 abandoned",
        b"\x1b[1;5",
        FINAL,
        skip(5),
    ));
    c.push(single(
        "split Linux console abandoned",
        b"\x1b[[",
        FINAL,
        skip(3),
    ));
    c.push(single(
        "split euro waits",
        b"\xe2\x82",
        0,
        Outcome::NeedMore,
    ));
    c.push(single("split euro abandoned", b"\xe2\x82", FINAL, skip(2)));
    c.push(stream(
        "sequence then text",
        b"\x1b[1;5Dx",
        0,
        vec![k(6, Left, NS, MOD_CTRL), u(1, "x", NS, 0)],
    ));
    c.push(stream(
        "two sequences back to back",
        b"\x1b[A\x1b[B",
        EVENTS,
        vec![k(3, Up, Press, 0), k(3, Down, Press, 0)],
    ));
    c.push(stream(
        "a CSI broken by ESC",
        b"\x1b[1\x1b[D",
        0,
        vec![skip(3), k(3, Left, NS, 0)],
    ));

    // --- kitty CSI u: press / repeat / release, modifiers --------------------
    c.push(stream(
        "kitty a press",
        b"\x1b[97u",
        EVENTS,
        vec![u(5, "a", Press, 0)],
    ));
    c.push(stream(
        "kitty a, events unknown",
        b"\x1b[97u",
        0,
        vec![u(5, "a", NS, 0)],
    ));
    c.push(stream(
        "kitty a press explicit",
        b"\x1b[97;1:1u",
        EVENTS,
        vec![u(9, "a", Press, 0)],
    ));
    c.push(stream(
        "kitty a repeat",
        b"\x1b[97;1:2u",
        EVENTS,
        vec![u(9, "a", Repeat, 0)],
    ));
    c.push(stream(
        "kitty a release has no text",
        b"\x1b[97;1:3u",
        EVENTS,
        vec![u(9, "a", Release, 0)],
    ));
    c.push(stream(
        "kitty Ctrl+a",
        b"\x1b[97;5u",
        EVENTS,
        vec![uc(7, "a", Press, MOD_CTRL, 0x01)],
    ));
    c.push(stream(
        "kitty Ctrl+a release",
        b"\x1b[97;5:3u",
        EVENTS,
        vec![Outcome::Token(Token {
            consumed: 9,
            key: Unicode,
            event: Release,
            mods: MOD_CTRL,
            grapheme: b"a".to_vec(),
            text: Text::None,
        })],
    ));
    c.push(stream(
        "kitty Alt+a",
        b"\x1b[97;3u",
        EVENTS,
        vec![u(7, "a", Press, MOD_ALT)],
    ));
    c.push(stream(
        "kitty Super+a is meta",
        b"\x1b[97;9u",
        EVENTS,
        vec![u(7, "a", Press, MOD_META)],
    ));
    c.push(stream(
        "kitty Meta+a is meta",
        b"\x1b[97;33u",
        EVENTS,
        vec![u(8, "a", Press, MOD_META)],
    ));
    c.push(stream(
        "kitty Alt+Ctrl+Super+a",
        b"\x1b[97;15u",
        EVENTS,
        vec![uc(8, "a", Press, MOD_ALT | MOD_CTRL | MOD_META, 0x01)],
    ));
    c.push(stream(
        "kitty Caps Lock and Num Lock are not modifiers",
        b"\x1b[97;193u",
        EVENTS,
        vec![u(9, "a", Press, 0)],
    ));
    c.push(stream(
        "kitty Shift+a without text is A",
        b"\x1b[97;2u",
        EVENTS,
        vec![u(7, "A", Press, MOD_SHIFT)],
    ));
    c.push(stream(
        "kitty Shift+a with alternate key",
        b"\x1b[97:65;2u",
        EVENTS,
        vec![u(10, "A", Press, MOD_SHIFT)],
    ));
    c.push(stream(
        "kitty Shift+a with associated text",
        b"\x1b[97;2;65u",
        EVENTS,
        vec![u(10, "A", Press, MOD_SHIFT)],
    ));
    c.push(stream(
        "kitty text-only event (key 0)",
        b"\x1b[0;;229u",
        EVENTS,
        vec![u(9, "å", Press, 0)],
    ));
    c.push(stream(
        "kitty text is a cluster of code points",
        b"\x1b[0;;101:769u",
        EVENTS,
        vec![u(13, "e\u{301}", Press, 0)],
    ));
    c.push(stream(
        "kitty Enter",
        b"\x1b[13u",
        EVENTS,
        vec![k(5, Enter, Press, 0)],
    ));
    c.push(stream(
        "kitty Enter release",
        b"\x1b[13;1:3u",
        EVENTS,
        vec![k(9, Enter, Release, 0)],
    ));
    c.push(stream(
        "kitty Tab",
        b"\x1b[9u",
        EVENTS,
        vec![k(4, Tab, Press, 0)],
    ));
    c.push(stream(
        "kitty Shift+Tab",
        b"\x1b[9;2u",
        EVENTS,
        vec![k(6, Tab, Press, MOD_SHIFT)],
    ));
    c.push(stream(
        "kitty Backspace",
        b"\x1b[127u",
        EVENTS,
        vec![k(6, Backspace, Press, 0)],
    ));
    c.push(stream(
        "kitty Escape",
        b"\x1b[27u",
        EVENTS,
        vec![k(5, Escape, Press, 0)],
    ));
    c.push(stream(
        "kitty Left release",
        b"\x1b[1;1:3D",
        EVENTS,
        vec![k(8, Left, Release, 0)],
    ));
    c.push(stream(
        "kitty Shift+Left repeat",
        b"\x1b[1;2:2D",
        EVENTS,
        vec![k(8, Left, Repeat, MOD_SHIFT)],
    ));
    c.push(stream(
        "kitty Ctrl+Delete release",
        b"\x1b[3;5:3~",
        EVENTS,
        vec![k(8, Delete, Release, MOD_CTRL)],
    ));
    c.push(stream(
        "kitty F5 press",
        b"\x1b[15;1:1~",
        EVENTS,
        vec![k(9, F5, Press, 0)],
    ));
    c.push(stream(
        "kitty KP_0 types 0",
        b"\x1b[57399u",
        EVENTS,
        vec![u(8, "0", Press, 0)],
    ));
    c.push(stream(
        "kitty KP_Enter",
        b"\x1b[57414u",
        EVENTS,
        vec![k(8, Enter, Press, 0)],
    ));
    c.push(stream(
        "kitty Shift+KP_Left",
        b"\x1b[57417;2u",
        EVENTS,
        vec![k(10, Left, Press, MOD_SHIFT)],
    ));
    c.push(stream(
        "kitty Left Shift key is dropped",
        b"\x1b[57441;2u",
        EVENTS,
        vec![skip(10)],
    ));
    c.push(stream(
        "kitty F13 is dropped",
        b"\x1b[57376u",
        EVENTS,
        vec![skip(8)],
    ));
    c.push(stream(
        "kitty media Play is dropped",
        b"\x1b[57428u",
        EVENTS,
        vec![skip(8)],
    ));
    c.push(stream(
        "kitty undefined event type is dropped",
        b"\x1b[97;1:9u",
        EVENTS,
        vec![skip(9)],
    ));
    c.push(stream(
        "kitty surrogate code point is dropped",
        b"\x1b[55296u",
        EVENTS,
        vec![skip(8)],
    ));

    // --- Responses and reports that are not keys ------------------------------
    c.push(stream(
        "kitty flags reply CSI ? 1 u",
        b"\x1b[?1u",
        0,
        vec![skip(5)],
    ));
    c.push(stream(
        "device attributes CSI ? 62;22 c",
        b"\x1b[?62;22c",
        0,
        vec![skip(9)],
    ));
    c.push(stream(
        "SGR mouse report not stripped",
        b"\x1b[<0;10;5M",
        0,
        vec![skip(10)],
    ));
    c.push(stream(
        "bracketed paste markers",
        b"\x1b[200~hi\x1b[201~",
        0,
        vec![skip(6), u(1, "h", NS, 0), u(1, "i", NS, 0), skip(6)],
    ));
    c.push(stream("unknown CSI final", b"\x1b[5y", 0, vec![skip(4)]));
    c.push(stream(
        "CSI with an intermediate",
        b"\x1b[1 q",
        0,
        vec![skip(5)],
    ));
    c.push(stream(
        "F13 CSI 25~ is dropped",
        b"\x1b[25~",
        0,
        vec![skip(5)],
    ));

    // --- Invalid UTF-8: dropped, not reported ---------------------------------
    c.push(stream("0xFF", b"\xff", 0, vec![skip(1)]));
    c.push(stream("lone continuation", b"\x80", 0, vec![skip(1)]));
    c.push(stream(
        "C0 lead (overlong 2-byte)",
        b"\xc0\x80",
        0,
        vec![skip(1), skip(1)],
    ));
    c.push(stream(
        "overlong 3-byte",
        b"\xe0\x80\x80",
        0,
        vec![skip(1), skip(1), skip(1)],
    ));
    c.push(stream(
        "surrogate D800",
        b"\xed\xa0\x80",
        0,
        vec![skip(1), skip(1), skip(1)],
    ));
    c.push(stream(
        "past U+10FFFF",
        b"\xf4\x90\x80\x80",
        0,
        vec![skip(1), skip(1), skip(1), skip(1)],
    ));
    c.push(stream(
        "bad continuation keeps the next character",
        b"\xe2\x28\xa1",
        0,
        vec![skip(1), u(1, "(", NS, 0), skip(1)],
    ));
    c.push(stream(
        "truncated sequence then text",
        b"\xe2\x82x",
        0,
        vec![skip(2), u(1, "x", NS, 0)],
    ));
    c.push(stream(
        "invalid between keys",
        b"a\xffb",
        0,
        vec![u(1, "a", NS, 0), skip(1), u(1, "b", NS, 0)],
    ));
    c.push(stream(
        "valid 4-byte",
        "😀".as_bytes(),
        0,
        vec![u(4, "😀", NS, 0)],
    ));

    // --- Clusters: ZWJ, combining, flags, Hangul ------------------------------
    c.push(stream(
        "combining acute is one grapheme",
        "e\u{301}".as_bytes(),
        CLUSTER,
        vec![u(3, "e\u{301}", NS, 0)],
    ));
    c.push(stream(
        "combining acute is two scalars",
        "e\u{301}".as_bytes(),
        0,
        vec![u(1, "e", NS, 0), u(2, "\u{301}", NS, 0)],
    ));
    c.push(single(
        "a base letter waits for a possible mark",
        b"e",
        CLUSTER,
        Outcome::NeedMore,
    ));
    c.push(single("a scalar does not wait", b"e", 0, u(1, "e", NS, 0)));
    c.push(single(
        "a base letter after the delay",
        b"e",
        CLUSTER | FINAL,
        u(1, "e", NS, 0),
    ));
    c.push(single(
        "a base letter and half a mark wait",
        b"e\xcc",
        CLUSTER,
        Outcome::NeedMore,
    ));
    c.push(single(
        "a following letter settles the cluster",
        b"ab",
        CLUSTER,
        u(1, "a", NS, 0),
    ));
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    c.push(stream(
        "ZWJ family is one grapheme",
        family.as_bytes(),
        CLUSTER,
        vec![u(family.len(), family, NS, 0)],
    ));
    c.push(stream(
        "flag pair is one grapheme",
        "\u{1F1EF}\u{1F1F5}".as_bytes(),
        CLUSTER,
        vec![u(8, "\u{1F1EF}\u{1F1F5}", NS, 0)],
    ));
    c.push(stream(
        "two flags are two graphemes",
        "\u{1F1EF}\u{1F1F5}\u{1F1FA}\u{1F1F8}".as_bytes(),
        CLUSTER,
        vec![
            u(8, "\u{1F1EF}\u{1F1F5}", NS, 0),
            u(8, "\u{1F1FA}\u{1F1F8}", NS, 0),
        ],
    ));
    c.push(stream(
        "Hangul L V T is one grapheme",
        "\u{1100}\u{1161}\u{11A8}".as_bytes(),
        CLUSTER,
        vec![u(9, "\u{1100}\u{1161}\u{11A8}", NS, 0)],
    ));
    c.push(stream(
        "cluster ends at a control",
        "e\u{301}\r".as_bytes(),
        CLUSTER,
        vec![u(3, "e\u{301}", NS, 0), k(1, Enter, NS, 0)],
    ));
    c.push(stream(
        "cluster ends at an escape sequence",
        b"e\x1b[D",
        CLUSTER,
        vec![u(1, "e", NS, 0), k(3, Left, NS, 0)],
    ));
    c.push(stream(
        "Alt+cluster",
        "\x1be\u{301}".as_bytes(),
        CLUSTER,
        vec![u(4, "e\u{301}", NS, MOD_ALT)],
    ));
    // A mark flood is split at 128 bytes: 'e' + 63 marks (2 bytes each) = 127
    // bytes fit, the 64th would make 129.
    let mut flood = String::from("e");
    for _ in 0..70 {
        flood.push('\u{301}');
    }
    let head = &flood[..127];
    let tail = &flood[127..];
    c.push(stream(
        "a combining flood is cut at the cap",
        flood.as_bytes(),
        CLUSTER,
        vec![u(127, head, NS, 0), u(tail.len(), tail, NS, 0)],
    ));

    // The cap's exact edge: U+00E9 (2 bytes) + 63 marks is 128 bytes and fits
    // whole; one more mark is the next token.
    let mut exact = String::from("\u{e9}");
    for _ in 0..63 {
        exact.push('\u{301}');
    }
    assert_eq!(exact.len(), 128);
    c.push(stream(
        "a cluster of exactly the cap fits",
        exact.as_bytes(),
        CLUSTER,
        vec![u(128, &exact, NS, 0)],
    ));
    let over = format!("{exact}\u{301}");
    c.push(stream(
        "one byte pair past the cap starts the next token",
        over.as_bytes(),
        CLUSTER,
        vec![u(128, &exact, NS, 0), u(2, "\u{301}", NS, 0)],
    ));

    // --- The macOS probe from the bug doc, as a console would send it ---------
    c.push(stream(
        "the Failing Reproduction's expected bytes",
        b"a\x1b[D\x1b[C\x1b[A\x1b[B\x7f\x1b[3~\x1b",
        0,
        vec![
            u(1, "a", NS, 0),
            k(3, Left, NS, 0),
            k(3, Right, NS, 0),
            k(3, Up, NS, 0),
            k(3, Down, NS, 0),
            k(1, Backspace, NS, 0),
            k(4, Delete, NS, 0),
            k(1, Escape, NS, 0),
        ],
    ));

    c
}
