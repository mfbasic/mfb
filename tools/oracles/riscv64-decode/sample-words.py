#!/usr/bin/env python3
"""Pick the rows of objdump corpora that `objdump_words.txt` keeps.

    sample-words.py <corpus>... > src/arch/riscv64/decode/objdump_words.txt

A row is kept when it is one of the first three operand shapes seen for its
encoding class: the mnemonic, the instruction's length, and its opcode bits (the
quadrant and funct3 of a 2-byte instruction, with the bits that pick a CA or CB
form; the opcode and funct3 of a 4-byte one). Compressed instructions print as
the base instruction they expand to, so one mnemonic (`addi`) stands for several
encodings (`c.addi`, `c.addi16sp`, `c.addi4spn`, `addi`); the class keeps each.
An operand shape spells registers by kind (`sp`, `zero`, `ra`, a register a
compressed form can name, any other) and numbers by sign.
"""
import re
import sys

COMPRESSED_X = {'s0', 's1', 'a0', 'a1', 'a2', 'a3', 'a4', 'a5'}
COMPRESSED_F = {'fs0', 'fs1', 'fa0', 'fa1', 'fa2', 'fa3', 'fa4', 'fa5'}
SPECIAL = {'sp', 'zero', 'ra', 'gp', 'tp'}


def token(operand):
    if operand in SPECIAL:
        return operand
    if operand in COMPRESSED_X:
        return 'cx'
    if operand in COMPRESSED_F:
        return 'cf'
    if re.fullmatch(r'(t[0-6]|s[0-9]|s1[01]|a[0-7])', operand):
        return 'x'
    if re.fullmatch(r'f(t[0-9]|t1[01]|s[0-9]|s1[01]|a[0-7])', operand):
        return 'f'
    if re.fullmatch(r'-?(0x)?[0-9a-f]+', operand):
        value = int(operand, 16) if '0x' in operand else int(operand, 10) if re.fullmatch(r'-?[0-9]+', operand) else None
        if value is None:
            return 'n'
        return 'neg' if value < 0 else 'zero#' if value == 0 else 'pos'
    return operand


def shape(text):
    parts = text.split('\t', 1)
    if len(parts) == 1:
        return ''
    operands = re.split(r'([,()])', parts[1])
    return ''.join(token(piece) if piece not in ',()' else piece for piece in operands)


def encoding_class(bytes_hex):
    value = int(bytes_hex, 16)
    if len(bytes_hex) == 4:
        quadrant = value & 3
        funct3 = value >> 13
        # CA and CB arithmetic (quadrant 1, funct3 100) and quadrant 2's
        # funct3 100 split further on bits 12, 10-11 and 5-6.
        extra = (value >> 10) & 0x1f if funct3 == 4 else 0
        if funct3 == 4 and quadrant == 1:
            extra = ((value >> 10) & 7, (value >> 5) & 3)
        return (2, quadrant, funct3, extra)
    return (4, value & 0x7f, (value >> 12) & 7)


def main():
    seen = {}
    rows = []
    for path in sys.argv[1:]:
        for line in open(path):
            pc, bytes_hex, text = line.rstrip('\n').split('\t', 2)
            mnemonic = text.split('\t')[0]
            key = (mnemonic, encoding_class(bytes_hex))
            shapes = seen.setdefault(key, set())
            form = shape(text)
            if form in shapes or len(shapes) >= 3:
                continue
            shapes.add(form)
            rows.append((pc, bytes_hex, text))
    print('# riscv64 instructions and the text GNU objdump prints for them, one row')
    print('# per encoding class and operand shape (up to three each):')
    print('# `<pc>\\t<bytes>\\t<text>`, <bytes> as objdump prints them (8 or 4 hex')
    print('# digits). Taken from the corpora of tools/oracles/riscv64-decode by')
    print('# sample-words.py; `decodes_words_as_objdump_does` holds the decoder to')
    print('# every row.')
    for pc, bytes_hex, text in rows:
        print(f'{pc}\t{bytes_hex}\t{text}')


main()
