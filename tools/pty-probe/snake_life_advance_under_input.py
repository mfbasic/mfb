#!/usr/bin/env python3
"""bug-703 regression probe: snake/life must advance on their fixed cadence
regardless of whether a key arrived that tick, and snake must never let two
same-tick turns compose into a reversal.

Three cases, each built and driven over a real pty (term::on() needs one):

1. snake "hold": hold 'd' (already-current direction, a no-op turn) for 1.5s
   at a 15ms send interval. Before the fix this starved stepSnake completely
   (the outer IF/ELSEIF only stepped when no key arrived); the head must keep
   moving throughout.
2. snake "reverse": send 'w' then 'a' ~10ms apart near the start. Before the
   fix, directionFor validated the second key against the first key's
   not-yet-applied dir, so RIGHT -> UP -> LEFT could compose into a net
   reversal and end the game (self-collision) within ~300ms. After the fix,
   the pair must not end the game that fast.
3. life "hold": hold the unmapped key 'z' for 1.5s at a 15ms interval; the
   'Gen' counter in the status line must keep advancing throughout.

Usage: python3 tools/pty-probe/snake_life_advance_under_input.py
Exits 0 and prints PASS lines on success; exits 1 with a diagnostic on the
first failing case.
"""
import os
import re
import subprocess
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from pty_probe import spawn, render, find_glyph  # noqa: E402

REPO_ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
MFB = os.path.join(REPO_ROOT, "target", "release", "mfb")
SNAKE_BIN = os.path.join(REPO_ROOT, "examples", "snake", "build", "snake.out")
LIFE_BIN = os.path.join(REPO_ROOT, "examples", "life", "build", "life.out")

HEAD_GLYPH = "☺"  # U+263A, snake's head glyph
GEN_RE = re.compile(r"Gen (\d+)")
COLS = 100


def build(example: str) -> None:
    subprocess.run([MFB, "build", os.path.join(REPO_ROOT, "examples", example)],
                    check=True, cwd=REPO_ROOT)


def find_head(raw: bytes):
    return find_glyph(render(raw, cols=COLS), HEAD_GLYPH)


def find_game_over(raw: bytes) -> bool:
    return b"GAME OVER" in raw


def find_gen(raw: bytes):
    grid = render(raw, cols=COLS)
    # Read the whole bottom-most drawn row's text and pull out the last "Gen N".
    if not grid:
        return None
    max_row = max(r for r, _ in grid.keys())
    line = "".join(grid.get((max_row, c), " ") for c in range(COLS))
    m = GEN_RE.search(line)
    return int(m.group(1)) if m else None


def max_gap(timestamps, window_start: float, window_end: float) -> float:
    """The largest gap between consecutive advances (each `timestamps` entry a
    seconds-since-start float), restricted to gaps that overlap
    [window_start, window_end] — i.e. the longest stretch of *no* advance
    while input was being sent continuously. This is the freeze bug's actual
    signature (a single long stall spanning the whole hold), which a plain
    "count of advances in the window" cannot distinguish from "advancing on
    a slower but still-moving cadence" (e.g. under host scheduling
    contention) — a run that is merely slow still has small, bounded gaps.
    """
    points = [window_start] + [t for t in timestamps if window_start <= t <= window_end] + [window_end]
    return max(b - a for a, b in zip(points, points[1:])) if len(points) > 1 else window_end - window_start


def snake_hold_case() -> None:
    session = spawn(SNAKE_BIN, rows=40, cols=100)
    try:
        t0 = time.monotonic()
        send_until = t0 + 1.5
        deadline = send_until + 1.2
        next_send = t0
        positions = []  # (t, pos)
        last_pos = None
        while True:
            now = time.monotonic()
            if now > deadline:
                break
            chunk = session.read_available(0.02)
            if chunk:
                pos = find_head(session.raw) or last_pos
                if pos != last_pos:
                    positions.append((now - t0, pos))
                    last_pos = pos
            if now < send_until and now >= next_send:
                session.send(b"d")
                next_send = now + 0.015
        gap = max_gap([t for t, _ in positions], 0.35, 1.5)
        # A generous multiple of frameDelayMs() (95ms) to tolerate host
        # scheduling jitter (this machine runs several parallel build/test
        # jobs); the freeze bug produces a gap on the order of the whole
        # 1.15s hold window, an order of magnitude past this bound.
        limit = 6 * 0.095
        if gap > limit:
            raise AssertionError(
                "snake hold: head did not move for "
                f"{gap:.3f}s while 'd' was held continuously (limit "
                f"{limit:.3f}s) — positions: {positions}")
        print(f"PASS: snake hold — longest gap without a head move during a "
              f"continuous 1.5s key hold was {gap:.3f}s (limit {limit:.3f}s)")
    finally:
        session.close()


def snake_reverse_case() -> None:
    session = spawn(SNAKE_BIN, rows=40, cols=100)
    try:
        t0 = time.monotonic()
        # A window short enough to catch the bug's near-instant self-collision
        # (documented at ~300ms after the second keystroke) but well short of
        # the unrelated, legitimate wall collision the snake will eventually
        # hit ~1.7s later just by continuing to move up after a correctly
        # accepted single turn (~18 rows at frameDelayMs()=95ms/step) — that
        # later collision is expected, correct behavior, not the bug.
        end_at = t0 + 1.0
        sent = False
        next_send = t0 + 0.3
        game_over = False
        while True:
            now = time.monotonic()
            if now > end_at:
                break
            chunk = session.read_available(0.02)
            if chunk and find_game_over(session.raw):
                game_over = True
                break
            if not sent and now >= next_send:
                session.send(b"w")
                time.sleep(0.010)
                session.send(b"a")
                sent = True
        if game_over:
            raise AssertionError(
                "snake reverse: two same-tick turns (w then a) ended the "
                "game within 1s — directionFor validated the second turn "
                "against the first's not-yet-applied direction instead of "
                "the last actually-moved direction")
        print("PASS: snake reverse — two same-tick turns did not end the game "
              "within 1s of being sent")
    finally:
        session.close()


def life_hold_case() -> None:
    session = spawn(LIFE_BIN, rows=40, cols=100)
    try:
        t0 = time.monotonic()
        send_until = t0 + 1.5
        deadline = send_until + 1.2
        next_send = t0
        gens = []  # (t, gen)
        last_gen = None
        while True:
            now = time.monotonic()
            if now > deadline:
                break
            chunk = session.read_available(0.02)
            if chunk:
                gen = find_gen(session.raw)
                if gen is not None and gen != last_gen:
                    gens.append((now - t0, gen))
                    last_gen = gen
            if now < send_until and now >= next_send:
                session.send(b"z")  # unmapped key
                next_send = now + 0.015
        gap = max_gap([t for t, _ in gens], 0.35, 1.5)
        limit = 6 * 0.090  # frameDelayMs() for life is 90ms
        if gap > limit:
            raise AssertionError(
                "life hold: 'Gen' did not advance for "
                f"{gap:.3f}s while an unmapped key ('z') was held "
                f"continuously (limit {limit:.3f}s) — advances: {gens}")
        print(f"PASS: life hold — longest gap without a 'Gen' advance during "
              f"a continuous 1.5s key hold was {gap:.3f}s (limit {limit:.3f}s)")
    finally:
        session.close()


def main() -> int:
    build("snake")
    build("life")
    cases = [snake_hold_case, snake_reverse_case, life_hold_case]
    for case in cases:
        try:
            case()
        except AssertionError as exc:
            print(f"FAIL: {exc}")
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
