#!/usr/bin/env python3
"""Write an oracle job file to stdout, deterministically.

    .venv/bin/python corpus.py offsets > jobs/offsets.txt
    .venv/bin/python corpus.py civil   > jobs/civil.txt

The job INPUTS may come from the package's own table (which instants are its
transitions), but every ANSWER comes from zoneinfo in oracle.py. A wrong table
therefore still produces jobs whose answers catch it.

offsets — one `offset <name> <unixSeconds>` per line (plan-135-B section 4.4):
  1. every stored transition t of every distinct zone: t-1, t, t+1;
  2. for every name whose 2030 January and July offsets differ, every offset
     change from 2026-01-01 through 2100-12-31, found by a daily scan and
     bisected to the second: c-1, c, c+1;
  3. for every name, 00:00 UTC on 1 January and 1 July of every year 1800-2100;
  4. for every name, instants before its first transition: the earliest
     instant both sides represent, and the first stored transition - 86400.

civil — one `civil <name> Y M D h m s` per line (plan-135-C section 4.2):
  1. for every transition the offsets corpus uses (sections 1 and 2), with
     offsets oPrev -> oNew, the wall readings t+oPrev-1, t+oPrev, t+oNew-1,
     t+oNew, and the midpoint of the gap or overlap;
  2. 09:00 on the 15th of every month 2026-2030, for every name.

Everything is kept inside the years Python's datetime represents (1..9999).
"""

import os
import sys
import zoneinfo
from datetime import datetime, timedelta, timezone

zoneinfo.reset_tzpath([])
from zoneinfo import ZoneInfo  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
DATA = os.path.join(HERE, "..", "src", "data.mfb")
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)
# plan-135-B section 4.4 item 4 names -2^40, which is before year 1. The nearest
# instant Python can convert in every zone is two days into year 1.
MIN_SECONDS = int((datetime(1, 1, 3, tzinfo=timezone.utc) - EPOCH).total_seconds())
MAX_SECONDS = int((datetime(9999, 12, 29, tzinfo=timezone.utc) - EPOCH).total_seconds())


def load_groups():
    """[(names, transitions)] per distinct zone, in data.mfb order."""
    lines = open(DATA, encoding="ascii").read().split("\n")
    groups = []
    for i, line in enumerate(lines):
        if line.startswith("PRIVATE FUNC zone") and line.endswith("() AS String"):
            names = lines[i - 1][2:].split(", ")
            body = lines[i + 1].strip()
            assert body.startswith('RETURN "') and body.endswith('"'), body
            _, transitions, _ = body[len('RETURN "'):-1].split("|")
            times = [int(r.split(",")[0]) for r in transitions.split(";")] if transitions else []
            groups.append((names, times))
    return groups


def all_names(groups):
    return sorted(n for group, _ in groups for n in group)


def utc_seconds(*fields):
    return int((datetime(*fields, tzinfo=timezone.utc) - EPOCH).total_seconds())


def utoff(zone, seconds):
    return int((EPOCH + timedelta(seconds=seconds)).astimezone(zone).utcoffset().total_seconds())


def future_changes(name):
    zone = ZoneInfo(name)
    if utoff(zone, utc_seconds(2030, 1, 1)) == utoff(zone, utc_seconds(2030, 7, 1)):
        return []
    changes = []
    day = utc_seconds(2026, 1, 1)
    end = utc_seconds(2101, 1, 1)
    before = utoff(zone, day)
    while day < end:
        nxt = day + 86400
        after = utoff(zone, nxt)
        if after != before:
            low, high = day, nxt
            while high - low > 1:
                middle = (low + high) // 2
                if utoff(zone, middle) == before:
                    low = middle
                else:
                    high = middle
            changes.append(high)
        before = after
        day = nxt
    return changes


def used_transitions(groups):
    """(name, t) for offsets sections 1 and 2, in corpus order."""
    for group, times in groups:
        for t in times:
            yield group[0], t
    for name in all_names(groups):
        for c in future_changes(name):
            yield name, c


class Jobs:
    def __init__(self):
        self.lines = {}

    def add(self, line):
        self.lines[line] = None


def in_range(seconds):
    return MIN_SECONDS <= seconds <= MAX_SECONDS


def offsets_jobs():
    groups = load_groups()
    names = all_names(groups)
    jobs = Jobs()

    def add(name, seconds):
        if in_range(seconds):
            jobs.add("offset %s %d" % (name, seconds))

    for name, t in used_transitions(groups):
        for s in (t - 1, t, t + 1):
            add(name, s)
    for name in names:
        for year in range(1800, 2101):
            add(name, utc_seconds(year, 1, 1))
            add(name, utc_seconds(year, 7, 1))
    first = {n: times[0] for group, times in groups if times for n in group}
    for name in names:
        add(name, MIN_SECONDS)
        if name in first:
            add(name, first[name] - 86400)
    return list(jobs.lines)


def civil_jobs():
    groups = load_groups()
    names = all_names(groups)
    jobs = Jobs()
    zones = {}

    def add(name, wall):
        if in_range(wall):
            w = EPOCH + timedelta(seconds=wall)
            jobs.add("civil %s %d %d %d %d %d %d" % (name, w.year, w.month, w.day, w.hour, w.minute, w.second))

    for name, t in used_transitions(groups):
        zone = zones.setdefault(name, ZoneInfo(name))
        if not in_range(t - 1):
            continue
        previous = utoff(zone, t - 1)
        new = utoff(zone, t)
        for wall in (t + previous - 1, t + previous, t + new - 1, t + new, t + (previous + new) // 2):
            add(name, wall)
    for name in names:
        for year in range(2026, 2031):
            for month in range(1, 13):
                add(name, utc_seconds(year, month, 15, 9))
    return list(jobs.lines)


MODES = {"offsets": offsets_jobs, "civil": civil_jobs}


def main():
    if len(sys.argv) != 2 or sys.argv[1] not in MODES:
        sys.stderr.write("usage: corpus.py {%s}\n" % "|".join(MODES))
        sys.exit(2)
    out = sys.stdout
    for job in MODES[sys.argv[1]]():
        out.write(job + "\n")


if __name__ == "__main__":
    main()
