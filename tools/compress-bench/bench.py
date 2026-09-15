#!/usr/bin/env python3
"""Throughput of the `compress` builtin beside Python's zlib on the same bytes.

Usage: bench.py <mfb> [op ...]          (no ops = every op)
Env:   OPT_LEVELS  optimization levels to build at (default "1 3")
       ROUNDS      interleaved rounds per row (default 3)

Every corpus file is generated before anything is timed. Timing then runs in ROUNDS
interleaved rounds, each visiting every (op, kind, size, level) row once, so a burst
of load on a shared host lands on all rows instead of on whichever row happened to run
during it. In each visit the MFB program (mfb/) runs the op five times and reports each
run's `datetime::monotonicNanos` delta around the op alone, and Python runs the op five
times in-process; a row's time is the median over all its samples. The MFB result must
equal Python's, or the row is a failure: a fast wrong answer is not a measurement.

Linear scaling is checked per op, kind and level: the 16 MiB median must be at most
4.4x the 4 MiB median. Exit 0 all rows correct and linear, 1 otherwise, 2 harness
failure.
"""

import os
import random
import shutil
import statistics
import subprocess
import sys
import tempfile
import time
import zlib

HERE = os.path.dirname(os.path.abspath(__file__))
MIB = 1024 * 1024
SIZES = [1, 4, 16]
KINDS = ["random", "text", "zero"]
RUNS = 5
LINEAR_LIMIT = 4.4


def python_crc32(data):
    return str(zlib.crc32(data))


OPS = {
    "crc32": python_crc32,
}


def corpus(kind, size):
    n = size * MIB
    if kind == "random":
        return random.Random(f"137-{size}").randbytes(n)
    if kind == "text":
        out = bytearray()
        i = 0
        while len(out) < n:
            out += f"{i:08d} the quick brown fox jumps over the lazy dog, record {i * 7 % 1000}\n".encode()
            i += 1
        return bytes(out[:n])
    if kind == "zero":
        return bytes(n)
    raise ValueError(kind)


def die(message):
    print(message, file=sys.stderr)
    sys.exit(2)


def build(mfb, level, work):
    project = os.path.join(work, f"bench-O{level}")
    shutil.copytree(os.path.join(HERE, "mfb"), project, ignore=shutil.ignore_patterns("build"))
    result = subprocess.run([mfb, "build", "-q", f"-O{level}", project], capture_output=True, text=True)
    if result.returncode != 0:
        die(f"mfb build -O{level} failed:\n{result.stdout}{result.stderr}")
    for line in result.stdout.splitlines():
        if line.startswith("Wrote executable to "):
            exe = line[len("Wrote executable to "):]
            if subprocess.run([exe], capture_output=True).returncode == 0:
                return exe
    die(f"no built flavor of -O{level} runs on this host:\n{result.stdout}")


def run_mfb(exe, op, path):
    """Five timed runs in one process: (seconds per run, result)."""
    result = subprocess.run([exe], capture_output=True, text=True, env={**os.environ, "BENCH_FILE": path, "BENCH_OP": op})
    if result.returncode != 0:
        die(f"{exe} failed on {op} {path}:\n{result.stderr}")
    runs = [line.split(" ", 2) for line in result.stdout.splitlines() if line.startswith("run ")]
    if len(runs) != RUNS:
        die(f"{exe} printed {len(runs)} run(s) for {op} {path}, expected {RUNS}")
    results = {r[2] for r in runs}
    if len(results) != 1:
        die(f"{exe} gave different results across runs for {op} {path}: {results}")
    return [int(r[1]) / 1e9 for r in runs], results.pop()


def run_python(fn, data):
    """Five timed in-process calls: (seconds per call, result)."""
    times = []
    for _ in range(RUNS):
        started = time.perf_counter()
        value = fn(data)
        times.append(time.perf_counter() - started)
    return times, value


def main():
    if len(sys.argv) < 2:
        die("usage: bench.py <mfb> [op ...]")
    mfb = sys.argv[1]
    ops = sys.argv[2:] or list(OPS)
    for op in ops:
        if op not in OPS:
            die(f"unknown op {op} (ops: {' '.join(OPS)})")
    levels = os.environ.get("OPT_LEVELS", "1 3").split()
    rounds = int(os.environ.get("ROUNDS", "3"))

    failures = 0
    with tempfile.TemporaryDirectory(prefix="compress-bench-") as work:
        exes = {level: build(mfb, level, work) for level in levels}
        paths = {}
        for kind in KINDS:
            for size in SIZES:
                paths[(kind, size)] = os.path.join(work, f"{kind}-{size}.bin")
                with open(paths[(kind, size)], "wb") as f:
                    f.write(corpus(kind, size))

        mfb_times, py_times, mismatches = {}, {}, {}
        for _ in range(rounds):
            for op in ops:
                for kind in KINDS:
                    for size in SIZES:
                        with open(paths[(kind, size)], "rb") as f:
                            data = f.read()
                        times, py_value = run_python(OPS[op], data)
                        py_times.setdefault((op, kind, size), []).extend(times)
                        del data
                        for level in levels:
                            key = (op, kind, size, level)
                            times, mfb_value = run_mfb(exes[level], op, paths[(kind, size)])
                            mfb_times.setdefault(key, []).extend(times)
                            if mfb_value != py_value:
                                mismatches[key] = f"MISMATCH mfb={mfb_value} py={py_value}"

        print(f"python zlib {zlib.ZLIB_RUNTIME_VERSION}; median of {rounds} interleaved rounds x {RUNS} runs;"
              " MFB timed in-process around the op")
        print(f"{'op':<8} {'corpus':<7} {'MiB':>3} {'-O':>2} {'mfb ms':>10} {'mfb MiB/s':>10} {'py ms':>9} {'py MiB/s':>9}  check")
        for op in ops:
            for kind in KINDS:
                for size in SIZES:
                    py_s = statistics.median(py_times[(op, kind, size)])
                    for level in levels:
                        key = (op, kind, size, level)
                        mfb_s = statistics.median(mfb_times[key])
                        check = mismatches.get(key, "ok")
                        failures += check != "ok"
                        print(f"{op:<8} {kind:<7} {size:>3} {level:>2} {mfb_s * 1e3:>10.2f} {size / mfb_s:>10.1f}"
                              f" {py_s * 1e3:>9.2f} {size / py_s:>9.1f}  {check}")
            for kind in KINDS:
                for level in levels:
                    ratio = statistics.median(mfb_times[(op, kind, 16, level)]) / statistics.median(mfb_times[(op, kind, 4, level)])
                    linear = ratio <= LINEAR_LIMIT
                    failures += not linear
                    print(f"linear {op} {kind} -O{level}: 16 MiB / 4 MiB = {ratio:.2f} ({'ok' if linear else f'FAIL > {LINEAR_LIMIT}'})")
    sys.exit(1 if failures else 0)


if __name__ == "__main__":
    main()
