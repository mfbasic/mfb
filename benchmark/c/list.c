/* GROUP: list (realloc-growable native arrays)
 *
 * Moved out of main.c into its own translation unit; shared timing/recording
 * infra comes from bench.h. See main.c for the suite overview. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "bench.h"
#include "list.h"

static void test_list_append(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int *nums = NULL; int len = 0, cap = 0;
    for (int i = 0; i < 1000; i++) {
      if (len == cap) { cap = cap ? cap * 2 : 1; nums = realloc(nums, cap * sizeof(int)); }
      nums[len++] = i;
    }
    checksum = len;
    t[r] = now_ns() - t0;
    free(nums);
  }
  fprintf(stderr, "list_append = %ld\n", checksum);
  record("list", "append", t, RUN);
  free(t);
}

static void test_list_append_batch(void) {
  int ten[10] = {0, 1, 2, 3, 4, 5, 6, 7, 8, 9};
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int *nums = NULL; int len = 0, cap = 0;
    for (int i = 0; i < 100; i++) {
      if (len + 10 > cap) {
        cap = cap ? cap * 2 : 10;
        while (cap < len + 10) cap *= 2;
        nums = realloc(nums, cap * sizeof(int));
      }
      memcpy(nums + len, ten, 10 * sizeof(int));
      len += 10;
    }
    checksum = len;
    t[r] = now_ns() - t0;
    free(nums);
  }
  fprintf(stderr, "list_append_batch = %ld\n", checksum);
  record("list", "append_batch", t, RUN);
  free(t);
}

static void test_list_prepend(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int *nums = NULL; int len = 0, cap = 0;
    for (int i = 0; i < 1000; i++) {
      if (len == cap) { cap = cap ? cap * 2 : 1; nums = realloc(nums, cap * sizeof(int)); }
      memmove(nums + 1, nums, len * sizeof(int));
      nums[0] = i; len++;
    }
    checksum = len;
    t[r] = now_ns() - t0;
    free(nums);
  }
  fprintf(stderr, "list_prepend = %ld\n", checksum);
  record("list", "prepend", t, RUN);
  free(t);
}

typedef struct { int n; char *s; } CopyRec;

static void test_list_copy(void) {
  char buf[16];
  char **strs = malloc(1000 * sizeof(char *));
  CopyRec *recs = malloc(1000 * sizeof(CopyRec));
  for (int i = 0; i < 1000; i++) {
    snprintf(buf, sizeof buf, "%d", i);
    strs[i] = strdup(buf);
    recs[i].n = i; recs[i].s = strdup(buf);
  }
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int i = 0; i < 1000; i++) {
      char **c = malloc(1000 * sizeof(char *));
      memcpy(c, strs, 1000 * sizeof(char *));
      acc += 1000;
      free(c);
    }
    for (int i = 0; i < 1000; i++) {
      CopyRec *c = malloc(1000 * sizeof(CopyRec));
      memcpy(c, recs, 1000 * sizeof(CopyRec));
      acc += 1000;
      free(c);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_copy = %ld\n", checksum);
  record("list", "copy", t, RUN);
  for (int i = 0; i < 1000; i++) { free(strs[i]); free(recs[i].s); }
  free(strs); free(recs); free(t);
}

static void test_list_distinct(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int *nums = malloc(5000 * sizeof(int));
  int *unique = malloc(5000 * sizeof(int));
  for (int r = 0; r < RUN; r++) {
    for (int i = 0; i < 5000; i++) nums[i] = i % 1000;
    long long t0 = now_ns();
    int ulen = 0;
    for (int i = 0; i < 5000; i++) {
      int seen = 0;
      for (int k = 0; k < ulen; k++) { if (unique[k] == nums[i]) { seen = 1; break; } }
      if (!seen) unique[ulen++] = nums[i];
    }
    checksum = ulen;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_distinct = %ld\n", checksum);
  record("list", "distinct", t, RUN);
  free(nums); free(unique); free(t);
}

#define GB_KEYS 100
static void test_list_groupby(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int *bucket[GB_KEYS]; int len[GB_KEYS] = {0}, cap[GB_KEYS] = {0};
    for (int i = 0; i < 2000; i++) {
      int k = i % GB_KEYS;
      if (len[k] == cap[k]) {
        cap[k] = cap[k] ? cap[k] * 2 : 1;
        bucket[k] = realloc(len[k] ? bucket[k] : NULL, cap[k] * sizeof(int));
      }
      bucket[k][len[k]++] = i;
    }
    int groups = 0;
    for (int k = 0; k < GB_KEYS; k++) if (len[k] > 0) groups++;
    checksum = groups;
    for (int k = 0; k < GB_KEYS; k++) if (cap[k]) free(bucket[k]);
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_groupby = %ld\n", checksum);
  record("list", "groupby", t, RUN);
  free(t);
}

static void test_list_set(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int nums[200];
  for (int r = 0; r < RUN; r++) {
    for (int i = 0; i < 200; i++) nums[i] = i;
    long long t0 = now_ns();
    for (int pass = 0; pass < 10; pass++)
      for (int j = 0; j < 200; j++) nums[j] = nums[j] + 1;
    t[r] = now_ns() - t0;
    long sum = 0;
    for (int j = 0; j < 200; j++) sum += nums[j];
    checksum = sum;
  }
  fprintf(stderr, "list_set = %ld\n", checksum);
  record("list", "set", t, RUN);
  free(t);
}

static int cmp_int(const void *a, const void *b) {
  int x = *(const int *)a, y = *(const int *)b;
  return (x > y) - (x < y);
}

static void test_list_sort(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[50], tmp[50];
  for (int r = 0; r < RUN; r++) {
    for (int i = 0; i < 50; i++) base[i] = rand() % 1000001;
    long long t0 = now_ns();
    for (int i = 0; i < 50; i++) tmp[i] = base[i];
    qsort(tmp, 50, sizeof(int), cmp_int);
    t[r] = now_ns() - t0;
    checksum = tmp[0];
  }
  fprintf(stderr, "list_sort = %ld\n", checksum);
  record("list", "sort", t, RUN);
  free(t);
}

/* --- 26 collections:: list benchmarks ------------------------------- */

