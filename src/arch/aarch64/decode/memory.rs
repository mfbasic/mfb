//! Loads and stores: exclusive and ordered accesses, compare-and-swap, literal
//! loads, pairs, single registers in every addressing mode, LSE atomics,
//! authenticated loads and the Advanced SIMD structure loads and stores.

use super::{
    address, bit, field, hex, instruction, rd, register, relative, rm, rn, sign_extend, signed_hex,
    to_target, x, Flow, Instruction,
};

pub(super) fn decode(word: u32, pc: u64) -> Option<Instruction> {
    if !bit(word, 31) && matches!(field(word, 23, 7), 0b0011000..=0b0011011) {
        return structures(word);
    }
    if field(word, 24, 6) == 0b001000 {
        return exclusive(word);
    }
    if field(word, 24, 6) == 0b011001 && !bit(word, 21) && field(word, 10, 2) == 0 {
        return ordered_unscaled(word);
    }
    match field(word, 27, 3) {
        0b011 if !bit(word, 24) => literal(word, pc),
        0b101 => pair(word),
        0b111 => single(word),
        _ => None,
    }
}

/// A scalar SIMD&FP register of `bytes` bytes: `b0`, `h0`, `s0`, `d0`, `q0`.
fn scalar(number: u32, bytes: u32) -> String {
    let prefix = match bytes {
        1 => "b",
        2 => "h",
        4 => "s",
        8 => "d",
        _ => "q",
    };
    format!("{prefix}{number}")
}

/// The `b`/`h` suffix of a byte or halfword access, by its size field.
fn size_suffix(size: u32) -> &'static str {
    match size {
        0 => "b",
        1 => "h",
        _ => "",
    }
}

/// `ldxr`, `stlxr`, `ldxp`, `stlr`, `ldar`, `cas`, `casp` and their sizes.
fn exclusive(word: u32) -> Option<Instruction> {
    let size = field(word, 30, 2);
    let (ordered, load, pair_or_cas, acquire_release) =
        (bit(word, 23), bit(word, 22), bit(word, 21), bit(word, 15));
    let (status, value, second, base) = (rm(word), rd(word), field(word, 10, 5), rn(word));
    let wide = size == 3;
    let at = format!("[{}]", x(base, true));
    let suffix = size_suffix(size);
    Some(match (ordered, pair_or_cas) {
        (false, false) => {
            let order = if acquire_release { "a" } else { "" };
            if load {
                instruction(
                    &format!("ld{order}xr{suffix}"),
                    format!("{}, {at}", register(value, wide, false)),
                )
            } else {
                let order = if acquire_release { "l" } else { "" };
                instruction(
                    &format!("st{order}xr{suffix}"),
                    format!(
                        "{}, {}, {at}",
                        register(status, false, false),
                        register(value, wide, false)
                    ),
                )
            }
        }
        (false, true) if size >= 2 => {
            let wide = size == 3;
            if load {
                let order = if acquire_release { "a" } else { "" };
                instruction(
                    &format!("ld{order}xp"),
                    format!(
                        "{}, {}, {at}",
                        register(value, wide, false),
                        register(second, wide, false)
                    ),
                )
            } else {
                let order = if acquire_release { "l" } else { "" };
                instruction(
                    &format!("st{order}xp"),
                    format!(
                        "{}, {}, {}, {at}",
                        register(status, false, false),
                        register(value, wide, false),
                        register(second, wide, false)
                    ),
                )
            }
        }
        (false, true) => {
            if status % 2 != 0 || value % 2 != 0 {
                return None;
            }
            let wide = size == 1;
            let order = match (load, acquire_release) {
                (false, false) => "",
                (true, false) => "a",
                (false, true) => "l",
                (true, true) => "al",
            };
            let reg = |number| register(number, wide, false);
            instruction(
                &format!("casp{order}"),
                format!(
                    "{}, {}, {}, {}, {at}",
                    reg(status),
                    reg(status + 1),
                    reg(value),
                    reg(value + 1)
                ),
            )
        }
        (true, false) => {
            let mnemonic = match (load, acquire_release) {
                (false, true) => "stlr",
                (false, false) => "stllr",
                (true, true) => "ldar",
                (true, false) => "ldlar",
            };
            instruction(
                &format!("{mnemonic}{suffix}"),
                format!("{}, {at}", register(value, wide, false)),
            )
        }
        (true, true) => {
            let order = match (load, acquire_release) {
                (false, false) => "",
                (true, false) => "a",
                (false, true) => "l",
                (true, true) => "al",
            };
            instruction(
                &format!("cas{order}{suffix}"),
                format!(
                    "{}, {}, {at}",
                    register(status, wide, false),
                    register(value, wide, false)
                ),
            )
        }
    })
}

