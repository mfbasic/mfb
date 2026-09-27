//! `_mfb_rt_input_decode` — the input decoder (bug-699 Fix Design layer 2).
//!
//! One token from the front of a byte buffer: a text scalar or grapheme cluster,
//! or a key with its event and modifiers, from any encoding a terminal or an
//! `--app` surface produces — UTF-8 text (invalid bytes dropped), the C0 keys,
//! legacy xterm/VT220/rxvt/Linux-console sequences (the tables in
//! [`super::keys`]), xterm modifier parameters and `modifyOtherKeys`, and the
//! kitty `CSI … u` form with event types, modifiers and associated text.
//!
//! It is a PURE function of the bytes and three flags — no I/O, no allocation, no
//! clock. Whether to wait for more input is the caller's decision: the decoder
//! answers "need more" when the bytes so far are a prefix of something longer,
//! and the caller either reads more or, once the delay has run out or the input
//! has ended, asks again with `FINAL` set. That keeps the one place a timer is
//! read out of this function, and it is what lets the whole decoder be tested on
//! byte strings.
//!
//! The specification is the Rust model in `super::model` (test only); this is
//! its transliteration, and `super::tests` runs both on the corpus and against
//! each other.
//!
//! # Interface
//!
//! `c_arg(0)` = buffer, `c_arg(1)` = its length, `c_arg(2)` = flags
//! ([`FLAG_FINAL`], [`FLAG_EVENTS`], [`FLAG_CLUSTER`]), `c_arg(3)` = the token
//! record ([`TOKEN_BYTES`] bytes) the result is written to. The status is also
//! returned in the result register.

use super::keys::*;
use super::{CSI_PARAM_CAP, GRAPHEME_CAP};
use crate::codegen::engine::operand::VirtualRegister;
use crate::codegen::engine::types::*;
use crate::codegen::engine::util::*;
use crate::codegen::string::unicode_props::{
    emit_grapheme_break_branch_free, emit_grapheme_state_update_free,
    emit_read_boundclass_icb_charwidth_free, emit_unicode_property_ptr_free,
    emit_utf8_codepoint_by_len,
};
use crate::target::shared::abi;

pub(crate) const INPUT_DECODE_SYMBOL: &str = "_mfb_rt_input_decode";

/// Flag: no more input will arrive for this decision (the delay expired, the
/// stream ended, or a pipe's read-ahead found nothing more).
pub(crate) const FLAG_FINAL: u64 = 1;
/// Flag: the source reports key events, so a sequence without an event field
/// is a Press (an `--app` surface, or a console with the kitty protocol on).
pub(crate) const FLAG_EVENTS: u64 = 2;
/// Flag: return a whole grapheme cluster for text, not one scalar.
pub(crate) const FLAG_CLUSTER: u64 = 4;

/// Token record: status (`STATUS_*`).
pub(crate) const TOKEN_STATUS: usize = 0;
/// Token record: bytes consumed from the front of the buffer.
pub(crate) const TOKEN_CONSUMED: usize = 8;
/// Token record: the `KeyName` ordinal.
pub(crate) const TOKEN_KEY: usize = 16;
/// Token record: the `KeyEvent` ordinal.
pub(crate) const TOKEN_EVENT: usize = 24;
/// Token record: modifier bits (`MOD_*`).
pub(crate) const TOKEN_MODS: usize = 32;
/// Token record: what a text reader gets (`TEXT_*`).
pub(crate) const TOKEN_TEXT_KIND: usize = 40;
/// Token record: the scalar, when the text kind is [`TEXT_SCALAR`].
pub(crate) const TOKEN_TEXT_SCALAR: usize = 48;
/// Token record: the grapheme's byte length.
pub(crate) const TOKEN_GRAPHEME_LEN: usize = 56;
/// Token record: the grapheme's UTF-8 bytes.
pub(crate) const TOKEN_GRAPHEME: usize = 64;
/// Size of the token record.
pub(crate) const TOKEN_BYTES: usize = TOKEN_GRAPHEME + GRAPHEME_CAP;
// The record sits in a reader's frame, which is laid out in 8-byte slots.
const _: () = assert!(TOKEN_BYTES % 8 == 0);

/// The bytes so far are a prefix of something longer.
pub(crate) const STATUS_NEED_MORE: u64 = 0;
/// A token; the record's fields are set.
pub(crate) const STATUS_TOKEN: u64 = 1;
/// Drop `consumed` bytes: nothing a program can receive.
pub(crate) const STATUS_SKIP: u64 = 2;

/// Text kind: the text is the grapheme.
pub(crate) const TEXT_GRAPHEME: u64 = 0;
/// Text kind: the text is the one scalar in [`TOKEN_TEXT_SCALAR`].
pub(crate) const TEXT_SCALAR: u64 = 1;
/// Text kind: no text (a key with no Unicode value, or a release).
pub(crate) const TEXT_NONE: u64 = 2;

/// "Absent" for a parsed CSI parameter.
const NONE: &str = "18446744073709551615";
/// Clamp for a parsed decimal parameter (no key number comes near it).
const PARAM_MAX: u64 = 1_000_000_000;
const ESC: u64 = 0x1B;

/// Emission state: the instruction/relocation sinks, a vreg source, and a
/// label counter. The long-lived registers are fields so every block reads the
/// same names.
struct Dec<'a> {
    ins: &'a mut Vec<CodeInstruction>,
    rel: &'a mut Vec<CodeRelocation>,
    v: Vregs,
    labels: usize,
    /// Whether this program's decoder joins grapheme clusters (`FLAG_CLUSTER`).
    clusters: bool,
    buf: VirtualRegister,
    len: VirtualRegister,
    flags: VirtualRegister,
    out: VirtualRegister,
    /// Alt-prefix ESCs before the token (a legacy terminal's Meta-sends-Escape).
    p: VirtualRegister,
    /// The event a sequence with no event field reports.
    evdef: VirtualRegister,
    // The token under construction.
    key: VirtualRegister,
    event: VirtualRegister,
    mods: VirtualRegister,
    /// Bytes consumed, counted from `p`.
    cons: VirtualRegister,
    glen: VirtualRegister,
}

