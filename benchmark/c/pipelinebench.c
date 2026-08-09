/* GROUP: pipeline — the C oracle for pipeline.mfb (plan-87 Theme 2, chained
 * collection HOF pipelines). Each row materializes the same intermediate lists
 * mfb's filter/transform/reduce/groupBy/mapValues/values chain does, so the
 * cross-language checksums match.
 *
 * Expected checksums: int=74990000, groupagg=49995000, str=412. */
#include <ctype.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "pipelinebench.h"

#define PIPE_N 10000

/* ----- pipeline int: filter (even) -> transform (x3+1) -> reduce (sum) ---- */

static void test_pipeline_int(void) {
  int *data = malloc(PIPE_N * sizeof(int));
  for (int i = 0; i < PIPE_N; i++) data[i] = i;

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long total = 0;
    for (int rep = 0; rep < 200; rep++) {
      /* filter: materialize the even elements */
      int *evens = malloc(PIPE_N * sizeof(int));
      int ne = 0;
      for (int i = 0; i < PIPE_N; i++)
        if (data[i] % 2 == 0) evens[ne++] = data[i];
      /* transform: materialize x*3+1 */
      long *tripled = malloc((size_t)ne * sizeof(long));
      for (int k = 0; k < ne; k++) tripled[k] = (long)evens[k] * 3 + 1;
      /* reduce: fold to the sum */
      long s = 0;
      for (int k = 0; k < ne; k++) s += tripled[k];
      total = s;
      free(evens);
      free(tripled);
    }
    checksum = total;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "pipeline_int = %ld\n", checksum);
  record("pipeline", "int", t, RUN);
  free(t);
  free(data);
}

/* ----- pipeline groupagg: groupBy -> mapValues(reduce) -> values -> reduce  */

#define PIPE_K 7

static void test_pipeline_groupagg(void) {
  int *data = malloc(PIPE_N * sizeof(int));
  for (int i = 0; i < PIPE_N; i++) data[i] = i;

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long total = 0;
    for (int rep = 0; rep < 100; rep++) {
      /* groupBy n MOD K: materialize K buckets as growable lists */
      int cap = PIPE_N / PIPE_K + 1;
      int *bucket[PIPE_K];
      int blen[PIPE_K];
      for (int k = 0; k < PIPE_K; k++) { bucket[k] = malloc((size_t)cap * sizeof(int)); blen[k] = 0; }
      for (int i = 0; i < PIPE_N; i++) {
        int k = i % PIPE_K;
        bucket[k][blen[k]++] = i;
      }
      /* mapValues(reduce sum) then values -> reduce sum of bucket sums */
      long bucketSums[PIPE_K];
      for (int k = 0; k < PIPE_K; k++) {
        long s = 0;
        for (int j = 0; j < blen[k]; j++) s += bucket[k][j];
        bucketSums[k] = s;
      }
      long s = 0;
      for (int k = 0; k < PIPE_K; k++) s += bucketSums[k];
      total = s;
      for (int k = 0; k < PIPE_K; k++) free(bucket[k]);
    }
    checksum = total;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "pipeline_groupagg = %ld\n", checksum);
  record("pipeline", "groupagg", t, RUN);
  free(t);
  free(data);
}

/* ----- pipeline str: filter (non-empty) -> transform (upper) -> reduce len  */

#define PIPE_STR_N 50

static void test_pipeline_str(void) {
  char data[PIPE_STR_N][32];
  for (int i = 0; i < PIPE_STR_N; i++) {
    if (i % 7 == 0)
      data[i][0] = '\0';
    else
      snprintf(data[i], sizeof data[i], "row%dValue", i);
  }

  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long total = 0;
    for (int rep = 0; rep < 20; rep++) {
      /* filter: keep the non-empty strings */
      const char *kept[PIPE_STR_N];
      int nk = 0;
      for (int i = 0; i < PIPE_STR_N; i++)
        if (data[i][0] != '\0') kept[nk++] = data[i];
      /* transform: materialize the uppercased copies */
      char *upped[PIPE_STR_N];
      for (int k = 0; k < nk; k++) {
        size_t len = strlen(kept[k]);
        upped[k] = malloc(len + 1);
        for (size_t c = 0; c < len; c++) upped[k][c] = (char)toupper((unsigned char)kept[k][c]);
        upped[k][len] = '\0';
      }
      /* reduce: fold the concatenated length */
      long acc = 0;
      for (int k = 0; k < nk; k++) acc += (long)strlen(upped[k]);
      total = acc;
      for (int k = 0; k < nk; k++) free(upped[k]);
    }
    checksum = total;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "pipeline_str = %ld\n", checksum);
  record("pipeline", "str", t, RUN);
  free(t);
}

void run_pipeline_group(void) {
  test_pipeline_int();
  test_pipeline_groupagg();
  test_pipeline_str();
}