/// `stlur`/`ldapur` (unscaled, release/acquire-PC).
fn ordered_unscaled(word: u32) -> Option<Instruction> {
    let size = field(word, 30, 2);
    let opc = field(word, 22, 2);
    let offset = sign_extend(field(word, 12, 9), 9);
    let (mnemonic, wide) = match (size, opc) {
        (0, 0) => ("stlurb", false),
        (0, 1) => ("ldapurb", false),
        (0, 2) => ("ldapursb", true),
        (0, 3) => ("ldapursb", false),
        (1, 0) => ("stlurh", false),
        (1, 1) => ("ldapurh", false),
        (1, 2) => ("ldapursh", true),
        (1, 3) => ("ldapursh", false),
        (2, 0) => ("stlur", false),
        (2, 1) => ("ldapur", false),
        (2, 2) => ("ldapursw", true),
        (3, 0) => ("stlur", true),
        (3, 1) => ("ldapur", true),
        _ => return None,
    };
    Some(instruction(
        mnemonic,
        format!(
            "{}, {}",
            register(rd(word), wide, false),
            address(rn(word), offset)
        ),
    ))
}

/// A load of a pc-relative literal, printed with its address: `ldr x0, 0x30`.
fn literal(word: u32, pc: u64) -> Option<Instruction> {
    let target = relative(word, pc, 5, 19);
    let number = rd(word);
    let destination = match (field(word, 30, 2), bit(word, 26)) {
        (0b00, false) => ("ldr", register(number, false, false)),
        (0b01, false) => ("ldr", register(number, true, false)),
        (0b10, false) => ("ldrsw", register(number, true, false)),
        (0b11, false) => ("prfm", prefetch(number)),
        (0b00, true) => ("ldr", scalar(number, 4)),
        (0b01, true) => ("ldr", scalar(number, 8)),
        (0b10, true) => ("ldr", scalar(number, 16)),
        _ => return None,
    };
    Some(to_target(
        destination.0,
        &format!("{}, ", destination.1),
        target,
        Flow::Next,
    ))
}

/// A `prfm` operation: `pldl1keep`, …, or its number.
fn prefetch(operation: u32) -> String {
    let kind = ["pld", "pli", "pst"].get(field(operation, 3, 2) as usize);
    let target = ["l1", "l2", "l3", "slc"].get(field(operation, 1, 2) as usize);
    match (kind, target) {
        (Some(kind), Some(target)) => {
            let policy = if operation & 1 != 0 { "strm" } else { "keep" };
            format!("{kind}{target}{policy}")
        }
        _ => hex(u64::from(operation)),
    }
}

/// `stp`/`ldp`/`ldpsw`/`stnp`/`ldnp` with a signed offset, pre- or post-index.
fn pair(word: u32) -> Option<Instruction> {
    let opc = field(word, 30, 2);
    let simd = bit(word, 26);
    let mode = field(word, 23, 2);
    let load = bit(word, 22);
    let (first, second, base) = (rd(word), field(word, 10, 5), rn(word));
    let (mnemonic, bytes) = match (simd, opc, load, mode) {
        (false, 0b00, _, _) => (if load { "ldp" } else { "stp" }, 4),
        (false, 0b01, true, 1..=3) => ("ldpsw", 4),
        (false, 0b10, _, _) => (if load { "ldp" } else { "stp" }, 8),
        (true, 0b00, _, _) => (if load { "ldp" } else { "stp" }, 4),
        (true, 0b01, _, _) => (if load { "ldp" } else { "stp" }, 8),
        (true, 0b10, _, _) => (if load { "ldp" } else { "stp" }, 16),
        _ => return None,
    };
    let mnemonic = match (mode, mnemonic) {
        (0, "ldp") => "ldnp",
        (0, "stp") => "stnp",
        _ => mnemonic,
    };
    let reg = |number| {
        if simd {
            scalar(number, bytes)
        } else {
            register(number, bytes == 8 || mnemonic == "ldpsw", false)
        }
    };
    let offset = sign_extend(field(word, 15, 7), 7) * i64::from(bytes);
    Some(instruction(
        mnemonic,
        format!(
            "{}, {}, {}",
            reg(first),
            reg(second),
            indexed(base, offset, mode)
        ),
    ))
}