static void test_list_all(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int pos[1000];
  for (int i = 0; i < 1000; i++) pos[i] = i + 1;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int ok = 1;
      for (int i = 0; i < 1000; i++) { if (!(pos[i] > 0)) { ok = 0; break; } }
      if (ok) acc++;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_all = %ld\n", checksum);
  record("list", "all", t, RUN);
  free(t);
}

static void test_list_any(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int neg[1000];
  for (int i = 0; i < 1000; i++) neg[i] = -(i + 1);
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int found = 0;
      for (int i = 0; i < 1000; i++) { if (neg[i] > 0) { found = 1; break; } }
      if (!found) acc++;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_any = %ld\n", checksum);
  record("list", "any", t, RUN);
  free(t);
}

static void test_list_chunks(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int nchunks = 0;
      for (int i = 0; i < 1000; i += 10) {
        int sz = 1000 - i < 10 ? 1000 - i : 10;
        int *chunk = malloc(sz * sizeof(int));
        memcpy(chunk, base + i, sz * sizeof(int));
        nchunks++;
        free(chunk);
      }
      acc += nchunks;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_chunks = %ld\n", checksum);
  record("list", "chunks", t, RUN);
  free(t);
}

static void test_list_contains(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      int found = 0;
      for (int i = 0; i < 1000; i++) { if (base[i] == 1000) { found = 1; break; } }
      if (!found) acc++;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_contains = %ld\n", checksum);
  record("list", "contains", t, RUN);
  free(t);
}

static void test_list_drop(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      int n = 1000 - 500;
      int *res = malloc(n * sizeof(int));
      memcpy(res, base + 500, n * sizeof(int));
      acc += n;
      free(res);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_drop = %ld\n", checksum);
  record("list", "drop", t, RUN);
  free(t);
}

static void test_list_filter(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int *res = malloc(1000 * sizeof(int));
      int n = 0;
      for (int i = 0; i < 1000; i++) if (base[i] % 2 == 0) res[n++] = base[i];
      acc += n;
      free(res);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_filter = %ld\n", checksum);
  record("list", "filter", t, RUN);
  free(t);
}

static void test_list_find(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      int idx = -1;
      for (int i = 0; i < 1000; i++) { if (base[i] == 999) { idx = i; break; } }
      acc += idx;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_find = %ld\n", checksum);
  record("list", "find", t, RUN);
  free(t);
}

