// bug-564 sighting-2 instrument, BOTH sides slowed. NEVER committed.
// B564_DELAY_EDOM_US  : usleep inside nw_error_get_error_domain (handler side:
//                       between STATE_INVOKE's CTX_STATE store and CTX_EDOM store)
// B564_DELAY_WRITE_US : usleep inside dispatch_data_create (writer side: before
//                       tls::write's CTX_STATE>=4 guard), only once the program
//                       has written B564_WRITE_SKIP payloads (default 1).
// B564_LOG=1          : log domain queries (with caller address) and slowed writes.
#include <dlfcn.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <pthread.h>
#include <stdatomic.h>
#include <mach-o/dyld.h>
#include <mach/mach_time.h>

typedef void *nw_error_t;
static int (*real_domain)(nw_error_t);
static int (*real_code)(nw_error_t);
static void *(*real_ddc)(const void *, size_t, void *, void *);
static useconds_t edom_us, write_us;
static int do_log;
static long write_skip = 1;
static atomic_long writes;

static double now_ms(void) {
  static mach_timebase_info_data_t tb;
  if (tb.denom == 0) mach_timebase_info(&tb);
  return (double)mach_absolute_time() * tb.numer / tb.denom / 1e6;
}
static void init_env(void) {
  const char *s;
  if ((s = getenv("B564_DELAY_EDOM_US"))) edom_us = (useconds_t)atoi(s);
  if ((s = getenv("B564_DELAY_WRITE_US"))) write_us = (useconds_t)atoi(s);
  if ((s = getenv("B564_WRITE_SKIP"))) write_skip = atol(s);
  do_log = getenv("B564_LOG") != NULL;
}

static int wrap_domain(nw_error_t e) {
  int d = real_domain(e);
  if (do_log) {
    int c = real_code ? real_code(e) : -1;
    char buf[200];
    unsigned long ra = (unsigned long)__builtin_return_address(0) -
                       (unsigned long)_dyld_get_image_vmaddr_slide(0);
    int n = snprintf(buf, sizeof buf, "[b564 %.3f thr=%p ra=0x%lx] domain=%d code=%d\n",
                     now_ms(), (void *)pthread_self(), ra, d, c);
    write(2, buf, n);
  }
  if (edom_us) usleep(edom_us);
  return d;
}

static void *wrap_ddc(const void *buf, size_t len, void *q, void *destructor) {
  long n = atomic_fetch_add(&writes, 1) + 1;
  // only slow the big payloads (the 64 KiB loop), and only after the first
  if (write_us && len >= 65536 && n > write_skip) {
    if (do_log) {
      char b[120];
      int k = snprintf(b, sizeof b, "[b564 %.3f] slow write #%ld\n", now_ms(), n);
      write(2, b, k);
    }
    usleep(write_us);
  }
  return real_ddc(buf, len, q, destructor);
}

static void *my_dlsym(void *h, const char *name) {
  void *p = dlsym(h, name);
  if (!name || !p) return p;
  if (strcmp(name, "nw_error_get_error_domain") == 0) {
    if (!real_domain) {
      real_domain = p;
      real_code = dlsym(RTLD_DEFAULT, "nw_error_get_error_code");
      init_env();
    }
    return (void *)wrap_domain;
  }
  if (strcmp(name, "dispatch_data_create") == 0) {
    if (!real_ddc) { real_ddc = p; init_env(); }
    return (void *)wrap_ddc;
  }
  return p;
}

typedef struct { const void *replacement; const void *replacee; } interpose_t;
__attribute__((used)) static const interpose_t interposers[]
    __attribute__((section("__DATA,__interpose"))) = {
        {(const void *)my_dlsym, (const void *)dlsym},
};