impl Dec<'_> {
    fn r(&mut self) -> VirtualRegister {
        self.v.next()
    }
    fn label(&mut self, name: &str) -> String {
        self.labels += 1;
        format!("{INPUT_DECODE_SYMBOL}_{name}_{}", self.labels)
    }
    fn push(&mut self, instruction: CodeInstruction) {
        self.ins.push(instruction);
    }
    fn imm(&mut self, dst: impl Into<abi::Reg>, value: u64) {
        let dst: abi::Reg = dst.into();
        self.push(abi::move_immediate(dst, "Integer", &value.to_string()));
    }
    fn at(&mut self, name: &str) {
        self.push(abi::label(name));
    }
    fn goto(&mut self, name: &str) {
        self.push(abi::branch(name));
    }
    /// `if reg == value goto target`.
    fn beq(&mut self, reg: impl Into<abi::Reg>, value: u64, target: &str) {
        let reg: abi::Reg = reg.into();
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_eq(target));
    }
    fn bne(&mut self, reg: impl Into<abi::Reg>, value: u64, target: &str) {
        let reg: abi::Reg = reg.into();
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_ne(target));
    }
    /// Unsigned `if reg < value goto target`.
    fn blo(&mut self, reg: impl Into<abi::Reg>, value: u64, target: &str) {
        let reg: abi::Reg = reg.into();
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_lo(target));
    }
    /// Unsigned `if reg > value goto target`.
    fn bhi(&mut self, reg: impl Into<abi::Reg>, value: u64, target: &str) {
        let reg: abi::Reg = reg.into();
        self.push(abi::compare_immediate(reg, &value.to_string()));
        self.push(abi::branch_hi(target));
    }
    /// Unsigned `if a >= b goto target` (the vocabulary has no `hs`).
    fn bge_u(&mut self, a: impl Into<abi::Reg>, b: impl Into<abi::Reg>, target: &str) {
        let a: abi::Reg = a.into();
        let b: abi::Reg = b.into();
        self.push(abi::compare_registers(b, a));
        self.push(abi::branch_ls(target));
    }
    /// `dst = buf[index]`.
    fn byte_at(&mut self, dst: impl Into<abi::Reg>, index: impl Into<abi::Reg>) {
        let dst: abi::Reg = dst.into();
        let index: abi::Reg = index.into();
        let addr = self.r();
        let buf = self.buf;
        self.push(abi::add_registers(&addr, &buf, index));
        self.push(abi::load_u8(dst, &addr, 0));
    }
    /// `if flags & bit == 0 goto target`.
    fn unless_flag(&mut self, bit: u64, target: &str) {
        let t = self.r();
        let m = self.r();
        self.imm(&m, bit);
        let flags = self.flags;
        self.push(abi::and_registers(&t, &flags, &m));
        self.beq(&t, 0, target);
    }
    /// Store one byte of the grapheme at `out + TOKEN_GRAPHEME + glen` and
    /// advance `glen`.
    fn append_grapheme_byte(&mut self, byte: impl Into<abi::Reg>) {
        let byte: abi::Reg = byte.into();
        let addr = self.r();
        let (out, glen) = (self.out, self.glen);
        self.push(abi::add_registers(&addr, &out, &glen));
        self.push(abi::store_u8(byte, &addr, TOKEN_GRAPHEME));
        self.push(abi::add_immediate(&glen, &glen, 1));
    }
    /// `KEY = value; goto target` for each `(match, value)` whose match equals
    /// `reg`; falls through when none does.
    fn lookup(
        &mut self,
        reg: impl Into<abi::Reg>,
        table: &[(u64, u64)],
        dst: impl Into<abi::Reg>,
        found: &str,
    ) {
        let reg: abi::Reg = reg.into();
        let dst: abi::Reg = dst.into();
        let mut arms = Vec::new();
        for (key, value) in table {
            let arm = self.label("arm");
            self.beq(reg, *key, &arm);
            arms.push((arm, *value));
        }
        let miss = self.label("miss");
        self.goto(&miss);
        for (arm, value) in arms {
            self.at(&arm);
            self.imm(dst, value);
            self.goto(found);
        }
        self.at(&miss);
    }
}

const fn k(key: KeyName) -> u64 {
    key as u64
}

/// Lower `_mfb_rt_input_decode`.
/// Lower the decoder. `clusters` says whether the program has a reader that
/// asks for whole grapheme clusters (`io::readGrapheme`, `io::readKey`): only
/// then is the cluster extension emitted, because it is what pulls the Unicode
/// property tables into the binary. Without it `FLAG_CLUSTER` is ignored and
/// text is one scalar per token — all a program with only `io::readLine`,
/// `io::readScalar` and `io::readRaw` ever asks for.
pub(crate) fn lower_input_decode(clusters: bool) -> Result<CodeFunction, String> {
    let mut instructions = vec![abi::label("entry")];
    let mut relocations = Vec::new();
    let mut v = Vregs::new();
    let names: Vec<VirtualRegister> = (0..11).map(|_| v.next()).collect();
    let mut d = Dec {
        ins: &mut instructions,
        rel: &mut relocations,
        v,
        labels: 0,
        clusters,
        buf: names[0],
        len: names[1],
        flags: names[2],
        out: names[3],
        p: names[4],
        evdef: names[5],
        key: names[6],
        event: names[7],
        mods: names[8],
        cons: names[9],
        glen: names[10],
    };
    emit_body(&mut d);
    Ok(finalize_vreg_helper(
        "runtime.input.decode",
        INPUT_DECODE_SYMBOL,
        "Integer",
        instructions,
        relocations,
    ))
}

/// The labels every block finishes through.
struct Exits {
    need_more: String,
    skip: String,
    named: String,
    unicode: String,
}

fn emit_body(d: &mut Dec) {
    let (buf, len, flags, out) = (d.buf, d.len, d.flags, d.out);
    let (p, evdef) = (d.p, d.evdef);
    d.push(abi::move_register(&buf, abi::c_arg(0)));
    d.push(abi::move_register(&len, abi::c_arg(1)));
    d.push(abi::move_register(&flags, abi::c_arg(2)));
    d.push(abi::move_register(&out, abi::c_arg(3)));
    d.imm(&p, 0);
    let glen = d.glen;
    d.imm(&glen, 0);
    // The event a sequence with no event field reports.
    let events_done = d.label("events_done");
    d.imm(&evdef, KeyEvent::NotSupported as u64);
    d.unless_flag(FLAG_EVENTS, &events_done);
    d.imm(&evdef, KeyEvent::Press as u64);
    d.at(&events_done);

    let exits = Exits {
        need_more: d.label("need_more"),
        skip: d.label("skip"),
        named: d.label("named"),
        unicode: d.label("unicode"),
    };

    // Alt prefixes: ESC followed by anything but `[`/`O` (which start a
    // sequence) is Meta-sends-Escape — the rest is the token, with Alt.
    let prefix = d.label("prefix");
    let dispatch = d.label("dispatch");
    d.at(&prefix);
    d.bge_u(&p, &len, &exits.need_more);
    let b = d.r();
    d.byte_at(&b, &p);
    d.bne(&b, ESC, &dispatch);
    let next = d.r();
    d.push(abi::add_immediate(&next, &p, 1));
    d.bge_u(&next, &len, &dispatch);
    let b1 = d.r();
    d.byte_at(&b1, &next);
    d.beq(&b1, u64::from(b'['), &dispatch);
    d.beq(&b1, u64::from(b'O'), &dispatch);
    d.push(abi::move_register(&p, &next));
    d.goto(&prefix);

    // Dispatch on the first byte of the token.
    d.at(&dispatch);
    let b0 = d.r();
    d.byte_at(&b0, &p);
    let rem = d.r();
    d.push(abi::subtract_registers(&rem, &len, &p));
    let esc = d.label("esc");
    let cr = d.label("cr");
    let c0 = d.label("c0");
    let text = d.label("text");
    d.beq(&b0, ESC, &esc);
    d.beq(&b0, 0x0D, &cr);
    d.blo(&b0, 0x20, &c0);
    d.beq(&b0, 0x7F, &c0);
    d.goto(&text);

    // ESC with nothing after it, or starting `[`/`O`.
    d.at(&esc);
    let csi = d.label("csi");
    let ss3 = d.label("ss3");
    let lone = d.label("lone_esc");
    d.beq(&rem, 1, &lone);
    let at1 = d.r();
    d.push(abi::add_immediate(&at1, &p, 1));
    let b1 = d.r();
    d.byte_at(&b1, &at1);
    d.beq(&b1, u64::from(b'['), &csi);
    d.goto(&ss3);
    d.at(&lone);
    d.unless_flag(FLAG_FINAL, &exits.need_more);
    emit_named_const(d, KeyName::Escape, None, 0, 1, &exits);

    // CR, and CR LF: one Enter.
    d.at(&cr);
    let cr_alone = d.label("cr_alone");
    let cr_one = d.label("cr_one");
    d.blo(&rem, 2, &cr_alone);
    let at1 = d.r();
    d.push(abi::add_immediate(&at1, &p, 1));
    let b1 = d.r();
    d.byte_at(&b1, &at1);
    d.bne(&b1, 0x0A, &cr_one);
    emit_named_const(d, KeyName::Enter, None, 0, 2, &exits);
    d.at(&cr_alone);
    d.unless_flag(FLAG_FINAL, &exits.need_more);
    d.at(&cr_one);
    emit_named_const(d, KeyName::Enter, None, 0, 1, &exits);

    // The other C0 bytes and DEL.
    d.at(&c0);
    emit_c0(d, &b0, &exits);

    d.at(&text);
    emit_text(d, &exits);

    d.at(&csi);
    emit_csi(d, &exits);

    d.at(&ss3);
    emit_ss3(d, &exits);

    emit_exits(d, &exits);
}