static void test_list_findIndex(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      int idx = -1;
      for (int i = 0; i < 1000; i++) { if (base[i] >= 999) { idx = i; break; } }
      acc += idx;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_findIndex = %ld\n", checksum);
  record("list", "findIndex", t, RUN);
  free(t);
}

static void test_list_findLastIndex(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      int idx = -1;
      for (int i = 999; i >= 0; i--) { if (base[i] <= 5) { idx = i; break; } }
      acc += idx;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_findLastIndex = %ld\n", checksum);
  record("list", "findLastIndex", t, RUN);
  free(t);
}

static void test_list_flatten(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int nested[100][10];
  for (int i = 0; i < 100; i++) for (int j = 0; j < 10; j++) nested[i][j] = j;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int *res = malloc(1000 * sizeof(int));
      int n = 0;
      for (int i = 0; i < 100; i++)
        for (int j = 0; j < 10; j++) res[n++] = nested[i][j];
      acc += n;
      free(res);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_flatten = %ld\n", checksum);
  record("list", "flatten", t, RUN);
  free(t);
}

static long forEachAcc = 0;
static void test_list_forEach(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    forEachAcc = 0;
    for (int k = 0; k < 200; k++)
      for (int i = 0; i < 1000; i++) forEachAcc += base[i];
    checksum = forEachAcc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_forEach = %ld\n", checksum);
  record("list", "forEach", t, RUN);
  free(t);
}

static void test_list_get(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int pass = 0; pass < 100; pass++)
      for (int i = 0; i < 1000; i++) acc += base[i];
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_get = %ld\n", checksum);
  record("list", "get", t, RUN);
  free(t);
}

static void test_list_getOr(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int pass = 0; pass < 100; pass++)
      for (int i = 0; i < 1000; i++) acc += (i >= 0 && i < 1000) ? base[i] : 0;
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_getOr = %ld\n", checksum);
  record("list", "getOr", t, RUN);
  free(t);
}

static void test_list_insert(void) {
  long long *t = alloc_times();
  long checksum = 0;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    int *nums = NULL; int len = 0, cap = 0;
    for (int i = 0; i < 1000; i++) {
      if (len == cap) { cap = cap ? cap * 2 : 1; nums = realloc(nums, cap * sizeof(int)); }
      int p = len / 2;
      memmove(nums + p + 1, nums + p, (len - p) * sizeof(int));
      nums[p] = i; len++;
    }
    checksum = len;
    t[r] = now_ns() - t0;
    free(nums);
  }
  fprintf(stderr, "list_insert = %ld\n", checksum);
  record("list", "insert", t, RUN);
  free(t);
}

static void test_list_mid(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      int n = 500;
      int *res = malloc(n * sizeof(int));
      memcpy(res, base + 250, n * sizeof(int));
      acc += n;
      free(res);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_mid = %ld\n", checksum);
  record("list", "mid", t, RUN);
  free(t);
}

static void test_list_partition(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int *matched = malloc(1000 * sizeof(int));
      int *unmatched = malloc(1000 * sizeof(int));
      int mn = 0, un = 0;
      for (int i = 0; i < 1000; i++) {
        if (base[i] % 2 == 0) matched[mn++] = base[i];
        else unmatched[un++] = base[i];
      }
      acc += mn;
      free(matched); free(unmatched);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_partition = %ld\n", checksum);
  record("list", "partition", t, RUN);
  free(t);
}

static void test_list_reduce(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      long fold = 0;
      for (int i = 0; i < 1000; i++) fold += base[i];
      acc += fold;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_reduce = %ld\n", checksum);
  record("list", "reduce", t, RUN);
  free(t);
}

static void test_list_reduceRight(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      long fold = 0;
      for (int i = 999; i >= 0; i--) fold += base[i];
      acc += fold;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_reduceRight = %ld\n", checksum);
  record("list", "reduceRight", t, RUN);
  free(t);
}