/// The address of a pair or register access by its index mode: 0 or 2 a
/// signed offset, 1 post-index, 3 pre-index.
fn indexed(base: u32, offset: i64, mode: u32) -> String {
    let base_name = x(base, true);
    match mode {
        1 => format!("[{base_name}], {}", signed_hex(offset)),
        3 => format!("[{base_name}, {}]!", signed_hex(offset)),
        _ => address(base, offset),
    }
}

/// What a single-register access of this size, register file and `opc` is:
/// its scaled mnemonic, the register it moves, and its access size in bytes.
fn single_kind(
    size: u32,
    simd: bool,
    opc: u32,
    number: u32,
) -> Option<(&'static str, String, u32)> {
    if simd {
        let (load, bytes) = match (size, opc) {
            (_, 0b00) => (false, 1 << size),
            (_, 0b01) => (true, 1 << size),
            (0, 0b10) => (false, 16),
            (0, 0b11) => (true, 16),
            _ => return None,
        };
        return Some((
            if load { "ldr" } else { "str" },
            scalar(number, bytes),
            bytes,
        ));
    }
    let (mnemonic, wide) = match (size, opc) {
        (0, 0) => ("strb", false),
        (0, 1) => ("ldrb", false),
        (0, 2) => ("ldrsb", true),
        (0, 3) => ("ldrsb", false),
        (1, 0) => ("strh", false),
        (1, 1) => ("ldrh", false),
        (1, 2) => ("ldrsh", true),
        (1, 3) => ("ldrsh", false),
        (2, 0) => ("str", false),
        (2, 1) => ("ldr", false),
        (2, 2) => ("ldrsw", true),
        (3, 0) => ("str", true),
        (3, 1) => ("ldr", true),
        (3, 2) => return Some(("prfm", prefetch(number), 8)),
        _ => return None,
    };
    Some((mnemonic, register(number, wide, false), 1 << size))
}

/// The unscaled (`ldur`…) or unprivileged (`ldtr`…) spelling of a scaled
/// mnemonic.
fn respelled(mnemonic: &str, infix: &str) -> Option<String> {
    if mnemonic == "prfm" {
        return (infix == "u").then(|| "prfum".to_string());
    }
    let (head, tail) = mnemonic.split_at(2);
    Some(format!("{head}{infix}{tail}"))
}

fn single(word: u32) -> Option<Instruction> {
    let size = field(word, 30, 2);
    let simd = bit(word, 26);
    let opc = field(word, 22, 2);
    let (number, base) = (rd(word), rn(word));
    if bit(word, 24) {
        let (mnemonic, target, bytes) = single_kind(size, simd, opc, number)?;
        let offset = i64::from(field(word, 10, 12)) * i64::from(bytes);
        return Some(instruction(
            mnemonic,
            format!("{target}, {}", address(base, offset)),
        ));
    }
    if !bit(word, 21) {
        let (mnemonic, target, _) = single_kind(size, simd, opc, number)?;
        let offset = sign_extend(field(word, 12, 9), 9);
        return Some(match field(word, 10, 2) {
            0b00 => instruction(
                &respelled(mnemonic, "u")?,
                format!("{target}, {}", address(base, offset)),
            ),
            0b10 if !simd => instruction(
                &respelled(mnemonic, "t")?,
                format!("{target}, {}", address(base, offset)),
            ),
            mode @ (0b01 | 0b11) if mnemonic != "prfm" => instruction(
                mnemonic,
                format!("{target}, {}", indexed(base, offset, mode)),
            ),
            _ => return None,
        });
    }
    match field(word, 10, 2) {
        0b10 => register_offset(word, size, simd, opc),
        0b00 if !simd => atomic(word, size),
        _ if !simd && size == 3 => authenticated(word),
        _ => None,
    }
}

