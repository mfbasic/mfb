#!/usr/bin/env python3
"""Words for lldb to disassemble, so the decoder is held to it beyond the sample.

    sweep.py flips <lldb_words.txt> <out.s>   every word within two flipped bits
                                              (bits 5..31) of a row of the sample
    sweep.py system <out.s>                   every system-class word (0xd5......
                                              bits 23:5), the branch-register
                                              class (0xd6/0xd7) and the exception
                                              class (0xd4)

The output is assembly (`.long` rows) that links to a host binary, which
`lldb-corpus.sh` then disassembles; `select.py` keeps one row per mnemonic and
operand shape. See README.md.
"""
import itertools
import sys


def write(path, words):
    words = sorted(set(words))
    with open(path, "w") as out:
        out.write(".text\n.globl _main\n_main:\n")
        for word in words:
            out.write(".long 0x%08x\n" % word)
    print(len(words), "words")


def flips(sample, path):
    seeds = []
    for line in open(sample):
        if line.startswith("#") or not line.strip():
            continue
        seeds.append(int(line.split("\t")[1], 16))
    bits = range(5, 32)
    words = set()
    for seed in seeds:
        words.add(seed)
        for bit in bits:
            words.add(seed ^ (1 << bit))
        for a, b in itertools.combinations(bits, 2):
            words.add(seed ^ (1 << a) ^ (1 << b))
    write(path, words)


def system(path):
    words = set()
    for body in range(1 << 19):
        for rt in (0, 1, 31):
            words.add(0xD5000000 | (body << 5) | rt)
    # Branch register / return: opc (24:21), op2 (20:16), op3 (15:10), Rn, op4.
    for body in range(1 << 15):
        for rn, op4 in ((0, 0), (31, 0), (1, 31), (31, 31), (9, 1)):
            words.add(0xD6000000 | (body << 10) | (rn << 5) | op4)
    # Exception generation: opc (23:21), imm16, op2 (4:2), LL (1:0).
    for opc in range(8):
        for low in range(32):
            for imm in (0, 1, 0x1234):
                words.add(0xD4000000 | (opc << 21) | (imm << 5) | low)
    write(path, words)


if __name__ == "__main__":
    if len(sys.argv) == 4 and sys.argv[1] == "flips":
        flips(sys.argv[2], sys.argv[3])
    elif len(sys.argv) == 3 and sys.argv[1] == "system":
        system(sys.argv[2])
    else:
        sys.exit(__doc__)