static void test_list_removeAt(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  int *work = malloc(1000 * sizeof(int));
  for (int r = 0; r < RUN; r++) {
    memcpy(work, base, 1000 * sizeof(int));
    int len = 1000;
    long long t0 = now_ns();
    long count = 0;
    while (len > 0) {
      memmove(work, work + 1, (len - 1) * sizeof(int));
      len--;
      count++;
    }
    checksum = count;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_removeAt = %ld\n", checksum);
  record("list", "removeAt", t, RUN);
  free(work); free(t);
}

static void test_list_replace(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int *res = malloc(1000 * sizeof(int));
      for (int i = 0; i < 1000; i++) res[i] = (base[i] == 500) ? 500 : base[i];
      acc += 1000;
      free(res);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_replace = %ld\n", checksum);
  record("list", "replace", t, RUN);
  free(t);
}

static int cmp_negdesc(const void *a, const void *b) {
  int x = *(const int *)a, y = *(const int *)b;
  /* sort ascending by key = -n  ==>  descending by value */
  int kx = -x, ky = -y;
  return (kx > ky) - (kx < ky);
}

static void test_list_sortBy(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base2[500];
  for (int i = 0; i < 500; i++) base2[i] = i;
  int *tmp = malloc(500 * sizeof(int));
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      memcpy(tmp, base2, 500 * sizeof(int));
      qsort(tmp, 500, sizeof(int), cmp_negdesc);
      acc += tmp[0];
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_sortBy = %ld\n", checksum);
  record("list", "sortBy", t, RUN);
  free(tmp); free(t);
}

static void test_list_sum(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 1000; k++) {
      long s = 0;
      for (int i = 0; i < 1000; i++) s += base[i];
      acc += s;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_sum = %ld\n", checksum);
  record("list", "sum", t, RUN);
  free(t);
}

static void test_list_take(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 500; k++) {
      int n = 500;
      int *res = malloc(n * sizeof(int));
      memcpy(res, base, n * sizeof(int));
      acc += n;
      free(res);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_take = %ld\n", checksum);
  record("list", "take", t, RUN);
  free(t);
}

static void test_list_transform(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 200; k++) {
      int *res = malloc(1000 * sizeof(int));
      for (int i = 0; i < 1000; i++) res[i] = base[i] * 2;
      acc += 1000;
      free(res);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_transform = %ld\n", checksum);
  record("list", "transform", t, RUN);
  free(t);
}

static void test_list_window(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 100; k++) {
      int nwin = 0;
      for (int i = 0; i + 10 <= 1000; i++) {
        int *win = malloc(10 * sizeof(int));
        memcpy(win, base + i, 10 * sizeof(int));
        nwin++;
        free(win);
      }
      acc += nwin;
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_window = %ld\n", checksum);
  record("list", "window", t, RUN);
  free(t);
}

static void test_list_zip(void) {
  long long *t = alloc_times();
  long checksum = 0;
  int base[1000];
  for (int i = 0; i < 1000; i++) base[i] = i;
  for (int r = 0; r < RUN; r++) {
    long long t0 = now_ns();
    long acc = 0;
    for (int k = 0; k < 100; k++) {
      int (*pairs)[2] = malloc(1000 * sizeof(*pairs));
      int n = 0;
      for (int i = 0; i < 1000; i++) { pairs[n][0] = base[i]; pairs[n][1] = base[i]; n++; }
      acc += n;
      free(pairs);
    }
    checksum = acc;
    t[r] = now_ns() - t0;
  }
  fprintf(stderr, "list_zip = %ld\n", checksum);
  record("list", "zip", t, RUN);
  free(t);
}

void run_list_group(void) {
  test_list_append();
  test_list_append_batch();
  test_list_prepend();
  test_list_copy();
  test_list_distinct();
  test_list_groupby();
  test_list_set();
  test_list_sort();

  test_list_all();
  test_list_any();
  test_list_chunks();
  test_list_contains();
  test_list_drop();
  test_list_filter();
  test_list_find();
  test_list_findIndex();
  test_list_findLastIndex();
  test_list_flatten();
  test_list_forEach();
  test_list_get();
  test_list_getOr();
  test_list_insert();
  test_list_mid();
  test_list_partition();
  test_list_reduce();
  test_list_reduceRight();
  test_list_removeAt();
  test_list_replace();
  test_list_sortBy();
  test_list_sum();
  test_list_take();
  test_list_transform();
  test_list_window();
  test_list_zip();
}
