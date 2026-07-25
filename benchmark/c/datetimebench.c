/* GROUP: datetime (datetime:: package coverage)
 *
 * Mirrors benchmark/mfb/src/datetimeb.mfb: civil date arithmetic and an ISO
 * format/parse round-trip. Uses Howard Hinnant's days<->civil algorithm — the
 * same proleptic-Gregorian math datetime::daysFromCivil/civilFromDays implement —
 * so the (year, month, day) sequences and day-spans match mfb and Python exactly.
 * addMonths clamps the day to the target month's length, matching
 * datetime::addMonths. The checksums fold only civil field values and day counts. */
#include <stdio.h>
#include <stdlib.h>

#include "bench.h"
#include "datetimebench.h"

static long days_from_civil(long y, int m, int d) {
  y -= m <= 2;
  long era = (y >= 0 ? y : y - 399) / 400;
  long yoe = y - era * 400;
  long doy = (153 * (m + (m > 2 ? -3 : 9)) + 2) / 5 + d - 1;
  long doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
  return era * 146097 + doe - 719468;
}

static void civil_from_days(long z, long *yy, int *mm, int *dd) {
  z += 719468;
  long era = (z >= 0 ? z : z - 146096) / 146097;
  long doe = z - era * 146097;
  long yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
  long y = yoe + era * 400;
  long doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  long mp = (5 * doy + 2) / 153;
  long d = doy - (153 * mp + 2) / 5 + 1;
  long m = mp < 10 ? mp + 3 : mp - 9;
  *yy = y + (m <= 2);
  *mm = (int)m;
  *dd = (int)d;
}

static int is_leap(long y) { return (y % 4 == 0 && y % 100 != 0) || y % 400 == 0; }

static int days_in_month(long y, int m) {
  if (m == 2) return is_leap(y) ? 29 : 28;
  return (m == 4 || m == 6 || m == 9 || m == 11) ? 30 : 31;
}

static void test_datetime_civil(void) {
  int days = 2000;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    long dn = days_from_civil(2000, 1, 1);
    for (int i = 0; i < days; i++) {
      dn++;
      long y;
      int m, d;
      civil_from_days(dn, &y, &m, &d);
      acc += y + m + d;
    }
    for (int k = 0; k < 48; k++) {
      long total = 2000L * 12 + 0 + k;
      long ny = total / 12;
      int nm = (int)(total % 12) + 1;
      int day = 31;
      int dim = days_in_month(ny, nm);
      if (day > dim) day = dim;
      acc += day + nm;
    }
    for (int m = 1; m <= 12; m++) acc += days_in_month(2000, m) + days_in_month(2100, m);
    long span = days_from_civil(2020, 1, 1) - days_from_civil(2000, 1, 1);
    acc += span;
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "datetime_civil = %ld\n", checksum);
  record("datetime", "civil", t, RUN);
  free(t);
}

static void test_datetime_iso(void) {
  int reps = 8; /* arena-gated in mfb (plan-44-J); C mirror keeps the count */
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    long dn = days_from_civil(2023, 1, 1);
    int H = 14, M = 30, S = 45;
    for (int rep = 0; rep < reps; rep++) {
      dn++;
      long y;
      int m, d;
      civil_from_days(dn, &y, &m, &d);
      char iso[32];
      snprintf(iso, sizeof iso, "%04ld-%02d-%02dT%02d:%02d:%02d", y, m, d, H, M, S);
      int py, pm, pd, pH, pM, pS;
      sscanf(iso, "%d-%d-%dT%d:%d:%d", &py, &pm, &pd, &pH, &pM, &pS);
      acc += py + pm + pd + pH + pM + pS;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "datetime_iso = %ld\n", checksum);
  record("datetime", "iso", t, RUN);
  free(t);
}

void run_datetime_group(void) {
  test_datetime_civil();
  test_datetime_iso();
}