/// `ldr x0, [x1, x2, lsl #3]`, `ldrb w0, [x1, w2, uxtw]`.
fn register_offset(word: u32, size: u32, simd: bool, opc: u32) -> Option<Instruction> {
    let option = field(word, 13, 3);
    if option & 0b010 == 0 {
        return None;
    }
    if size == 3 && !simd && opc == 2 && rd(word) >> 3 == 0b11 {
        // `rprfm`: the register-offset prefetch with a `11xxx` operation. Its
        // 6-bit operation is `option<2>:option<0>:S:Rt<2:0>`, and the offset
        // register is always an X register.
        let operation =
            (option >> 2) << 5 | (option & 1) << 4 | field(word, 12, 1) << 3 | rd(word) & 0b111;
        return Some(instruction(
            "rprfm",
            format!(
                "{}, {}, [{}]",
                hex(u64::from(operation)),
                x(rm(word), false),
                x(rn(word), true)
            ),
        ));
    }
    let (mnemonic, target, bytes) = single_kind(size, simd, opc, rd(word))?;
    let amount = if bit(word, 12) {
        bytes.trailing_zeros()
    } else {
        0
    };
    let index = register(rm(word), option & 1 == 1, false);
    let extend = match (option, bit(word, 12)) {
        (0b011, false) => String::new(),
        (0b011, true) => format!(", lsl #{amount}"),
        (_, scaled) => {
            let name = match option {
                0b010 => "uxtw",
                0b110 => "sxtw",
                _ => "sxtx",
            };
            if scaled {
                format!(", {name} #{amount}")
            } else {
                format!(", {name}")
            }
        }
    };
    Some(instruction(
        mnemonic,
        format!("{target}, [{}, {index}{extend}]", x(rn(word), true)),
    ))
}

/// The LSE atomics (`ldadd`, `swp`, …, their `st` aliases) and `ldapr`.
fn atomic(word: u32, size: u32) -> Option<Instruction> {
    let (acquire, release) = (bit(word, 23), bit(word, 22));
    let (source, target, base) = (rm(word), rd(word), rn(word));
    let opcode = field(word, 12, 3);
    let wide = size == 3;
    let suffix = size_suffix(size);
    let at = format!("[{}]", x(base, true));
    let reg = |number| register(number, wide, false);
    if bit(word, 15) {
        return match opcode {
            0b000 => {
                let order = match (acquire, release) {
                    (false, false) => "",
                    (true, false) => "a",
                    (false, true) => "l",
                    (true, true) => "al",
                };
                Some(instruction(
                    &format!("swp{order}{suffix}"),
                    format!("{}, {}, {at}", reg(source), reg(target)),
                ))
            }
            0b100 if acquire && !release && source == 31 => Some(instruction(
                &format!("ldapr{suffix}"),
                format!("{}, {at}", reg(target)),
            )),
            _ => None,
        };
    }
    let operation = ["add", "clr", "eor", "set", "smax", "smin", "umax", "umin"][opcode as usize];
    if target == 31 && !acquire {
        let order = if release { "l" } else { "" };
        return Some(instruction(
            &format!("st{operation}{order}{suffix}"),
            format!("{}, {at}", reg(source)),
        ));
    }
    let order = match (acquire, release) {
        (false, false) => "",
        (true, false) => "a",
        (false, true) => "l",
        (true, true) => "al",
    };
    Some(instruction(
        &format!("ld{operation}{order}{suffix}"),
        format!("{}, {}, {at}", reg(source), reg(target)),
    ))
}

