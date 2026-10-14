#!/usr/bin/env python3
"""Reduce one row's MFB_CANVAS_STATS file to its log block and its checksum lines.

    reduce.py ROW STATS DUMP VSYNC LOG SUMS

ROW is the row's name ("rect fill 100"), STATS the stats file of its timed run, DUMP
the last frame's RGBA from its dump run, VSYNC "on" or "off". The log block is appended
to LOG and the checksum lines to SUMS. Exits non-zero, naming the row, when a frame was
not drawn by Metal straight to the window, or when the layer's display sync is not what
VSYNC says: such a row measured something else, and its numbers must not be kept.

Every `prof…` field is cumulative, so a frame is one line minus the line before. Frame 1
is dropped: it compiles the pipeline and fills every cache. The walk column is the
graphics thread's scene walk and draw list (`phaseOffsetsMs` + `phaseDrawsMs`); those
phases run before the render, so a line includes its own frame's, as it does its
`prof…` fields. They are whole milliseconds on the line, so one frame's walk is only
resolved to a millisecond; the average over the frames is the number to read.
"""

import hashlib
import re
import statistics
import sys

# (column, fields summed into it, scale to microseconds)
COLUMNS = [
    ("vsync", ["profAcquireUs"], 1),
    ("fill", ["profFillUs"], 1),
    ("encode", ["profEncodeUs"], 1),
    ("wait", ["profWaitUs"], 1),
    ("gpu", ["profGpuUs"], 1),
    ("present", ["profPresentUs"], 1),
    ("work", ["profFillUs", "profEncodeUs", "profWaitUs", "profPresentUs"], 1),
    ("walk", ["phaseOffsetsMs", "phaseDrawsMs"], 1000),
]
# Per-frame counts. Constant for a row, so they are its checksum, not its timing.
COUNTS = [("draws", "profDraws"), ("bytes", "profBytes"), ("texels", "profTexels"),
          ("passes", "profPasses")]


def main():
    row, stats_path, dump_path, vsync, log_path, sums_path = sys.argv[1:7]
    lines = [dict(re.findall(r"(\w+)=(-?[\d.]+)", l)) for l in open(stats_path)]
    if len(lines) < 4:
        sys.exit("%s: %d stats lines, need at least 4" % (row, len(lines)))
    last = lines[-1]
    frames = int(last["frames"])
    for field in ("gpuFrames", "directFrames"):
        if int(last[field]) != frames:
            sys.exit("%s: %s=%s of frames=%d -- not every frame went straight to the "
                     "window on Metal" % (row, field, last[field], frames))
    want = "1" if vsync == "off" else "0"
    if last.get("profVsyncOff") != want:
        sys.exit("%s: profVsyncOff=%s, but this run asked for vsync %s"
                 % (row, last.get("profVsyncOff"), vsync))

    def delta(i, fields, scale):
        return sum(float(lines[i][f]) - float(lines[i - 1][f]) for f in fields) * scale

    key = re.sub(r"\W+", "_", row)
    with open(log_path, "a") as log:
        log.write("%s:\n" % row)
        for name, fields, scale in COLUMNS:
            v = [delta(i, fields, scale) for i in range(2, len(lines))]
            log.write("  %-8s : %10.1f, %10.1f, %10.1f, %10.1f\n"
                      % (name, statistics.median(v), statistics.fmean(v), min(v), max(v)))
        log.write("\n")
    with open(sums_path, "a") as sums:
        for name, field in COUNTS:
            v = {int(lines[i][field]) - int(lines[i - 1][field]) for i in range(2, len(lines))}
            value = str(v.pop()) if len(v) == 1 else "varies:%d-%d" % (min(v), max(v))
            sums.write("%s_%s = %s\n" % (key, name, value))
        with open(dump_path, "rb") as dump:
            sums.write("%s_pixels = %s\n" % (key, hashlib.sha256(dump.read()).hexdigest()[:16]))


if __name__ == "__main__":
    main()
