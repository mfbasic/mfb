// bug-564 OPEN-finding probe: log what Network.framework hands the state,
// receive and send completion blocks. Never committed.
#include <dlfcn.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>
#include <stdbool.h>
#include <Block.h>
#include <dispatch/dispatch.h>

typedef void *nw_t;
typedef void (^state_h)(int, nw_t);
typedef void (^recv_h)(dispatch_data_t, nw_t, bool, nw_t);
typedef void (^send_h)(nw_t);
static int (*edom)(nw_t); static int (*ecode)(nw_t); static bool (*isfinal)(nw_t);
static void (*r_set_state)(nw_t, state_h);
static void (*r_recv)(nw_t, unsigned, unsigned, recv_h);
static void (*r_send)(nw_t, dispatch_data_t, nw_t, bool, send_h);
static int nsend;
#define LOG(...) do { char b[256]; int n = snprintf(b, sizeof b, __VA_ARGS__); write(2, b, n); } while (0)
static void init(void) {
  edom = dlsym(RTLD_DEFAULT, "nw_error_get_error_domain");
  ecode = dlsym(RTLD_DEFAULT, "nw_error_get_error_code");
  isfinal = dlsym(RTLD_DEFAULT, "nw_content_context_get_is_final");
}
static void w_set_state(nw_t c, state_h h) {
  state_h o = Block_copy(h);
  r_set_state(c, ^(int st, nw_t e) {
    LOG("[probe] state=%d err=%s dom=%d code=%d\n", st, e ? "Y" : "N", e ? edom(e) : 0, e ? ecode(e) : 0);
    o(st, e);
  });
}
static void w_recv(nw_t c, unsigned mn, unsigned mx, recv_h h) {
  recv_h o = Block_copy(h);
  r_recv(c, mn, mx, ^(dispatch_data_t d, nw_t ctx, bool done, nw_t e) {
    LOG("[probe] recv size=%zu complete=%d final=%d err=%s dom=%d code=%d\n",
        d ? dispatch_data_get_size(d) : 0, done, ctx ? isfinal(ctx) : -1,
        e ? "Y" : "N", e ? edom(e) : 0, e ? ecode(e) : 0);
    o(d, ctx, done, e);
  });
}
static void w_send(nw_t c, dispatch_data_t d, nw_t ctx, bool done, send_h h) {
  send_h o = Block_copy(h);
  int k = ++nsend;
  r_send(c, d, ctx, done, ^(nw_t e) {
    if (k <= 3 || e || k % 500 == 0)
      LOG("[probe] send#%d err=%s dom=%d code=%d\n", k, e ? "Y" : "N", e ? edom(e) : 0, e ? ecode(e) : 0);
    o(e);
  });
}
static void *my_dlsym(void *hd, const char *name) {
  void *p = dlsym(hd, name);
  if (!name || !p) return p;
  if (!edom) init();
  if (!strcmp(name, "nw_connection_set_state_changed_handler")) { r_set_state = p; return (void *)w_set_state; }
  if (!strcmp(name, "nw_connection_receive")) { r_recv = p; return (void *)w_recv; }
  if (!strcmp(name, "nw_connection_send")) { r_send = p; return (void *)w_send; }
  return p;
}
typedef struct { const void *a; const void *b; } interpose_t;
__attribute__((used)) static const interpose_t I[] __attribute__((section("__DATA,__interpose"))) = {
  {(const void *)my_dlsym, (const void *)dlsym}};