/// `KEY`/`EVENT`/`MODS`/`CONS` from constants (`event: None` = `NotSupported`),
/// then the named-key finish.
fn emit_named_const(
    d: &mut Dec,
    key: KeyName,
    event: Option<abi::Reg>,
    mods: u64,
    consumed: u64,
    exits: &Exits,
) {
    let (kr, er, mr, cr) = (d.key, d.event, d.mods, d.cons);
    d.imm(&kr, k(key));
    match event {
        Some(reg) => d.push(abi::move_register(&er, reg)),
        None => d.imm(&er, KeyEvent::NotSupported as u64),
    }
    d.imm(&mr, mods);
    d.imm(&cr, consumed);
    d.goto(&exits.named);
}

/// A C0 byte (not ESC or CR) or DEL.
fn emit_c0(d: &mut Dec, b0: impl Into<abi::Reg>, exits: &Exits) {
    let b0: abi::Reg = b0.into();
    let backspace = d.label("c0_bs");
    let tab = d.label("c0_tab");
    let lf = d.label("c0_lf");
    let ctrl = d.label("c0_ctrl");
    d.beq(b0, 0x08, &backspace);
    d.beq(b0, 0x7F, &backspace);
    d.beq(b0, 0x09, &tab);
    d.beq(b0, 0x0A, &lf);
    d.goto(&ctrl);
    d.at(&backspace);
    emit_named_const(d, KeyName::Backspace, None, 0, 1, exits);
    d.at(&tab);
    emit_named_const(d, KeyName::Tab, None, 0, 1, exits);
    d.at(&lf);
    emit_named_const(d, KeyName::Enter, None, 0, 1, exits);
    // Ctrl+<key>: the grapheme is the key (`c0_key`), the text the byte itself.
    d.at(&ctrl);
    let g = d.r();
    let found = d.label("c0_found");
    let table: Vec<(u64, u64)> = (0u8..=0x1F)
        .filter(|b| !matches!(b, 0x08 | 0x09 | 0x0A | 0x0D | 0x1B))
        .map(|b| (u64::from(b), u64::from(c0_key(b).1)))
        .collect();
    d.lookup(b0, &table, &g, &found);
    d.goto(&exits.skip); // unreachable: every C0 byte that gets here is in the table
    d.at(&found);
    d.append_grapheme_byte(&g);
    let (er, mr, cr) = (d.event, d.mods, d.cons);
    d.imm(&er, KeyEvent::NotSupported as u64);
    d.imm(&mr, MOD_CTRL);
    d.imm(&cr, 1);
    d.goto(&exits.unicode);
}

/// Classify the UTF-8 scalar at `buf[pos]`, with `len` bounding what has
/// arrived: `ok` with its length in `slen`, `invalid` with the maximal
/// ill-formed subpart's length in `bad`, or `truncated`.
fn emit_scalar_len(
    d: &mut Dec,
    pos: impl Into<abi::Reg>,
    slen: impl Into<abi::Reg>,
    bad: impl Into<abi::Reg>,
    ok: &str,
    invalid: &str,
    truncated: &str,
) {
    let bad: abi::Reg = bad.into();
    let pos: abi::Reg = pos.into();
    let slen: abi::Reg = slen.into();
    let lead = d.r();
    d.byte_at(&lead, pos);
    let (n, lo, hi) = (d.r(), d.r(), d.r());
    let two = d.label("u8_two");
    let three = d.label("u8_three");
    let four = d.label("u8_four");
    let cont = d.label("u8_cont");
    let invalid1 = d.label("u8_invalid1");
    let one = d.label("u8_one");
    d.blo(&lead, 0x80, &one);
    d.blo(&lead, 0xC2, &invalid1);
    d.bhi(&lead, 0xF4, &invalid1);
    d.blo(&lead, 0xE0, &two);
    d.blo(&lead, 0xF0, &three);
    d.goto(&four);
    d.at(&one);
    d.imm(slen, 1);
    d.goto(ok);
    d.at(&invalid1);
    d.imm(bad, 1);
    d.goto(invalid);
    d.at(&two);
    d.imm(&n, 2);
    d.imm(&lo, 0x80);
    d.imm(&hi, 0xBF);
    d.goto(&cont);
    // Three bytes: E0 needs A0..BF second (no overlong), ED needs 80..9F (no
    // surrogate).
    d.at(&three);
    d.imm(&n, 3);
    d.imm(&lo, 0x80);
    d.imm(&hi, 0xBF);
    let not_e0 = d.label("u8_not_e0");
    let not_ed = d.label("u8_not_ed");
    d.bne(&lead, 0xE0, &not_e0);
    d.imm(&lo, 0xA0);
    d.at(&not_e0);
    d.bne(&lead, 0xED, &not_ed);
    d.imm(&hi, 0x9F);
    d.at(&not_ed);
    d.goto(&cont);
    // Four bytes: F0 needs 90..BF (no overlong), F4 needs 80..8F (<= U+10FFFF).
    d.at(&four);
    d.imm(&n, 4);
    d.imm(&lo, 0x80);
    d.imm(&hi, 0xBF);
    let not_f0 = d.label("u8_not_f0");
    let not_f4 = d.label("u8_not_f4");
    d.bne(&lead, 0xF0, &not_f0);
    d.imm(&lo, 0x90);
    d.at(&not_f0);
    d.bne(&lead, 0xF4, &not_f4);
    d.imm(&hi, 0x8F);
    d.at(&not_f4);
    // Continuation bytes 1..n.
    d.at(&cont);
    let i = d.r();
    d.imm(&i, 1);
    let top = d.label("u8_top");
    let done = d.label("u8_done");
    let bad_byte = d.label("u8_bad_byte");
    let later = d.label("u8_later");
    let check = d.label("u8_check");
    d.at(&top);
    d.bge_u(&i, &n, &done);
    let at = d.r();
    d.push(abi::add_registers(&at, pos, &i));
    let len = d.len;
    d.bge_u(&at, &len, truncated);
    let c = d.r();
    d.byte_at(&c, &at);
    d.bne(&i, 1, &later);
    d.push(abi::compare_registers(&c, &lo));
    d.push(abi::branch_lo(&bad_byte));
    d.push(abi::compare_registers(&c, &hi));
    d.push(abi::branch_hi(&bad_byte));
    d.goto(&check);
    d.at(&later);
    d.blo(&c, 0x80, &bad_byte);
    d.bhi(&c, 0xBF, &bad_byte);
    d.at(&check);
    d.push(abi::add_immediate(&i, &i, 1));
    d.goto(&top);
    d.at(&bad_byte);
    d.push(abi::move_register(bad, &i));
    d.goto(invalid);
    d.at(&done);
    d.push(abi::move_register(slen, &n));
    d.goto(ok);
}

/// Copy `count` bytes from `buf[from]` onto the grapheme.
fn emit_copy_to_grapheme(d: &mut Dec, from: impl Into<abi::Reg>, count: impl Into<abi::Reg>) {
    let count: abi::Reg = count.into();
    let from: abi::Reg = from.into();
    let i = d.r();
    d.imm(&i, 0);
    let top = d.label("copy_top");
    let done = d.label("copy_done");
    d.at(&top);
    d.bge_u(&i, count, &done);
    let at = d.r();
    d.push(abi::add_registers(&at, from, &i));
    let b = d.r();
    d.byte_at(&b, &at);
    d.append_grapheme_byte(&b);
    d.push(abi::add_immediate(&i, &i, 1));
    d.goto(&top);
    d.at(&done);
}

