/* GROUP: parse — the C oracle for the csv/json/regex parse benchmarks.
 *
 * Mirrors the mfb (`csv::parse` / `json::parse`+`stringify` / `regex::findAll`)
 * and Python (`csv.reader` / `json.loads`+`dumps` / `re.findall`) workloads so
 * the three columns line up. C has no standard-library CSV or JSON parser, so
 * this file vendors two widely-used single-purpose libraries:
 *
 *   - JSON: parson  (MIT, parson.c/parson.h) — full DOM parse + navigate +
 *           serialize, matching json.loads(...)["tail"] then json.dumps(...).
 *   - CSV:  libcsv  (LGPL-2.1, libcsv.c/csv.h) — RFC-4180 streaming parser; the
 *           callbacks materialize a full grid of strdup'd fields so the timed
 *           work matches mfb's `List OF List OF String` + Python's list-of-lists.
 *
 * Regex needs no dependency: POSIX <regex.h> (regcomp/regexec) is in libc on
 * both macOS and Linux, so `[0-9]+` find-all uses it directly.
 *
 * As in the mfb/Python versions, the input is generated to a temp file first
 * and the file read is done OUTSIDE the timed region — only the parse/traverse
 * is timed. Checksums (csv=6003000, json="5000", regex=200) match across all
 * three languages. */
#include <regex.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "csv.h"
#include "parsebench.h"
#include "parson.h"

/* ----- shared helpers -------------------------------------------------- */

static void parse_path(char *out, size_t n, const char *name) {
  const char *tmp = getenv("TMPDIR");
  if (!tmp || !*tmp) tmp = "/tmp";
  snprintf(out, n, "%s/%s", tmp, name);
}

/* Read the whole file into a fresh NUL-terminated buffer (untimed setup). */
static char *read_all(const char *path, size_t *len_out) {
  FILE *f = fopen(path, "rb");
  if (!f) return NULL;
  fseek(f, 0, SEEK_END);
  long sz = ftell(f);
  fseek(f, 0, SEEK_SET);
  char *buf = malloc((size_t)sz + 1);
  size_t got = fread(buf, 1, (size_t)sz, f);
  fclose(f);
  buf[got] = '\0';
  if (len_out) *len_out = got;
  return buf;
}

/* ----- csv: 2000 rows of "i,i+1,i+2", parse to grid, sum every cell ---- */

/* A materialized grid mirroring `List OF List OF String`: a growable list of
 * rows, each a growable list of NUL-terminated field copies. */
typedef struct {
  char **cells;
  size_t n, cap;
} CsvRow;
typedef struct {
  CsvRow *rows;
  size_t n, cap;
} CsvGrid;

static void csv_field_cb(void *s, size_t len, void *data) {
  CsvGrid *g = data;
  CsvRow *row = &g->rows[g->n]; /* current (in-progress) row */
  if (row->n == row->cap) {
    row->cap = row->cap ? row->cap * 2 : 4;
    row->cells = realloc(row->cells, row->cap * sizeof(char *));
  }
  char *copy = malloc(len + 1);
  memcpy(copy, s, len);
  copy[len] = '\0';
  row->cells[row->n++] = copy;
}

static void csv_row_cb(int c, void *data) {
  (void)c;
  CsvGrid *g = data;
  g->n++; /* finalize the current row, open the next */
  if (g->n == g->cap) {
    g->cap = g->cap ? g->cap * 2 : 16;
    g->rows = realloc(g->rows, g->cap * sizeof(CsvRow));
  }
  g->rows[g->n].cells = NULL;
  g->rows[g->n].n = 0;
  g->rows[g->n].cap = 0;
}

