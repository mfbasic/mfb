"""Regex benchmarks (group `regexbench`).

Mirrors benchmark/mfb/src/regexbench.mfb: compile-once/match-many, capture-group
rewriting, alternation find-all, and pattern-driven replace. All inputs are ASCII
so the match counts / result lengths are identical across mfb / C (POSIX) /
Python. Python compiles once (mfb re-compiles per call) -- that gap is part of
what the row measures, but the checksums are unaffected.
"""
import re
import sys

RUN = 1
now_ns = None
record = None


def test_regex_compile():
    lines = ["row" + str(i) + " val " + str(i * 7) for i in range(25)]
    pat = re.compile(r"[0-9]+")
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        total = 0
        for line in lines:
            total += len(pat.findall(line))
        checksum = total
        times.append(now_ns() - t0)
    print("regex_compile = %d" % checksum, file=sys.stderr)
    record("regexbench", "compile", times)


def test_regex_capture():
    toks = [str(2000 + i) + "-" + str(i % 12) + "-" + str(i % 28) for i in range(70)]
    text = " ".join(toks)
    pat = re.compile(r"([0-9]+)-([0-9]+)-([0-9]+)")
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        out = pat.sub(r"\1\2\3", text)
        checksum = len(out)
        times.append(now_ns() - t0)
    print("regex_capture = %d" % checksum, file=sys.stderr)
    record("regexbench", "capture", times)


def test_regex_alternation():
    frags = ["the cat and dog saw a bird near fish and owl" for _ in range(30)]
    text = " ".join(frags)
    pat = re.compile("cat|dog|bird|fish|owl")
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        checksum = len(pat.findall(text))
        times.append(now_ns() - t0)
    print("regex_alternation = %d" % checksum, file=sys.stderr)
    record("regexbench", "alternation", times)


def test_regex_replace():
    toks = [str(i) + "-x" for i in range(300)]
    text = " ".join(toks)
    pat = re.compile(r"[0-9]+")
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        out = pat.sub("#", text)
        checksum = len(out)
        times.append(now_ns() - t0)
    print("regex_replace = %d" % checksum, file=sys.stderr)
    record("regexbench", "replace", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_regex_compile()
    test_regex_capture()
    test_regex_alternation()
    test_regex_replace()