/// The code point of the valid scalar of `slen` bytes at `buf[pos]`.
fn emit_codepoint(
    d: &mut Dec,
    pos: impl Into<abi::Reg>,
    slen: impl Into<abi::Reg>,
    cp: impl Into<abi::Reg>,
) {
    let cp: abi::Reg = cp.into();
    let slen: abi::Reg = slen.into();
    let pos: abi::Reg = pos.into();
    let ptr = d.r();
    let buf = d.buf;
    d.push(abi::add_registers(&ptr, &buf, pos));
    let lead = d.r();
    d.push(abi::load_u8(&lead, &ptr, 0));
    let (byte, mask) = (d.r(), d.r());
    let prefix = d.label("cp");
    emit_utf8_codepoint_by_len(&prefix, &ptr, slen, &lead, cp, &byte, &mask, d.ins);
}

/// Grapheme-break properties of `cp`.
fn emit_break_props(
    d: &mut Dec,
    cp: impl Into<abi::Reg>,
    bc: impl Into<abi::Reg>,
    icb: impl Into<abi::Reg>,
) {
    let bc: abi::Reg = bc.into();
    let cp: abi::Reg = cp.into();
    let icb: abi::Reg = icb.into();
    let (prop, scratch, width) = (d.r(), d.r(), d.r());
    let prefix = d.label("props");
    emit_unicode_property_ptr_free(
        INPUT_DECODE_SYMBOL,
        &prefix,
        cp,
        &prop,
        &scratch,
        d.ins,
        d.rel,
    );
    emit_read_boundclass_icb_charwidth_free(&prop, bc, icb, &width, &scratch, d.ins);
}

/// Plain text: one scalar, or (CLUSTER) one grapheme cluster.
fn emit_text(d: &mut Dec, exits: &Exits) {
    let p = d.p;
    let (slen, bad) = (d.r(), d.r());
    let ok = d.label("text_ok");
    let invalid = d.label("text_invalid");
    let truncated = d.label("text_truncated");
    emit_scalar_len(d, &p, &slen, &bad, &ok, &invalid, &truncated);
    d.at(&invalid);
    let cons = d.cons;
    d.push(abi::move_register(&cons, &bad));
    d.goto(&exits.skip);
    // Truncated at the end of what arrived: wait, or (FINAL) drop it.
    d.at(&truncated);
    d.unless_flag(FLAG_FINAL, &exits.need_more);
    let len = d.len;
    d.push(abi::subtract_registers(&cons, &len, &p));
    d.goto(&exits.skip);

    d.at(&ok);
    emit_copy_to_grapheme(d, &p, &slen);
    let finish = d.label("text_finish");
    if !d.clusters {
        let (cons, glen, er, mr) = (d.cons, d.glen, d.event, d.mods);
        d.push(abi::move_register(&cons, &glen));
        d.imm(&er, KeyEvent::NotSupported as u64);
        d.imm(&mr, 0);
        d.goto(&exits.unicode);
        return;
    }
    d.unless_flag(FLAG_CLUSTER, &finish);

    // Extend the cluster while the next scalar does not break from it.
    let (cp, sbc, sicb) = (d.r(), d.r(), d.r());
    emit_codepoint(d, &p, &slen, &cp);
    emit_break_props(d, &cp, &sbc, &sicb);
    let end = d.r();
    d.push(abi::add_registers(&end, &p, &slen));
    let top = d.label("cluster_top");
    let open = d.label("cluster_open");
    d.at(&top);
    d.bge_u(&end, &len, &open);
    let b = d.r();
    d.byte_at(&b, &end);
    d.blo(&b, 0x20, &finish);
    d.beq(&b, 0x7F, &finish);
    let (nlen, nbad) = (d.r(), d.r());
    let next_ok = d.label("cluster_next_ok");
    emit_scalar_len(d, &end, &nlen, &nbad, &next_ok, &finish, &open);
    d.at(&next_ok);
    let (ncp, pbc, picb) = (d.r(), d.r(), d.r());
    emit_codepoint(d, &end, &nlen, &ncp);
    emit_break_props(d, &ncp, &pbc, &picb);
    let joins = d.label("cluster_joins");
    let prefix = d.label("gb");
    emit_grapheme_break_branch_free(&prefix, &sbc, &sicb, &pbc, &picb, &finish, &joins, d.ins);
    d.at(&joins);
    // The cap: a scalar that would carry the grapheme past it starts the next token.
    let grown = d.r();
    let glen = d.glen;
    d.push(abi::add_registers(&grown, &glen, &nlen));
    d.bhi(&grown, GRAPHEME_CAP as u64, &finish);
    emit_copy_to_grapheme(d, &end, &nlen);
    let prefix = d.label("gs");
    emit_grapheme_state_update_free(&prefix, &sbc, &sicb, &pbc, &picb, d.ins);
    d.push(abi::add_registers(&end, &end, &nlen));
    d.goto(&top);
    // The cluster reaches the end of what has arrived: more of it may follow.
    d.at(&open);
    d.unless_flag(FLAG_FINAL, &exits.need_more);
    d.at(&finish);
    let (er, mr) = (d.event, d.mods);
    d.push(abi::move_register(&cons, &glen));
    d.imm(&er, KeyEvent::NotSupported as u64);
    d.imm(&mr, 0);
    d.goto(&exits.unicode);
}

/// `ESC <c>` where `<c>` starts a sequence that never came: Alt+`<c>`.
fn emit_alt_introducer(d: &mut Dec, c: u8, exits: &Exits) {
    let g = d.r();
    d.imm(&g, u64::from(c));
    d.append_grapheme_byte(&g);
    let (er, mr, cr) = (d.event, d.mods, d.cons);
    d.imm(&er, KeyEvent::NotSupported as u64);
    d.imm(&mr, MOD_ALT);
    d.imm(&cr, 2);
    d.goto(&exits.unicode);
}

/// `mods = modifier_bits(parameter)` (`NONE` counts as 1).
fn emit_modifier_bits(d: &mut Dec, parameter: impl Into<abi::Reg>, mods: impl Into<abi::Reg>) {
    let mods: abi::Reg = mods.into();
    let parameter: abi::Reg = parameter.into();
    let (bits, t, m) = (d.r(), d.r(), d.r());
    let none = d.label("mods_none");
    let have = d.label("mods_have");
    let done = d.label("mods_done");
    d.imm(mods, 0);
    d.push(abi::move_immediate(&t, "Integer", NONE));
    d.push(abi::compare_registers(parameter, &t));
    d.push(abi::branch_eq(&none));
    d.beq(parameter, 0, &none);
    d.goto(&have);
    d.at(&have);
    d.push(abi::subtract_immediate(&bits, parameter, 1));
    d.imm(&m, MOD_SHIFT | MOD_ALT | MOD_CTRL);
    d.push(abi::and_registers(mods, &bits, &m));
    d.imm(&m, 8 | 32);
    d.push(abi::and_registers(&t, &bits, &m));
    d.beq(&t, 0, &done);
    d.imm(&m, MOD_META);
    d.push(abi::or_registers(mods, mods, &m));
    d.goto(&done);
    d.at(&none);
    d.at(&done);
}

