"""Datetime-group benchmark (datetime:: package surface).

Mirrors benchmark/mfb/src/datetimeb.mfb: civil date arithmetic (addDays/addMonths
walk, daysInMonth, a between day-span) and an ISO format/parse round-trip. All
arithmetic is proleptic Gregorian (Python's datetime), matching mfb's civil math,
and the checksums fold only civil field values and day-differences so the mfb/C/
Python references agree exactly. addMonths clamps the day to the target month's
length, matching datetime::addMonths.
"""
import datetime as _dt
import sys

RUN = 1
now_ns = None
record = None


def _dim(y, m):
    if m == 2:
        return 29 if (y % 4 == 0 and y % 100 != 0) or y % 400 == 0 else 28
    return 30 if m in (4, 6, 9, 11) else 31


def _add_months(y, mo, day, months):
    total = y * 12 + (mo - 1) + months
    ny = total // 12
    nm = total % 12 + 1
    nd = min(day, _dim(ny, nm))
    return ny, nm, nd


def test_datetime_civil():
    days = 2000
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        d = _dt.date(2000, 1, 1)
        for _i in range(days):
            d = d + _dt.timedelta(days=1)
            acc += d.year + d.month + d.day
        for k in range(48):
            ny, nm, nd = _add_months(2000, 1, 31, k)
            acc += nd + nm
        for m in range(1, 13):
            acc += _dim(2000, m) + _dim(2100, m)
        span = (_dt.date(2020, 1, 1) - _dt.date(2000, 1, 1)).days
        acc += span
        checksum = acc
        times.append(now_ns() - t0)
    print("datetime_civil = %d" % checksum, file=sys.stderr)
    record("datetime", "civil", times)


def test_datetime_iso():
    # Arena-gated (plan-44-J): tiny reps; raise when the arena fix lands.
    reps = 8                       # TODO(plan-44-J): raise to 2000
    times = []
    checksum = 0
    for _ in range(RUN):
        t0 = now_ns()
        acc = 0
        dt = _dt.datetime(2023, 1, 1, 14, 30, 45)
        for _rep in range(reps):
            dt = dt + _dt.timedelta(days=1)
            iso = dt.isoformat()
            back = _dt.datetime.fromisoformat(iso)
            acc += back.year + back.month + back.day
            acc += back.hour + back.minute + back.second
        checksum = acc
        times.append(now_ns() - t0)
    print("datetime_iso = %d" % checksum, file=sys.stderr)
    record("datetime", "iso", times)


def run_all(run, now_ns_fn, record_fn):
    global RUN, now_ns, record
    RUN, now_ns, record = run, now_ns_fn, record_fn
    test_datetime_civil()
    test_datetime_iso()
