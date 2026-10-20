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

mod csi;
mod text;
use csi::*;
use text::*;

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