/// `ESC [` … — CSI.
fn emit_csi(d: &mut Dec, exits: &Exits) {
    let (p, len) = (d.p, d.len);
    let cons = d.cons;
    let base = d.r();
    d.push(abi::add_immediate(&base, &p, 2));

    // Linux console F1–F5: `ESC [ [ A`–`E`.
    let not_linux = d.label("csi_not_linux");
    d.bge_u(&base, &len, &not_linux);
    let b = d.r();
    d.byte_at(&b, &base);
    d.bne(&b, u64::from(b'['), &not_linux);
    let at = d.r();
    d.push(abi::add_immediate(&at, &base, 1));
    let linux_have = d.label("linux_have");
    let linux_skip = d.label("linux_skip");
    let found = d.label("linux_found");
    d.push(abi::compare_registers(&len, &at));
    d.push(abi::branch_hi(&linux_have));
    d.unless_flag(FLAG_FINAL, &exits.need_more);
    d.goto(&linux_skip);
    d.at(&linux_have);
    let c = d.r();
    d.byte_at(&c, &at);
    let key = d.key;
    let table: Vec<(u64, u64)> = LINUX_CONSOLE_F_KEYS
        .iter()
        .map(|(c, key)| (u64::from(*c), k(*key)))
        .collect();
    d.lookup(&c, &table, &key, &found);
    d.at(&linux_skip);
    d.imm(&cons, 3);
    d.goto(&exits.skip);
    d.at(&found);
    let (er, mr) = (d.event, d.mods);
    d.imm(&er, KeyEvent::NotSupported as u64);
    d.imm(&mr, 0);
    d.imm(&cons, 4);
    d.goto(&exits.named);

    d.at(&not_linux);
    // Parameter bytes, then intermediates (except `$`, rxvt's Shift final).
    let j = d.r();
    d.push(abi::move_register(&j, &base));
    let params_top = d.label("params_top");
    let params_done = d.label("params_done");
    d.at(&params_top);
    d.bge_u(&j, &len, &params_done);
    let b = d.r();
    d.byte_at(&b, &j);
    d.blo(&b, 0x30, &params_done);
    d.bhi(&b, 0x3F, &params_done);
    d.push(abi::add_immediate(&j, &j, 1));
    let n = d.r();
    d.push(abi::subtract_registers(&n, &j, &base));
    let within = d.label("params_within");
    d.push(abi::compare_immediate(&n, &CSI_PARAM_CAP.to_string()));
    d.push(abi::branch_ls(&within));
    d.push(abi::subtract_registers(&cons, &j, &p));
    d.goto(&exits.skip);
    d.at(&within);
    d.goto(&params_top);
    d.at(&params_done);
    let pe = d.r();
    d.push(abi::move_register(&pe, &j));
    let inter_top = d.label("inter_top");
    let inter_done = d.label("inter_done");
    d.at(&inter_top);
    d.bge_u(&j, &len, &inter_done);
    let b = d.r();
    d.byte_at(&b, &j);
    d.blo(&b, 0x20, &inter_done);
    d.bhi(&b, 0x2F, &inter_done);
    d.beq(&b, u64::from(b'$'), &inter_done);
    d.push(abi::add_immediate(&j, &j, 1));
    d.goto(&inter_top);
    d.at(&inter_done);

    // Ran out of input before a final byte.
    let have_final = d.label("csi_have_final");
    d.push(abi::compare_registers(&len, &j));
    d.push(abi::branch_hi(&have_final));
    d.unless_flag(FLAG_FINAL, &exits.need_more);
    let not_bare = d.label("csi_not_bare");
    d.push(abi::compare_registers(&j, &base));
    d.push(abi::branch_ne(&not_bare));
    emit_alt_introducer(d, b'[', exits);
    d.at(&not_bare);
    d.push(abi::subtract_registers(&cons, &j, &p));
    d.goto(&exits.skip);

    d.at(&have_final);
    let fin = d.r();
    d.byte_at(&fin, &j);
    let final_ok = d.label("csi_final_ok");
    let final_bad = d.label("csi_final_bad");
    d.beq(&fin, u64::from(b'$'), &final_ok);
    d.blo(&fin, 0x40, &final_bad);
    d.bhi(&fin, 0x7E, &final_bad);
    d.goto(&final_ok);
    d.at(&final_bad);
    d.push(abi::subtract_registers(&cons, &j, &p));
    d.goto(&exits.skip);
    d.at(&final_ok);
    // consumed = j + 1 - p
    d.push(abi::subtract_registers(&cons, &j, &p));
    d.push(abi::add_immediate(&cons, &cons, 1));
    // Intermediates, or a private marker anywhere: not a key.
    d.push(abi::compare_registers(&pe, &j));
    d.push(abi::branch_ne(&exits.skip));
    let scan = d.r();
    d.push(abi::move_register(&scan, &base));
    let marker_top = d.label("marker_top");
    let marker_done = d.label("marker_done");
    let marker_next = d.label("marker_next");
    d.at(&marker_top);
    d.bge_u(&scan, &pe, &marker_done);
    let b = d.r();
    d.byte_at(&b, &scan);
    d.blo(&b, 0x3C, &marker_next);
    d.goto(&exits.skip);
    d.at(&marker_next);
    d.push(abi::add_immediate(&scan, &scan, 1));
    d.goto(&marker_top);
    d.at(&marker_done);

    let params = emit_parse_params(d, &base, &pe);
    // Modifiers and event, from the second field.
    let mods = d.mods;
    emit_modifier_bits(d, &params.f10, &mods);
    let event = d.event;
    emit_event(d, &params.f11, &event, &exits.skip);

    emit_csi_final(d, &fin, &params, exits);
}

/// Parsed CSI parameters: the subfields the decoder reads, and whether the
/// third field (kitty's associated text) was well-formed.
struct Params {
    f00: VirtualRegister,
    f01: VirtualRegister,
    f10: VirtualRegister,
    f11: VirtualRegister,
    f20: VirtualRegister,
    /// Count of text code points seen in the third field.
    text_count: VirtualRegister,
    /// A text code point was not a scalar value.
    text_bad: VirtualRegister,
}

/// Parse `buf[from..to)` as `;`-separated fields of `:`-separated decimals.
/// The third field's code points are encoded onto the grapheme as they are read
/// (kitty's associated text), up to the cap.
fn emit_parse_params(d: &mut Dec, from: impl Into<abi::Reg>, to: impl Into<abi::Reg>) -> Params {
    let to: abi::Reg = to.into();
    let from: abi::Reg = from.into();
    let params = Params {
        f00: d.r(),
        f01: d.r(),
        f10: d.r(),
        f11: d.r(),
        f20: d.r(),
        text_count: d.r(),
        text_bad: d.r(),
    };
    let text_full = d.r();
    for reg in [
        &params.f00,
        &params.f01,
        &params.f10,
        &params.f11,
        &params.f20,
    ] {
        d.push(abi::move_immediate(reg, "Integer", NONE));
    }
    d.imm(&params.text_count, 0);
    d.imm(&params.text_bad, 0);
    d.imm(&text_full, 0);
    let (fi, si, acc, has) = (d.r(), d.r(), d.r(), d.r());
    d.imm(&fi, 0);
    d.imm(&si, 0);
    d.imm(&acc, 0);
    d.imm(&has, 0);
    let k = d.r();
    d.push(abi::move_register(&k, from));
    let top = d.label("pp_top");
    let end = d.label("pp_end");
    let digit = d.label("pp_digit");
    let colon = d.label("pp_colon");
    let semi = d.label("pp_semi");
    let next = d.label("pp_next");
    d.at(&top);
    d.bge_u(&k, to, &end);
    let c = d.r();
    d.byte_at(&c, &k);
    d.beq(&c, u64::from(b':'), &colon);
    d.beq(&c, u64::from(b';'), &semi);
    d.goto(&digit);
    d.at(&digit);
    let (ten, dv, cap) = (d.r(), d.r(), d.r());
    d.imm(&ten, 10);
    d.push(abi::multiply_registers(&acc, &acc, &ten));
    d.push(abi::subtract_immediate(&dv, &c, 48));
    d.push(abi::add_registers(&acc, &acc, &dv));
    let clamped = d.label("pp_clamped");
    d.imm(&cap, PARAM_MAX);
    d.push(abi::compare_registers(&acc, &cap));
    d.push(abi::branch_ls(&clamped));
    d.push(abi::move_register(&acc, &cap));
    d.at(&clamped);
    d.imm(&has, 1);
    d.goto(&next);
    d.at(&colon);
    emit_store_param(d, &params, &fi, &si, &acc, &has, &text_full);
    d.push(abi::add_immediate(&si, &si, 1));
    d.goto(&next);
    d.at(&semi);
    emit_store_param(d, &params, &fi, &si, &acc, &has, &text_full);
    d.push(abi::add_immediate(&fi, &fi, 1));
    d.imm(&si, 0);
    d.at(&next);
    d.push(abi::add_immediate(&k, &k, 1));
    d.goto(&top);
    d.at(&end);
    emit_store_param(d, &params, &fi, &si, &acc, &has, &text_full);
    params
}

