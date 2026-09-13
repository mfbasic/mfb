#!/usr/bin/env python3
"""Answer an oracle job file with Python zoneinfo, one answer line per job line.

    .venv/bin/python oracle.py jobs/offsets.txt > jobs/offsets.expected

The rules come from the `tzdata` wheel pinned in requirements.txt, never from
the host: the search path is emptied before the first lookup, and the wheel's
release must equal the one tools/tzdb/gen_timezones_data.py vendors.

  offset <name> <s>          ->  <utoff> <abbreviation>
  civil <name> Y M D h m s   ->  <utcSeconds> <utoff> <abbreviation> <wall>

A civil reading is resolved with fold=0, the RFC 9557 "compatible" rule. Its
utoff, abbreviation and wall (YYYY-MM-DDTHH:MM:SS) are those of the resulting
instant, so a skipped reading reports the later wall clock it became.
"""

import os
import re
import sys
import zoneinfo
from datetime import datetime, timedelta, timezone

zoneinfo.reset_tzpath([])
from zoneinfo import ZoneInfo  # noqa: E402

import tzdata  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
GENERATOR = os.path.join(HERE, "..", "..", "..", "tools", "tzdb", "gen_timezones_data.py")
EPOCH = datetime(1970, 1, 1, tzinfo=timezone.utc)


def release():
    text = open(GENERATOR, encoding="utf-8").read()
    return re.search(r'^RELEASE = "([^"]+)"$', text, re.M).group(1)


class Oracle:
    def __init__(self):
        self.zones = {}

    def zone(self, name):
        zone = self.zones.get(name)
        if zone is None:
            zone = self.zones[name] = ZoneInfo(name)
        return zone

    def offset(self, name, seconds):
        local = (EPOCH + timedelta(seconds=seconds)).astimezone(self.zone(name))
        return "%d %s" % (int(local.utcoffset().total_seconds()), local.tzname())

    def civil(self, name, fields):
        zone = self.zone(name)
        reading = datetime(*fields, tzinfo=zone, fold=0)
        seconds = (reading - EPOCH) // timedelta(seconds=1)
        local = (EPOCH + timedelta(seconds=seconds)).astimezone(zone)
        return "%d %d %s %04d-%02d-%02dT%02d:%02d:%02d" % (
            seconds, int(local.utcoffset().total_seconds()), local.tzname(),
            local.year, local.month, local.day, local.hour, local.minute, local.second)


def main():
    if len(sys.argv) != 2:
        sys.stderr.write("usage: oracle.py <jobs.txt>\n")
        sys.exit(2)
    if tzdata.IANA_VERSION != release():
        sys.stderr.write("oracle: tzdata is %s but the package vendors %s; bump requirements.txt\n"
                         % (tzdata.IANA_VERSION, release()))
        sys.exit(2)
    oracle = Oracle()
    out = []
    with open(sys.argv[1], encoding="ascii") as jobs:
        for line in jobs:
            fields = line.split()
            if fields[0] == "offset":
                out.append(oracle.offset(fields[1], int(fields[2])))
            elif fields[0] == "civil":
                out.append(oracle.civil(fields[1], [int(v) for v in fields[2:8]]))
            else:
                sys.stderr.write("oracle: unknown job %r\n" % line)
                sys.exit(2)
    sys.stdout.write("\n".join(out) + ("\n" if out else ""))


if __name__ == "__main__":
    main()