static void test_parse_csv(void) {
  char path[512];
  parse_path(path, sizeof path, "c-bench-parse-csv.csv");
  FILE *f = fopen(path, "w");
  for (int i = 0; i < 2000; i++) fprintf(f, "%d,%d,%d\n", i, i + 1, i + 2);
  fclose(f);

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    size_t len = 0;
    char *text = read_all(path, &len);

    long long t0 = now_ns();
    CsvGrid g = {0};
    g.cap = 16;
    g.rows = malloc(g.cap * sizeof(CsvRow));
    g.rows[0].cells = NULL;
    g.rows[0].n = 0;
    g.rows[0].cap = 0;

    struct csv_parser p;
    csv_init(&p, 0);
    csv_parse(&p, text, len, csv_field_cb, csv_row_cb, &g);
    csv_fini(&p, csv_field_cb, csv_row_cb, &g);
    csv_free(&p);

    long sumv = 0;
    for (size_t ri = 0; ri < g.n; ri++)
      for (size_t ci = 0; ci < g.rows[ri].n; ci++) sumv += atol(g.rows[ri].cells[ci]);
    checksum = sumv;
    t[r] = now_ns() - t0;

    for (size_t ri = 0; ri <= g.n; ri++) {
      for (size_t ci = 0; ci < g.rows[ri].n; ci++) free(g.rows[ri].cells[ci]);
      free(g.rows[ri].cells);
    }
    free(g.rows);
    free(text);
  }
  remove(path);
  fprintf(stderr, "parse_csv = %ld\n", checksum);
  record("parse", "csv", t, RUN);
  free(t);
}

/* ----- json: {"nums":[0..4999],"tail":5000}, parse + get tail + serialize */

static void test_parse_json(void) {
  char path[512];
  parse_path(path, sizeof path, "c-bench-parse-json.json");
  FILE *f = fopen(path, "w");
  fputs("{\"nums\":[", f);
  for (int i = 0; i < 5000; i++) {
    if (i > 0) fputc(',', f);
    fprintf(f, "%d", i);
  }
  fputs("],\"tail\":5000}", f);
  fclose(f);

  long long *t = alloc_times();
  char checksum[64] = "";
  for (int r = 0; r < RUN; r++) {
    char *text = read_all(path, NULL);

    long long t0 = now_ns();
    JSON_Value *root = json_parse_string(text);
    JSON_Value *tail = json_object_get_value(json_value_get_object(root), "tail");
    char *tail_str = json_serialize_to_string(tail);
    long long t1 = now_ns();

    snprintf(checksum, sizeof checksum, "%s", tail_str ? tail_str : "");
    t[r] = t1 - t0;
    json_free_serialized_string(tail_str);
    json_value_free(root);
    free(text);
  }
  remove(path);
  fprintf(stderr, "parse_json = %s\n", checksum);
  record("parse", "json", t, RUN);
  free(t);
}

/* ----- regex: "0 1 2 ... 199", find every [0-9]+ run, count them --------- */

static void test_parse_regex(void) {
  char path[512];
  parse_path(path, sizeof path, "c-bench-parse-regex.txt");
  FILE *f = fopen(path, "w");
  for (int i = 0; i < 200; i++) {
    if (i > 0) fputc(' ', f);
    fprintf(f, "%d", i);
  }
  fclose(f);

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    char *text = read_all(path, NULL);

    long long t0 = now_ns();
    /* Compile inside the timed region to match mfb's regex::findAll, which
     * compiles the pattern on every call (Python pre-compiles; the pattern is
     * trivial so this is negligible either way). */
    regex_t re;
    regcomp(&re, "[0-9]+", REG_EXTENDED);
    long count = 0;
    const char *cursor = text;
    regmatch_t m;
    while (regexec(&re, cursor, 1, &m, 0) == 0) {
      count++;
      cursor += m.rm_eo > 0 ? m.rm_eo : 1; /* advance past this match */
    }
    regfree(&re);
    checksum = count;
    t[r] = now_ns() - t0;
    free(text);
  }
  remove(path);
  fprintf(stderr, "parse_regex = %ld\n", checksum);
  record("parse", "regex", t, RUN);
  free(t);
}

void run_parse_group(void) {
  test_parse_csv();
  test_parse_json();
  test_parse_regex();
}