/// Store the subfield just ended (`acc`, if it had digits) into its slot and
/// reset the accumulator.
fn emit_store_param(
    d: &mut Dec,
    params: &Params,
    fi: impl Into<abi::Reg>,
    si: impl Into<abi::Reg>,
    acc: impl Into<abi::Reg>,
    has: impl Into<abi::Reg>,
    text_full: impl Into<abi::Reg>,
) {
    let fi: abi::Reg = fi.into();
    let has: abi::Reg = has.into();
    let si: abi::Reg = si.into();
    let text_full: abi::Reg = text_full.into();
    let acc: abi::Reg = acc.into();
    let value = d.r();
    let have = d.label("sp_have");
    let chosen = d.label("sp_chosen");
    d.push(abi::move_immediate(&value, "Integer", NONE));
    d.beq(has, 0, &chosen);
    d.goto(&have);
    d.at(&have);
    d.push(abi::move_register(&value, acc));
    d.at(&chosen);
    let done = d.label("sp_done");
    let field1 = d.label("sp_f1");
    let field2 = d.label("sp_f2");
    let f01 = d.label("sp_f01");
    d.beq(fi, 1, &field1);
    d.beq(fi, 2, &field2);
    d.bne(fi, 0, &done);
    d.bne(si, 0, &f01);
    d.push(abi::move_register(&params.f00, &value));
    d.goto(&done);
    d.at(&f01);
    d.bne(si, 1, &done);
    d.push(abi::move_register(&params.f01, &value));
    d.goto(&done);
    d.at(&field1);
    let f11 = d.label("sp_f11");
    d.bne(si, 0, &f11);
    d.push(abi::move_register(&params.f10, &value));
    d.goto(&done);
    d.at(&f11);
    d.bne(si, 1, &done);
    d.push(abi::move_register(&params.f11, &value));
    d.goto(&done);
    // The third field: kitty's associated text, one code point per subfield.
    d.at(&field2);
    let text = d.label("sp_text");
    d.bne(si, 0, &text);
    d.push(abi::move_register(&params.f20, &value));
    d.at(&text);
    d.beq(has, 0, &done);
    d.bne(text_full, 0, &done);
    d.push(abi::add_immediate(
        &params.text_count,
        &params.text_count,
        1,
    ));
    let bad = d.label("sp_text_bad");
    let full = d.label("sp_text_full");
    emit_encode_utf8(d, &value, &bad, Some(&full));
    d.goto(&done);
    d.at(&bad);
    d.imm(&params.text_bad, 1);
    d.goto(&done);
    d.at(&full);
    d.imm(text_full, 1);
    d.at(&done);
    d.imm(acc, 0);
    d.imm(has, 0);
}

/// Append the UTF-8 of `cp` to the grapheme; `bad` when it is not a scalar
/// value, `full` (when given) when it would carry the grapheme past the cap.
fn emit_encode_utf8(d: &mut Dec, cp: impl Into<abi::Reg>, bad: &str, full: Option<&str>) {
    let cp: abi::Reg = cp.into();
    let n = d.r();
    let (l1, l2, l3, l4, sized) = (
        d.label("enc_l1"),
        d.label("enc_l2"),
        d.label("enc_l3"),
        d.label("enc_l4"),
        d.label("enc_sized"),
    );
    d.blo(cp, 0x80, &l1);
    d.blo(cp, 0x800, &l2);
    d.blo(cp, 0xD800, &l3);
    d.blo(cp, 0xE000, bad);
    d.blo(cp, 0x10000, &l3);
    d.bhi(cp, 0x10FFFF, bad);
    d.goto(&l4);
    for (label, bytes) in [(&l1, 1), (&l2, 2), (&l3, 3), (&l4, 4)] {
        d.at(label);
        d.imm(&n, bytes);
        d.goto(&sized);
    }
    d.at(&sized);
    if let Some(full) = full {
        let grown = d.r();
        let glen = d.glen;
        d.push(abi::add_registers(&grown, &glen, &n));
        d.bhi(&grown, GRAPHEME_CAP as u64, full);
    }
    // Bytes, most significant first.
    let (b, t, m) = (d.r(), d.r(), d.r());
    let (w2, w3, w4, done) = (
        d.label("enc_w2"),
        d.label("enc_w3"),
        d.label("enc_w4"),
        d.label("enc_done"),
    );
    d.beq(&n, 2, &w2);
    d.beq(&n, 3, &w3);
    d.beq(&n, 4, &w4);
    d.push(abi::move_register(&b, cp));
    d.append_grapheme_byte(&b);
    d.goto(&done);
    let lead_and_tail = |d: &mut Dec, lead_mask: u64, count: u8| {
        // The lead byte carries the high bits, each continuation byte six more.
        d.push(abi::shift_right_immediate(&t, cp, 6 * (count - 1)));
        d.imm(&m, lead_mask);
        d.push(abi::or_registers(&b, &t, &m));
        d.append_grapheme_byte(&b);
        for i in (0..count - 1).rev() {
            d.push(abi::shift_right_immediate(&t, cp, 6 * i));
            d.imm(&m, 0x3F);
            d.push(abi::and_registers(&t, &t, &m));
            d.imm(&m, 0x80);
            d.push(abi::or_registers(&b, &t, &m));
            d.append_grapheme_byte(&b);
        }
    };
    d.at(&w2);
    lead_and_tail(d, 0xC0, 2);
    d.goto(&done);
    d.at(&w3);
    lead_and_tail(d, 0xE0, 3);
    d.goto(&done);
    d.at(&w4);
    lead_and_tail(d, 0xF0, 4);
    d.at(&done);
}

/// `event` from a CSI's `:event` subfield; `invalid` for an undefined one.
fn emit_event(
    d: &mut Dec,
    subfield: impl Into<abi::Reg>,
    event: impl Into<abi::Reg>,
    invalid: &str,
) {
    let event: abi::Reg = event.into();
    let subfield: abi::Reg = subfield.into();
    let (t, done) = (d.r(), d.label("ev_done"));
    let evdef = d.evdef;
    d.push(abi::move_register(event, &evdef));
    d.push(abi::move_immediate(&t, "Integer", NONE));
    d.push(abi::compare_registers(subfield, &t));
    d.push(abi::branch_eq(&done));
    let table = [
        (1, KeyEvent::Press as u64),
        (2, KeyEvent::Repeat as u64),
        (3, KeyEvent::Release as u64),
    ];
    d.lookup(subfield, &table, event, &done);
    d.goto(invalid);
    d.at(&done);
}

fn table_of(entries: &[(u8, KeyName)]) -> Vec<(u64, u64)> {
    entries
        .iter()
        .map(|(c, key)| (u64::from(*c), k(*key)))
        .collect()
}