/// `ldraa`/`ldrab`, with a scaled signed offset and optional pre-index.
fn authenticated(word: u32) -> Option<Instruction> {
    let mnemonic = if bit(word, 23) { "ldrab" } else { "ldraa" };
    let raw = (field(word, 22, 1) << 9) | field(word, 12, 9);
    let offset = sign_extend(raw, 10) * 8;
    let base = rn(word);
    let at = if bit(word, 11) {
        format!("[{}, {}]!", x(base, true), signed_hex(offset))
    } else {
        address(base, offset)
    };
    Some(instruction(
        mnemonic,
        format!("{}, {at}", x(rd(word), false)),
    ))
}

/// The arrangement of a vector of `2^size`-byte elements, 128 bits wide when
/// `full`: `8b`, `16b`, `4h`, `8h`, `2s`, `4s`, `1d`, `2d`.
pub(super) fn arrangement(size: u32, full: bool) -> &'static str {
    ["8b", "16b", "4h", "8h", "2s", "4s", "1d", "2d"][(size * 2 + u32::from(full)) as usize]
}

/// `{ v0, v1 }`: `count` registers from `first`, wrapping past `v31`.
pub(super) fn register_list(first: u32, count: u32) -> String {
    let names = (0..count)
        .map(|index| format!("v{}", (first + index) % 32))
        .collect::<Vec<_>>();
    format!("{{ {} }}", names.join(", "))
}

/// `ld1`–`ld4`/`st1`–`st4` of whole registers, of one lane, and `ld1r`–`ld4r`.
fn structures(word: u32) -> Option<Instruction> {
    let full = bit(word, 30);
    let post = bit(word, 23);
    let single_lane = bit(word, 24);
    let load = bit(word, 22);
    let (first, base, index_register) = (rd(word), rn(word), rm(word));
    if !post && field(word, 16, 6) != 0 {
        return None;
    }
    let size = field(word, 10, 2);
    let after = |bytes: u32| {
        if !post {
            String::new()
        } else if index_register == 31 {
            format!(", #{bytes}")
        } else {
            format!(", {}", x(index_register, false))
        }
    };
    let at = format!("[{}]", x(base, true));
    if !single_lane {
        if bit(word, 21) {
            return None;
        }
        let (elements, registers) = match field(word, 12, 4) {
            0b0000 => (4, 4),
            0b0010 => (1, 4),
            0b0100 => (3, 3),
            0b0110 => (1, 3),
            0b0111 => (1, 1),
            0b1000 => (2, 2),
            0b1010 => (1, 2),
            _ => return None,
        };
        if size == 3 && !full && elements != 1 {
            return None;
        }
        let mnemonic = format!(
            "{}{elements}.{}",
            if load { "ld" } else { "st" },
            arrangement(size, full)
        );
        let bytes = registers * if full { 16 } else { 8 };
        return Some(instruction(
            &mnemonic,
            format!("{}, {at}{}", register_list(first, registers), after(bytes)),
        ));
    }
    let opcode = field(word, 13, 3);
    let elements = ((opcode & 1) << 1 | field(word, 21, 1)) + 1;
    let s = field(word, 12, 1);
    let q = u32::from(full);
    if opcode >= 0b110 {
        if !load || s != 0 {
            return None;
        }
        let mnemonic = format!("ld{elements}r.{}", arrangement(size, full));
        return Some(instruction(
            &mnemonic,
            format!(
                "{}, {at}{}",
                register_list(first, elements),
                after(elements << size)
            ),
        ));
    }
    let (letter, lane, bytes) = match opcode >> 1 {
        0b00 => ("b", (q << 3) | (s << 2) | size, 1),
        0b01 if size & 1 == 0 => ("h", (q << 2) | (s << 1) | (size >> 1), 2),
        0b10 if size == 0 => ("s", (q << 1) | s, 4),
        0b10 if size == 1 && s == 0 => ("d", q, 8),
        _ => return None,
    };
    let mnemonic = format!("{}{elements}.{letter}", if load { "ld" } else { "st" });
    Some(instruction(
        &mnemonic,
        format!(
            "{}[{lane}], {at}{}",
            register_list(first, elements),
            after(elements * bytes)
        ),
    ))
}
