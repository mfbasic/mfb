#ifndef BENCH_H
#define BENCH_H
extern int RUN;
long long now_ns(void);
long long *alloc_times(void);
void record(const char *group, const char *name, long long *times, int n);

/* Opaque use of a materialized result: the empty asm claims to read p and all
 * of memory, so the compiler must actually build (and keep) the data even when
 * the checksum only folds a count and the block is freed right after. */
static inline void bench_opaque(const void *p) {
  __asm__ volatile("" : : "g"(p) : "memory");
}
#endif