/// Everything after a CSI's parameters are parsed.
fn emit_csi_final(d: &mut Dec, fin: impl Into<abi::Reg>, params: &Params, exits: &Exits) {
    let fin: abi::Reg = fin.into();
    let (key, event, mods) = (d.key, d.event, d.mods);
    // CSI [1;m] <letter>
    d.lookup(fin, &table_of(CSI_LETTER_KEYS), &key, &exits.named);
    // rxvt Shift+arrow
    let rxvt = d.label("csi_rxvt");
    d.lookup(fin, &table_of(RXVT_ARROWS), &key, &rxvt);
    let not_z = d.label("csi_not_z");
    d.bne(fin, u64::from(b'Z'), &not_z);
    d.imm(&key, k(KeyName::Tab));
    let m = d.r();
    d.imm(&m, MOD_SHIFT);
    d.push(abi::or_registers(&mods, &mods, &m));
    d.goto(&exits.named);
    d.at(&not_z);
    let tilde = d.label("csi_tilde");
    let kitty = d.label("csi_kitty");
    d.beq(fin, u64::from(b'~'), &tilde);
    d.beq(fin, u64::from(b'$'), &tilde);
    d.beq(fin, u64::from(b'^'), &tilde);
    d.beq(fin, u64::from(b'@'), &tilde);
    d.beq(fin, u64::from(b'u'), &kitty);
    d.goto(&exits.skip);

    d.at(&rxvt);
    d.imm(&event, KeyEvent::NotSupported as u64);
    d.imm(&mods, MOD_SHIFT);
    d.goto(&exits.named);

    // CSI n [;m] ~ (and rxvt's $ ^ @).
    d.at(&tilde);
    let n = d.r();
    let t = d.r();
    let have_n = d.label("tilde_have_n");
    d.push(abi::move_register(&n, &params.f00));
    d.push(abi::move_immediate(&t, "Integer", NONE));
    d.push(abi::compare_registers(&n, &t));
    d.push(abi::branch_ne(&have_n));
    d.imm(&n, 0);
    d.at(&have_n);
    let not_mok = d.label("tilde_not_mok");
    d.bne(fin, u64::from(b'~'), &not_mok);
    d.bne(&n, TILDE_MODIFY_OTHER_KEYS, &not_mok);
    // xterm modifyOtherKeys: CSI 27 ; m ; code ~
    d.push(abi::compare_registers(&params.f20, &t));
    d.push(abi::branch_eq(&exits.skip));
    let shifted = d.r();
    d.push(abi::move_immediate(&shifted, "Integer", NONE));
    emit_code_token(d, &params.f20, &shifted, exits);
    d.at(&not_mok);
    let found = d.label("tilde_found");
    let tilde_keys: Vec<(u64, u64)> = TILDE_KEYS.iter().map(|(n, key)| (*n, k(*key))).collect();
    d.lookup(&n, &tilde_keys, &key, &found);
    d.goto(&exits.skip);
    d.at(&found);
    let (shift, ctrl, ctrl_shift) = (
        d.label("tilde_shift"),
        d.label("tilde_ctrl"),
        d.label("tilde_ctrl_shift"),
    );
    d.beq(fin, u64::from(b'$'), &shift);
    d.beq(fin, u64::from(b'^'), &ctrl);
    d.beq(fin, u64::from(b'@'), &ctrl_shift);
    d.goto(&exits.named);
    for (label, bits) in [
        (shift, MOD_SHIFT),
        (ctrl, MOD_CTRL),
        (ctrl_shift, MOD_CTRL | MOD_SHIFT),
    ] {
        d.at(&label);
        d.imm(&mods, bits);
        d.imm(&event, KeyEvent::NotSupported as u64);
        d.goto(&exits.named);
    }

    // kitty CSI code[:shifted[:base]] ; mods[:event] ; text u
    d.at(&kitty);
    let t = d.r();
    d.push(abi::move_immediate(&t, "Integer", NONE));
    d.push(abi::compare_registers(&params.f00, &t));
    d.push(abi::branch_eq(&exits.skip));
    let no_text = d.label("kitty_no_text");
    let text_token = d.label("kitty_text_token");
    d.beq(&params.text_count, 0, &no_text);
    d.bne(&params.text_bad, 0, &exits.skip);
    // Associated text names a Unicode key — unless the code is one of the named
    // keys (kitty sends no text for those, but a code that says Enter is Enter).
    d.beq(&params.f00, 0, &text_token);
    let named_codes: Vec<(u64, u64)> = KITTY_CODES
        .iter()
        .filter(|(_, what)| matches!(what, KittyCode::Named(_)))
        .map(|(code, _)| (*code, 0))
        .collect();
    let is_named = d.label("kitty_text_named");
    let scratch = d.r();
    d.lookup(&params.f00, &named_codes, &scratch, &is_named);
    d.goto(&text_token);
    d.at(&text_token);
    d.goto(&exits.unicode);
    d.at(&is_named);
    // The parsed text went onto the grapheme; a code token writes its own.
    let glen = d.glen;
    d.imm(&glen, 0);
    d.goto(&no_text);
    d.at(&no_text);
    d.beq(&params.f00, 0, &exits.skip);
    let f00 = params.f00;
    let f01 = params.f01;
    emit_code_token(d, &f00, &f01, exits);
}

/// A token for the key whose kitty/xterm code is `code` (`shifted` is the
/// alternate-key subfield, or `NONE`).
fn emit_code_token(
    d: &mut Dec,
    code: impl Into<abi::Reg>,
    shifted: impl Into<abi::Reg>,
    exits: &Exits,
) {
    let code: abi::Reg = code.into();
    let shifted: abi::Reg = shifted.into();
    let key = d.key;
    let glen = d.glen;
    d.imm(&glen, 0);
    let named: Vec<(u64, u64)> = KITTY_CODES
        .iter()
        .filter_map(|(c, what)| match what {
            KittyCode::Named(key) => Some((*c, k(*key))),
            KittyCode::Text(_) => None,
        })
        .collect();
    d.lookup(code, &named, &key, &exits.named);
    let texts: Vec<(u64, u64)> = KITTY_CODES
        .iter()
        .filter_map(|(c, what)| match what {
            KittyCode::Text(b) => Some((*c, u64::from(*b))),
            KittyCode::Named(_) => None,
        })
        .collect();
    let g = d.r();
    let text_found = d.label("code_text");
    d.lookup(code, &texts, &g, &text_found);
    // Keys `io::KeyName` has no member for, and C0 codes kitty never sends.
    let not_pua = d.label("code_not_pua");
    d.blo(code, KITTY_PUA_FIRST, &not_pua);
    d.bhi(code, KITTY_PUA_LAST, &not_pua);
    d.goto(&exits.skip);
    d.at(&not_pua);
    d.blo(code, 0x20, &exits.skip);
    // The grapheme is what the key types: with Shift, the shifted key.
    let cp = d.r();
    d.push(abi::move_register(&cp, code));
    let (m, t) = (d.r(), d.r());
    let chosen = d.label("code_chosen");
    let mods = d.mods;
    d.imm(&m, MOD_SHIFT);
    d.push(abi::and_registers(&t, &mods, &m));
    d.beq(&t, 0, &chosen);
    let no_alternate = d.label("code_no_alternate");
    d.push(abi::move_immediate(&t, "Integer", NONE));
    d.push(abi::compare_registers(shifted, &t));
    d.push(abi::branch_eq(&no_alternate));
    d.push(abi::move_register(&cp, shifted));
    d.goto(&chosen);
    d.at(&no_alternate);
    d.blo(code, u64::from(b'a'), &chosen);
    d.bhi(code, u64::from(b'z'), &chosen);
    d.push(abi::subtract_immediate(&cp, code, 32));
    d.at(&chosen);
    emit_encode_utf8(d, &cp, &exits.skip, None);
    d.goto(&exits.unicode);
    d.at(&text_found);
    d.append_grapheme_byte(&g);
    d.goto(&exits.unicode);
}

