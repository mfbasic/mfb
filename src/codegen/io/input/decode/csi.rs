//! plan-176-P: the input decoder's escape sequences — CSI/SS3 introducers, modifiers,
//! parameters, events and final bytes. The token layout, `lower_input_decode` and the decode
//! body live in `io/input/decode/mod.rs`.

use super::*;

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
pub(super) fn emit_csi(d: &mut Dec, exits: &Exits) {
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
pub(super) fn emit_ss3(d: &mut Dec, exits: &Exits) {
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
