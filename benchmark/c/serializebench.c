/* GROUP: serialize (json::/csv:: encode direction) — plan-65 Theme 2.
 *
 * Mirrors benchmark/mfb/src/serialize.mfb: json stringify, json parse->stringify
 * round-trip, and csv stringify. The checksum is the *length* of the emitted text
 * (accumulated over the reps) — order-independent, so it matches the mfb and
 * Python columns even though json object members may be emitted in a different
 * order. JSON serialization reuses the vendored parson (json_serialize_to_string
 * is compact); CSV is hand-rolled to mfb's csv::stringify rules (quote a field iff
 * it holds a comma / quote / CR / LF; double interior quotes; join fields with
 * ',', rows with LF, no trailing newline).
 *
 * These rows are arena-gated in mfb (plan-64-A, per-call String building), so the
 * reps are tiny; the C mirror keeps the same counts so the table lines up. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "parson.h"
#include "serializebench.h"

/* Canonical compact JSON: ASCII strings + integers only, no '/', no fractional
 * numbers, so every compact serializer emits the same length (133). */
static const char *JSON_TEXT =
    "{\"name\":\"benchmark\",\"version\":3,\"tags\":[\"alpha\",\"beta\",\"gamma\"],"
    "\"nested\":{\"count\":42,\"label\":\"node\",\"values\":[1,2,3,4,5]},"
    "\"active\":1}";

/* A grid exercising the quote-when-needed path: one comma cell, one quote cell. */
static const char *GRID[4][3] = {
    {"id", "name", "note"},
    {"1", "Grace", "Hop,per"},
    {"2", "Ada", "plain"},
    {"3", "Alan", "say\"hi"}};

static void test_serialize_json(void) {
  JSON_Value *tree = json_parse_string(JSON_TEXT);
  int reps = 200;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) {
      char *s = json_serialize_to_string(tree);
      acc += (long)strlen(s);
      json_free_serialized_string(s);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  json_value_free(tree);
  fprintf(stderr, "serialize_json = %ld\n", checksum);
  record("serialize", "json", t, RUN);
  free(t);
}

static void test_serialize_roundtrip(void) {
  int reps = 100;
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) {
      JSON_Value *tree = json_parse_string(JSON_TEXT);
      char *s = json_serialize_to_string(tree);
      acc += (long)strlen(s);
      json_free_serialized_string(s);
      json_value_free(tree);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "serialize_roundtrip = %ld\n", checksum);
  record("serialize", "roundtrip", t, RUN);
  free(t);
}

/* Append `field` to `out` (at *len), quoting per RFC 4180 / mfb rules. */
static void csv_field(char *out, size_t *len, const char *field) {
  int needs_quote = 0;
  for (const char *p = field; *p; p++)
    if (*p == ',' || *p == '"' || *p == '\r' || *p == '\n') { needs_quote = 1; break; }
  if (!needs_quote) {
    size_t n = strlen(field);
    memcpy(out + *len, field, n);
    *len += n;
    return;
  }
  out[(*len)++] = '"';
  for (const char *p = field; *p; p++) {
    if (*p == '"') out[(*len)++] = '"'; /* double interior quotes */
    out[(*len)++] = *p;
  }
  out[(*len)++] = '"';
}

/* Stringify the 4x3 GRID to `out` (mfb csv::stringify format); returns length. */
static size_t csv_stringify(char *out) {
  size_t len = 0;
  for (int r = 0; r < 4; r++) {
    if (r > 0) out[len++] = '\n';
    for (int c = 0; c < 3; c++) {
      if (c > 0) out[len++] = ',';
      csv_field(out, &len, GRID[r][c]);
    }
  }
  out[len] = '\0';
  return len;
}

static void test_serialize_csv(void) {
  int reps = 200;
  char buf[256];
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int rep = 0; rep < reps; rep++) acc += (long)csv_stringify(buf);
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "serialize_csv = %ld\n", checksum);
  record("serialize", "csv", t, RUN);
  free(t);
}

void run_serialize_group(void) {
  test_serialize_json();
  test_serialize_roundtrip();
  test_serialize_csv();
}
