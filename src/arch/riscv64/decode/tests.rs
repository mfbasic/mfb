//! The riscv64 decoder against GNU `objdump` (`tools/oracles/riscv64-decode`).

use super::*;

/// One corpus row: the address, the instruction's bytes, and `objdump`'s text.
struct Row<'a> {
    pc: u64,
    bytes: Vec<u8>,
    text: &'a str,
}

/// The rows of a corpus in `<pc>\t<bytes>\t<text>` form; `#` lines are
/// comments. `<bytes>` is the instruction as a number, 8 or 4 hex digits.
fn rows(corpus: &str) -> Vec<Row<'_>> {
    corpus
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .map(|line| {
            let mut fields = line.splitn(3, '\t');
            let pc = u64::from_str_radix(fields.next().expect("a pc"), 16).expect("hex pc");
            let digits = fields.next().expect("bytes");
            let value = u32::from_str_radix(digits, 16).expect("hex bytes");
            let bytes = value.to_le_bytes()[..digits.len() / 2].to_vec();
            Row {
                pc,
                bytes,
                text: fields.next().expect("text"),
            }
        })
        .collect()
}

/// Whether `objdump` itself left the row undecoded.
fn unknown_to_objdump(text: &str) -> bool {
    text.starts_with(".insn") || text.starts_with(".word") || text.starts_with(".short")
}

/// The rows this decoder is held to: 4-byte ones outside the floating-point
/// and vector opcodes.
fn held(row: &Row) -> bool {
    row.bytes.len() == 4
        && !matches!(
            row.bytes[0] & 0x7f,
            0x07 | 0x27 | 0x43 | 0x47 | 0x4b | 0x4f | 0x53 | 0x57
        )
}

/// The rows `decode` spells differently from `objdump`, and the ones it does
/// not decode, by mnemonic.
fn compare<'a>(rows: &[Row<'a>]) -> (Vec<String>, std::collections::BTreeMap<&'a str, usize>) {
    let mut differing = Vec::new();
    let mut undecoded = std::collections::BTreeMap::new();
    for row in rows.iter().filter(|row| !unknown_to_objdump(row.text)) {
        match decode(&row.bytes, row.pc) {
            Some(instruction)
                if instruction.text == row.text && instruction.length == row.bytes.len() => {}
            Some(instruction) => differing.push(format!(
                "{:x} {:02x?}: objdump '{}', decoder '{}' ({} bytes)",
                row.pc, row.bytes, row.text, instruction.text, instruction.length
            )),
            None => {
                let mnemonic = row.text.split('\t').next().unwrap_or(row.text);
                *undecoded.entry(mnemonic).or_insert(0) += 1;
            }
        }
    }
    (differing, undecoded)
}

#[test]
fn decodes_words_as_objdump_does() {
    let all = rows(include_str!("objdump_words.txt"));
    let rows = all.into_iter().filter(held).collect::<Vec<_>>();
    assert!(rows.len() > 300, "{} rows", rows.len());
    let (differing, undecoded) = compare(&rows);
    assert!(differing.is_empty(), "{}", differing.join("\n"));
    assert!(undecoded.is_empty(), "undecoded: {undecoded:?}");
}

/// Every row of the corpus `DECODE_CORPUS` names decodes to `objdump`'s
/// text; the rows left undecoded are counted by mnemonic.
#[test]
#[ignore = "needs a corpus written by tools/oracles/riscv64-decode/objdump-corpus.sh"]
fn decodes_a_corpus_as_objdump_does() {
    let path = std::env::var("DECODE_CORPUS").expect("DECODE_CORPUS names a corpus");
    let corpus = std::fs::read_to_string(&path).expect("a readable corpus");
    let rows = rows(&corpus);
    let (differing, undecoded) = compare(&rows);
    for line in differing.iter().take(50) {
        println!("{line}");
    }
    let undecoded_rows = undecoded.values().sum::<usize>();
    println!("undecoded by mnemonic: {undecoded:?}");
    println!(
        "{path}: {} rows, {} differing, {undecoded_rows} undecoded",
        rows.len(),
        differing.len()
    );
    assert!(differing.is_empty(), "{} rows differ", differing.len());
}

/// The bytes of a 4-byte instruction.
fn word(value: u32) -> [u8; 4] {
    value.to_le_bytes()
}

#[test]
fn flow_and_targets() {
    // Rows of the corpora: `beq t4,t1,1060` at 0x1058 (debug-glibc.out),
    // `jal 27400` at 0x28842 (libc.so.6), `j 1030` at 0x1040.
    let beq = decode(&word(0x006e8463), 0x1058).unwrap();
    assert_eq!((beq.flow, beq.target), (Flow::Branch, Some(0x1060)));
    let jal = decode(&word(0xbbffe0ef), 0x28842).unwrap();
    assert_eq!((jal.flow, jal.target), (Flow::Call, Some(0x27400)));
    let j = decode(&word(0xff1ff06f), 0x1040).unwrap();
    assert_eq!(
        (j.flow, j.target, j.text.as_str()),
        (Flow::Jump, Some(0x1030), "j\t1030")
    );
    // `jalr -260(ra)` at 0x195c, after `auipc ra,0xf` at 0x1958: through ra,
    // linking ra.
    let jalr = decode(&word(0xefc080e7), 0x195c).unwrap();
    assert_eq!(
        (jalr.flow, jalr.text.as_str()),
        (
            Flow::Indirect {
                register: 1,
                offset: -260,
                link: true
            },
            "jalr\t-260(ra)"
        )
    );
    // jr t3: through t3, linking nothing.
    let jr = decode(&word(0x000e0067), 0).unwrap();
    assert_eq!(
        jr.flow,
        Flow::Indirect {
            register: 28,
            offset: 0,
            link: false
        }
    );
    let ret = decode(&word(0x00008067), 0).unwrap();
    assert_eq!((ret.flow, ret.text.as_str()), (Flow::Return, "ret"));
    let ecall = decode(&word(0x00000073), 0).unwrap();
    assert_eq!((ecall.flow, ecall.text.as_str()), (Flow::Trap, "ecall"));
    // `lr.w a2,(t3)` (libc.so.6).
    let lr = decode(&word(0x100e262f), 0).unwrap();
    assert_eq!(
        (lr.flow, lr.text.as_str()),
        (Flow::AtomicStart, "lr.w\ta2,(t3)")
    );
    // The `auipc` forms 0x10958; the `jalr` adds -260 to it.
    let auipc = decode(&word(0x0000f097), 0x1958).unwrap();
    assert_eq!(auipc.page, Some((1, 0x10958)));
    assert_eq!(jalr.page_offset, Some((1, -260)));
    // li a1,3 sets 3.
    assert_eq!(decode(&word(0x00300593), 0).unwrap().value, Some(3));
}

#[test]
fn length_is_read_from_the_first_parcel() {
    assert_eq!(length(&[0x13, 0x01, 0x01, 0xf3]), 4);
    assert_eq!(length(&[0x41, 0x11]), 2);
    assert_eq!(length(&[0x1f, 0x00]), 6);
    assert_eq!(length(&[0x3f, 0x00]), 8);
    assert_eq!(length(&[]), 2);
    // Too few bytes for the whole instruction: nothing to decode.
    assert_eq!(decode(&[0x13, 0x01], 0), None);
}