/// `ESC O` … — SS3.
fn emit_ss3(d: &mut Dec, exits: &Exits) {
    let (p, len) = (d.p, d.len);
    let cons = d.cons;
    let j = d.r();
    d.push(abi::add_immediate(&j, &p, 2));
    let (value, has) = (d.r(), d.r());
    d.imm(&value, 0);
    d.imm(&has, 0);
    let top = d.label("ss3_top");
    let digits_done = d.label("ss3_digits_done");
    d.at(&top);
    d.bge_u(&j, &len, &digits_done);
    let b = d.r();
    d.byte_at(&b, &j);
    d.blo(&b, u64::from(b'0'), &digits_done);
    d.bhi(&b, u64::from(b'9'), &digits_done);
    let (ten, dv, cap) = (d.r(), d.r(), d.r());
    d.imm(&ten, 10);
    d.push(abi::multiply_registers(&value, &value, &ten));
    d.push(abi::subtract_immediate(&dv, &b, 48));
    d.push(abi::add_registers(&value, &value, &dv));
    let clamped = d.label("ss3_clamped");
    d.imm(&cap, 1_000_000);
    d.push(abi::compare_registers(&value, &cap));
    d.push(abi::branch_ls(&clamped));
    d.push(abi::move_register(&value, &cap));
    d.at(&clamped);
    d.imm(&has, 1);
    d.push(abi::add_immediate(&j, &j, 1));
    d.goto(&top);
    d.at(&digits_done);
    let have_final = d.label("ss3_have_final");
    d.push(abi::compare_registers(&len, &j));
    d.push(abi::branch_hi(&have_final));
    d.unless_flag(FLAG_FINAL, &exits.need_more);
    let base = d.r();
    d.push(abi::add_immediate(&base, &p, 2));
    let not_bare = d.label("ss3_not_bare");
    d.push(abi::compare_registers(&j, &base));
    d.push(abi::branch_ne(&not_bare));
    emit_alt_introducer(d, b'O', exits);
    d.at(&not_bare);
    d.push(abi::subtract_registers(&cons, &j, &p));
    d.goto(&exits.skip);

    d.at(&have_final);
    let fin = d.r();
    d.byte_at(&fin, &j);
    d.push(abi::subtract_registers(&cons, &j, &p));
    d.push(abi::add_immediate(&cons, &cons, 1));
    let (key, event, mods) = (d.key, d.event, d.mods);
    d.imm(&event, KeyEvent::NotSupported as u64);
    let parameter = d.r();
    let mods_set = d.label("ss3_mods_set");
    d.push(abi::move_immediate(&parameter, "Integer", NONE));
    d.beq(&has, 0, &mods_set);
    d.push(abi::move_register(&parameter, &value));
    d.at(&mods_set);
    emit_modifier_bits(d, &parameter, &mods);
    d.lookup(&fin, &table_of(SS3_KEYS), &key, &exits.named);
    let rxvt = d.label("ss3_rxvt");
    d.lookup(&fin, &table_of(RXVT_ARROWS), &key, &rxvt);
    let g = d.r();
    let keypad = d.label("ss3_keypad");
    let keypad_table: Vec<(u64, u64)> = SS3_KEYPAD_TEXT
        .iter()
        .map(|(c, b)| (u64::from(*c), u64::from(*b)))
        .collect();
    d.lookup(&fin, &keypad_table, &g, &keypad);
    let other = d.label("ss3_other");
    d.blo(&fin, 0x40, &other);
    d.bhi(&fin, 0x7E, &other);
    d.goto(&exits.skip);
    // `ESC O` then a byte that is no SS3 final: Alt+O, and that byte is next.
    d.at(&other);
    let glen = d.glen;
    d.imm(&glen, 0);
    emit_alt_introducer(d, b'O', exits);
    d.at(&rxvt);
    d.imm(&mods, MOD_CTRL);
    d.goto(&exits.named);
    d.at(&keypad);
    d.append_grapheme_byte(&g);
    d.goto(&exits.unicode);
}

/// The four finishing blocks, shared by every path.
fn emit_exits(d: &mut Dec, exits: &Exits) {
    let (out, p) = (d.out, d.p);
    let (key, event, mods, cons, glen) = (d.key, d.event, d.mods, d.cons, d.glen);
    let status = d.r();
    let text_kind = d.r();
    let scalar = d.r();
    let ret = d.label("ret");

    d.at(&exits.need_more);
    d.imm(&status, STATUS_NEED_MORE);
    d.push(abi::store_u64(&status, &out, TOKEN_STATUS));
    d.goto(&ret);

    // Skip: after Alt prefixes, bytes that are not a key make the last prefix
    // ESC the Escape key (the earlier ones its Alt).
    d.at(&exits.skip);
    let prefixed = d.label("skip_prefixed");
    d.bne(&p, 0, &prefixed);
    d.imm(&status, STATUS_SKIP);
    d.push(abi::store_u64(&status, &out, TOKEN_STATUS));
    d.push(abi::store_u64(&cons, &out, TOKEN_CONSUMED));
    d.goto(&ret);
    d.at(&prefixed);
    d.push(abi::subtract_immediate(&p, &p, 1));
    d.imm(&key, k(KeyName::Escape));
    d.imm(&event, KeyEvent::NotSupported as u64);
    d.imm(&mods, 0);
    d.imm(&cons, 1);
    d.goto(&exits.named);

    // Named key: its grapheme and text are Decision 2's.
    let common = d.label("common");
    d.at(&exits.named);
    d.imm(&glen, 0);
    d.imm(&text_kind, TEXT_NONE);
    let g = d.r();
    let has = d.label("named_has");
    let table: Vec<(u64, u64)> = [
        KeyName::Enter,
        KeyName::Tab,
        KeyName::Backspace,
        KeyName::Escape,
    ]
    .iter()
    .map(|key| (k(*key), u64::from(named_key_text(*key).expect("text key"))))
    .collect();
    d.lookup(&key, &table, &g, &has);
    d.goto(&common);
    d.at(&has);
    d.append_grapheme_byte(&g);
    d.beq(&event, KeyEvent::Release as u64, &common);
    d.imm(&text_kind, TEXT_GRAPHEME);
    d.goto(&common);

    // Unicode key: the grapheme is in place; the text is it, or Ctrl's C0
    // code, or nothing on a release.
    d.at(&exits.unicode);
    d.imm(&key, k(KeyName::Unicode));
    d.imm(&text_kind, TEXT_NONE);
    d.beq(&event, KeyEvent::Release as u64, &common);
    d.imm(&text_kind, TEXT_GRAPHEME);
    let (m, t) = (d.r(), d.r());
    d.imm(&m, MOD_CTRL);
    d.push(abi::and_registers(&t, &mods, &m));
    d.beq(&t, 0, &common);
    d.bne(&glen, 1, &common);
    let c = d.r();
    d.push(abi::load_u8(&c, &out, TOKEN_GRAPHEME));
    let ctrl_table: Vec<(u64, u64)> = (0u8..=0x7F)
        .filter_map(|b| ctrl_code(b).map(|code| (u64::from(b), u64::from(code))))
        .collect();
    let mapped = d.label("ctrl_mapped");
    d.lookup(&c, &ctrl_table, &scalar, &mapped);
    d.goto(&common);
    d.at(&mapped);
    d.imm(&text_kind, TEXT_SCALAR);
    d.push(abi::store_u64(&scalar, &out, TOKEN_TEXT_SCALAR));

    // Common: Alt for every prefix ESC, the consumed count includes them.
    d.at(&common);
    let no_prefix = d.label("no_prefix");
    d.beq(&p, 0, &no_prefix);
    let (am, total) = (d.r(), d.r());
    d.imm(&am, MOD_ALT);
    d.push(abi::or_registers(&mods, &mods, &am));
    d.at(&no_prefix);
    d.push(abi::add_registers(&total, &cons, &p));
    d.imm(&status, STATUS_TOKEN);
    d.push(abi::store_u64(&status, &out, TOKEN_STATUS));
    d.push(abi::store_u64(&total, &out, TOKEN_CONSUMED));
    d.push(abi::store_u64(&key, &out, TOKEN_KEY));
    d.push(abi::store_u64(&event, &out, TOKEN_EVENT));
    d.push(abi::store_u64(&mods, &out, TOKEN_MODS));
    d.push(abi::store_u64(&text_kind, &out, TOKEN_TEXT_KIND));
    d.push(abi::store_u64(&glen, &out, TOKEN_GRAPHEME_LEN));

    d.at(&ret);
    d.push(abi::load_u64(abi::return_register(), &out, TOKEN_STATUS));
    d.push(abi::return_());
}
