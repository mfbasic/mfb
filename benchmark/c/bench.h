#ifndef BENCH_H
#define BENCH_H
extern int RUN;
long long now_ns(void);
long long *alloc_times(void);
void record(const char *group, const char *name, long long *times, int n);
#endif
