//! plan-176-P: the input decoder's text tokens — scalar length, grapheme copy, codepoint and
//! break properties, and UTF-8 encoding. The token layout, `lower_input_decode` and the decode
//! body live in `io/input/decode/mod.rs`.

use super::*;

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
pub(super) fn emit_text(d: &mut Dec, exits: &Exits) {
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

/// Append the UTF-8 of `cp` to the grapheme; `bad` when it is not a scalar
/// value, `full` (when given) when it would carry the grapheme past the cap.
pub(super) fn emit_encode_utf8(
    d: &mut Dec,
    cp: impl Into<abi::Reg>,
    bad: &str,
    full: Option<&str>,
) {
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
