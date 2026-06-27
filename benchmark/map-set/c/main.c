/* Builds a 1000-item string->int hash map, then looks each key up to verify. */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CAP 4096 /* power of two; keeps the load factor under 0.25 for 1000 keys */

typedef struct {
  char *key;
  long val;
  int used;
} Slot;

static Slot table[CAP];

static unsigned long hash(const char *s) {
  unsigned long h = 5381;
  int c;
  while ((c = *s++)) {
    h = ((h << 5) + h) + c;
  }
  return h;
}

static void put(const char *key, long val) {
  unsigned long i = hash(key) & (CAP - 1);
  while (table[i].used) {
    if (strcmp(table[i].key, key) == 0) {
      table[i].val = val;
      return;
    }
    i = (i + 1) & (CAP - 1);
  }
  table[i].key = strdup(key);
  table[i].val = val;
  table[i].used = 1;
}

static long get(const char *key) {
  unsigned long i = hash(key) & (CAP - 1);
  while (table[i].used) {
    if (strcmp(table[i].key, key) == 0) {
      return table[i].val;
    }
    i = (i + 1) & (CAP - 1);
  }
  return -1;
}

int main(void) {
  char buf[16];

  int count = 0;
  for (int i = 0; i < 1000; i++) {
    snprintf(buf, sizeof buf, "%d", i);
    put(buf, i);
    count = count + 1;
  }

  long sum = 0;
  for (int i = 0; i < 1000; i++) {
    snprintf(buf, sizeof buf, "%d", i);
    sum = sum + get(buf);
  }

  printf("count: %d sum: %ld\n", count, sum);
  return 0;
}
